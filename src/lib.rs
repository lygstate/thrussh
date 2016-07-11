// Copyright 2016 Pierre-Étienne Meunier
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
// http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.
//

extern crate libc;
extern crate libsodium_sys;
extern crate rand;

#[macro_use]
extern crate log;
extern crate byteorder;

extern crate rustc_serialize; // config: read base 64.
extern crate time;

mod sodium;
mod cryptobuf;
use cryptobuf::CryptoBuf;

mod sshbuffer;
use sshbuffer::{SSHBuffer};

use std::sync::{Once, ONCE_INIT};
use std::io::{Read, BufRead, BufReader};


use byteorder::{ByteOrder, BigEndian};
use rustc_serialize::base64::FromBase64;
use std::path::Path;
use std::fs::File;
use std::collections::HashMap;


static SODIUM_INIT: Once = ONCE_INIT;
mod state;
use state::*;

#[derive(Debug)]
pub enum Error {
    CouldNotReadKey,
    Base64(rustc_serialize::base64::FromBase64Error),
    KexInit,
    Version,
    Kex,
    DH,
    PacketAuth,
    NewKeys,
    Inconsistent,
    HUP,
    IndexOutOfBounds,
    Utf8(std::str::Utf8Error),
    UnknownKey,
    IO(std::io::Error),
}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Error {
        Error::IO(e)
    }
}
impl From<std::str::Utf8Error> for Error {
    fn from(e: std::str::Utf8Error) -> Error {
        Error::Utf8(e)
    }
}
impl From<rustc_serialize::base64::FromBase64Error> for Error {
    fn from(e: rustc_serialize::base64::FromBase64Error) -> Error {
        Error::Base64(e)
    }
}

mod negociation;

mod msg;
mod kex;

mod cipher;
use cipher::CipherT;
pub mod key;

// mod mac;
// use mac::*;
// mod compression;

mod encoding;
use encoding::*;

mod auth;
macro_rules! transport {
    ( $x:expr ) => {
        {
            match $x[0] {
                msg::DISCONNECT => return Ok(ReturnCode::Disconnect),
                msg::IGNORE => return Ok(ReturnCode::Ok),
                msg::UNIMPLEMENTED => return Ok(ReturnCode::Ok),
                msg::DEBUG => return Ok(ReturnCode::Ok),
                _ => {}
            }
        }
    };
}
pub enum ReturnCode {
    Ok,
    NotEnoughBytes,
    Disconnect,
    WrongPacket,
}

pub mod server;
pub mod client;

const SSH_EXTENDED_DATA_STDERR: u32 = 1;

pub struct SignalName<'a> {
    name: &'a str,
}
pub const SIGABRT: SignalName<'static> = SignalName { name: "ABRT" };
pub const SIGALRM: SignalName<'static> = SignalName { name: "ALRM" };
pub const SIGFPE: SignalName<'static> = SignalName { name: "FPE" };
pub const SIGHUP: SignalName<'static> = SignalName { name: "HUP" };
pub const SIGILL: SignalName<'static> = SignalName { name: "ILL" };
pub const SIGINT: SignalName<'static> = SignalName { name: "INT" };
pub const SIGKILL: SignalName<'static> = SignalName { name: "KILL" };
pub const SIGPIPE: SignalName<'static> = SignalName { name: "PIPE" };
pub const SIGQUIT: SignalName<'static> = SignalName { name: "QUIT" };
pub const SIGSEGV: SignalName<'static> = SignalName { name: "SEGV" };
pub const SIGTERM: SignalName<'static> = SignalName { name: "TERM" };
pub const SIGUSR1: SignalName<'static> = SignalName { name: "USR1" };

