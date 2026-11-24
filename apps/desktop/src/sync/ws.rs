//! Live WebSocket change-notice listener for Ottero Cloud sync (`/v1/sync/ws`).
//!
//! Runs on a dedicated OS thread with automatic exponential-backoff reconnects,
//! tight frame limits, and ping/pong liveness detection. Dropping
//! [`SyncWsHandle`] signals the thread to exit promptly.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use ottero_crypto::Zeroizing;
use ottero_sync_protocol::{WsClientMessage, WsServerMessage};
use tungstenite::client::IntoClientRequest;
use tungstenite::http::HeaderValue;
use tungstenite::protocol::WebSocketConfig;
use tungstenite::stream::MaybeTlsStream;
use tungstenite::{Error as WsError, Message};
use url::Url;

const INITIAL_BACKOFF: Duration = Duration::from_secs(2);
const MAX_BACKOFF: Duration = Duration::from_secs(30);
const READ_TIMEOUT: Duration = Duration::from_secs(2);
const PING_INTERVAL: Duration = Duration::from_secs(25);
const PONG_TIMEOUT: Duration = Duration::from_secs(10);
const STOP_POLL: Duration = Duration::from_millis(250);
const MAX_WS_FRAME_BYTES: usize = 16 * 1024;

/// Notices forwarded from the sync WebSocket thread to [`crate::library::Library`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WsNotice {
    SeqAdvanced(i64),
    DeviceRevoked,
    EntitlementChanged(bool),
    AuthRejected,
}

/// Handle to a running sync WebSocket listener. Dropping it stops the thread.
pub struct SyncWsHandle {
    stop: Arc<AtomicBool>,
}

