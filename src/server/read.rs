use super::*;
use super::super::key;
use super::super::negociation;
use super::super::msg;
use super::super::{Exchange, Error, read, CryptoBuf};

use rand::{thread_rng, Rng};
use std;
use std::io::BufRead;
use byteorder::{ByteOrder, BigEndian, ReadBytesExt};


impl<T, S: Serve<T>> ServerSession<T, S> {

    pub fn read_client_id<R: BufRead>(&mut self, stream: &mut R) -> Result<bool, Error> {
        let (len, result) = {
            let buf = try!(stream.fill_buf());
            let mut i = 0;
            while i < buf.len() - 1 {
                if &buf[i..i + 2] == b"\r\n" {
                    break;
                }
                i += 1
            }
            if buf.len() <= 8 || i >= buf.len() - 1 {
                // Not enough bytes. Don't consume, wait until we have more bytes. The buffer is larger than 255 anyway.
                return Ok(false);
            }
            (buf.len(),
             if &buf[0..8] == b"SSH-2.0-" {
                 let mut exchange = Exchange::new();
                 exchange.client_id.extend(&buf[0..i]);
                 // println!("{:?}", std::str::from_utf8(&buf[ 0 .. i ]));
                 self.state = Some(ServerState::VersionOk(exchange));
                 Ok(true)
             } else {
                 Err(Error::Version)
             })
        };
        stream.consume(len);
        result
    }
    pub fn read_cleartext_kexinit<R: BufRead>(&mut self,
                                          stream: &mut R,
                                          mut kexinit: KexInit,
                                          keys: &[key::Algorithm])
                                          -> Result<bool, Error> {
        if kexinit.algo.is_none() {
            // read algo from packet.
            if self.read_len == 0 {
                try!(self.set_clear_len(stream));
            }
            if try!(read(stream, &mut self.read_buffer, self.read_len)) {
                {
                    let payload = self.get_current_payload();
                    kexinit.algo = Some(try!(negociation::read_kex(payload, keys)));
                    kexinit.exchange.client_kex_init.extend(payload);
                }
                self.recv_seqn += 1;
                self.read_buffer.clear();
                self.read_len = 0;
                self.state = Some(ServerState::Kex(try!(kexinit.kexinit())));
                Ok(true)
            } else {
                // A complete packet could not be read, we need to read more.
                self.state = Some(ServerState::Kex(Kex::KexInit(kexinit)));
                Ok(false)
            }
        } else {
            self.state = Some(ServerState::Kex(try!(kexinit.kexinit())));
            Ok(true)
        }
    }
}

pub fn read_encrypted<A:Authenticate, T, S:super::Serve<T>>(auth:&A, enc:&mut Encrypted<S>, buf:&[u8], buffer:&mut CryptoBuf) -> EncryptedState {
    // If we've successfully read a packet.

    let state = std::mem::replace(&mut enc.state, None);
    match state {
        Some(EncryptedState::WaitingServiceRequest) if buf[0] ==
            msg::SERVICE_REQUEST => {

                let len = BigEndian::read_u32(&buf[1..]) as usize;
                let request = &buf[5..(5 + len)];
                debug!("request: {:?}", std::str::from_utf8(request));
                debug!("decrypted {:?}", buf);
                if request == b"ssh-userauth" {
                    EncryptedState::ServiceRequest
                } else {
                    EncryptedState::WaitingServiceRequest
                }
            }
        Some(EncryptedState::WaitingAuthRequest(auth_request)) => {
            if buf[0] == msg::USERAUTH_REQUEST {

                auth_request.auth_request(auth, buf)

            } else {
                // Wrong request
                EncryptedState::WaitingAuthRequest(auth_request)
            }
        }

        Some(EncryptedState::WaitingSignature(auth_request)) => {
            debug!("receiving signature, {:?}", buf);
            if buf[0] == msg::USERAUTH_REQUEST {
                
                auth_request.waiting_signature(buf,
                                               enc.session_id
                                               .as_bytes(),
                                               buffer)

            } else {
                EncryptedState::RejectAuthRequest(auth_request)
            }
        }
        Some(EncryptedState::WaitingChannelOpen) if buf[0] == msg::CHANNEL_OPEN => {

            let typ_len = BigEndian::read_u32(&buf[1..]) as usize;
            let typ = &buf[5..5 + typ_len];
            let sender = BigEndian::read_u32(&buf[5 + typ_len..]);
            let window = BigEndian::read_u32(&buf[9 + typ_len..]);
            let maxpacket = BigEndian::read_u32(&buf[13 + typ_len..]);

            debug!("waiting channel open: type = {:?} sender = {:?} window = {:?} maxpacket = {:?}",
                   String::from_utf8_lossy(typ),
                   sender,
                   window,
                   maxpacket);

            let mut sender_channel: u32 = 1;
            while enc.channels.contains_key(&sender_channel) || sender_channel == 0 {
                sender_channel = thread_rng().gen()
            }

            EncryptedState::ChannelOpenConfirmation(ChannelParameters {
                recipient_channel: sender,
                sender_channel: sender_channel,
                initial_window_size: window,
                maximum_packet_size: maxpacket,
            })

        }
        Some(EncryptedState::ChannelOpened(mut channels)) => {
            if buf[0] == msg::CHANNEL_DATA {
                debug!("buf: {:?}", buf);

                let channel_num = BigEndian::read_u32(&buf[1..]);
                if let Some(ref mut channel) = enc.channels.get_mut(&channel_num) {

                    let len = BigEndian::read_u32(&buf[5..]) as usize;
                    let data = &buf[9..9 + len];
                    buffer.clear();
                    if let Ok(()) = channel.server.data(&data,
                                                        &mut channel.stdout,
                                                        &mut channel.stderr) {
                        if channel.stdout.len() > 0 || channel.stderr.len() > 0 {
                            channels.insert(channel_num);
                        }
                    } else {
                        unimplemented!()
                    }
                }
            }
            EncryptedState::ChannelOpened(channels)
        }
        Some(state) => {
            debug!("buf: {:?}", buf);
            debug!("replacing state: {:?}", state);
            state
        },
        None => unreachable!()
    }

}