impl<'a> SignalName<'a> {
    pub fn other(name: &'a str) -> SignalName<'a> {
        SignalName { name: name }
    }
}

pub struct ChannelBuf<'a> {
    buffer: &'a mut CryptoBuf,
    channel: &'a mut ChannelParameters,
    write_buffer: &'a mut SSHBuffer,
    cipher: &'a mut cipher::CipherPair,
    wants_reply: bool,
}
impl<'a> ChannelBuf<'a> {
    fn output(&mut self, extended: Option<u32>, buf: &[u8]) -> usize {
        debug!("output {:?} {:?}", self.channel, buf);
        let mut buf = if buf.len() as u32 > self.channel.recipient_window_size {
            &buf[0..self.channel.recipient_window_size as usize]
        } else {
            buf
        };
        let buf_len = buf.len();

        while buf.len() > 0 && self.channel.recipient_window_size > 0 {

            // Compute the length we're allowed to send.
            let off = std::cmp::min(buf.len(),
                                    self.channel.recipient_maximum_packet_size as usize);
            let off = std::cmp::min(off, self.channel.recipient_window_size as usize);

            //
            self.buffer.clear();

            if let Some(ext) = extended {
                self.buffer.push(msg::CHANNEL_EXTENDED_DATA);
                self.buffer.push_u32_be(self.channel.recipient_channel);
                self.buffer.push_u32_be(ext);
            } else {
                self.buffer.push(msg::CHANNEL_DATA);
                self.buffer.push_u32_be(self.channel.recipient_channel);
            }
            self.buffer.extend_ssh_string(&buf[..off]);
            debug!("buffer = {:?}", self.buffer.as_slice());
            self.cipher.write(self.buffer.as_slice(), self.write_buffer);

            self.channel.recipient_window_size -= off as u32;

            buf = &buf[off..]
        }
        buf_len
    }
    pub fn stdout(&mut self, stdout: &[u8]) -> usize {
        self.output(None, stdout)
    }
    pub fn stderr(&mut self, stderr: &[u8]) -> usize {
        self.output(Some(SSH_EXTENDED_DATA_STDERR), stderr)
    }

    fn reply(&mut self, msg: u8) {
        self.buffer.clear();
        self.buffer.push(msg);
        self.buffer.push_u32_be(self.channel.recipient_channel);
        debug!("reply {:?}", self.buffer.as_slice());
        self.cipher.write(self.buffer.as_slice(), self.write_buffer);
    }
    pub fn success(&mut self) {
        if self.wants_reply {
            self.reply(msg::CHANNEL_SUCCESS);
            self.wants_reply = false
        }
    }
    pub fn failure(&mut self) {
        if self.wants_reply {
            self.reply(msg::CHANNEL_FAILURE);
            self.wants_reply = false
        }
    }
    pub fn eof(&mut self) {
        self.reply(msg::CHANNEL_EOF);
    }
    pub fn close(mut self) {
        self.reply(msg::CHANNEL_CLOSE);
    }

    pub fn exit_status(&mut self, exit_status: u32) {
        // https://tools.ietf.org/html/rfc4254#section-6.10
        self.buffer.clear();
        self.buffer.push(msg::CHANNEL_REQUEST);
        self.buffer.push_u32_be(self.channel.recipient_channel);
        self.buffer.extend_ssh_string(b"exit-status");
        self.buffer.push(0);
        self.buffer.push_u32_be(exit_status);
        self.cipher.write(self.buffer.as_slice(), self.write_buffer);
    }

    pub fn exit_signal(&mut self,
                       signal_name: SignalName,
                       core_dumped: bool,
                       error_message: &str,
                       language_tag: &str) {
        // https://tools.ietf.org/html/rfc4254#section-6.10
        // Windows compatibility: we can't use Unix signal names here.
        self.buffer.clear();
        self.buffer.push(msg::CHANNEL_REQUEST);
        self.buffer.push_u32_be(self.channel.recipient_channel);
        self.buffer.extend_ssh_string(b"exit-signal");
        self.buffer.push(0);

        self.buffer.extend_ssh_string(signal_name.name.as_bytes());
        self.buffer.push(if core_dumped {
            1
        } else {
            0
        });
        self.buffer.extend_ssh_string(error_message.as_bytes());
        self.buffer.extend_ssh_string(language_tag.as_bytes());

        self.cipher.write(self.buffer.as_slice(), self.write_buffer);
    }
}

