extern crate libc;
extern crate sodiumoxide;
#[macro_use]
extern crate log;
extern crate byteorder;
extern crate regex;
extern crate rustc_serialize;

use rustc_serialize::hex::ToHex;

use byteorder::{ByteOrder,BigEndian, ReadBytesExt, WriteBytesExt};

use std::io::{ Read, Write, BufRead };

use std::sync::{Once, ONCE_INIT};

pub mod config;
// use sodiumoxide::crypto::hash::sha256::Digest;
use sodiumoxide::crypto::sign::ed25519;

static SODIUM_INIT: Once = ONCE_INIT;
#[derive(Debug)]
pub enum Error {
    CouldNotReadKey,
    KexInit,
    Version,
    Kex,
    DH,
    PacketAuth,
    IO(std::io::Error)
}

impl From<std::io::Error> for Error {
    fn from(e:std::io::Error) -> Error {
        Error::IO(e)
    }
}

mod msg;
mod kex;

pub mod key;
mod cipher;

mod mac;
use mac::*;

#[derive(Debug)]
pub struct Exchange {
    client_id:Option<Vec<u8>>,
    server_id:Option<Vec<u8>>,
    client_kex_init:Option<Vec<u8>>,
    server_kex_init:Option<Vec<u8>>,
    client_ephemeral:Option<Vec<u8>>,
    server_ephemeral:Option<Vec<u8>>
}

impl Exchange {
    fn new() -> Self {
        Exchange { client_id: None,
                   server_id: None,
                   client_kex_init: None,
                   server_kex_init: None,
                   client_ephemeral: None,
                   server_ephemeral: None }
    }
}


#[derive(Debug)]
pub struct ServerSession<'a> {
    keys:&'a[key::Algorithm],
    recv_seqn: usize,
    sent_seqn: usize,
    state: Option<ServerState>
}

#[derive(Debug)]
pub enum ServerState {
    VersionOk(Exchange), // Version number received.
    KexInit { // Version number sent. `algo` and `sent` tell wether kexinit has been received, and sent, respectively.
        algo: Option<Names>,
        exchange: Exchange,
        session_id: Option<kex::Digest>,
        sent: bool
    },
    KexDh { // Algorithms have been determined, the DH algorithm should run.
        exchange: Exchange,
        kex: kex::Name,
        key: key::Algorithm,
        cipher: cipher::Name,
        mac: Mac,
        session_id: Option<kex::Digest>,
        follows: bool
    },
    KexDhDone { // The kex has run.
        exchange: Exchange,
        kex: kex::Algorithm,
        key: key::Algorithm,
        cipher: cipher::Name,
        mac: Mac,
        session_id: Option<kex::Digest>,
        follows: bool
    },
    NewKeys { // The DH is over, we've sent the NEWKEYS packet, and are waiting the NEWKEYS from the other side.
        exchange: Exchange,
        kex: kex::Algorithm,
        key: key::Algorithm,
        cipher: cipher::Cipher,
        mac: Mac,
        session_id: kex::Digest,
    },
    Encrypted { // Session is now encrypted.
        exchange: Exchange,
        kex: kex::Algorithm,
        key: key::Algorithm,
        cipher: cipher::Cipher,
        mac: Mac,
        session_id: kex::Digest,
    },
}

pub type Names = (kex::Name, key::Algorithm, cipher::Name, Mac, bool);

trait Named:Sized {
    fn from_name(&[u8]) -> Option<Self>;
}

trait Preferred:Sized {
    fn preferred() -> &'static [&'static str];
}

fn select<A:Named + 'static>(list:&[u8]) -> Option<A> {
    for l in list.split(|&x| x == b',') {
        if let Some(x) = A::from_name(l) {
            return Some(x)
        }
    }
    None
}
fn select_key(list:&[u8], keys:&[key::Algorithm]) -> Option<key::Algorithm> {
    for l in list.split(|&x| x == b',') {
        for k in keys {
            if l == k.name().as_bytes() {
                return Some(k.clone())
            }
        }
    }
    None
}


