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
use byteorder::{ByteOrder, BigEndian};
use super::super::Error;
use std::io::BufRead;
use sshbuffer::SSHBuffer;

use super::super::sodium;
use sodium::chacha20;
use sodium::poly1305;
use sodium::randombytes;

#[derive(Debug)]
pub struct Cipher {
    k1: chacha20::Key,
    k2: chacha20::Key,
}

impl Cipher {
    pub fn init(key: &[u8]) -> Cipher {
        Cipher {
            k1: chacha20::Key::copy_from_slice(&key[32..64]),
            k2: chacha20::Key::copy_from_slice(&key[0..32]),
        }
    }
}

impl super::CipherT for Cipher {
    fn read<'a, R: BufRead>(&self,
                            stream: &mut R,
                            read_buffer: &'a mut SSHBuffer)
                            -> Result<Option<&'a [u8]>, Error> {

        // http://cvsweb.openbsd.org/cgi-bin/cvsweb/src/usr.bin/ssh/PROTOCOL.chacha20poly1305?annotate=HEAD
        let mut nonce = [0; 8];
        BigEndian::write_u32(&mut nonce[4..], read_buffer.seqn as u32);
        let nonce = chacha20::Nonce::copy_from_slice(&nonce);

        // - Compute the length, by chacha20-stream-xoring the first 4
        // bytes with the last 32 bytes of the client key.
        if read_buffer.len == 0 {
            read_buffer.buffer.clear();
            let mut len = [0; 4];
            try!(stream.read_exact(&mut len));
            read_buffer.buffer.extend(&len);

            chacha20::stream_xor_inplace(&mut len, &nonce, &self.k1);

            read_buffer.len = BigEndian::read_u32(&len) as usize + poly1305::TAGBYTES;
            debug!("buffer len: {:?}", read_buffer.len);
        }
        // - Compute the Poly1305 auth on the first (4+length) first bytes of the packet.
        if try!(super::read(stream,
                            &mut read_buffer.buffer,
                            read_buffer.len,
                            &mut read_buffer.bytes)) {

            let mut poly_key = [0; 32];
            chacha20::stream_xor_inplace(&mut poly_key, &nonce, &self.k2);
            let poly_key = poly1305::Key::copy_from_slice(&poly_key);

            let mut tag = poly1305::Tag::new_blank();
            let read_len = read_buffer.len;
            poly1305::authenticate(&mut tag,
                                   &mut read_buffer.buffer[0..4 + read_len - poly1305::TAGBYTES],
                                   &poly_key);

            // - Constant-time-compare it with the Poly1305 at the end
            // of the packet (right after the 4+length first bytes).
            if sodium::memcmp(&tag, &read_buffer.buffer[4 + read_buffer.len - poly1305::TAGBYTES..]) {

                // - If the auth is correct, chacha20-xor the length
                // bytes after the first 4 ones, with ic 1.
                // (actually, the above doc says "ic = LE encoding of
                // 1", which is different from the libsodium
                // interface).

                chacha20::xor_inplace(&mut read_buffer.buffer[4..(4 + read_buffer.len -
                                                                  poly1305::TAGBYTES)],
                                      &nonce,
                                      1,
                                      &self.k2);
                let padding = read_buffer.buffer[4] as usize;
                let result = Some(&read_buffer.buffer[5..(5 + read_buffer.len - poly1305::TAGBYTES -
                                                          padding -
                                                          1)]);
                read_buffer.seqn += 1;
                read_buffer.len = 0;
                Ok(result)

            } else {
                Err(Error::PacketAuth)
            }
        } else {
            Ok(None)
        }
    }

    /// Append an encrypted packet with contents `packet_content` at the end of `buffer`.
    fn write(&self, packet_content: &[u8], buffer: &mut SSHBuffer) {

        // http://cvsweb.openbsd.org/cgi-bin/cvsweb/src/usr.bin/ssh/PROTOCOL.chacha20poly1305?annotate=HEAD
        let offset = buffer.buffer.len();
        // - Compute the length, by chacha20-stream-xoring the first 4
        // bytes with the last 32 bytes of the client key.

        let block_size = 8;
        let padding_len = if packet_content.len() + 5 <= 16 {
            16 - packet_content.len() - 1
        } else {
            (block_size - ((1 + packet_content.len()) % block_size))
        };
        let padding_len = if padding_len < 4 {
            padding_len + block_size
        } else {
            padding_len
        };

        buffer.buffer.push_u32_be((packet_content.len() + padding_len + 1) as u32);

        let mut nonce = [0; 8];
        BigEndian::write_u32(&mut nonce[4..], buffer.seqn as u32);
        let nonce = chacha20::Nonce::copy_from_slice(&nonce);

        chacha20::stream_xor_inplace(&mut buffer.buffer[offset..(offset + 4)], &nonce, &self.k1);
        // the first 4 bytes of buffer now contain the encrypted length.
        // - Append the encrypted packet

        // Compute the amount of padding.
        // println!("padding_len {:?}", padding_len);
        buffer.buffer.push(padding_len as u8);

        buffer.buffer.extend(packet_content);

        // println!("buffer before padding: {:?}", &(buffer.as_slice())[offset..]);

        let mut padding = [0; 256];

        randombytes::into(&mut padding[0..padding_len]);
        buffer.buffer.extend(&padding[0..padding_len]);

        // println!("buffer before encryption: {:?}", &(buffer.as_slice())[offset..]);

        chacha20::xor_inplace(&mut buffer.buffer[offset + 4..], &nonce, 1, &self.k2);

        // println!("buffer before tag: {:?}", &(buffer.as_slice())[offset..]);
        // - Compute the Poly1305 auth on the first (4+length) first bytes of the packet.

        let mut poly_key = [0; poly1305::KEYBYTES];
        chacha20::stream_xor_inplace(&mut poly_key, &nonce, &self.k2);
        let poly_key = poly1305::Key::copy_from_slice(&poly_key);

        let mut tag = poly1305::Tag::new_blank();
        poly1305::authenticate(&mut tag, &buffer.buffer[offset..], &poly_key);

        buffer.buffer.extend(&tag);
        buffer.seqn += 1;
    }
}
