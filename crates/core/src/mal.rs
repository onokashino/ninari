//! Signing in to MyAnimeList, keeping the list there in step with what is done here, and taking
//! the list from there.
//!
//! Shikimori's anime ids are MyAnimeList's, so a title here is the same title there. MyAnimeList
//! signs in with PKCE and no secret: only the app's id comes in at build time as MAL_CLIENT_ID
//! (see crates/core/build.rs). The browser comes back to the same 127.0.0.1 listener as for
//! Shikimori, at /oauth/mal.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use serde_json::Value;
use tokio::net::TcpListener;

use crate::account::{
    desired, parse_token, wait_for_code, Desired, Profile, Session, PORT, SIGN_IN_WAIT,
};
use crate::catalog::calendar::unix_of;
use crate::catalog::Catalog;
use crate::db::{self, now_ts, Db};
use crate::error::AppError;
use crate::library::Library;
use crate::net::HttpClient;
use crate::sync::ImportReport;

const AUTH: &str = "https://myanimelist.net/v1/oauth2";
const API: &str = "https://api.myanimelist.net/v2";
pub const REDIRECT: &str = "http://127.0.0.1:47831/oauth/mal";
const SESSION_SETTING: &str = "mal.session";
const CLIENT_ID: Option<&str> = option_env!("MAL_CLIENT_ID");

fn client_id() -> Option<&'static str> {
    CLIENT_ID.filter(|id| !id.is_empty())
}

pub fn configured() -> bool {
    client_id().is_some()
}

fn key() -> Result<&'static str, AppError> {
    client_id().ok_or_else(|| {
        AppError::new(
            "NotConfigured",
            "в этой сборке нет ключа приложения MyAnimeList",
        )
    })
}

/// MyAnimeList takes only the "plain" challenge: the verifier itself.
fn authorize_url(client_id: &str, state: &str, verifier: &str) -> String {
    let mut url = url::Url::parse(&format!("{AUTH}/authorize")).expect("static url");
    url.query_pairs_mut()
        .append_pair("response_type", "code")
        .append_pair("client_id", client_id)
        .append_pair("redirect_uri", REDIRECT)
        .append_pair("state", state)
        .append_pair("code_challenge", verifier)
        .append_pair("code_challenge_method", "plain");
    url.into()
}

/// The library status of a MyAnimeList one; rewatching is watching again.
fn status_of(mal: &str, rewatching: bool) -> Option<&'static str> {
    Some(match mal {
        _ if rewatching => "watching",
        "plan_to_watch" => "planned",
        "watching" => "watching",
        "completed" => "completed",
        "on_hold" => "on_hold",
        "dropped" => "dropped",
        _ => return None,
    })
}

fn mal_status(library: &str) -> &str {
    match library {
        "planned" => "plan_to_watch",
        other => other,
    }
}

/// The form fields to send, `None` when MyAnimeList already says the same. As with Shikimori,
/// nothing there is lost: episodes only go up, a missing score leaves the one there.
fn list_body(want: &Desired, remote: Option<&Value>) -> Option<Vec<(&'static str, String)>> {
    let mut body = Vec::new();
    let remote_status = remote.and_then(|r| {
        status_of(
            r["status"].as_str()?,
            r["is_rewatching"].as_bool().unwrap_or(false),
        )
    });
    if remote_status != Some(want.status.as_str()) {
        body.push(("status", mal_status(&want.status).to_string()));
    }
    let episodes = remote
        .and_then(|r| r["num_episodes_watched"].as_i64())
        .unwrap_or(0);
    if want.episodes > episodes {
        body.push(("num_watched_episodes", want.episodes.to_string()));
    }
    if let Some(s) = want
        .score
        .filter(|s| (1..=10).contains(s))
        .filter(|s| remote.and_then(|r| r["score"].as_i64()) != Some(*s))
    {
        body.push(("score", s.to_string()));
    }
    (!body.is_empty()).then_some(body)
}

/// A title of the list: (id, status, episodes watched, changed at).
fn parse_list(page: &Value) -> Vec<(i64, &'static str, i64, i64)> {
    page["data"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|item| {
            let s = &item["list_status"];
            Some((
                item["node"]["id"].as_i64()?,
                status_of(
                    s["status"].as_str()?,
                    s["is_rewatching"].as_bool().unwrap_or(false),
                )?,
                s["num_episodes_watched"].as_i64().unwrap_or(0).max(0),
                s["updated_at"].as_str().and_then(unix_of).unwrap_or(0),
            ))
        })
        .collect()
}