enum CompressionAlgorithm {
    None
}
const COMPRESSION_NONE:&'static str = "none";
const COMPRESSIONS: &'static [&'static str;1] = &[
    COMPRESSION_NONE
];

impl Named for CompressionAlgorithm {
    fn from_name(name: &[u8]) -> Option<Self> {
        if name == COMPRESSION_NONE.as_bytes() {
            return Some(CompressionAlgorithm::None)
        }
        None
    }
}
impl Preferred for CompressionAlgorithm {
    fn preferred() -> &'static [&'static str] {
        COMPRESSIONS
    }
}

fn write_packet<W:Write>(stream:&mut W, buf:&[u8]) -> Result<(),Error> {

    let block_size = 8; // no MAC yet.
    let padding_len = {
        (block_size - ((5+buf.len()) % block_size))
    };
    let padding_len = if padding_len < 4 { padding_len + block_size } else { padding_len };
    let mac_len = 0;

    let packet_len = 1 + buf.len() + padding_len + mac_len;
    try!(stream.write_u32::<BigEndian>(packet_len as u32));

    println!("len {:?}, padding {:?}", buf.len(), padding_len);
    try!(stream.write_u8(padding_len as u8));
    try!(stream.write_all(buf));

    let mut padding = [0;256];
    sodiumoxide::randombytes::randombytes_into(&mut padding[0..padding_len]);
    try!(stream.write_all(&padding[0..padding_len]));

    Ok(())
}

trait SSHString:Write {
    fn write_ssh_string(&mut self, s:&[u8]) -> Result<(), std::io::Error> {
        try!(self.write_u32::<BigEndian>(s.len() as u32));
        try!(self.write(s));
        Ok(())
    }
    fn write_ssh_mpint(&mut self, s:&[u8]) -> Result<(), std::io::Error> {
        let mut i = 0;
        while i < s.len() && s[i] == 0 {
            i+=1
        }
        if s[i] & 0x80 != 0 {
            try!(self.write_u32::<BigEndian>((s.len() - i + 1) as u32));
            try!(self.write_u8(0));
        } else {
            try!(self.write_u32::<BigEndian>((s.len() - i) as u32));
        }
        try!(self.write(&s[i..]));
        Ok(())
    }
}
impl<T:Write> SSHString for T {}

fn read_kex(buffer:&[u8], keys:&[key::Algorithm]) -> Result<Names,Error> {
    if buffer[0] != msg::KEXINIT {
        Err(Error::KexInit)
    } else {
        const FIELD_KEX_ALGORITHM: usize = 0;
        const FIELD_KEY_ALGORITHM: usize = 1;
        const FIELD_CIPHER_CLIENT_TO_SERVER: usize = 2;
        // const FIELD_CIPHER_SERVER_TO_CLIENT: usize = 3;
        const FIELD_MAC: usize = 4;
        const FIELD_FOLLOWS: usize = 9;
        let mut i = 17;
        let mut field = 0;
        let mut kex_algorithm = None;
        let mut key_algorithm = None;
        let mut cipher = None;
        let mut mac = None;
        let mut follows = None;
        while field < 10 {
            assert!(i+3 < buffer.len());
            let len = BigEndian::read_u32(&buffer[i..]) as usize;
            if field == FIELD_KEX_ALGORITHM {
                debug!("kex_algorithms: {:?}", std::str::from_utf8(&buffer[(i+4)..(i+4+len)]));
                kex_algorithm = select(&buffer[(i+4)..(i+4+len)])
            } else  if field == FIELD_KEY_ALGORITHM {
                debug!("key_algorithms: {:?}", std::str::from_utf8(&buffer[(i+4)..(i+4+len)]));

                key_algorithm = select_key(&buffer[(i+4)..(i+4+len)], keys)

            } else  if field == FIELD_CIPHER_CLIENT_TO_SERVER {
                debug!("ciphers_client_to_server: {:?}", std::str::from_utf8(&buffer[(i+4)..(i+4+len)]));
                cipher = select(&buffer[(i+4)..(i+4+len)])
            } else  if field == FIELD_MAC {
                debug!("mac: {:?}", std::str::from_utf8(&buffer[(i+4)..(i+4+len)]));
                mac = select(&buffer[(i+4)..(i+4+len)])
            } else  if field == FIELD_FOLLOWS {
                debug!("follows: {:?}", buffer[i] != 0);
                follows = Some(buffer[i] != 0)
            }
            i+=4+len;
            field += 1;
        }
        match (kex_algorithm, key_algorithm, cipher, mac, follows) {
            (Some(a), Some(b), Some(c), Some(d), Some(e)) => Ok((a,b,c,d,e)),
            _ => Err(Error::KexInit)
        }
    }
}


