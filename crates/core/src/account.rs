//! Signing in to Shikimori, and keeping the list there in step with what is done here.
//!
//! The browser asks the user on Shikimori, which sends a one-time code back to a listener on
//! 127.0.0.1, the only http address Shikimori redirects to. The app's keys come in at build time
//! as SHIKIMORI_CLIENT_ID and SHIKIMORI_CLIENT_SECRET (see crates/core/build.rs).

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

use crate::db::{self, now_ts, Db};
use crate::error::AppError;
use crate::library::{EpisodeProgress, Library, LibraryEntry};
use crate::net::HttpClient;

const DOMAIN: &str = "https://shikimori.io";
const PORT: u16 = 47831;
pub const REDIRECT: &str = "http://127.0.0.1:47831/oauth/shikimori";
const SESSION_SETTING: &str = "shikimori.session";
/// How long the browser has to come back with the code.
const SIGN_IN_WAIT: Duration = Duration::from_secs(300);
const CLIENT_ID: Option<&str> = option_env!("SHIKIMORI_CLIENT_ID");
const CLIENT_SECRET: Option<&str> = option_env!("SHIKIMORI_CLIENT_SECRET");

const DONE_PAGE: &str = "<!doctype html><meta charset=utf-8><title>ninari</title>\
<body style=\"font:16px system-ui;background:#101018;color:#eee;display:grid;place-items:center;height:90vh\">\
<p>Готово, вход выполнен. Вкладку можно закрыть и вернуться в ninari.</p>";
const DENIED_PAGE: &str = "<!doctype html><meta charset=utf-8><title>ninari</title>\
<body style=\"font:16px system-ui;background:#101018;color:#eee;display:grid;place-items:center;height:90vh\">\
<p>Вход отменён. Вкладку можно закрыть.</p>";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Profile {
    pub id: i64,
    pub nickname: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
struct Session {
    access: String,
    refresh: String,
    expires_at: i64,
    profile: Option<Profile>,
}

/// What the list on Shikimori should say about a title.
#[derive(Debug, Clone, PartialEq)]
struct Desired {
    status: String,
    /// The furthest episode watched.
    episodes: i64,
    score: Option<i64>,
    note: Option<String>,
}

/// The keys this build carries; a secret the release workflow lacks comes in empty.
fn built_in() -> Option<(&'static str, &'static str)> {
    CLIENT_ID
        .zip(CLIENT_SECRET)
        .filter(|(id, secret)| !id.is_empty() && !secret.is_empty())
}

pub fn configured() -> bool {
    built_in().is_some()
}

fn keys() -> Result<(&'static str, &'static str), AppError> {
    built_in().ok_or_else(|| {
        AppError::new(
            "NotConfigured",
            "в этой сборке нет ключей приложения Shikimori",
        )
    })
}

fn authorize_url(client_id: &str, state: &str) -> String {
    let mut url = url::Url::parse(&format!("{DOMAIN}/oauth/authorize")).expect("static url");
    url.query_pairs_mut()
        .append_pair("client_id", client_id)
        .append_pair("redirect_uri", REDIRECT)
        .append_pair("response_type", "code")
        .append_pair("scope", "user_rates")
        .append_pair("state", state);
    url.into()
}

/// The code in the browser's request ("GET /oauth/shikimori?code=..&state=.. HTTP/1.1").
/// `Ok(None)` for anything that is not the answer to this sign-in: a favicon, a stale state.
fn callback(request: &str, state: &str) -> Result<Option<String>, AppError> {
    let Some(target) = request.lines().next().and_then(|l| l.split(' ').nth(1)) else {
        return Ok(None);
    };
    let Ok(url) = url::Url::parse(&format!("http://127.0.0.1{target}")) else {
        return Ok(None);
    };
    let q: HashMap<_, _> = url.query_pairs().collect();
    if url.path() != "/oauth/shikimori" || q.get("state").map(|s| s.as_ref()) != Some(state) {
        return Ok(None);
    }
    if q.contains_key("error") {
        return Err(AppError::new("Cancelled", "вход отменён на Shikimori"));
    }
    Ok(q.get("code").map(|c| c.to_string()))
}

async fn wait_for_code(listener: TcpListener, state: &str) -> Result<String, AppError> {
    loop {
        let (mut sock, _) = listener.accept().await?;
        let mut buf = vec![0u8; 8192];
        let Ok(Ok(n)) = tokio::time::timeout(Duration::from_secs(5), sock.read(&mut buf)).await
        else {
            continue;
        };
        let answer = callback(&String::from_utf8_lossy(&buf[..n]), state);
        let (status, page) = match &answer {
            Ok(Some(_)) => ("200 OK", DONE_PAGE),
            Err(_) => ("200 OK", DENIED_PAGE),
            Ok(None) => ("404 Not Found", ""),
        };
        let reply = format!(
            "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{page}",
            page.len()
        );
        let _ = sock.write_all(reply.as_bytes()).await;
        if let Some(code) = answer? {
            return Ok(code);
        }
    }
}

fn parse_token(body: &Value, now: i64) -> Result<Session, AppError> {
    let field = |k: &str| body[k].as_str().map(str::to_string);
    let (Some(access), Some(refresh)) = (field("access_token"), field("refresh_token")) else {
        return Err(AppError::new("Unauthorized", "Shikimori не выдал токен"));
    };
    let created = body["created_at"].as_i64().unwrap_or(now);
    Ok(Session {
        access,
        refresh,
        expires_at: created + body["expires_in"].as_i64().unwrap_or(0),
        profile: None,
    })
}

/// Nothing for a title only remembered here (a chosen dub, a score without a status).
fn desired(entry: &LibraryEntry, progress: &[EpisodeProgress]) -> Option<Desired> {
    if entry.status == "none" {
        return None;
    }
    Some(Desired {
        status: entry.status.clone(),
        episodes: progress
            .iter()
            .filter(|p| p.watched)
            .map(|p| p.episode)
            .max()
            .unwrap_or(0),
        score: entry.score,
        note: entry.note.clone(),
    })
}

/// The fields to send, `None` when Shikimori already says the same. What is there is never lost:
/// episodes only go up, a missing score or note here leaves the one there, rewatching stays.
fn rate_body(want: &Desired, remote: Option<&Value>) -> Option<Value> {
    let mut body = serde_json::Map::new();
    let remote_status = remote
        .and_then(|r| r["status"].as_str())
        .and_then(crate::sync::status_of);
    if remote_status != Some(want.status.as_str()) {
        body.insert("status".into(), json!(want.status));
    }
    if want.episodes > remote.and_then(|r| r["episodes"].as_i64()).unwrap_or(0) {
        body.insert("episodes".into(), json!(want.episodes));
    }
    if let Some(s) = want
        .score
        .filter(|s| remote.and_then(|r| r["score"].as_i64()) != Some(*s))
    {
        body.insert("score".into(), json!(s));
    }
    if let Some(n) = want
        .note
        .as_ref()
        .filter(|n| remote.and_then(|r| r["text"].as_str()) != Some(n.as_str()))
    {
        body.insert("text".into(), json!(n));
    }
    (!body.is_empty()).then_some(Value::Object(body))
}

pub struct Account {
    http: HttpClient,
    db: Db,
    library: Arc<Library>,
    /// A sign-in waiting for the browser; sending to it gives up.
    pending: Mutex<Option<tokio::sync::oneshot::Sender<()>>>,
    /// What was last sent per title, so the same state is sent once.
    sent: Mutex<HashMap<i64, Desired>>,
    /// One request to Shikimori at a time: token refreshes and list edits do not race.
    turn: tokio::sync::Mutex<()>,
    last_error: Mutex<Option<String>>,
}

impl Account {
    pub fn new(http: HttpClient, db: Db, library: Arc<Library>) -> Arc<Self> {
        Arc::new(Self {
            http,
            db,
            library,
            pending: Mutex::new(None),
            sent: Mutex::new(HashMap::new()),
            turn: tokio::sync::Mutex::new(()),
            last_error: Mutex::new(None),
        })
    }

    fn session(&self) -> Option<Session> {
        let v = db::get_setting(&self.db.lock().unwrap(), SESSION_SETTING).ok()??;
        serde_json::from_value(v).ok()
    }

    fn store(&self, s: Option<&Session>) -> Result<(), AppError> {
        let conn = self.db.lock().unwrap();
        match s {
            Some(s) => db::set_setting(&conn, SESSION_SETTING, &serde_json::to_value(s)?)?,
            None => {
                conn.execute("DELETE FROM settings WHERE key = ?1", [SESSION_SETTING])?;
            }
        }
        Ok(())
    }

    pub fn profile(&self) -> Option<Profile> {
        self.session()?.profile
    }

    pub fn last_error(&self) -> Option<String> {
        self.last_error.lock().unwrap().clone()
    }

    /// Opens Shikimori in the browser with `open` and waits for the user to allow access.
    pub async fn sign_in(
        &self,
        open: impl FnOnce(&str) -> Result<(), AppError>,
    ) -> Result<Profile, AppError> {
        let (client_id, _) = keys()?;
        let listener = TcpListener::bind(("127.0.0.1", PORT)).await.map_err(|_| {
            AppError::new(
                "Busy",
                format!("порт {PORT} занят: вход уже идёт или порт занят другой программой"),
            )
        })?;
        let state = hex::encode(rand::random::<[u8; 16]>());
        let (tx, rx) = tokio::sync::oneshot::channel();
        *self.pending.lock().unwrap() = Some(tx);
        let code = async {
            open(&authorize_url(client_id, &state))?;
            tokio::select! {
                r = tokio::time::timeout(SIGN_IN_WAIT, wait_for_code(listener, &state)) => r
                    .map_err(|_| AppError::new("Timeout", "Shikimori не ответил за 5 минут, попробуй ещё раз"))?,
                _ = rx => Err(AppError::new("Cancelled", "вход отменён")),
            }
        }
        .await;
        self.pending.lock().unwrap().take();
        let code = code?;

        let _turn = self.turn.lock().await;
        let mut session = self
            .token(&[
                ("grant_type", "authorization_code"),
                ("code", &code),
                ("redirect_uri", REDIRECT),
            ])
            .await?;
        let me = self
            .call(
                &session.access,
                reqwest::Method::GET,
                "/api/users/whoami",
                None,
            )
            .await?;
        let profile = Profile {
            id: me["id"]
                .as_i64()
                .ok_or_else(|| AppError::new("Internal", "Shikimori не назвал пользователя"))?,
            nickname: me["nickname"].as_str().unwrap_or_default().to_string(),
        };
        session.profile = Some(profile.clone());
        self.store(Some(&session))?;
        self.sent.lock().unwrap().clear();
        *self.last_error.lock().unwrap() = None;
        Ok(profile)
    }

    pub fn cancel(&self) {
        if let Some(tx) = self.pending.lock().unwrap().take() {
            let _ = tx.send(());
        }
    }

    pub fn sign_out(&self) -> Result<(), AppError> {
        self.sent.lock().unwrap().clear();
        *self.last_error.lock().unwrap() = None;
        self.store(None)
    }

    async fn token(&self, grant: &[(&str, &str)]) -> Result<Session, AppError> {
        let (id, secret) = keys()?;
        let body: String = url::form_urlencoded::Serializer::new(String::new())
            .append_pair("client_id", id)
            .append_pair("client_secret", secret)
            .extend_pairs(grant)
            .finish();
        let res = self
            .http
            .inner()
            .post(format!("{DOMAIN}/oauth/token"))
            .header("Content-Type", "application/x-www-form-urlencoded")
            .body(body)
            .send()
            .await
            .map_err(|e| AppError::new("Network", format!("Shikimori недоступен: {e}")))?;
        if matches!(res.status().as_u16(), 400 | 401) {
            return Err(AppError::new(
                "Unauthorized",
                "Shikimori не принял вход, войди заново",
            ));
        }
        let res = res
            .error_for_status()
            .map_err(|e| AppError::new("Network", format!("Shikimori: {e}")))?;
        let json: Value = res
            .json()
            .await
            .map_err(|e| AppError::new("Network", e.to_string()))?;
        parse_token(&json, now_ts())
    }

    /// A fresh access token and who it belongs to; refreshes the token when it is about to expire.
    /// Call with the turn held.
    async fn access(&self) -> Result<(String, Profile), AppError> {
        let not_in = || AppError::new("Unauthorized", "вход в Shikimori не выполнен");
        let mut s = self.session().ok_or_else(not_in)?;
        let profile = s.profile.clone().ok_or_else(not_in)?;
        if s.expires_at - 60 <= now_ts() {
            let refreshed = self
                .token(&[
                    ("grant_type", "refresh_token"),
                    ("refresh_token", &s.refresh),
                ])
                .await;
            match refreshed {
                Ok(fresh) => {
                    s = Session {
                        profile: Some(profile.clone()),
                        ..fresh
                    }
                }
                Err(e) if e.kind == "Unauthorized" => {
                    self.store(None)?;
                    return Err(AppError::new(
                        "Unauthorized",
                        "вход в Shikimori истёк, войди заново",
                    ));
                }
                Err(e) => return Err(e),
            }
            self.store(Some(&s))?;
        }
        Ok((s.access, profile))
    }

    async fn call(
        &self,
        token: &str,
        method: reqwest::Method,
        path: &str,
        body: Option<Value>,
    ) -> Result<Value, AppError> {
        let mut req = self
            .http
            .inner()
            .request(method, format!("{DOMAIN}{path}"))
            .bearer_auth(token);
        if let Some(b) = body {
            req = req.json(&b);
        }
        let res = req
            .send()
            .await
            .map_err(|e| AppError::new("Network", format!("Shikimori недоступен: {e}")))?;
        let status = res.status();
        if !status.is_success() {
            let text = res.text().await.unwrap_or_default();
            return Err(AppError::new(
                "Network",
                format!(
                    "Shikimori ответил {status}: {}",
                    text.chars().take(200).collect::<String>()
                ),
            ));
        }
        if status == reqwest::StatusCode::NO_CONTENT {
            return Ok(Value::Null);
        }
        res.json()
            .await
            .map_err(|e| AppError::new("Network", e.to_string()))
    }

    /// Sends what the library says about a title to the list on Shikimori, in the background.
    /// Does nothing when signed out or when the same was already sent. Call it on the core's
    /// runtime.
    pub fn push(self: &Arc<Self>, id: i64) {
        if self.session().is_none() {
            return;
        }
        let (Ok(Some(entry)), Ok(progress)) =
            (self.library.get_entry(id), self.library.progress(id))
        else {
            return;
        };
        let Some(want) = desired(&entry, &progress) else {
            return;
        };
        if self.sent.lock().unwrap().insert(id, want.clone()).as_ref() == Some(&want) {
            return;
        }
        let me = self.clone();
        tokio::spawn(async move {
            let result = me.upsert(id, &want).await;
            if let Err(e) = &result {
                tracing::warn!("shikimori: cannot update {id}: {}", e.message);
                me.sent.lock().unwrap().remove(&id);
            }
            *me.last_error.lock().unwrap() = result.err().map(|e| e.message);
        });
    }

    async fn upsert(&self, id: i64, want: &Desired) -> Result<(), AppError> {
        let _turn = self.turn.lock().await;
        let (token, me) = self.access().await?;
        let found = self
            .call(
                &token,
                reqwest::Method::GET,
                &format!(
                    "/api/v2/user_rates?user_id={}&target_id={id}&target_type=Anime",
                    me.id
                ),
                None,
            )
            .await?;
        let remote = found.as_array().and_then(|l| l.first());
        let Some(mut body) = rate_body(want, remote) else {
            return Ok(());
        };
        match remote.and_then(|r| r["id"].as_i64()) {
            Some(rate) => {
                self.call(
                    &token,
                    reqwest::Method::PATCH,
                    &format!("/api/v2/user_rates/{rate}"),
                    Some(json!({ "user_rate": body })),
                )
                .await?;
            }
            None => {
                body["user_id"] = json!(me.id);
                body["target_id"] = json!(id);
                body["target_type"] = json!("Anime");
                self.call(
                    &token,
                    reqwest::Method::POST,
                    "/api/v2/user_rates",
                    Some(json!({ "user_rate": body })),
                )
                .await?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn want(status: &str, episodes: i64, score: Option<i64>, note: Option<&str>) -> Desired {
        Desired {
            status: status.into(),
            episodes,
            score,
            note: note.map(Into::into),
        }
    }

    #[test]
    fn the_browser_answer_is_taken_only_for_this_sign_in() {
        let req = |target: &str| format!("GET {target} HTTP/1.1\r\nHost: 127.0.0.1:47831\r\n\r\n");
        assert_eq!(
            callback(&req("/oauth/shikimori?code=abc&state=s1"), "s1")
                .unwrap()
                .as_deref(),
            Some("abc")
        );
        assert_eq!(
            callback(&req("/oauth/shikimori?code=abc&state=other"), "s1").unwrap(),
            None
        );
        assert_eq!(callback(&req("/favicon.ico"), "s1").unwrap(), None);
        assert_eq!(callback("garbage", "s1").unwrap(), None);
        assert!(callback(&req("/oauth/shikimori?error=access_denied&state=s1"), "s1").is_err());
    }

    #[test]
    fn the_authorize_link_asks_for_the_list_and_comes_back_here() {
        let url = authorize_url("id 1", "st");
        assert!(url.starts_with("https://shikimori.io/oauth/authorize?client_id=id+1&"));
        assert!(url.contains("redirect_uri=http%3A%2F%2F127.0.0.1%3A47831%2Foauth%2Fshikimori"));
        assert!(url.contains("scope=user_rates") && url.ends_with("state=st"));
    }

    #[test]
    fn a_token_reply_says_when_it_expires() {
        let s = parse_token(
            &json!({"access_token": "a", "refresh_token": "r", "expires_in": 86400, "created_at": 1000}),
            5,
        )
        .unwrap();
        assert_eq!(
            (s.access.as_str(), s.refresh.as_str(), s.expires_at),
            ("a", "r", 87400)
        );
        assert!(parse_token(&json!({"error": "invalid_grant"}), 5).is_err());
    }

    #[test]
    fn only_what_differs_is_sent_and_nothing_there_is_lost() {
        let remote =
            json!({"id": 9, "status": "rewatching", "episodes": 7, "score": 8, "text": "старое"});
        assert_eq!(
            rate_body(&want("watching", 5, None, None), Some(&remote)),
            None
        );
        assert_eq!(
            rate_body(
                &want("completed", 12, Some(9), Some("новое")),
                Some(&remote)
            ),
            Some(json!({"status": "completed", "episodes": 12, "score": 9, "text": "новое"}))
        );
        assert_eq!(
            rate_body(&want("planned", 0, None, None), None),
            Some(json!({"status": "planned"}))
        );
    }

    #[test]
    fn a_title_only_remembered_here_is_not_sent() {
        let entry = |status: &str| LibraryEntry {
            shikimori_id: 1,
            status: status.into(),
            chosen_dub_key: None,
            notify: true,
            added_at: 0,
            updated_at: 0,
            score: Some(7),
            note: None,
        };
        let p = |episode: i64, watched: bool| EpisodeProgress {
            shikimori_id: 1,
            episode,
            position_sec: 0.0,
            duration_sec: 0.0,
            watched,
            dub_key: None,
            extension_id: None,
            updated_at: 0,
        };
        assert_eq!(desired(&entry("none"), &[]), None);
        assert_eq!(
            desired(&entry("watching"), &[p(1, true), p(3, true), p(4, false)]),
            Some(want("watching", 3, Some(7), None))
        );
    }
}