pub trait Server {
    fn new_channel(&mut self, channel: &ChannelParameters);
    fn data(&mut self, _: &[u8], _: ChannelBuf) -> Result<(), Error> {
        Ok(())
    }
    fn exec(&mut self, _: &[u8], _: ChannelBuf) -> Result<(), Error> {
        Ok(())
    }
}
pub trait Client {
    fn auth_banner(&mut self, _: &str) {}
    fn new_channel(&mut self, _: &ChannelParameters) {}
    fn data(&mut self, _: Option<u32>, _: &[u8], _: ChannelBuf) -> Result<(), Error> {
        Ok(())
    }
    fn check_server_key(&self, _: &key::PublicKey) -> bool {
        false
    }
}


fn complete_packet(buf: &mut CryptoBuf, off: usize) {

    let block_size = 8; // no MAC yet.
    let padding_len = {
        (block_size - ((buf.len() - off) % block_size))
    };
    let padding_len = if padding_len < 4 {
        padding_len + block_size
    } else {
        padding_len
    };
    let mac_len = 0;

    let packet_len = buf.len() - off - 4 + padding_len + mac_len;
    {
        let buf = buf.as_mut_slice();
        BigEndian::write_u32(&mut buf[off..], packet_len as u32);
        buf[off + 4] = padding_len as u8;
    }


    let mut padding = [0; 256];
    sodium::randombytes::into(&mut padding[0..padding_len]);

    buf.extend(&padding[0..padding_len]);

}

#[derive(Debug)]
pub struct ChannelParameters {
    pub recipient_channel: u32,
    pub sender_channel: u32,
    pub recipient_window_size: u32,
    pub sender_window_size: u32,
    pub recipient_maximum_packet_size: u32,
    pub sender_maximum_packet_size: u32,
}
fn adjust_window_size(write_buffer: &mut SSHBuffer,
                      cipher: &mut cipher::CipherPair,
                      target: u32,
                      buffer: &mut CryptoBuf,
                      channel: &mut ChannelParameters) {
    buffer.clear();
    buffer.push(msg::CHANNEL_WINDOW_ADJUST);
    buffer.push_u32_be(channel.recipient_channel);
    buffer.push_u32_be(target - channel.sender_window_size);
    cipher.write(buffer.as_slice(), write_buffer);
    channel.sender_window_size = target;
}


/// Fills the read buffer, and returns whether a complete message has been read.
///
/// It would be tempting to return either a slice of `stream`, or a
/// slice of `read_buffer`, but except for a very small number of
/// messages, we need double buffering anyway to decrypt in place on
/// `read_buffer`.
fn read<R: BufRead>(stream: &mut R,
                    read_buffer: &mut CryptoBuf,
                    read_len: usize,
                    bytes_read: &mut usize)
                    -> Result<bool, Error> {
    // This loop consumes something or returns, it cannot loop forever.
    loop {
        let consumed_len = match stream.fill_buf() {
            Ok(buf) => {
                if read_buffer.len() + buf.len() < read_len + 4 {

                    read_buffer.extend(buf);
                    buf.len()

                } else {
                    let consumed_len = read_len + 4 - read_buffer.len();
                    read_buffer.extend(&buf[0..consumed_len]);
                    consumed_len
                }
            }
            Err(e) => {
                if e.kind() == std::io::ErrorKind::WouldBlock {
                    return Ok(false);
                } else {
                    return Err(Error::IO(e));
                }
            }
        };
        stream.consume(consumed_len);
        *bytes_read += consumed_len;
        if read_buffer.len() >= 4 + read_len {
            return Ok(true);
        }
    }
}


