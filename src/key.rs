use super::negociation::{ Named, Preferred };
use super::{CryptoBuf, Error};
pub use super::sodium::ed25519;

#[derive(Debug,Clone)]
pub enum Name {
    Ed25519 // "ssh-ed25519"
}

#[derive(Debug,Clone, PartialEq, Eq)]
pub enum PublicKey {
    Ed25519(ed25519::PublicKey)
}
#[derive(Debug,Clone)]
pub enum SecretKey {
    Ed25519(ed25519::SecretKey)
}


impl PublicKey {
    pub fn extend_pubkey(&self, buffer:&mut CryptoBuf) {
        match self {
            &PublicKey::Ed25519(ref public_host_key) => {

                buffer.push_u32_be(
                    (KEY_ED25519.len()
                     + ed25519::PUBLICKEYBYTES
                     + 8) as u32
                );
                buffer.extend_ssh_string(KEY_ED25519.as_bytes());
                buffer.extend_ssh_string(public_host_key.as_bytes());
            }
        }
    }

    pub fn name(&self) -> &'static str {
        match self {
            &PublicKey::Ed25519 (_) => "ssh-ed25519"
        }
    }
}


#[derive(Debug,Clone)]
pub struct Algorithm {
    pub public_host_key: PublicKey,
    pub secret_host_key: SecretKey,
}


pub const KEY_ED25519:&'static str = "ssh-ed25519";
pub const KEY_ALGORITHMS: &'static [&'static str;1] = &[
    KEY_ED25519
];

impl Named for Name {
    fn from_name(name: &[u8]) -> Option<Self> {
        if name == KEY_ED25519.as_bytes() {
            return Some(Name::Ed25519)
        }
        None
    }
}

impl Preferred for Algorithm {
    fn preferred() -> &'static [&'static str] {
        KEY_ALGORITHMS
    }
}
use std::path::Path;
impl Algorithm {
    pub fn load_keypair_ed25519<P:AsRef<Path>, Q:AsRef<Path>>(public:P, secret:Q) -> Result<Algorithm, Error> {
        Ok(Algorithm {
            public_host_key: try!(super::load_public_key(public)),
            secret_host_key: try!(super::load_secret_key(secret)),
        })
    }

    pub fn name(&self) -> &'static str {
        self.public_host_key.name()
    }
    
    pub fn add_signature(&self, buffer: &mut CryptoBuf, hash:&[u8]) {
        match self.secret_host_key {
            SecretKey::Ed25519(ref secret_host_key) => {

                let mut sign = ed25519::Signature::new_blank();
                ed25519::sign_detached(&mut sign, &hash, secret_host_key);

                buffer.push_u32_be(
                    (KEY_ED25519.len()
                     + ed25519::SIGNATUREBYTES
                     + 8) as u32
                );
                buffer.extend_ssh_string(KEY_ED25519.as_bytes());
                buffer.extend_ssh_string(sign.as_bytes());
            }
        }
    }
}
