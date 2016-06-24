use std::io::{Write, BufRead};
use time;
use std;

use super::*;
pub use super::auth::*;
use super::msg;

#[derive(Debug)]
pub struct Config<Auth> {
    pub server_id: String,
    pub methods: auth::Methods,
    pub auth_banner: Option<&'static str>,
    pub keys: Vec<key::Algorithm>,
    pub auth: Auth,
    pub rekey_write_limit: usize,
    pub rekey_read_limit: usize,
    pub rekey_time_limit_s: f64
}

impl<A> Config<A> {
    pub fn default(a:A, keys:Vec<key::Algorithm>) -> Config<A> {
        Config {
            // Must begin with "SSH-2.0-".
            server_id: "SSH-2.0-SSH.rs_0.1".to_string(),
            methods: auth::Methods::all(),
            auth_banner: Some("SSH Authentication\r\n"), // CRLF separated lines.
            keys: keys.to_vec(),
            auth: a,
            // Following the recommendations of https://tools.ietf.org/html/rfc4253#section-9
            rekey_write_limit: 1<<30, // 1 Gb
            rekey_read_limit: 1<<30, // 1Gb
            rekey_time_limit_s: 3600.0
        }
    }
}

pub struct ServerSession {
    buffers: super::SSHBuffers,
    state: Option<ServerState>,
}


const SSH_EXTENDED_DATA_STDERR: u32 = 1;

impl<'a> ChannelBuf<'a> {
    pub fn stdout(&mut self, stdout:&[u8]) -> Result<(), Error> {
        self.buffer.clear();
        self.buffer.push(msg::CHANNEL_DATA);
        self.buffer.push_u32_be(self.recipient_channel);
        self.buffer.extend_ssh_string(stdout);

        self.cipher.write_server_packet(*self.sent_seqn,
                                        self.buffer.as_slice(),
                                        self.write_buffer);
                        
        *self.sent_seqn += 1;
        Ok(())
    }
    pub fn stderr(&mut self, stderr:&[u8]) -> Result<(), Error> {
        self.buffer.clear();
        self.buffer.push(msg::CHANNEL_EXTENDED_DATA);
        self.buffer.push_u32_be(self.recipient_channel);
        self.buffer.push_u32_be(SSH_EXTENDED_DATA_STDERR);
        self.buffer.extend_ssh_string(stderr);
        self.cipher.write_server_packet(*self.sent_seqn,
                                        self.buffer.as_slice(),
                                        self.write_buffer);
                        
        *self.sent_seqn += 1;
        Ok(())
    }
}


pub fn hexdump(x: &CryptoBuf) {
    let x = x.as_slice();
    let mut buf = Vec::new();
    let mut i = 0;
    while i < x.len() {
        if i % 16 == 0 {
            print!("{:04}: ", i)
        }
        print!("{:02x} ", x[i]);
        if x[i] >= 0x20 && x[i] <= 0x7e {
            buf.push(x[i]);
        } else {
            buf.push(b'.');
        }
        if i % 16 == 15 || i == x.len() - 1 {
            while i % 16 != 15 {
                print!("   ");
                i += 1
            }
            println!(" {}", std::str::from_utf8(&buf).unwrap());
            buf.clear();
        }
        i += 1
    }
}



mod read;
mod write;

impl ServerSession {
    pub fn new() -> Self {
        super::SODIUM_INIT.call_once(|| {
            super::sodium::init();
        });
        ServerSession {
            buffers: super::SSHBuffers::new(),
            state: None,
        }
    }