const KEYTYPE_ED25519: &'static [u8] = b"ssh-ed25519";

pub fn load_public_key<P: AsRef<Path>>(p: P) -> Result<key::PublicKey, Error> {

    let mut pubkey = String::new();
    let mut file = try!(File::open(p.as_ref()));
    try!(file.read_to_string(&mut pubkey));

    let mut split = pubkey.split_whitespace();

    match (split.next(), split.next()) {
        (Some(ssh_), Some(key)) if ssh_.starts_with("ssh-") => {
            let base = try!(key.from_base64());
            read_public_key(&base)
        }
        _ => Err(Error::CouldNotReadKey),
    }
}

pub fn read_public_key(p: &[u8]) -> Result<key::PublicKey, Error> {
    let mut pos = p.reader(0);
    if try!(pos.read_string()) == b"ssh-ed25519" {
        if let Ok(pubkey) = pos.read_string() {
            return Ok(key::PublicKey::Ed25519(sodium::ed25519::PublicKey::copy_from_slice(pubkey)));
        }
    }
    Err(Error::CouldNotReadKey)
}

pub fn load_secret_key<P: AsRef<Path>>(p: P) -> Result<key::SecretKey, Error> {

    let file = try!(File::open(p.as_ref()));
    let file = BufReader::new(file);

    let mut secret = String::new();
    let mut started = false;

    for l in file.lines() {
        let l = try!(l);
        if l == "-----BEGIN OPENSSH PRIVATE KEY-----" {
            started = true
        } else if l == "-----END OPENSSH PRIVATE KEY-----" {
            break;
        } else if started {
            secret.push_str(&l)
        }
    }
    let secret = try!(secret.from_base64());

    if &secret[0..15] == b"openssh-key-v1\0" {
        let mut position = secret.reader(15);

        let ciphername = try!(position.read_string());
        let kdfname = try!(position.read_string());
        let kdfoptions = try!(position.read_string());
        info!("ciphername: {:?}", std::str::from_utf8(ciphername));
        debug!("kdf: {:?} {:?}",
                 std::str::from_utf8(kdfname),
                 std::str::from_utf8(kdfoptions));

        let nkeys = try!(position.read_u32());

        for _ in 0..nkeys {
            let public_string = try!(position.read_string());
            let mut pos = public_string.reader(0);
            if try!(pos.read_string()) == KEYTYPE_ED25519 {
                if let Ok(pubkey) = pos.read_string() {
                    let public = sodium::ed25519::PublicKey::copy_from_slice(pubkey);
                    info!("public: {:?}", public);
                } else {
                    info!("warning: no public key");
                }
            }
        }
        info!("there are {} keys in this file", nkeys);
        let secret = try!(position.read_string());
        if kdfname == b"none" {
            let mut position = secret.reader(0);
            let check0 = try!(position.read_u32());
            let check1 = try!(position.read_u32());
            debug!("check0: {:?}", check0);
            debug!("check1: {:?}", check1);
            for _ in 0..nkeys {

                let key_type = try!(position.read_string());
                if key_type == KEYTYPE_ED25519 {
                    let pubkey = try!(position.read_string());
                    debug!("pubkey = {:?}", pubkey);
                    let seckey = try!(position.read_string());
                    let comment = try!(position.read_string());
                    debug!("comment = {:?}", comment);
                    let secret = sodium::ed25519::SecretKey::copy_from_slice(seckey);
                    return Ok(key::SecretKey::Ed25519(secret));
                } else {
                    info!("unsupported key type {:?}", std::str::from_utf8(key_type));
                }
            }
            Err(Error::CouldNotReadKey)
        } else {
            info!("unsupported secret key cipher: {:?}", std::str::from_utf8(kdfname));
            Err(Error::CouldNotReadKey)
        }
    } else {
        Err(Error::CouldNotReadKey)
    }
}