impl SyncWsHandle {
    pub fn stop(&self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

impl Drop for SyncWsHandle {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Converts and validates an HTTP(S) sync base URL into the corresponding
/// WebSocket URL. Requires `https://` (`wss://`) except for loopback hosts
/// (`127.0.0.1`, `localhost`, `::1`) where `http://` (`ws://`) is permitted.
#[must_use]
pub fn ws_url_for(sync_base_url: &str) -> Option<String> {
    let parsed = Url::parse(sync_base_url.trim()).ok()?;
    let is_local = matches!(parsed.host_str(), Some("127.0.0.1" | "localhost" | "::1"));
    let ws_scheme = match parsed.scheme() {
        "https" => "wss",
        "http" if is_local => "ws",
        _ => return None,
    };
    let mut ws = parsed;
    ws.set_scheme(ws_scheme).ok()?;
    let base = ws.as_str().trim_end_matches('/');
    Some(format!("{base}/v1/sync/ws"))
}

/// Spawns the background WebSocket listener. Returns `None` if `sync_base_url`
/// fails scheme/host validation.
pub fn spawn_ws_listener(
    sync_base_url: &str,
    bearer_token: Zeroizing<String>,
    notify: async_channel::Sender<WsNotice>,
) -> Option<SyncWsHandle> {
    let url = ws_url_for(sync_base_url)?;
    let stop = Arc::new(AtomicBool::new(false));
    let thread_stop = Arc::clone(&stop);
    let _ = thread::Builder::new()
        .name("ottero-sync-ws".into())
        .spawn(move || run_loop(&url, &bearer_token, &notify, &thread_stop));
    Some(SyncWsHandle { stop })
}

fn run_loop(
    url: &str,
    bearer_token: &Zeroizing<String>,
    notify: &async_channel::Sender<WsNotice>,
    stop: &AtomicBool,
) {
    let mut backoff = INITIAL_BACKOFF;
    let config = WebSocketConfig::default()
        .max_message_size(Some(MAX_WS_FRAME_BYTES))
        .max_frame_size(Some(MAX_WS_FRAME_BYTES));

    while !stop.load(Ordering::Relaxed) && !notify.is_closed() {
        let Ok(mut req) = url.into_client_request() else {
            return;
        };
        let Ok(auth) = HeaderValue::from_str(&format!("Bearer {}", bearer_token.as_str())) else {
            return;
        };
        req.headers_mut().insert("Authorization", auth);

        match tungstenite::client::connect_with_config(req, Some(config), 3) {
            Ok((mut socket, _)) => {
                backoff = INITIAL_BACKOFF;
                set_read_timeout(socket.get_ref(), Some(READ_TIMEOUT));
                let mut last_activity = Instant::now();
                let mut awaiting_pong: Option<Instant> = None;
                let mut terminal = false;

                while !stop.load(Ordering::Relaxed) && !notify.is_closed() {
                    if let Some(sent_at) = awaiting_pong {
                        if sent_at.elapsed() >= PONG_TIMEOUT {
                            break; // half-open socket; reconnect
                        }
                    } else if last_activity.elapsed() >= PING_INTERVAL
                        && let Ok(ping) = serde_json::to_string(&WsClientMessage::Ping)
                    {
                        if socket.send(Message::Text(ping.into())).is_err() {
                            break;
                        }
                        awaiting_pong = Some(Instant::now());
                    }

                    match socket.read() {
                        Ok(Message::Text(txt)) => {
                            last_activity = Instant::now();
                            if let Ok(msg) = serde_json::from_str::<WsServerMessage>(&txt) {
                                match msg {
                                    WsServerMessage::SeqAdvanced { latest_seq } => {
                                        let _ = notify.try_send(WsNotice::SeqAdvanced(latest_seq));
                                    }
                                    WsServerMessage::DeviceRevoked => {
                                        let _ = notify.try_send(WsNotice::DeviceRevoked);
                                        terminal = true;
                                        break;
                                    }
                                    WsServerMessage::EntitlementChanged { active } => {
                                        let _ =
                                            notify.try_send(WsNotice::EntitlementChanged(active));
                                        if !active {
                                            terminal = true;
                                            break;
                                        }
                                    }
                                    WsServerMessage::Pong => {
                                        awaiting_pong = None;
                                    }
                                }
                            }
                        }
                        Ok(Message::Ping(payload)) => {
                            last_activity = Instant::now();
                            let _ = socket.send(Message::Pong(payload));
                        }
                        Ok(Message::Pong(_)) => {
                            last_activity = Instant::now();
                            awaiting_pong = None;
                        }
                        Ok(Message::Close(_)) => break,
                        Ok(_) => {}
                        Err(WsError::Io(ref err))
                            if matches!(
                                err.kind(),
                                std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                            ) =>
                        {
                            // Timeout so we can re-check `stop` and ping liveness.
                        }
                        Err(_) => break,
                    }
                }
                let _ = socket.close(None);
                if terminal {
                    return;
                }
            }
            Err(WsError::Http(resp)) => {
                let code = resp.status().as_u16();
                if code == 401 || code == 403 {
                    let _ = notify.try_send(WsNotice::AuthRejected);
                    return;
                }
                if code == 402 {
                    let _ = notify.try_send(WsNotice::EntitlementChanged(false));
                    return;
                }
            }
            Err(_) => {
                // Network offline or server restarting; fall through to backoff.
            }
        }

        sleep_interruptible(backoff, stop);
        backoff = (backoff * 2).min(MAX_BACKOFF);
    }
}

fn set_read_timeout(stream: &MaybeTlsStream<std::net::TcpStream>, timeout: Option<Duration>) {
    match stream {
        MaybeTlsStream::Plain(tcp) => {
            let _ = tcp.set_read_timeout(timeout);
        }
        MaybeTlsStream::Rustls(tls) => {
            let _ = tls.get_ref().set_read_timeout(timeout);
        }
        _ => {}
    }
}

fn sleep_interruptible(total: Duration, stop: &AtomicBool) {
    let mut remaining = total;
    while !remaining.is_zero() && !stop.load(Ordering::Relaxed) {
        let step = remaining.min(STOP_POLL);
        thread::sleep(step);
        remaining = remaining.saturating_sub(step);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_and_validates_sync_urls_to_ws() {
        assert_eq!(
            ws_url_for("https://sync.nexdark.com").as_deref(),
            Some("wss://sync.nexdark.com/v1/sync/ws")
        );
        assert_eq!(
            ws_url_for("https://sync.nexdark.com/").as_deref(),
            Some("wss://sync.nexdark.com/v1/sync/ws")
        );
        assert_eq!(
            ws_url_for("http://127.0.0.1:8788").as_deref(),
            Some("ws://127.0.0.1:8788/v1/sync/ws")
        );
        assert_eq!(
            ws_url_for("http://localhost:8788").as_deref(),
            Some("ws://localhost:8788/v1/sync/ws")
        );
        assert_eq!(
            ws_url_for("http://sync.nexdark.com"),
            None,
            "reject non-loopback plaintext http/ws"
        );
        assert_eq!(ws_url_for("ftp://example.com"), None);
    }
}
