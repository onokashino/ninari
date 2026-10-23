//! Syncing the library between devices on one local network.
//!
//! Each device runs a small HTTP server. Two devices pair once by a six-digit code (see `crypto`);
//! after that they send each other the rows changed since the last exchange, sealed with the key
//! of their pair. Every side asks for changes by the other side's own clock, so clocks that
//! disagree lose nothing; rows merge by `Library::apply` (the newer edit wins).

pub mod crypto;
pub mod lan;

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use axum::body::Bytes;
use axum::extract::{DefaultBodyLimit, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};

use crate::db::{self, now_ts, Db};
use crate::error::AppError;
use crate::library::{Changes, Library};
use crypto::{Answer, Host, Joiner, PairError};

pub const DEVICE_HEADER: &str = "x-ninari-device";
pub const ENABLED_SETTING: &str = "devices.enabled";
pub const CHANGED_EVENT: &str = "devices://changed";
const ID_SETTING: &str = "devices.id";
const PAIRING_SECONDS: i64 = 180;
/// A full library with every progress row; the default 2 MB of axum is too little for long lists.
const BODY_LIMIT: usize = 64 * 1024 * 1024;

fn err(message: impl Into<String>) -> AppError {
    AppError::new("Devices", message)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Hello {
    pub id: String,
    pub name: String,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PairHello {
    from: String,
    name: String,
    msg: String,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PairAnswer {
    msg: String,
    proof: String,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PairProof {
    from: String,
    proof: String,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SyncAsk {
    /// Send me your rows changed after this moment of your clock.
    since: i64,
    changes: Changes,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SyncReply {
    /// Your clock when you answered: where to ask from next time.
    now: i64,
    changes: Changes,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Peer {
    pub id: String,
    pub name: String,
    pub last_sync: Option<i64>,
    pub paired_at: i64,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PairingCode {
    pub code: String,
    pub expires_at: i64,
}

/// A code shown on this device. It is spent by the first device that tries it, right or wrong.
struct Pairing {
    code: String,
    expires_at: i64,
    tried: bool,
    pending: Option<(String, String, Host)>,
}

pub struct Devices {
    db: Db,
    pub library: Arc<Library>,
    pub id: String,
    pub name: String,
    http: reqwest::Client,
    pairing: Mutex<Option<Pairing>>,
    on_change: Mutex<Option<Box<dyn Fn() + Send + Sync>>>,
    server: Mutex<Option<tokio::sync::oneshot::Sender<()>>>,
}

impl Devices {
    pub fn new(db: Db, library: Arc<Library>, name: String) -> Arc<Self> {
        let id = {
            let conn = db.lock().unwrap();
            match db::get_setting(&conn, ID_SETTING).ok().flatten() {
                Some(serde_json::Value::String(id)) => id,
                _ => {
                    let id = hex(&rand::random::<[u8; 16]>());
                    let _ = db::set_setting(&conn, ID_SETTING, &serde_json::json!(id));
                    id
                }
            }
        };
        Arc::new(Self {
            db,
            library,
            id,
            name,
            http: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(30))
                .build()
                .expect("an HTTP client with a timeout"),
            pairing: Mutex::new(None),
            on_change: Mutex::new(None),
            server: Mutex::new(None),
        })
    }

    /// Called whenever peers change or a sync brought something in.
    pub fn set_on_change(&self, f: impl Fn() + Send + Sync + 'static) {
        *self.on_change.lock().unwrap() = Some(Box::new(f));
    }

    pub(crate) fn changed(&self) {
        if let Some(f) = self.on_change.lock().unwrap().as_ref() {
            f();
        }
    }

    /// Starts the server on `bind` and returns the address it listens on.
    pub async fn serve(self: &Arc<Self>, bind: SocketAddr) -> Result<SocketAddr, AppError> {
        let listener = tokio::net::TcpListener::bind(bind)
            .await
            .map_err(|e| err(format!("не удалось открыть порт: {e}")))?;
        let addr = listener.local_addr().map_err(|e| err(e.to_string()))?;
        let app = Router::new()
            .route("/hello", get(hello))
            .route("/pair", post(pair_hello))
            .route("/pair/confirm", post(pair_confirm))
            .route("/sync", post(sync))
            .layer(DefaultBodyLimit::max(BODY_LIMIT))
            .with_state(self.clone());
        let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
        tokio::spawn(async move {
            let _ = axum::serve(listener, app)
                .with_graceful_shutdown(async move {
                    let _ = stopped.await;
                })
                .await;
        });
        if let Some(old) = self.server.lock().unwrap().replace(stop) {
            let _ = old.send(());
        }
        Ok(addr)
    }

    pub fn stop_server(&self) {
        if let Some(stop) = self.server.lock().unwrap().take() {
            let _ = stop.send(());
        }
    }

    // ---------- pairing, the side that shows the code ----------

    pub fn start_pairing(&self) -> PairingCode {
        let code = crypto::new_code();
        let expires_at = now_ts() + PAIRING_SECONDS;
        *self.pairing.lock().unwrap() = Some(Pairing {
            code: code.clone(),
            expires_at,
            tried: false,
            pending: None,
        });
        PairingCode { code, expires_at }
    }

    pub fn stop_pairing(&self) {
        *self.pairing.lock().unwrap() = None;
    }

    /// The code on screen, while it can still be used.
    pub fn pairing(&self) -> Option<PairingCode> {
        self.pairing
            .lock()
            .unwrap()
            .as_ref()
            .filter(|p| !p.tried && p.expires_at > now_ts())
            .map(|p| PairingCode {
                code: p.code.clone(),
                expires_at: p.expires_at,
            })
    }

    fn answer(&self, hello: PairHello) -> Result<PairAnswer, (StatusCode, String)> {
        let mut guard = self.pairing.lock().unwrap();
        let pairing = guard.as_mut().filter(|p| p.expires_at > now_ts()).ok_or((
            StatusCode::FORBIDDEN,
            "На этом устройстве не открыт код для связывания".to_string(),
        ))?;
        if pairing.tried {
            return Err((
                StatusCode::FORBIDDEN,
                "Этот код уже использован, покажи новый".to_string(),
            ));
        }
        pairing.tried = true;
        let msg = B64
            .decode(&hello.msg)
            .map_err(|_| (StatusCode::BAD_REQUEST, "битое сообщение".to_string()))?;
        let (host, answer) = Host::answer(&pairing.code, &hello.from, &self.id, &msg)
            .map_err(|_| (StatusCode::BAD_REQUEST, "битое сообщение".to_string()))?;
        pairing.pending = Some((hello.from, hello.name, host));
        Ok(PairAnswer {
            msg: B64.encode(answer.msg),
            proof: B64.encode(answer.proof),
        })
    }

    fn confirm(&self, proof: PairProof) -> Result<(), (StatusCode, String)> {
        let pending = self.pairing.lock().unwrap().take().and_then(|p| p.pending);
        let Some((from, name, host)) = pending.filter(|(from, _, _)| *from == proof.from) else {
            return Err((StatusCode::FORBIDDEN, "Связывание не начато".to_string()));
        };
        let proof = B64.decode(&proof.proof).unwrap_or_default();
        let key = host
            .finish(&proof)
            .map_err(|_| (StatusCode::FORBIDDEN, "Неверный код".to_string()))?;
        self.save_peer(&from, &name, &key)
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.message))?;
        self.changed();
        Ok(())
    }

    // ---------- pairing, the side the code is typed into ----------

    pub async fn hello(&self, addr: SocketAddr) -> Result<Hello, AppError> {
        let res = self
            .http
            .get(format!("http://{addr}/hello"))
            .send()
            .await
            .map_err(|e| err(format!("устройство не отвечает: {e}")))?;
        res.json().await.map_err(|e| err(e.to_string()))
    }

    pub async fn pair(&self, addr: SocketAddr, code: &str) -> Result<Peer, AppError> {
        let host = self.hello(addr).await?;
        let (joiner, msg) = Joiner::start(code.trim(), &self.id, &host.id);
        let answer: PairAnswer = self
            .post_json(
                addr,
                "/pair",
                &PairHello {
                    from: self.id.clone(),
                    name: self.name.clone(),
                    msg: B64.encode(msg),
                },
            )
            .await?;
        let answer = Answer {
            msg: B64.decode(&answer.msg).map_err(|_| err("битый ответ"))?,
            proof: B64
                .decode(&answer.proof)
                .ok()
                .and_then(|p| p.try_into().ok())
                .ok_or_else(|| err("битый ответ"))?,
        };
        let (key, proof) = match joiner.finish(&answer) {
            Ok(done) => done,
            Err(e) => {
                // Tell the other side, so its code is spent there too.
                let _ = self
                    .post_json::<_, serde_json::Value>(
                        addr,
                        "/pair/confirm",
                        &PairProof {
                            from: self.id.clone(),
                            proof: String::new(),
                        },
                    )
                    .await;
                return Err(err(match e {
                    PairError::WrongCode => "Неверный код",
                    PairError::Garbled => "Устройство ответило не так, как ожидалось",
                }));
            }
        };
        let _: serde_json::Value = self
            .post_json(
                addr,
                "/pair/confirm",
                &PairProof {
                    from: self.id.clone(),
                    proof: B64.encode(proof),
                },
            )
            .await?;
        self.save_peer(&host.id, &host.name, &key)?;
        self.changed();
        self.peer(&host.id)?
            .ok_or_else(|| err("устройство не сохранилось"))
    }

    async fn post_json<T: Serialize, R: for<'de> Deserialize<'de>>(
        &self,
        addr: SocketAddr,
        path: &str,
        body: &T,
    ) -> Result<R, AppError> {
        let res = self
            .http
            .post(format!("http://{addr}{path}"))
            .json(body)
            .send()
            .await
            .map_err(|e| err(format!("устройство не отвечает: {e}")))?;
        if !res.status().is_success() {
            let text = res.text().await.unwrap_or_default();
            return Err(err(if text.is_empty() {
                "устройство отказало".into()
            } else {
                text
            }));
        }
        res.json().await.map_err(|e| err(e.to_string()))
    }

    // ---------- peers ----------

    fn save_peer(&self, id: &str, name: &str, key: &[u8; 32]) -> Result<(), AppError> {
        self.db.lock().unwrap().execute(
            "INSERT INTO peers (device_id, name, key, paired_at) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(device_id) DO UPDATE SET name = excluded.name, key = excluded.key,
               sent_until = 0, received_until = 0, paired_at = excluded.paired_at",
            params![id, name, key.as_slice(), now_ts()],
        )?;
        Ok(())
    }

    pub fn peers(&self) -> Result<Vec<Peer>, AppError> {
        let conn = self.db.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT device_id, name, last_sync, paired_at FROM peers ORDER BY paired_at",
        )?;
        let rows = stmt
            .query_map([], |r| {
                Ok(Peer {
                    id: r.get(0)?,
                    name: r.get(1)?,
                    last_sync: r.get(2)?,
                    paired_at: r.get(3)?,
                })
            })?
            .collect::<Result<_, _>>()?;
        Ok(rows)
    }

    fn peer(&self, id: &str) -> Result<Option<Peer>, AppError> {
        Ok(self.peers()?.into_iter().find(|p| p.id == id))
    }

    pub fn unpair(&self, id: &str) -> Result<(), AppError> {
        self.db
            .lock()
            .unwrap()
            .execute("DELETE FROM peers WHERE device_id = ?1", [id])?;
        self.changed();
        Ok(())
    }

    /// (key, sent_until, received_until) of a paired device.
    fn link(&self, id: &str) -> Result<Option<([u8; 32], i64, i64)>, AppError> {
        let row: Option<(Vec<u8>, i64, i64)> = self
            .db
            .lock()
            .unwrap()
            .query_row(
                "SELECT key, sent_until, received_until FROM peers WHERE device_id = ?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?;
        Ok(row.and_then(|(k, s, r)| Some((k.try_into().ok()?, s, r))))
    }

    // ---------- syncing ----------

    /// Sends this device's changes to a paired device and takes its changes back.
    /// Returns (titles, episodes) changed here.
    pub async fn sync_with(&self, id: &str, addr: SocketAddr) -> Result<(usize, usize), AppError> {
        let (key, sent_until, received_until) =
            self.link(id)?.ok_or_else(|| err("устройство не связано"))?;
        let started = now_ts();
        let ask = SyncAsk {
            since: received_until,
            changes: self.library.changes_since(sent_until)?,
        };
        let sealed = crypto::seal(&key, self.id.as_bytes(), &serde_json::to_vec(&ask)?);
        let res = self
            .http
            .post(format!("http://{addr}/sync"))
            .header(DEVICE_HEADER, self.id.as_str())
            .body(sealed)
            .send()
            .await
            .map_err(|e| err(format!("устройство не отвечает: {e}")))?;
        if !res.status().is_success() {
            return Err(err(match res.status() {
                StatusCode::FORBIDDEN => "Другое устройство больше не связано с этим",
                _ => "устройство отказало",
            }));
        }
        let body = res.bytes().await.map_err(|e| err(e.to_string()))?;
        let reply: SyncReply = crypto::open(&key, id.as_bytes(), &body)
            .and_then(|plain| serde_json::from_slice(&plain).ok())
            .ok_or_else(|| err("ответ не расшифровался"))?;
        let applied = self.library.apply(&reply.changes)?;
        // A second back: rows changed within the same second are sent again rather than missed.
        self.mark_synced(id, started - 1, reply.now - 1)?;
        if applied != (0, 0) {
            self.changed();
        }
        Ok(applied)
    }

    fn mark_synced(&self, id: &str, sent_until: i64, received_until: i64) -> Result<(), AppError> {
        self.db.lock().unwrap().execute(
            "UPDATE peers SET sent_until = ?2, received_until = ?3, last_sync = ?4 WHERE device_id = ?1",
            params![id, sent_until, received_until, now_ts()],
        )?;
        Ok(())
    }

    /// The other side of `sync_with`: merges what came in and answers with this device's changes.
    fn answer_sync(&self, from: &str, body: &[u8]) -> Result<Vec<u8>, StatusCode> {
        let (key, _, _) = self
            .link(from)
            .ok()
            .flatten()
            .ok_or(StatusCode::FORBIDDEN)?;
        let ask: SyncAsk = crypto::open(&key, from.as_bytes(), body)
            .and_then(|plain| serde_json::from_slice(&plain).ok())
            .ok_or(StatusCode::FORBIDDEN)?;
        let applied = self
            .library
            .apply(&ask.changes)
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        let reply = SyncReply {
            now: now_ts(),
            changes: self
                .library
                .changes_since(ask.since)
                .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?,
        };
        let _ = self.db.lock().unwrap().execute(
            "UPDATE peers SET last_sync = ?2 WHERE device_id = ?1",
            params![from, now_ts()],
        );
        if applied != (0, 0) {
            self.changed();
        }
        let plain = serde_json::to_vec(&reply).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        Ok(crypto::seal(&key, self.id.as_bytes(), &plain))
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

// ---------- HTTP handlers ----------

async fn hello(State(d): State<Arc<Devices>>) -> Json<Hello> {
    Json(Hello {
        id: d.id.clone(),
        name: d.name.clone(),
    })
}

async fn pair_hello(State(d): State<Arc<Devices>>, Json(body): Json<PairHello>) -> Response {
    match d.answer(body) {
        Ok(answer) => Json(answer).into_response(),
        Err((status, message)) => (status, message).into_response(),
    }
}

async fn pair_confirm(State(d): State<Arc<Devices>>, Json(body): Json<PairProof>) -> Response {
    match d.confirm(body) {
        Ok(()) => Json(serde_json::json!({ "ok": true })).into_response(),
        Err((status, message)) => (status, message).into_response(),
    }
}

async fn sync(State(d): State<Arc<Devices>>, headers: HeaderMap, body: Bytes) -> Response {
    let Some(from) = headers.get(DEVICE_HEADER).and_then(|v| v.to_str().ok()) else {
        return StatusCode::FORBIDDEN.into_response();
    };
    match d.answer_sync(from, &body) {
        Ok(sealed) => sealed.into_response(),
        Err(status) => status.into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    async fn device(name: &str) -> (Arc<Devices>, SocketAddr) {
        let db = Arc::new(Mutex::new(crate::db::open_in_memory().unwrap()));
        let library = Arc::new(Library::new(db.clone()));
        let d = Devices::new(db, library, name.into());
        let addr = d.serve("127.0.0.1:0".parse().unwrap()).await.unwrap();
        (d, addr)
    }

    #[tokio::test]
    async fn two_devices_pair_by_code_and_sync_both_ways() {
        let (pc, pc_addr) = device("Компьютер").await;
        let (laptop, _) = device("Ноутбук").await;
        let code = pc.start_pairing().code;
        let peer = laptop.pair(pc_addr, &code).await.unwrap();
        assert_eq!(peer.name, "Компьютер");
        assert_eq!(pc.peers().unwrap()[0].name, "Ноутбук");
        assert!(pc.pairing().is_none(), "a code works once");

        laptop.library.set_status(52991, "watching").unwrap();
        pc.library.set_status(21, "planned").unwrap();
        assert_eq!(laptop.sync_with(&peer.id, pc_addr).await.unwrap(), (1, 0));
        assert_eq!(
            pc.library.get_entry(52991).unwrap().unwrap().status,
            "watching"
        );
        assert_eq!(
            laptop.library.get_entry(21).unwrap().unwrap().status,
            "planned"
        );
        assert_eq!(
            laptop.sync_with(&peer.id, pc_addr).await.unwrap(),
            (0, 0),
            "nothing new, nothing changes"
        );
        assert!(laptop.peers().unwrap()[0].last_sync.is_some());
    }

    #[tokio::test]
    async fn a_wrong_code_pairs_nothing_and_burns_the_code() {
        let (pc, pc_addr) = device("Компьютер").await;
        let (laptop, _) = device("Ноутбук").await;
        let code = pc.start_pairing().code;
        let wrong = if code == "000000" { "111111" } else { "000000" };
        assert!(laptop.pair(pc_addr, wrong).await.is_err());
        assert!(pc.peers().unwrap().is_empty() && laptop.peers().unwrap().is_empty());
        assert!(
            laptop.pair(pc_addr, &code).await.is_err(),
            "after a wrong guess the code is gone"
        );
    }

    #[tokio::test]
    async fn without_pairing_mode_or_a_key_a_device_gets_nothing() {
        let (pc, pc_addr) = device("Компьютер").await;
        let (laptop, _) = device("Ноутбук").await;
        assert!(laptop.pair(pc_addr, "123456").await.is_err());
        pc.library.set_status(21, "planned").unwrap();
        let sealed = crypto::seal(&[1u8; 32], laptop.id.as_bytes(), b"{}");
        let res = reqwest::Client::new()
            .post(format!("http://{pc_addr}/sync"))
            .header(DEVICE_HEADER, laptop.id.as_str())
            .body(sealed)
            .send()
            .await
            .unwrap();
        assert_eq!(res.status(), reqwest::StatusCode::FORBIDDEN);
    }
}