fn write_list(buf:&mut Vec<u8>, list:&[&str]) {
    let len0 = buf.len();
    buf.extend(&[0,0,0,0]);
    let mut first = true;
    for i in list {
        if !first {
            buf.push(b',')
        } else {
            first = false;
        }
        buf.extend(i.as_bytes())
    }
    let len = (buf.len() - len0 - 4) as u32;
    BigEndian::write_u32(&mut buf[len0..], len);
    println!("write_list: {:?}", &buf[len0..len0+4]);
}

fn write_key_list(buf:&mut Vec<u8>, list:&[key::Algorithm]) {
    let len0 = buf.len();
    buf.extend(&[0,0,0,0]);
    let mut first = true;
    for i in list {
        if !first {
            buf.push(b',')
        } else {
            first = false;
        }
        buf.extend(i.name().as_bytes())
    }
    let len = (buf.len() - len0 - 4) as u32;
    BigEndian::write_u32(&mut buf[len0..], len);
    println!("write_list: {:?}", &buf[len0..len0+4]);
}





pub fn hexdump(x:&[u8]) {
    let mut buf = Vec::new();
    let mut i = 0;
    while i < x.len() {
        if i%16 == 0 {
            print!("{:04}: ", i)
        }
        print!("{:02x} ", x[i]);
        if x[i] >= 0x20 && x[i]<= 0x7e {
            buf.push(x[i]);
        } else {
            buf.push(b'.');
        }
        if i % 16 == 15 || i == x.len() -1 {
            while i%16 != 15 {
                print!("   ");
                i += 1
            }
            println!(" {}", std::str::from_utf8(&buf).unwrap());
            buf.clear();
        }
        i += 1
    }
}


impl<'a> ServerSession<'a> {

    pub fn new(keys: &'a [key::Algorithm]) -> Self {
        SODIUM_INIT.call_once(|| { sodiumoxide::init(); });
        ServerSession {
            keys:keys,
            recv_seqn: 0,
            sent_seqn: 0,
            state: None
        }
    }

