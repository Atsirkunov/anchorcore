//! Pure-Rust Fernet tokens (AES-128-CBC + HMAC-SHA256), byte-compatible with
//! the `cryptography`/`fernet` implementations. Replaces the openssl-backed
//! `fernet` crate so no platform build needs OpenSSL.

use aes::Aes128;
use base64::{engine::general_purpose::URL_SAFE, Engine as _};
use cbc::{
    cipher::{block_padding::Pkcs7, generic_array::GenericArray, BlockDecryptMut, BlockEncryptMut, KeyIvInit},
    Decryptor, Encryptor,
};
use hmac::{Hmac, Mac};
use rand::RngCore;
use sha2::Sha256;
use std::time::{SystemTime, UNIX_EPOCH};

type Aes128CbcEnc = Encryptor<Aes128>;
type Aes128CbcDec = Decryptor<Aes128>;
type HmacSha256 = Hmac<Sha256>;

const VERSION: u8 = 0x80;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DecryptionError;

pub struct Fernet {
    signing_key: [u8; 16],
    encryption_key: [u8; 16],
}

impl Fernet {
    /// 32 url-safe base64 bytes (44 chars); first half signs, second half encrypts.
    pub fn new(key: &str) -> Option<Self> {
        let raw = URL_SAFE.decode(key.trim()).ok()?;
        if raw.len() != 32 {
            return None;
        }
        let mut signing_key = [0u8; 16];
        let mut encryption_key = [0u8; 16];
        signing_key.copy_from_slice(&raw[..16]);
        encryption_key.copy_from_slice(&raw[16..]);
        Some(Self {
            signing_key,
            encryption_key,
        })
    }

    pub fn generate_key() -> String {
        let mut raw = [0u8; 32];
        rand::thread_rng().fill_bytes(&mut raw);
        URL_SAFE.encode(raw)
    }

    pub fn encrypt(&self, data: &[u8]) -> String {
        let mut iv = [0u8; 16];
        rand::thread_rng().fill_bytes(&mut iv);
        let mut msg = Vec::with_capacity(57 + data.len());
        msg.push(VERSION);
        msg.extend_from_slice(&now_secs().to_be_bytes());
        msg.extend_from_slice(&iv);
        let mut buf = data.to_vec();
        let pos = buf.len();
        buf.resize(pos + 16, 0u8);
        let ct = Aes128CbcEnc::new(
            GenericArray::from_slice(&self.encryption_key),
            GenericArray::from_slice(&iv),
        )
        .encrypt_padded_mut::<Pkcs7>(&mut buf, pos)
        .expect("buffer sized for one padding block");
        msg.extend_from_slice(ct);
        msg.extend_from_slice(&hmac_of(&self.signing_key, &msg));
        URL_SAFE.encode(msg)
    }

    /// No TTL enforcement (matches previous `fernet` crate usage for stored secrets).
    pub fn decrypt(&self, token: &str) -> Result<Vec<u8>, DecryptionError> {
        let raw = URL_SAFE.decode(token.trim()).map_err(|_| DecryptionError)?;
        let (msg, tag) = split_tag(&raw)?;
        verify_version(msg)?;
        let mut mac =
            HmacSha256::new_from_slice(&self.signing_key).map_err(|_| DecryptionError)?;
        mac.update(msg);
        mac.verify_slice(tag).map_err(|_| DecryptionError)?;
        let (iv, ct) = split_iv_ct(msg)?;
        let mut buf = ct.to_vec();
        let pt = Aes128CbcDec::new(
            GenericArray::from_slice(&self.encryption_key),
            GenericArray::from_slice(&iv),
        )
        .decrypt_padded_mut::<Pkcs7>(&mut buf)
        .map_err(|_| DecryptionError)?;
        Ok(pt.to_vec())
    }
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn hmac_of(key: &[u8; 16], msg: &[u8]) -> [u8; 32] {
    let mut mac = HmacSha256::new_from_slice(key).expect("16-byte HMAC key");
    mac.update(msg);
    mac.finalize().into_bytes().into()
}

/// Minimum token: version(1) + time(8) + iv(16) + block(16) + hmac(32).
fn split_tag(raw: &[u8]) -> Result<(&[u8], &[u8]), DecryptionError> {
    if raw.len() < 73 {
        return Err(DecryptionError);
    }
    let at = raw.len() - 32;
    Ok((&raw[..at], &raw[at..]))
}

fn verify_version(msg: &[u8]) -> Result<(), DecryptionError> {
    if msg.first() == Some(&VERSION) {
        Ok(())
    } else {
        Err(DecryptionError)
    }
}

/// Header is 25 bytes; the rest is whole AES blocks.
fn split_iv_ct(msg: &[u8]) -> Result<([u8; 16], &[u8]), DecryptionError> {
    if msg.len() < 41 || msg.len() % 16 != 9 {
        return Err(DecryptionError);
    }
    let mut iv = [0u8; 16];
    iv.copy_from_slice(&msg[9..25]);
    Ok((iv, &msg[25..]))
}

#[cfg(test)]
mod tests {
    use super::*;

    // Generated once with Python `cryptography` Fernet (independent oracle).
    const PY_KEY: &str = "k2Omk37ek8mxclfgSvJTEmSiw1VzceexyI63e21ukMo=";
    const PY_TOKEN: &str = "gAAAAABqx7KGVAciMUZn-MOCAjApsuKwmOqv9hElqfzkEGs-SPKPJNGaWyVPEPmfXCkbZtiZMDSqhQGNHRxltDpQSkoyfbqVmO4DWFQEmufaGP2RbBlDSpQ=";

    #[test]
    fn decrypts_python_vector() {
        let f = Fernet::new(PY_KEY).expect("valid key");
        let pt = f.decrypt(PY_TOKEN).expect("valid token");
        assert_eq!(pt, b"tok123-secret-value");
    }

    #[test]
    fn roundtrip() {
        let key = Fernet::generate_key();
        assert_eq!(key.len(), 44);
        let f = Fernet::new(&key).expect("fresh key");
        let pt = b"\xf0\x9f\x94\x91 binary \x00\xff payload";
        assert_eq!(f.decrypt(&f.encrypt(pt)).expect("roundtrip"), pt);
    }

    #[test]
    fn rejects_tampered_token_and_bad_keys() {
        let f = Fernet::new(PY_KEY).expect("valid key");
        let mut bad = PY_TOKEN.to_string();
        bad.replace_range(10..11, "A");
        assert!(f.decrypt(&bad).is_err());
        assert!(f.decrypt("not-base64!!!").is_err());
        assert!(Fernet::new("too-short").is_none());
        assert!(Fernet::new(&"A".repeat(44)).is_none()); // 33 bytes, not 32
    }
}
