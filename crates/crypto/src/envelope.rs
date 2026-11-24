//! Versioned authenticated-encryption envelope.
//!
//! Layout (all lengths in bytes):
//!
//! ```text
//! | magic "KS" (2) | version (1) | nonce (24) | ciphertext + Poly1305 tag (n + 16) |
//! ```
//!
//! The header (magic + version) is also fed in as associated data, so changing
//! it breaks authentication. Callers add their own associated data (for
//! example the record ID) so a ciphertext can't be moved to another record.

use chacha20poly1305::aead::{AeadInOut, KeyInit};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use zeroize::Zeroizing;

use crate::{CryptoError, CryptoResult, SecretKey, random_bytes};

const MAGIC: [u8; 2] = *b"KS";
const VERSION_1: u8 = 1;
const HEADER_LEN: usize = MAGIC.len() + 1;
const NONCE_LEN: usize = 24;
const TAG_LEN: usize = 16;

/// Bytes `seal` adds on top of the plaintext length.
pub const ENVELOPE_OVERHEAD: usize = HEADER_LEN + NONCE_LEN + TAG_LEN;

/// Encrypts `plaintext`, binding it to `aad`. A fresh random nonce is used
/// every call, which is safe for XChaCha20's 192-bit nonce space.
pub fn seal(key: &SecretKey, plaintext: &[u8], aad: &[u8]) -> CryptoResult<Vec<u8>> {
    let mut nonce = [0u8; NONCE_LEN];
    random_bytes(&mut nonce)?;

    let mut out = Vec::with_capacity(plaintext.len() + ENVELOPE_OVERHEAD);
    out.extend_from_slice(&MAGIC);
    out.push(VERSION_1);
    out.extend_from_slice(&nonce);

    let mut buffer = Zeroizing::new(Vec::with_capacity(plaintext.len() + TAG_LEN));
    buffer.extend_from_slice(plaintext);
    cipher(key)?
        .encrypt_in_place(
            &XNonce::from(nonce),
            &full_aad(&out[..HEADER_LEN], aad),
            &mut *buffer,
        )
        .map_err(|_| CryptoError::Encrypt)?;
    out.extend_from_slice(&buffer);
    Ok(out)
}

/// Decrypts an envelope produced by [`seal`]. Fails with [`CryptoError::Decrypt`]
/// for a wrong key, mismatched `aad`, or any tampering.
pub fn open(key: &SecretKey, sealed: &[u8], aad: &[u8]) -> CryptoResult<Zeroizing<Vec<u8>>> {
    if sealed.len() < ENVELOPE_OVERHEAD || sealed[..MAGIC.len()] != MAGIC {
        return Err(CryptoError::Format);
    }
    if sealed[MAGIC.len()] != VERSION_1 {
        return Err(CryptoError::Format);
    }
    let (header, rest) = sealed.split_at(HEADER_LEN);
    let (nonce, ciphertext) = rest.split_at(NONCE_LEN);
    let nonce: [u8; NONCE_LEN] = nonce.try_into().map_err(|_| CryptoError::Format)?;

    let mut buffer = Zeroizing::new(ciphertext.to_vec());
    cipher(key)
        .map_err(|_| CryptoError::Decrypt)?
        .decrypt_in_place(&XNonce::from(nonce), &full_aad(header, aad), &mut *buffer)
        .map_err(|_| CryptoError::Decrypt)?;
    Ok(buffer)
}

fn cipher(key: &SecretKey) -> CryptoResult<XChaCha20Poly1305> {
    // new_from_slice reads the key in place, so no extra copy needs wiping.
    XChaCha20Poly1305::new_from_slice(key.expose()).map_err(|_| CryptoError::Encrypt)
}

/// Header || caller AAD, length-prefixed so (header, aad) pairs can't collide.
fn full_aad(header: &[u8], aad: &[u8]) -> Vec<u8> {
    let mut v = Vec::with_capacity(header.len() + 8 + aad.len());
    v.extend_from_slice(header);
    v.extend_from_slice(&(aad.len() as u64).to_le_bytes());
    v.extend_from_slice(aad);
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key() -> SecretKey {
        SecretKey::generate().expect("key")
    }

    #[test]
    fn round_trip() {
        let k = key();
        let sealed = seal(&k, b"hunter2", b"item-1").expect("seal");
        assert_eq!(sealed.len(), 7 + ENVELOPE_OVERHEAD);
        assert_eq!(&*open(&k, &sealed, b"item-1").expect("open"), b"hunter2");
    }

    #[test]
    fn empty_plaintext_round_trips() {
        let k = key();
        let sealed = seal(&k, b"", b"").expect("seal");
        assert!(open(&k, &sealed, b"").expect("open").is_empty());
    }

    #[test]
    fn nonces_are_unique_per_message() {
        let k = key();
        let a = seal(&k, b"same", b"").expect("seal");
        let b = seal(&k, b"same", b"").expect("seal");
        assert_ne!(a, b);
        assert_ne!(
            a[HEADER_LEN..HEADER_LEN + NONCE_LEN],
            b[HEADER_LEN..HEADER_LEN + NONCE_LEN]
        );
    }

    #[test]
    fn wrong_key_fails() {
        let sealed = seal(&key(), b"secret", b"").expect("seal");
        assert_eq!(
            open(&key(), &sealed, b"").map(|_| ()),
            Err(CryptoError::Decrypt)
        );
    }

    #[test]
    fn wrong_aad_fails() {
        let k = key();
        let sealed = seal(&k, b"secret", b"item-1").expect("seal");
        assert_eq!(
            open(&k, &sealed, b"item-2").map(|_| ()),
            Err(CryptoError::Decrypt)
        );
    }

    #[test]
    fn any_flipped_bit_fails() {
        let k = key();
        let sealed = seal(&k, b"secret", b"aad").expect("seal");
        for i in 0..sealed.len() {
            let mut damaged = sealed.clone();
            damaged[i] ^= 0x01;
            assert!(
                open(&k, &damaged, b"aad").is_err(),
                "byte {i} tamper not detected"
            );
        }
    }

    #[test]
    fn truncated_or_foreign_data_is_a_format_error() {
        let k = key();
        assert_eq!(open(&k, b"", b"").map(|_| ()), Err(CryptoError::Format));
        assert_eq!(
            open(&k, &[0u8; ENVELOPE_OVERHEAD], b"").map(|_| ()),
            Err(CryptoError::Format)
        );
        let mut future = seal(&k, b"x", b"").expect("seal");
        future[2] = 99;
        assert_eq!(open(&k, &future, b"").map(|_| ()), Err(CryptoError::Format));
    }
}