    // returns whether a complete packet has been read.
    pub fn read<R: BufRead, A: auth::Authenticate,S:SSHHandler>(
        &mut self,
        server:&mut S,
        config: &Config<A>,
        stream: &mut R,
        buffer: &mut CryptoBuf,
        buffer2: &mut CryptoBuf)
        -> Result<bool, Error> {

        
        let state = std::mem::replace(&mut self.state, None);
        // println!("state: {:?}", state);
        match state {
            None => {
                let mut exchange;
                {
                    let client_id = try!(self.buffers.read_ssh_id(stream));
                    if let Some(client_id) = client_id {
                        exchange = Exchange::new();
                        exchange.client_id.extend(client_id);
                        debug!("client id, exchange = {:?}", exchange);
                    } else {
                        return Ok(false)
                    }
                }
                // Preparing the response
                self.buffers.send_ssh_id(config.server_id.as_bytes());
                exchange.server_id.extend(config.server_id.as_bytes());

                self.state = Some(ServerState::Kex(Kex::KexInit(KexInit {
                    exchange: exchange,
                    algo: None,
                    sent: false,
                    session_id: None,
                })));
                Ok(true)
            },

            Some(ServerState::Kex(Kex::KexInit(mut kexinit))) => {
                let result = try!(self.server_read_cleartext_kexinit(stream, &mut kexinit, &config.keys));
                if result {
                    self.state = Some(self.buffers.cleartext_write_kex_init(&config.keys,
                                                                            true, // is_server
                                                                            kexinit));
                } else {
                    self.state = Some(ServerState::Kex(Kex::KexInit(kexinit)))
                }
                Ok(result)
            },

            Some(ServerState::Kex(Kex::KexDh(kexdh))) => self.server_read_cleartext_kexdh(stream, kexdh, buffer, buffer2),

            Some(ServerState::Kex(Kex::NewKeys(newkeys))) => self.server_read_cleartext_newkeys(stream, newkeys),

            Some(ServerState::Encrypted(mut enc)) => {
                debug!("read: encrypted {:?} {:?}", enc.state, enc.rekey);
                let (buf_is_some, rekeying_done) =
                    if let Some(buf) = try!(enc.cipher.read_client_packet(stream, &mut self.buffers.read)) {

                        let rek = try!(enc.server_read_rekey(buf, &config.keys, buffer, buffer2, &mut self.buffers.write));
                        if rek && enc.rekey.is_none() && buf[0] == msg::NEWKEYS {
                            // rekeying is finished.
                            (true, true)
                        } else {
                            debug!("calling read_encrypted");
                            enc.server_read_encrypted(config, server, buf, buffer, &mut self.buffers.write);
                            (true, false)
                        }
                    } else {
                        (false, false)
                    };

                if buf_is_some {
                    if rekeying_done {
                        self.buffers.read.bytes = 0;
                        self.buffers.write.bytes = 0;
                        self.buffers.last_rekey_s = time::precise_time_s();
                    }
                    if enc.rekey.is_none() &&
                        (self.buffers.read.bytes >= config.rekey_read_limit
                         || self.buffers.write.bytes >= config.rekey_write_limit
                         || time::precise_time_s() >= self.buffers.last_rekey_s + config.rekey_time_limit_s) {

                            let mut kexinit = KexInit {
                                exchange: std::mem::replace(&mut enc.exchange, None).unwrap(),
                                algo: None,
                                sent: true,
                                session_id: Some(enc.session_id.clone()),
                            };
                            kexinit.exchange.client_kex_init.clear();
                            kexinit.exchange.server_kex_init.clear();
                            kexinit.exchange.client_ephemeral.clear();
                            kexinit.exchange.server_ephemeral.clear();

                            debug!("sending kexinit");
                            enc.write_kexinit(&config.keys, &mut kexinit, buffer, &mut self.buffers.write);
                            enc.rekey = Some(Kex::KexInit(kexinit))
                        }
                    self.buffers.read.seqn += 1;
                    self.buffers.read.buffer.clear();
                    self.buffers.read.len = 0;
                }

                self.state = Some(ServerState::Encrypted(enc));
                Ok(buf_is_some)
            }
            _ => {
                debug!("read: unhandled");
                Err(Error::Inconsistent)
            }
        }
    }

    // Returns whether the connexion is still alive.

    pub fn write<W: Write, A: auth::Authenticate>(
        &mut self,
        config: &Config<A>,
        stream: &mut W)
        -> Result<bool, Error> {

        // Finish pending writes, if any.
        if !try!(self.buffers.write_all(stream)) {
            // If there are still bytes to write.
            return Ok(true);
        }

        match self.state {
            Some(ServerState::Encrypted(ref mut enc)) => {
                debug!("write: encrypted {:?} {:?}", enc.state, enc.rekey);
                if enc.rekey.is_none() {
                    let state = std::mem::replace(&mut enc.state, None);
                    match state {
                        Some(EncryptedState::ChannelOpenConfirmation(channel)) => {
                            // The write buffer was already filled and written above.
                            let sender_channel = channel.sender_channel;
                            enc.channels.insert(sender_channel, channel);
                            enc.state = Some(EncryptedState::ChannelOpened(sender_channel));
                        }
                        state => enc.state = state,
                    }
                }
                Ok(true)
            }
            _ => {
                Ok(true)
            }
        }
    }
}
