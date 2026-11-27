//! Verified update-feed checks for direct Android APK builds.
//!
//! Reuses `ottero_core::update`'s exact minisign public key, release redirect
//! pinning, and payload validation rules so Android and Desktop share identical
//! cryptographic update guarantees.

use std::time::Duration;

use ottero_core::update::{
    Feed, MAX_FEED_BYTES, RELEASE_KEY, RELEASES_LATEST_URL, feed_url_for_release,
};
use ureq::ResponseExt as _;

use crate::dto::MobileUpdateInfo;
use crate::error::MobileError;

const FEED_TIMEOUT: Duration = Duration::from_secs(30);
const ANDROID_PLATFORM: &str = "android-apk";

/// Verifies a `stable.json` + `stable.json.minisig` pair fetched by Android's
/// native OkHttp stack against Ottero's pinned `RELEASE_KEY` and release URL.
pub(crate) fn verify_update_feed(
    location: &str,
    feed_bytes: &[u8],
    signature: &str,
) -> Result<Option<MobileUpdateInfo>, MobileError> {
    if feed_url_for_release(location).is_none() {
        return Err(MobileError::Storage {
            reason: "Invalid release redirect location.".into(),
        });
    }
    if feed_bytes.len() > MAX_FEED_BYTES {
        return Err(MobileError::Storage {
            reason: "Update metadata exceeds size limit.".into(),
        });
    }
    let feed =
        Feed::verified(feed_bytes, signature, RELEASE_KEY).map_err(|_| MobileError::Storage {
            reason: "Update signature verification failed.".into(),
        })?;
    let current = env!("CARGO_PKG_VERSION");
    let Some(download) = feed.newer_for(current, ANDROID_PLATFORM).cloned() else {
        return Ok(None);
    };
    Ok(Some(MobileUpdateInfo {
        version: feed.version,
        date: feed.date,
        notes: feed.notes,
        download_url: download.url,
        sha256: download.sha256,
        size_bytes: download.size,
    }))
}

fn update_agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(FEED_TIMEOUT))
        .timeout_connect(Some(Duration::from_secs(15)))
        .timeout_recv_response(Some(Duration::from_secs(30)))
        .https_only(true)
        .user_agent(concat!("Ottero/", env!("CARGO_PKG_VERSION"), " (Android)"))
        .build()
        .into()
}

fn fetch_bytes(agent: &ureq::Agent, url: &str, limit: u64) -> Result<Vec<u8>, MobileError> {
    agent
        .get(url)
        .call()
        .map_err(|_| MobileError::Storage {
            reason: "Couldn't check for updates. Check your connection.".into(),
        })?
        .body_mut()
        .with_config()
        .limit(limit)
        .read_to_vec()
        .map_err(|_| MobileError::Storage {
            reason: "Couldn't read update metadata.".into(),
        })
}

/// Checks `ottero-releases` for a signed release newer than this build.
pub(crate) fn check_for_update() -> Result<Option<MobileUpdateInfo>, MobileError> {
    let agent = update_agent();
    let response = agent
        .head(RELEASES_LATEST_URL)
        .call()
        .map_err(|_| MobileError::Storage {
            reason: "Couldn't check for updates. Check your connection.".into(),
        })?;
    let location = response.get_uri().to_string();
    let feed_url = feed_url_for_release(&location).ok_or_else(|| MobileError::Storage {
        reason: "Invalid release redirect location.".into(),
    })?;
    let bytes = fetch_bytes(&agent, &feed_url, MAX_FEED_BYTES as u64)?;
    let sig_bytes = fetch_bytes(&agent, &format!("{feed_url}.minisig"), 4096)?;
    let signature = String::from_utf8(sig_bytes).map_err(|_| MobileError::Storage {
        reason: "Invalid update signature encoding.".into(),
    })?;
    verify_update_feed(&location, &bytes, &signature)
}