    fn read_packet<R:Read>(&mut self, stream:&mut R, buf:&mut Vec<u8>) -> Result<usize,Error> {

        let packet_length = try!(stream.read_u32::<BigEndian>()) as usize;
        let padding_length = try!(stream.read_u8()) as usize;

        println!("packet_length {:?}", packet_length);
        buf.resize(packet_length - 1, 0);
        try!(stream.read_exact(&mut buf[0..(packet_length - 1)]));

        self.recv_seqn += 1;
        // return the read length without padding.
        Ok(packet_length - 1 - padding_length)
    }
    fn write_kex(&self, buf:&mut Vec<u8>) {
        buf.clear();
        buf.push(msg::KEXINIT);

        let mut cookie = [0;16];
        sodiumoxide::randombytes::randombytes_into(&mut cookie);

        buf.extend(&cookie); // cookie
        println!("buf len :{:?}", buf.len());
        write_list(buf, kex::Name::preferred()); // kex algo

        write_key_list(buf, self.keys);

        write_list(buf, cipher::Name::preferred()); // cipher client to server
        write_list(buf, cipher::Name::preferred()); // cipher server to client

        write_list(buf, Mac::preferred()); // mac client to server
        write_list(buf, Mac::preferred()); // mac server to client
        write_list(buf, CompressionAlgorithm::preferred()); // compress client to server
        write_list(buf, CompressionAlgorithm::preferred()); // compress server to client
        write_list(buf, &[]); // languages client to server
        write_list(buf, &[]); // languagesserver to client

        buf.push(0); // doesn't follow
        buf.extend(&[0,0,0,0]); // reserved
    }

    
    pub fn read<R:Read>(&mut self, stream:&mut R, buffer:&mut Vec<u8>, buffer2:&mut Vec<u8>) -> Result<(), Error> {
        let state = std::mem::replace(&mut self.state, None);
        // println!("state: {:?}", state);
        match state {
            None => {

                let mut client_id = [0;255];
                let read = stream.read(&mut client_id).unwrap();
                if read < 8 {
                    Ok(())
                } else {
                    if &client_id[0..8] == b"SSH-2.0-" {
                        println!("read = {:?}", read);
                        let mut i = 0;
                        while i < read {
                            if client_id[i] == b'\n' || client_id[i] == b'\r' {
                                break
                            }
                            i += 1
                        }
                        if i < read {
                            let mut exchange = Exchange::new();
                            exchange.client_id = Some((&client_id[0..i]).to_vec());
                            self.state = Some(ServerState::VersionOk(exchange));
                            Ok(())
                        } else {
                            Err(Error::Version)
                        }
                    } else {
                        Err(Error::Version)
                    }
                }
            },
            Some(ServerState::KexInit { mut exchange, algo, sent, session_id }) => {
                let algo = if algo.is_none() {

                    let mut kex_init = Vec::new();
                    let read = self.read_packet(stream, &mut kex_init).unwrap();
                    kex_init.truncate(read);
                    let kex = read_kex(&kex_init, self.keys).unwrap();
                    // println!("kex = {:?}", kex_init);
                    exchange.client_kex_init = Some(kex_init);
                    Some(kex)

                } else {
                    algo
                };

                if !sent {
                    self.state = Some(ServerState::KexInit {
                        exchange: exchange,
                        algo:algo,
                        sent:sent,
                        session_id: session_id
                    });
                    Ok(())
                } else {
                    if let Some((kex,key,cipher,mac,follows)) = algo {
                        self.state = Some(
                            ServerState::KexDh {
                                exchange:exchange,
                                kex:kex, key:key,
                                cipher:cipher, mac:mac, follows:follows,
                                session_id: session_id
                            });
                        Ok(())
                    } else {
                        Err(Error::Kex)
                    }
                }
            },
            Some(ServerState::KexDh { mut exchange, mut kex, key, cipher, mac, follows, session_id }) => {

                buffer.clear();
                let read = try!(self.read_packet(stream, buffer));
                buffer.truncate(read);

                assert!(buffer[0] == msg::KEX_ECDH_INIT);

                let kex = try!(kex.dh(&mut exchange, &buffer));

                exchange.client_ephemeral = Some((&buffer[5..]).to_vec());
                self.state = Some(
                    ServerState::KexDhDone {
                        exchange:exchange,
                        kex:kex,
                        key:key,
                        cipher:cipher, mac:mac, follows:follows,
                        session_id: session_id
                    });
                Ok(())
            },
            Some(ServerState::NewKeys { exchange, kex, key, cipher, mac, session_id }) => {

                // We are waiting for the NEWKEYS packet.
                buffer.clear();
                let read = try!(self.read_packet(stream, buffer));
                if read > 0 && buffer[0] == msg::NEWKEYS {
                    self.state = Some(
                        ServerState::Encrypted { exchange: exchange, kex:kex, key:key,
                                                 cipher:cipher, mac:mac,
                                                 session_id: session_id,
                        }
                    );
                    Ok(())
                } else {
                    self.state = Some(
                        ServerState::NewKeys { exchange: exchange, kex:kex, key:key,
                                               cipher:cipher, mac:mac,
                                               session_id: session_id,
                        }
                    );
                    Ok(())
                }
            },
            mut state @ Some(ServerState::Encrypted { .. }) => {
                println!("read: encrypted");
                match state {
                    Some(ServerState::Encrypted { ref mut cipher, .. }) => {

                        let buf = try!(cipher.read_client_packet(&mut self.recv_seqn, stream, buffer));
                        println!("decrypted {:?}", buf);
                    },
                    _ => unreachable!()
                }
                self.state = state;
                Ok(())
            },
            _ => {
                println!("read: unhandled");
                Ok(())
            }
        }
    }

