//! Per-collection sync key wrapping and item payload sealing (plan 06 §3).
//!
//! ```text
//! wrap_key = HKDF-SHA256(ikm = account_key, salt = none, info = "ottero sync collection wrap v1", 32)
//! wrapped  = seal(wrap_key, collection_key, aad = "ottero sync collection key v1|<collection>")
//! ```
//!
//! Each syncable collection (`notes`, `tasks`, `events`, `habits`, `focus`,
//! `projects`, `settings`, `vault`) gets its own random 256-bit key wrapped
//! under `account_key`. Item payloads are sealed with their collection key and
//! canonical AAD from `ottero-sync-protocol::item_aad`.

use hkdf::Hkdf;
use sha2::Sha256;
use zeroize::Zeroizing;

use crate::{CryptoError, CryptoResult, SecretKey, open, seal};

const INFO_SYNC_WRAP: &[u8] = b"ottero sync collection wrap v1";
const AAD_PREFIX_COLLECTION_KEY: &str = "ottero sync collection key v1|";

fn collection_wrap_key(account_key: &SecretKey) -> CryptoResult<SecretKey> {
    let mut out = Zeroizing::new([0u8; SecretKey::LEN]);
    Hkdf::<Sha256>::new(None, account_key.expose())
        .expand(INFO_SYNC_WRAP, &mut *out)
        .map_err(|_| CryptoError::Kdf)?;
    Ok(SecretKey::from_bytes(&mut out))
}

fn collection_key_aad(collection_code: &str) -> CryptoResult<Vec<u8>> {
    if collection_code.is_empty()
        || !collection_code
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b == b'_')
    {
        return Err(CryptoError::Format);
    }
    Ok(format!("{AAD_PREFIX_COLLECTION_KEY}{collection_code}").into_bytes())
}

/// Wraps a random per-collection sync key under `account_key`.
pub fn wrap_collection_key(
    account_key: &SecretKey,
    collection_code: &str,
    collection_key: &SecretKey,
) -> CryptoResult<Vec<u8>> {
    let wrap_key = collection_wrap_key(account_key)?;
    let aad = collection_key_aad(collection_code)?;
    seal(&wrap_key, collection_key.expose(), &aad)
}

/// Unwraps a per-collection sync key previously created by [`wrap_collection_key`].
pub fn unwrap_collection_key(
    account_key: &SecretKey,
    collection_code: &str,
    wrapped: &[u8],
) -> CryptoResult<SecretKey> {
    let wrap_key = collection_wrap_key(account_key)?;
    let aad = collection_key_aad(collection_code)?;
    let bytes = open(&wrap_key, wrapped, &aad)?;
    if bytes.len() != SecretKey::LEN {
        return Err(CryptoError::Format);
    }
    let mut raw = Zeroizing::new([0u8; SecretKey::LEN]);
    raw.copy_from_slice(&bytes);
    Ok(SecretKey::from_bytes(&mut raw))
}

/// Seals a synced item payload with its collection key and canonical item AAD.
pub fn seal_sync_item(
    collection_key: &SecretKey,
    plaintext: &[u8],
    aad: &[u8],
) -> CryptoResult<Vec<u8>> {
    seal(collection_key, plaintext, aad)
}

/// Opens a synced item payload sealed by [`seal_sync_item`].
pub fn open_sync_item(
    collection_key: &SecretKey,
    ciphertext: &[u8],
    aad: &[u8],
) -> CryptoResult<Zeroizing<Vec<u8>>> {
    open(collection_key, ciphertext, aad)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(start: u8) -> SecretKey {
        let mut raw = [0u8; SecretKey::LEN];
        for (i, b) in raw.iter_mut().enumerate() {
            *b = start.wrapping_add(i as u8);
        }
        SecretKey::from_bytes(&mut raw)
    }

    #[test]
    fn collection_key_round_trips_and_binds_collection_name() {
        let account = key(1);
        let notes_key = key(42);
        let wrapped = wrap_collection_key(&account, "notes", &notes_key).expect("wrap");

        let opened = unwrap_collection_key(&account, "notes", &wrapped).expect("unwrap");
        assert_eq!(opened.expose(), notes_key.expose());

        // Cannot open under a different collection name or account key.
        assert_eq!(
            unwrap_collection_key(&account, "tasks", &wrapped).unwrap_err(),
            CryptoError::Decrypt
        );
        assert_eq!(
            unwrap_collection_key(&key(2), "notes", &wrapped).unwrap_err(),
            CryptoError::Decrypt
        );
        assert_eq!(
            wrap_collection_key(&account, "notes|evil", &notes_key).unwrap_err(),
            CryptoError::Format
        );
    }

    #[test]
    fn sync_item_seal_open_rejects_tampering_and_wrong_aad() {
        let ck = key(7);
        let aad = b"ottero-sync-item-v1|acct_1|notes|n1|1";
        let mut ct = seal_sync_item(&ck, br#"{"title":"Hello"}"#, aad).expect("seal");
        let pt = open_sync_item(&ck, &ct, aad).expect("open");
        assert_eq!(&*pt, br#"{"title":"Hello"}"#);

        assert_eq!(
            open_sync_item(&ck, &ct, b"ottero-sync-item-v1|acct_1|notes|n2|1").unwrap_err(),
            CryptoError::Decrypt
        );
        let last = ct.len() - 1;
        ct[last] ^= 0x01;
        assert_eq!(
            open_sync_item(&ck, &ct, aad).unwrap_err(),
            CryptoError::Decrypt
        );
    }
}
