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

use super::encoding;
use cryptobuf::CryptoBuf;

bitflags! {
    pub flags M: u32 {
        const NONE = 1,
        const PASSWORD = 2,
        const PUBLICKEY = 4,
        const HOSTBASED = 8
    }
}

macro_rules! iter {
    ( $y:expr, $x:expr ) => {
        {
            if $y.contains($x) {
                $y.remove($x);
                return Some($x)
            }
        }
    };
}


impl Iterator for M {
    type Item = M;
    fn next(&mut self) -> Option<M> {
        iter!(self, NONE);
        iter!(self, PASSWORD);
        iter!(self, PUBLICKEY);
        iter!(self, HOSTBASED);
        None
    }
}


#[derive(Debug)]
pub enum Method<'a, K> {
    None,
    Password {
        user: &'a str,
        password: &'a str,
    },
    PublicKey {
        user: &'a str,
        public_key: K
    },
    Hostbased,
}

impl<'a,K> Method<'a,K> {
    pub fn num(&self) -> M {
        match *self {
            Method::None => NONE,
            Method::Password { .. } => PASSWORD,
            Method::PublicKey { .. } => PUBLICKEY,
            Method::Hostbased => HOSTBASED,
        }
    }
}

impl encoding::Bytes for M {
    fn bytes(&self) -> &'static [u8] {
        match *self {
            NONE => b"none",
            PASSWORD => b"password",
            PUBLICKEY => b"publickey",
            HOSTBASED => b"hostbased",
            _ => unreachable!(),
        }
    }
}

impl M {
    pub fn from_bytes(b:&[u8]) -> Option<M> {
        match b {
            b"none" => Some(NONE),
            b"password" => Some(PASSWORD),
            b"publickey" => Some(PUBLICKEY),
            b"hostbased" => Some(HOSTBASED),
            _ => None
        }
    }
}

#[derive(Debug)]
pub enum Auth {
    Success,
    Reject {
        remaining_methods: M,
        partial_success: bool,
    },
}

#[derive(Debug)]
pub struct AuthRequest {
    pub methods: M,
    pub partial_success: bool,
    pub public_key: CryptoBuf,
    pub public_key_algorithm: CryptoBuf,
    pub public_key_is_ok: bool,
    pub sent_pk_ok: bool,
    pub was_rejected: bool
}
