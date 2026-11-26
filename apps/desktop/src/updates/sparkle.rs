//! UI-free Swift/Sparkle bridge. All installation work stays inside Sparkle;
//! stdout carries bounded status records and stdin carries explicit commands.
use anyhow::{Context as _, Result};
use gpui_kit::{App, Global};
use serde::Deserialize;
use std::io::{BufRead as _, Read as _, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Command, Stdio};

struct Session {
    _process: Child,
    input: ChildStdin,
}
impl Global for Session {}

#[derive(Debug, Deserialize)]
struct Event {
    event: String,
    #[serde(default)]
    version: String,
    #[serde(default)]
    message: String,
}

pub fn bundled_app() -> Option<PathBuf> {
    if cfg!(debug_assertions) {
        return None;
    }
    let exe = std::env::current_exe().ok()?;
    exe.ancestors()
        .find(|p| p.extension().is_some_and(|e| e == "app"))
        .map(|p| p.to_path_buf())
}

pub fn check(feed: &ottero_core::update::Feed, cx: &mut App) -> Result<bool> {
    let Some(app) = bundled_app() else {
        return Ok(false);
    };
    if !cx.has_global::<Session>() {
        let mut child = Command::new(
            app.join("Contents/Helpers/OtteroUpdater.app/Contents/MacOS/OtteroUpdater"),
        )
        .arg(&app)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .context("The native updater is missing. Reinstall Ottero.")?;
        let input = child.stdin.take().context("updater input")?;
        let output = child.stdout.take().context("updater output")?;
        cx.set_global(Session {
            _process: child,
            input,
        });
        let (tx, rx) = async_channel::bounded(64);
        cx.background_executor()
            .spawn(async move {
                let mut reader = std::io::BufReader::new(output);
                loop {
                    let mut line = Vec::new();
                    match reader
                        .by_ref()
                        .take(64 * 1024 + 1)
                        .read_until(b'\n', &mut line)
                    {
                        Ok(0) | Err(_) => break,
                        Ok(_) if line.len() > 64 * 1024 => break,
                        Ok(_) => {
                            if let Ok(event) = serde_json::from_slice::<Event>(&line)
                                && tx.send_blocking(event).is_err()
                            {
                                break;
                            }
                        }
                    }
                }
            })
            .detach();
        cx.spawn(async move |cx| {
            while let Ok(event) = rx.recv().await {
                cx.update(|cx| {
                    use super::Status;
                    let version = event.version;
                    let status = match event.event.as_str() {
                        "initialized" | "checking" => Status::Checking,
                        "found" | "downloading" => Status::Downloading { version },
                        "verifying" => Status::Verifying { version },
                        "ready" if ottero_core::update::parse_version(&version).is_some() => {
                            Status::Ready {
                                version,
                                notes: Vec::new(),
                            }
                        }
                        "up_to_date" => Status::UpToDate,
                        "installing" => Status::Installing { version },
                        "failed" => {
                            let dir = cx.global::<super::Updates>().dir.clone();
                            super::record_failure(&dir, ottero_core::clock::now(), cx);
                            Status::Failed {
                                message: if event.message.is_empty() {
                                    "The native update failed. Try again.".into()
                                } else {
                                    event.message
                                },
                            }
                        }
                        _ => return,
                    };
                    super::set_status(status, cx);
                });
            }
            cx.update(|cx| {
                cx.remove_global::<Session>();
                if matches!(
                    super::status(cx),
                    super::Status::Checking
                        | super::Status::Downloading { .. }
                        | super::Status::Verifying { .. }
                ) {
                    super::set_status(
                        super::Status::Failed {
                            message: "The native updater stopped. Try again.".into(),
                        },
                        cx,
                    );
                }
            });
        })
        .detach();
    }
    command(
        &format!(
            "check https://github.com/MohammedSafwan10/ottero-releases/releases/download/v{}/appcast.xml {}",
            feed.version, feed.version
        ),
        cx,
    )?;
    Ok(true)
}

fn command(command: &str, cx: &mut App) -> Result<()> {
    let session = cx.global_mut::<Session>();
    writeln!(session.input, "{command}")?;
    session.input.flush()?;
    Ok(())
}

pub fn restart(cx: &mut App) -> Result<()> {
    command("install", cx)
}
