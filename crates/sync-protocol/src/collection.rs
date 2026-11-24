//! Syncable collections and canonical AEAD associated data (AAD) construction.

use serde::{Deserialize, Serialize};

use crate::{ProtocolError, ProtocolResult};

/// Data collections that each have their own encryption key wrapped under `account_key`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Collection {
    Notes,
    Tasks,
    Events,
    Habits,
    Focus,
    Projects,
    Settings,
    Vault,
}

impl Collection {
    pub const ALL: [Self; 8] = [
        Self::Notes,
        Self::Tasks,
        Self::Events,
        Self::Habits,
        Self::Focus,
        Self::Projects,
        Self::Settings,
        Self::Vault,
    ];

    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::Notes => "notes",
            Self::Tasks => "tasks",
            Self::Events => "events",
            Self::Habits => "habits",
            Self::Focus => "focus",
            Self::Projects => "projects",
            Self::Settings => "settings",
            Self::Vault => "vault",
        }
    }

    #[must_use]
    pub fn from_code(code: &str) -> Option<Self> {
        match code {
            "notes" => Some(Self::Notes),
            "tasks" => Some(Self::Tasks),
            "events" => Some(Self::Events),
            "habits" => Some(Self::Habits),
            "focus" => Some(Self::Focus),
            "projects" => Some(Self::Projects),
            "settings" => Some(Self::Settings),
            "vault" => Some(Self::Vault),
            _ => None,
        }
    }
}

/// Validates a safe ASCII identifier used in AADs, HLCs, and wire metadata.
pub(crate) fn validate_ident(
    value: &str,
    field: &'static str,
    max_len: usize,
) -> ProtocolResult<()> {
    if value.is_empty()
        || value.len() > max_len
        || !value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
    {
        return Err(ProtocolError::InvalidIdentifier(field));
    }
    Ok(())
}

/// Builds the canonical AEAD associated data (AAD) for a synced item payload:
/// `ottero-sync-item-v1|{account_id}|{collection}|{item_id}|{schema_version}`.
///
/// Rejects separator characters (`|`) and control bytes in `account_id` or
/// `item_id` so two distinct items can never share an AAD.
pub fn item_aad(
    account_id: &str,
    collection: Collection,
    item_id: &str,
    schema_version: u16,
) -> ProtocolResult<Vec<u8>> {
    validate_ident(account_id, "account_id", 128)?;
    validate_ident(item_id, "item_id", 128)?;
    Ok(format!(
        "ottero-sync-item-v1|{account_id}|{}|{item_id}|{schema_version}",
        collection.code()
    )
    .into_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collections_round_trip_codes_and_json() {
        for c in Collection::ALL {
            assert_eq!(Collection::from_code(c.code()), Some(c));
            let json = serde_json::to_string(&c).expect("json");
            let back: Collection = serde_json::from_str(&json).expect("back");
            assert_eq!(back, c);
        }
        assert_eq!(Collection::from_code("unknown"), None);
    }

    #[test]
    fn item_aad_is_canonical_and_rejects_separator_injection() {
        let aad = item_aad("acct_123", Collection::Notes, "0195-note-1", 1).expect("aad");
        assert_eq!(aad, b"ottero-sync-item-v1|acct_123|notes|0195-note-1|1");
        assert_eq!(
            item_aad("acct|evil", Collection::Notes, "id", 1),
            Err(ProtocolError::InvalidIdentifier("account_id"))
        );
        assert_eq!(
            item_aad("acct", Collection::Notes, "id|1", 1),
            Err(ProtocolError::InvalidIdentifier("item_id"))
        );
        assert_eq!(
            item_aad("acct", Collection::Notes, "", 1),
            Err(ProtocolError::InvalidIdentifier("item_id"))
        );
    }
}