    pub fn write<W:Write>(&mut self, stream:&mut W, buffer:&mut Vec<u8>, buffer2:&mut Vec<u8>) -> Result<(), Error> {

        let state = std::mem::replace(&mut self.state, None);

        match state {
            Some(ServerState::VersionOk(mut exchange)) => {
                debug!("writing");
                let mut server_id = b"SSH-2.0-SSH.rs_0.1\r\n".to_vec();
                try!(stream.write_all(&mut server_id));
                let len = server_id.len();
                server_id.truncate(len - 2); // Drop CRLF.
                exchange.server_id = Some(server_id);

                try!(stream.flush());
                self.state = Some(
                    ServerState::KexInit {
                        exchange:exchange,
                        algo:None, sent:false,
                        session_id: None
                    }
                );
                Ok(())
            },
            Some(ServerState::KexInit { mut exchange, algo, sent, session_id }) => {
                if !sent {
                    let mut server_kex = Vec::new();
                    self.write_kex(&mut server_kex);
                    try!(write_packet(stream, &server_kex));
                    exchange.server_kex_init = Some(server_kex);
                    try!(stream.flush());
                }
                if let Some((kex,key,cipher,mac,follows)) = algo {

                    self.state = Some(
                        ServerState::KexDh {
                            exchange:exchange,
                            kex:kex, key:key, cipher:cipher, mac:mac, follows:follows,
                            session_id: session_id
                    });
                    Ok(())
                } else {
                    self.state = Some(
                        ServerState::KexInit {
                            exchange:exchange,
                            algo:algo, sent:true,
                            session_id: session_id
                        }
                    );
                    Ok(())
                }
            },
            Some(ServerState::KexDhDone { exchange, kex, key, mut cipher, mac, follows, session_id }) => {

                let hash = try!(kex.compute_exchange_hash(&key, &exchange, buffer));

                let mut ok = false;

                if let Some(ref server_ephemeral) = exchange.server_ephemeral {

                    buffer.clear();

                    // ECDH Key exchange.
                    // http://tools.ietf.org/html/rfc5656#section-4

                    buffer.push(msg::KEX_ECDH_REPLY);
                    
                    try!(key.write_pubkey(buffer));
                    // Server ephemeral
                    try!(buffer.write_ssh_string(server_ephemeral));

                    // Hash signature
                    key.sign(buffer, hash.as_slice());
                    //
                    try!(write_packet(stream, &buffer));
                } else {
                    return Err(Error::DH)
                }
                // Sending the NEWKEYS packet.
                // https://tools.ietf.org/html/rfc4253#section-7.3
                buffer.clear();
                buffer.push(msg::NEWKEYS);
                try!(write_packet(stream, &buffer));
                try!(stream.flush());

                let session_id = if let Some(session_id) = session_id {
                    session_id
                } else {
                    hash.clone()
                };
                // Now computing keys.
                let c = kex.compute_keys(&session_id, &hash, buffer, buffer2, &mut cipher);
                // keys.dump(); //println!("keys: {:?}", keys);
                //
                self.state = Some(
                    ServerState::NewKeys {
                        exchange: exchange,
                        kex:kex, key:key,
                        cipher: c,
                        mac:mac,
                        session_id: session_id,
                    }
                );
                Ok(())
            },
            mut enc @ Some(ServerState::Encrypted { .. }) => {
                match enc {
                    Some(ServerState::Encrypted { ref mut cipher, .. }) => {
                        // unimplemented!()
                    },
                    _ => unreachable!()
                }
                self.state = enc;
                Ok(())
            },
            session => {
                // println!("write: unhandled {:?}", session);
                self.state = session;
                Ok(())
            }
        }
    }
}


#[cfg(test)]
mod tests {
    #[test]
    fn it_works() {
    }
}
