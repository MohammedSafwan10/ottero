//! Cryptographic primitives for the Ottero vault.
//!
//! This crate wraps audited RustCrypto implementations. It does not invent
//! algorithms. Read `crates/crypto/AGENTS.md` before changing anything here.
//!
//! * Key derivation: Argon2id v1.3 ([`derive_key`]).
//! * Encryption: XChaCha20-Poly1305 with a random 24-byte nonce per message
//!   ([`seal`] / [`open`]), wrapped in a versioned envelope.
//! * Ottero Cloud account keys: Argon2id + Secret Key + HKDF ([`account`]).
//! * The vault key wrapped by the account key ([`vault_link`], ADR 0014).
//! * All key material lives in types that zero their memory on drop and never
//!   print their contents.

pub mod account;
mod envelope;
mod error;
mod kdf;
mod key;
pub mod sync;
pub mod vault_link;

pub use envelope::{ENVELOPE_OVERHEAD, open, seal};
pub use error::{CryptoError, CryptoResult};
pub use kdf::{KdfParams, Salt, derive_key};
pub use key::SecretKey;
pub use secrecy::{ExposeSecret, SecretString};
pub use zeroize::{Zeroize, Zeroizing};

/// Fills `buf` with bytes from the operating system's CSPRNG.
pub fn random_bytes(buf: &mut [u8]) -> CryptoResult<()> {
    getrandom::fill(buf).map_err(|_| CryptoError::Random)
}

/// A uniform random integer in `0..n` (`n > 0`), by rejection sampling so
/// no value is more likely than another (no modulo bias).
pub fn random_below(n: u32) -> CryptoResult<u32> {
    if n <= 1 {
        return Ok(0);
    }
    // Largest multiple of n that fits; draws at or above it are retried.
    let zone = u32::MAX - (u32::MAX % n);
    loop {
        let mut buf = [0u8; 4];
        random_bytes(&mut buf)?;
        let v = u32::from_le_bytes(buf);
        if v < zone {
            return Ok(v % n);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn random_below_stays_in_range_and_covers_it() {
        let mut seen = [false; 7];
        for _ in 0..2_000 {
            let v = random_below(7).expect("rng");
            assert!(v < 7);
            seen[v as usize] = true;
        }
        assert!(seen.iter().all(|s| *s), "every value appears");
        assert_eq!(random_below(1).expect("rng"), 0);
        assert_eq!(random_below(0).expect("rng"), 0);
    }
}