pub struct Mal {
    http: HttpClient,
    db: Db,
    library: Arc<Library>,
    pending: Mutex<Option<tokio::sync::oneshot::Sender<()>>>,
    sent: Mutex<HashMap<i64, Desired>>,
    turn: tokio::sync::Mutex<()>,
    last_error: Mutex<Option<String>>,
}

impl Mal {
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

    /// Opens MyAnimeList in the browser with `open` and waits for the user to allow access.
    pub async fn sign_in(
        &self,
        open: impl FnOnce(&str) -> Result<(), AppError>,
    ) -> Result<Profile, AppError> {
        let client_id = key()?;
        let listener = TcpListener::bind(("127.0.0.1", PORT)).await.map_err(|_| {
            AppError::new(
                "Busy",
                format!("порт {PORT} занят: вход уже идёт или порт занят другой программой"),
            )
        })?;
        let state = hex::encode(rand::random::<[u8; 16]>());
        // 64 hex characters: within the 43 to 128 PKCE wants.
        let verifier = hex::encode(rand::random::<[u8; 32]>());
        let (tx, rx) = tokio::sync::oneshot::channel();
        *self.pending.lock().unwrap() = Some(tx);
        let code = async {
            open(&authorize_url(client_id, &state, &verifier))?;
            tokio::select! {
                r = tokio::time::timeout(SIGN_IN_WAIT, wait_for_code(listener, "/oauth/mal", &state)) => r
                    .map_err(|_| AppError::new("Timeout", "MyAnimeList не ответил за 5 минут, попробуй ещё раз"))?,
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
                ("code_verifier", &verifier),
            ])
            .await?;
        let me = self
            .call(&session.access, reqwest::Method::GET, "/users/@me", None)
            .await?;
        let profile = Profile {
            id: me["id"]
                .as_i64()
                .ok_or_else(|| AppError::new("Internal", "MyAnimeList не назвал пользователя"))?,
            nickname: me["name"].as_str().unwrap_or_default().to_string(),
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
        let body: String = url::form_urlencoded::Serializer::new(String::new())
            .append_pair("client_id", key()?)
            .extend_pairs(grant)
            .finish();
        let res = self
            .http
            .inner()
            .post(format!("{AUTH}/token"))
            .header("Content-Type", "application/x-www-form-urlencoded")
            .body(body)
            .send()
            .await
            .map_err(|e| AppError::new("Network", format!("MyAnimeList недоступен: {e}")))?;
        if matches!(res.status().as_u16(), 400 | 401) {
            return Err(AppError::new(
                "Unauthorized",
                "MyAnimeList не принял вход, войди заново",
            ));
        }
        let res = res
            .error_for_status()
            .map_err(|e| AppError::new("Network", format!("MyAnimeList: {e}")))?;
        let json: Value = res
            .json()
            .await
            .map_err(|e| AppError::new("Network", e.to_string()))?;
        parse_token(&json, now_ts())
    }

    /// A fresh access token; refreshes it when it is about to expire. Call with the turn held.
    async fn access(&self) -> Result<String, AppError> {
        let not_in = || AppError::new("Unauthorized", "вход в MyAnimeList не выполнен");
        let mut s = self.session().ok_or_else(not_in)?;
        let profile = s.profile.clone().ok_or_else(not_in)?;
        if s.expires_at - 60 <= now_ts() {
            match self
                .token(&[
                    ("grant_type", "refresh_token"),
                    ("refresh_token", &s.refresh),
                ])
                .await
            {
                Ok(fresh) => {
                    s = Session {
                        profile: Some(profile),
                        ..fresh
                    }
                }
                Err(e) if e.kind == "Unauthorized" => {
                    self.store(None)?;
                    return Err(AppError::new(
                        "Unauthorized",
                        "вход в MyAnimeList истёк, войди заново",
                    ));
                }
                Err(e) => return Err(e),
            }
            self.store(Some(&s))?;
        }
        Ok(s.access)
    }

    /// `path` under the API, with `form` as the body when there is one.
    async fn call(
        &self,
        token: &str,
        method: reqwest::Method,
        path: &str,
        form: Option<&[(&str, String)]>,
    ) -> Result<Value, AppError> {
        let url = if path.starts_with("https://") {
            path.to_string()
        } else {
            format!("{API}{path}")
        };
        let mut req = self.http.inner().request(method, url).bearer_auth(token);
        if let Some(f) = form {
            req = req
                .header("Content-Type", "application/x-www-form-urlencoded")
                .body(
                    url::form_urlencoded::Serializer::new(String::new())
                        .extend_pairs(f)
                        .finish(),
                );
        }
        let res = req
            .send()
            .await
            .map_err(|e| AppError::new("Network", format!("MyAnimeList недоступен: {e}")))?;
        let status = res.status();
        if !status.is_success() {
            let text = res.text().await.unwrap_or_default();
            return Err(AppError::new(
                "Network",
                format!(
                    "MyAnimeList ответил {status}: {}",
                    text.chars().take(200).collect::<String>()
                ),
            ));
        }
        res.json()
            .await
            .map_err(|e| AppError::new("Network", e.to_string()))
    }

    /// Sends what the library says about a title to the list on MyAnimeList, in the background.
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
            let result = me.update(id, &want).await;
            if let Err(e) = &result {
                tracing::warn!("mal: cannot update {id}: {}", e.message);
                me.sent.lock().unwrap().remove(&id);
            }
            *me.last_error.lock().unwrap() = result.err().map(|e| e.message);
        });
    }

    async fn update(&self, id: i64, want: &Desired) -> Result<(), AppError> {
        let _turn = self.turn.lock().await;
        let token = self.access().await?;
        let found = self
            .call(
                &token,
                reqwest::Method::GET,
                &format!("/anime/{id}?fields=my_list_status"),
                None,
            )
            .await?;
        let remote = found.get("my_list_status");
        if let Some(body) = list_body(want, remote) {
            self.call(
                &token,
                reqwest::Method::PATCH,
                &format!("/anime/{id}/my_list_status"),
                Some(&body),
            )
            .await?;
        }
        Ok(())
    }

    /// Adds the titles of the user's list there that the library does not have yet.
    pub async fn import(&self, catalog: &Catalog) -> Result<ImportReport, AppError> {
        let mut rates = Vec::new();
        {
            let _turn = self.turn.lock().await;
            let token = self.access().await?;
            let mut next =
                Some("/users/@me/animelist?fields=list_status&limit=1000&nsfw=true".to_string());
            while let Some(path) = next.take() {
                let page = self.call(&token, reqwest::Method::GET, &path, None).await?;
                rates.extend(parse_list(&page));
                next = page["paging"]["next"].as_str().map(str::to_string);
            }
        }
        // Titles the library shows must be in the catalog cache.
        let ids: Vec<i64> = rates.iter().map(|r| r.0).collect();
        catalog.get_many(&ids).await?;
        let mut added = 0;
        for (id, status, episodes, at) in &rates {
            if self.library.import_rate(*id, status, *episodes, *at)? {
                added += 1;
            }
        }
        Ok(ImportReport {
            total: rates.len(),
            added,
            skipped: rates.len() - added,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn want(status: &str, episodes: i64, score: Option<i64>) -> Desired {
        Desired {
            status: status.into(),
            episodes,
            score,
            note: None,
        }
    }

    #[test]
    fn the_authorize_link_carries_the_challenge_and_comes_back_here() {
        let url = authorize_url("id", "st", "v".repeat(64).as_str());
        assert!(url.starts_with(
            "https://myanimelist.net/v1/oauth2/authorize?response_type=code&client_id=id&"
        ));
        assert!(url.contains("redirect_uri=http%3A%2F%2F127.0.0.1%3A47831%2Foauth%2Fmal"));
        assert!(url.contains(&format!("code_challenge={}", "v".repeat(64))));
        assert!(url.ends_with("code_challenge_method=plain"));
    }

    #[test]
    fn only_what_differs_is_sent_and_nothing_there_is_lost() {
        let remote = json!({"status": "completed", "is_rewatching": true, "num_episodes_watched": 7, "score": 8});
        assert_eq!(list_body(&want("watching", 5, None), Some(&remote)), None);
        assert_eq!(
            list_body(&want("planned", 0, None), None),
            Some(vec![("status", "plan_to_watch".to_string())])
        );
        let remote = json!({"status": "watching", "num_episodes_watched": 3, "score": 0});
        assert_eq!(
            list_body(&want("completed", 12, Some(9)), Some(&remote)),
            Some(vec![
                ("status", "completed".to_string()),
                ("num_watched_episodes", "12".to_string()),
                ("score", "9".to_string()),
            ])
        );
    }

    #[test]
    fn the_list_parses_with_statuses_as_the_library_names_them() {
        let page = json!({"data": [
            {"node": {"id": 1}, "list_status": {"status": "plan_to_watch", "num_episodes_watched": 0, "updated_at": "2024-05-01T07:00:00+00:00"}},
            {"node": {"id": 5114}, "list_status": {"status": "completed", "is_rewatching": true, "num_episodes_watched": 3}},
            {"node": {"id": 9}, "list_status": {"status": "unknown"}}
        ]});
        assert_eq!(
            parse_list(&page),
            vec![(1, "planned", 0, 1714546800), (5114, "watching", 3, 0)]
        );
    }
}
