use super::super::msg;
use super::super::kex;
use super::*;
use super::super::{CryptoBuf, KexDhDone, Encrypted, ChannelParameters, complete_packet};
use super::super::auth;

impl ServerSession {

    pub fn server_cleartext_kex_ecdh_reply(&mut self,
                                           kexdhdone: &KexDhDone,
                                           hash: &kex::Digest) {
        // ECDH Key exchange.
        // http://tools.ietf.org/html/rfc5656#section-4
        self.buffers.write.buffer.extend(b"\0\0\0\0\0");
        self.buffers.write.buffer.push(msg::KEX_ECDH_REPLY);
        kexdhdone.key.public_host_key.extend_pubkey(&mut self.buffers.write.buffer);
        // Server ephemeral
        self.buffers.write.buffer.extend_ssh_string(&kexdhdone.exchange.server_ephemeral);
        // Hash signature
        kexdhdone.key.add_signature(&mut self.buffers.write.buffer, hash.as_bytes());
        //
        complete_packet(&mut self.buffers.write.buffer, 0);
        self.buffers.write.seqn += 1;
    }
    pub fn server_cleartext_send_newkeys(&mut self) {
        // Sending the NEWKEYS packet.
        // https://tools.ietf.org/html/rfc4253#section-7.3
        // buffer.clear();
        let pos = self.buffers.write.buffer.len();
        self.buffers.write.buffer.extend(b"\0\0\0\0\0");
        self.buffers.write.buffer.push(msg::NEWKEYS);
        complete_packet(&mut self.buffers.write.buffer, pos);
        self.buffers.write.seqn += 1;
    }


    pub fn server_reject_auth_request(&mut self,
                                      enc:&mut Encrypted,
                                      buffer: &mut CryptoBuf,
                                      auth_request: &AuthRequest) {
        buffer.clear();
        buffer.push(msg::USERAUTH_FAILURE);

        buffer.extend_list(auth_request.methods);
        buffer.push(if auth_request.partial_success {
            1
        } else {
            0
        });

        enc.cipher.write_server_packet(self.buffers.write.seqn, buffer.as_slice(), &mut self.buffers.write.buffer);

        self.buffers.write.seqn += 1;
    }


    pub fn server_send_pk_ok(&mut self,
                             enc: &mut Encrypted,
                             buffer: &mut CryptoBuf,
                             auth_request: &mut AuthRequest) {
        buffer.clear();
        buffer.push(msg::USERAUTH_PK_OK);
        buffer.extend_ssh_string(auth_request.public_key_algorithm.as_slice());
        buffer.extend_ssh_string(auth_request.public_key.as_slice());
        enc.cipher
            .write_server_packet(self.buffers.write.seqn, buffer.as_slice(), &mut self.buffers.write.buffer);
        self.buffers.write.seqn += 1;
        auth_request.sent_pk_ok = true;
    }
}

use super::super::EncryptedState;
use sodium;
use encoding::Reader;

impl Encrypted {
    pub fn server_confirm_channel_open(&mut self,
                                       buffer: &mut CryptoBuf,
                                       channel: &ChannelParameters,
                                       write_buffer: &mut super::super::SSHBuffer) {
        buffer.clear();
        buffer.push(msg::CHANNEL_OPEN_CONFIRMATION);
        buffer.push_u32_be(channel.recipient_channel);
        buffer.push_u32_be(channel.sender_channel);
        buffer.push_u32_be(channel.initial_window_size);
        buffer.push_u32_be(channel.maximum_packet_size);
        self.cipher.write_server_packet(write_buffer.seqn, buffer.as_slice(), &mut write_buffer.buffer);
        write_buffer.seqn += 1;
    }

    pub fn server_accept_service(&mut self,
                                 banner: Option<&str>,
                                 methods: auth::Methods,
                                 buffer: &mut CryptoBuf,
                                 write_buffer: &mut super::super::SSHBuffer)
                                 -> AuthRequest {
        buffer.clear();
        buffer.push(msg::SERVICE_ACCEPT);
        buffer.extend_ssh_string(b"ssh-userauth");
        self.cipher.write_server_packet(write_buffer.seqn, buffer.as_slice(), &mut write_buffer.buffer);
        write_buffer.seqn += 1;

        if let Some(ref banner) = banner {

            buffer.clear();
            buffer.push(msg::USERAUTH_BANNER);
            buffer.extend_ssh_string(banner.as_bytes());
            buffer.extend_ssh_string(b"");

            self.cipher
               .write_server_packet(write_buffer.seqn, buffer.as_slice(), &mut write_buffer.buffer);
            write_buffer.seqn += 1;
        }

        AuthRequest {
            methods: methods,
            partial_success: false, // not used immediately anway.
            public_key: CryptoBuf::new(),
            public_key_algorithm: CryptoBuf::new(),
            sent_pk_ok: false,
            public_key_is_ok: false
        }
    }

    pub fn server_auth_request_success(&mut self, buffer:&mut CryptoBuf, write_buffer:&mut super::super::SSHBuffer) {
        buffer.clear();
        buffer.push(msg::USERAUTH_SUCCESS);
        self.cipher.write_server_packet(write_buffer.seqn,
                                        buffer.as_slice(),
                                        &mut write_buffer.buffer);
        write_buffer.seqn += 1;
        self.state = Some(EncryptedState::WaitingChannelOpen);

    }

    pub fn server_verify_signature(&mut self, buf:&[u8], buffer:&mut CryptoBuf, auth_request: AuthRequest) -> EncryptedState {
        // https://tools.ietf.org/html/rfc4252#section-5
        let mut r = buf.reader(1);
        let user_name = r.read_string().unwrap();
        let service_name = r.read_string().unwrap();
        let method = r.read_string().unwrap();
        let is_probe = r.read_byte().unwrap() == 0;
        // TODO: check that the user is the same (maybe?)
        if service_name == b"ssh-connection" && method == b"publickey" && !is_probe {

            let algo = r.read_string().unwrap();
            let key = r.read_string().unwrap();

            let pos0 = r.position;
            if algo == b"ssh-ed25519" {

                let key = {
                    let mut k = key.reader(0);
                    k.read_string(); // should be equal to algo.
                    sodium::ed25519::PublicKey::copy_from_slice(k.read_string().unwrap())
                };
                let signature = r.read_string().unwrap();
                let mut s = signature.reader(0);
                let algo_ = s.read_string().unwrap();
                let sig = sodium::ed25519::Signature::copy_from_slice(s.read_string().unwrap());

                buffer.clear();
                buffer.extend_ssh_string(self.session_id.as_bytes());
                buffer.extend(&buf[0..pos0]);
                // Verify signature.
                if sodium::ed25519::verify_detached(&sig, buffer.as_slice(), &key) {
                    EncryptedState::AuthRequestSuccess(auth_request)
                } else {
                    EncryptedState::RejectAuthRequest(auth_request)
                }
            } else {
                EncryptedState::RejectAuthRequest(auth_request)
            }
        } else {
            EncryptedState::RejectAuthRequest(auth_request)
        }
    }


}
