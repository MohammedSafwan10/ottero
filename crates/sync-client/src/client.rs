//! Blocking HTTPS client for the Ottero Cloud sync server (`sync.nexdark.com`).
//!
//! Only ciphertext envelopes and minimal routing metadata cross the wire
//! (`ottero-sync-protocol`). Never sends plaintext content or encryption keys.

use std::time::Duration;

use ottero_sync_protocol::{
    KeysResponse, PullRequest, PullResponse, PushRequest, PushResponse, PutKeysRequest,
    SyncErrorCode, SyncErrorResponse,
};
use thiserror::Error;
use url::Url;

/// Production sync server base URL.
pub const DEFAULT_SYNC_URL: &str = "https://sync.nexdark.com";

const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_RESPONSE_BYTES: u64 = 16 * 1024 * 1024;

#[derive(Debug, Error)]
pub enum SyncClientError {
    #[error("invalid sync server URL")]
    InvalidUrl,

    #[error("sync request validation failed: {0}")]
    Protocol(#[from] ottero_sync_protocol::ProtocolError),

    #[error("session expired; sign in again")]
    Unauthorized,

    #[error("Ottero Cloud subscription required to sync")]
    EntitlementRequired,

    #[error("sync cursor expired; full resync required")]
    CursorExpired,

    #[error("cloud storage quota exceeded")]
    QuotaExceeded,

    #[error("sync server error ({code:?}): {message}")]
    Server {
        code: SyncErrorCode,
        message: String,
    },

    #[error("network error: {0}")]
    Network(String),

    #[error("invalid response from sync server: {0}")]
    Decode(String),

    #[error("storage error: {0}")]
    Storage(#[from] ottero_storage::StorageError),

    #[error("crypto error: {0}")]
    Crypto(#[from] ottero_crypto::CryptoError),
}

#[derive(Debug, Clone)]
pub struct SyncClient {
    base_url: String,
    agent: ureq::Agent,
}

/// Resolves the sync base URL (allowing `OTTERO_SYNC_URL` in debug builds only).
#[must_use]
pub fn resolve_sync_url() -> String {
    #[cfg(debug_assertions)]
    if let Ok(url) = std::env::var("OTTERO_SYNC_URL")
        && !url.trim().is_empty()
    {
        return url.trim().to_owned();
    }
    DEFAULT_SYNC_URL.to_owned()
}

impl SyncClient {
    /// Creates a client for `base_url`. Requires `https://` except for local
    /// loopback (`127.0.0.1` / `localhost`) in tests and local dev.
    pub fn new(base_url: &str) -> Result<Self, SyncClientError> {
        let parsed = Url::parse(base_url).map_err(|_| SyncClientError::InvalidUrl)?;
        let is_local = matches!(parsed.host_str(), Some("127.0.0.1" | "localhost" | "::1"));
        if parsed.scheme() != "https" && !(parsed.scheme() == "http" && is_local) {
            return Err(SyncClientError::InvalidUrl);
        }
        let config = ureq::Agent::config_builder()
            .timeout_global(Some(REQUEST_TIMEOUT))
            .http_status_as_error(false)
            .build();
        Ok(Self {
            base_url: base_url.trim_end_matches('/').to_owned(),
            agent: config.into(),
        })
    }

    /// Resolves the sync URL (allowing `OTTERO_SYNC_URL` in debug builds).
    pub fn default_client() -> Result<Self, SyncClientError> {
        Self::new(&resolve_sync_url())
    }

    pub fn get_keys(&self, bearer_token: &str) -> Result<KeysResponse, SyncClientError> {
        let url = format!("{}/v1/sync/keys", self.base_url);
        let mut resp = self
            .agent
            .get(&url)
            .header("Authorization", &format!("Bearer {bearer_token}"))
            .call()
            .map_err(|err| SyncClientError::Network(err.to_string()))?;
        parse_json_response(&mut resp)
    }

    pub fn put_keys(
        &self,
        bearer_token: &str,
        req: &PutKeysRequest,
    ) -> Result<KeysResponse, SyncClientError> {
        req.validate()?;
        let url = format!("{}/v1/sync/keys", self.base_url);
        let body =
            serde_json::to_vec(req).map_err(|err| SyncClientError::Decode(err.to_string()))?;
        let mut resp = self
            .agent
            .put(&url)
            .header("Authorization", &format!("Bearer {bearer_token}"))
            .header("Content-Type", "application/json")
            .send(&body)
            .map_err(|err| SyncClientError::Network(err.to_string()))?;
        parse_json_response(&mut resp)
    }

    pub fn push(
        &self,
        bearer_token: &str,
        req: &PushRequest,
    ) -> Result<PushResponse, SyncClientError> {
        req.validate()?;
        let url = format!("{}/v1/sync/push", self.base_url);
        let body =
            serde_json::to_vec(req).map_err(|err| SyncClientError::Decode(err.to_string()))?;
        let mut resp = self
            .agent
            .post(&url)
            .header("Authorization", &format!("Bearer {bearer_token}"))
            .header("Content-Type", "application/json")
            .send(&body)
            .map_err(|err| SyncClientError::Network(err.to_string()))?;
        parse_json_response(&mut resp)
    }

    pub fn pull(
        &self,
        bearer_token: &str,
        req: &PullRequest,
    ) -> Result<PullResponse, SyncClientError> {
        req.validate()?;
        let url = format!(
            "{}/v1/sync/pull?protocol={}&after_seq={}&limit={}",
            self.base_url, req.protocol, req.after_seq, req.limit
        );
        let mut resp = self
            .agent
            .get(&url)
            .header("Authorization", &format!("Bearer {bearer_token}"))
            .call()
            .map_err(|err| SyncClientError::Network(err.to_string()))?;
        parse_json_response(&mut resp)
    }
}

fn parse_json_response<T: serde::de::DeserializeOwned>(
    resp: &mut ureq::http::Response<ureq::Body>,
) -> Result<T, SyncClientError> {
    let status = resp.status().as_u16();
    let bytes = resp
        .body_mut()
        .with_config()
        .limit(MAX_RESPONSE_BYTES)
        .read_to_vec()
        .map_err(|err| SyncClientError::Network(err.to_string()))?;

    if (200..300).contains(&status) {
        return serde_json::from_slice(&bytes)
            .map_err(|err| SyncClientError::Decode(err.to_string()));
    }

    if let Ok(err) = serde_json::from_slice::<SyncErrorResponse>(&bytes) {
        return Err(match err.code {
            SyncErrorCode::Unauthorized => SyncClientError::Unauthorized,
            SyncErrorCode::EntitlementRequired => SyncClientError::EntitlementRequired,
            SyncErrorCode::CursorExpired => SyncClientError::CursorExpired,
            SyncErrorCode::QuotaExceeded => SyncClientError::QuotaExceeded,
            code => SyncClientError::Server {
                code,
                message: err.message,
            },
        });
    }

    Err(match status {
        401 => SyncClientError::Unauthorized,
        402 => SyncClientError::EntitlementRequired,
        410 => SyncClientError::CursorExpired,
        _ => SyncClientError::Network(format!("HTTP {status}")),
    })
}
