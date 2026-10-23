//! mpv JSON IPC: a named pipe on Windows, a unix socket elsewhere.

use std::sync::Arc;
use std::time::Duration;

use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, WriteHalf};
use tokio::sync::{mpsc, Mutex};

#[cfg(windows)]
type Stream = tokio::net::windows::named_pipe::NamedPipeClient;
#[cfg(unix)]
type Stream = tokio::net::UnixStream;

async fn open(path: &str) -> std::io::Result<Stream> {
    #[cfg(windows)]
    return tokio::net::windows::named_pipe::ClientOptions::new().open(path);
    #[cfg(unix)]
    return tokio::net::UnixStream::connect(path).await;
}

#[derive(Debug, Clone, PartialEq)]
pub enum MpvEvent {
    Property { name: String, data: Value },
    EndFile { reason: String },
    ClientMessage { args: Vec<String> },
    FileLoaded,
    Shutdown,
    Other(String),
}

/// Parses one line from mpv. Command replies (`request_id`) are ignored.
pub fn parse_line(line: &str) -> Option<MpvEvent> {
    let v: Value = serde_json::from_str(line).ok()?;
    let event = v.get("event")?.as_str()?;
    Some(match event {
        "property-change" => MpvEvent::Property {
            name: v.get("name")?.as_str()?.to_string(),
            data: v.get("data").cloned().unwrap_or(Value::Null),
        },
        "end-file" => MpvEvent::EndFile {
            reason: v
                .get("reason")
                .and_then(|r| r.as_str())
                .unwrap_or("")
                .to_string(),
        },
        "client-message" => MpvEvent::ClientMessage {
            args: v
                .get("args")
                .and_then(|a| a.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|x| x.as_str().map(str::to_string))
                        .collect()
                })
                .unwrap_or_default(),
        },
        "file-loaded" => MpvEvent::FileLoaded,
        "shutdown" => MpvEvent::Shutdown,
        other => MpvEvent::Other(other.to_string()),
    })
}

pub struct Ipc {
    writer: Mutex<WriteHalf<Stream>>,
}

impl Ipc {
    /// Connects to mpv's pipe, retrying while mpv starts up. Events arrive on the returned
    /// channel; `Shutdown` is sent when the pipe closes.
    pub async fn connect(
        pipe: &str,
        timeout: Duration,
    ) -> std::io::Result<(Arc<Ipc>, mpsc::UnboundedReceiver<MpvEvent>)> {
        let deadline = tokio::time::Instant::now() + timeout;
        let client = loop {
            match open(pipe).await {
                Ok(c) => break c,
                Err(e) if tokio::time::Instant::now() < deadline => {
                    tracing::debug!("waiting for mpv pipe: {e}");
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
                Err(e) => return Err(e),
            }
        };
        let (read, write) = tokio::io::split(client);
        let (tx, rx) = mpsc::unbounded_channel();
        tokio::spawn(async move {
            let mut lines = BufReader::new(read).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                if let Some(ev) = parse_line(&line) {
                    if tx.send(ev).is_err() {
                        return;
                    }
                }
            }
            let _ = tx.send(MpvEvent::Shutdown);
        });
        Ok((
            Arc::new(Ipc {
                writer: Mutex::new(write),
            }),
            rx,
        ))
    }

    /// Sends a command given as a JSON array, e.g. `["loadfile", url]`.
    pub async fn command(&self, args: Value) -> std::io::Result<()> {
        let mut line = json!({ "command": args }).to_string();
        line.push('\n');
        let mut w = self.writer.lock().await;
        w.write_all(line.as_bytes()).await?;
        w.flush().await
    }

    pub async fn observe(&self, id: u64, name: &str) -> std::io::Result<()> {
        self.command(json!(["observe_property", id, name])).await
    }
}

/// For tests that drive a real mpv.
#[cfg(test)]
pub(crate) mod testing {
    /// The bundled mpv on Windows (scripts/fetch-mpv.ps1), the system one elsewhere.
    pub fn mpv_path() -> std::path::PathBuf {
        if cfg!(windows) {
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("binaries/mpv-x86_64-pc-windows-msvc.exe")
        } else {
            "mpv".into()
        }
    }

    /// An IPC endpoint of this test run: a named pipe on Windows, a socket elsewhere.
    pub fn ipc_path(tag: &str) -> String {
        let name = format!("ninari-{tag}-{}", std::process::id());
        if cfg!(windows) {
            format!(r"\\.\pipe\{name}")
        } else {
            std::env::temp_dir()
                .join(format!("{name}.sock"))
                .to_string_lossy()
                .into_owned()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::testing::{ipc_path, mpv_path};
    use super::*;

    #[test]
    fn parses_events() {
        assert_eq!(
            parse_line(r#"{"event":"property-change","id":1,"name":"time-pos","data":12.5}"#),
            Some(MpvEvent::Property {
                name: "time-pos".into(),
                data: serde_json::json!(12.5)
            })
        );
        assert_eq!(
            parse_line(r#"{"event":"end-file","reason":"eof","playlist_entry_id":1}"#),
            Some(MpvEvent::EndFile {
                reason: "eof".into()
            })
        );
        assert_eq!(
            parse_line(r#"{"event":"client-message","args":["ninari-next"]}"#),
            Some(MpvEvent::ClientMessage {
                args: vec!["ninari-next".into()]
            })
        );
        assert_eq!(
            parse_line(r#"{"event":"file-loaded"}"#),
            Some(MpvEvent::FileLoaded)
        );
        assert_eq!(
            parse_line(r#"{"event":"shutdown"}"#),
            Some(MpvEvent::Shutdown)
        );
        assert_eq!(
            parse_line(r#"{"request_id":3,"error":"success","data":null}"#),
            None
        );
        assert_eq!(parse_line("not json"), None);
    }

    /// Drives a real mpv over the named pipe: a 1 s generated clip must report progress and end with eof.
    #[tokio::test]
    #[ignore = "needs mpv: scripts/fetch-mpv.ps1 on Windows, the system mpv elsewhere"]
    async fn real_mpv_reports_progress_and_eof() {
        let pipe = ipc_path("test");
        let mut child = std::process::Command::new(mpv_path())
            .args([
                "--idle=yes",
                "--no-config",
                "--terminal=no",
                "--vo=null",
                "--ao=null",
            ])
            .arg(format!("--input-ipc-server={pipe}"))
            .spawn()
            .unwrap();
        let (ipc, mut events) = Ipc::connect(&pipe, std::time::Duration::from_secs(10))
            .await
            .unwrap();
        ipc.observe(1, "time-pos").await.unwrap();
        ipc.command(serde_json::json!([
            "loadfile",
            "av://lavfi:testsrc=duration=1:size=64x64:rate=10"
        ]))
        .await
        .unwrap();
        let mut saw_time = false;
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(15);
        loop {
            let ev = tokio::time::timeout_at(deadline, events.recv())
                .await
                .expect("timed out")
                .expect("closed");
            match ev {
                MpvEvent::Property { name, data } if name == "time-pos" && data.is_number() => {
                    saw_time = true
                }
                MpvEvent::EndFile { reason } => {
                    assert_eq!(reason, "eof");
                    break;
                }
                _ => {}
            }
        }
        assert!(saw_time);
        ipc.command(serde_json::json!(["quit"])).await.unwrap();
        let _ = child.wait();
    }
}
