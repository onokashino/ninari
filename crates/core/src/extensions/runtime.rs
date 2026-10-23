//! One QuickJS runtime per extension. The extension sees the outside world only through the
//! `ctx` object built by `prelude.js` on top of the `__host_*` functions registered here.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

use base64::engine::general_purpose::{GeneralPurpose, GeneralPurposeConfig};
use base64::engine::DecodePaddingMode;
use base64::Engine;
use rquickjs::prelude::{Async, Func};
use rquickjs::{
    AsyncContext, AsyncRuntime, CatchResultExt, Ctx, Function, Module, Object, Promise, Value,
};
use rusqlite::{params, OptionalExtension};
use serde::de::DeserializeOwned;
use serde_json::json;

use super::types::{EpisodeList, Manifest, Match, SearchQuery, Stream};
use crate::db::{now_ts, Db};
use crate::error::AppError;
use crate::net::{ErrorKind, FetchRequest, HttpClient};

const PRELUDE: &str = include_str!("prelude.js");
const MEMORY_LIMIT: usize = 64 << 20;
pub const CALL_TIMEOUT: Duration = Duration::from_secs(15);
/// One `ctx.fetch` unless the extension asks for another timeout.
const FETCH_TIMEOUT: Duration = Duration::from_secs(10);
/// Requests an extension may send at once, and how many a second after that.
const FETCH_BURST: f64 = 20.0;
const FETCH_PER_SEC: f64 = 5.0;

/// Token bucket: `capacity` requests at once, refilled at `per_sec`.
struct Bucket {
    capacity: f64,
    per_sec: f64,
    tokens: f64,
    at: Instant,
}

impl Bucket {
    fn new(capacity: f64, per_sec: f64, now: Instant) -> Self {
        Self {
            capacity,
            per_sec,
            tokens: capacity,
            at: now,
        }
    }

    /// Takes a token and returns how long to wait before using it.
    fn take(&mut self, now: Instant) -> Duration {
        let elapsed = now.saturating_duration_since(self.at).as_secs_f64();
        self.tokens = (self.tokens + elapsed * self.per_sec).min(self.capacity);
        self.at = self.at.max(now);
        self.tokens -= 1.0;
        if self.tokens >= 0.0 {
            Duration::ZERO
        } else {
            Duration::from_secs_f64(-self.tokens / self.per_sec)
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ExtError {
    pub kind: ErrorKind,
    pub message: String,
}

impl ExtError {
    pub fn broken(message: impl Into<String>) -> Self {
        Self {
            kind: ErrorKind::Broken,
            message: message.into(),
        }
    }

    /// Recognizes the `__EXT:<Kind>:<message>` markers thrown by the prelude.
    fn from_js(text: String) -> Self {
        if let Some(pos) = text.find("__EXT:") {
            if let Some((kind, rest)) = text[pos + 6..].split_once(':') {
                if let Some(kind) = ErrorKind::parse(kind) {
                    let message = rest.lines().next().unwrap_or_default().trim().to_string();
                    return Self { kind, message };
                }
            }
        }
        Self::broken(text)
    }
}

impl std::fmt::Display for ExtError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.kind.as_str(), self.message)
    }
}

impl From<ExtError> for AppError {
    fn from(e: ExtError) -> Self {
        AppError::new(e.kind.as_str(), e.message)
    }
}

fn js_err(e: rquickjs::Error) -> ExtError {
    ExtError::broken(e.to_string())
}

/// Per-extension key-value storage backed by the `extension_kv` table.
#[derive(Clone)]
pub struct KvStore {
    db: Db,
    ext_id: String,
}

impl KvStore {
    pub fn new(db: Db, ext_id: String) -> Self {
        Self { db, ext_id }
    }

    fn get(&self, key: &str) -> Option<String> {
        let conn = self.db.lock().ok()?;
        let row: Option<(String, Option<i64>)> = conn
            .query_row(
                "SELECT value, expires_at FROM extension_kv WHERE extension_id = ?1 AND key = ?2",
                params![self.ext_id, key],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
            .ok()?;
        match row {
            Some((_, Some(exp))) if exp <= now_ts() => None,
            Some((v, _)) => Some(v),
            None => None,
        }
    }

    fn set(&self, key: &str, value: &str, ttl_sec: f64) {
        let expires = (ttl_sec > 0.0).then(|| now_ts() + ttl_sec as i64);
        if let Ok(conn) = self.db.lock() {
            let _ = conn.execute(
                "INSERT OR REPLACE INTO extension_kv (extension_id, key, value, expires_at) VALUES (?1, ?2, ?3, ?4)",
                params![self.ext_id, key, value, expires],
            );
        }
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Host patterns an extension may fetch: hosts of its domains plus `allowedHosts`
/// (exact hosts or `*.suffix`).
fn allowed_patterns(m: &Manifest) -> Vec<String> {
    m.domains
        .iter()
        .filter_map(|d| url::Url::parse(d).ok()?.host_str().map(str::to_string))
        .chain(m.allowed_hosts.iter().cloned())
        .collect()
}

fn host_allowed(patterns: &[String], url: &str) -> Result<(), String> {
    let parsed = url::Url::parse(url).map_err(|_| format!("bad url {url}"))?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err(format!("scheme not allowed: {}", parsed.scheme()));
    }
    let host = parsed.host_str().unwrap_or_default().to_ascii_lowercase();
    let ok = patterns.iter().any(|p| {
        let p = p.to_ascii_lowercase();
        match p.strip_prefix("*.") {
            Some(suffix) => host == suffix || host.ends_with(&format!(".{suffix}")),
            None => host == p,
        }
    });
    if ok {
        Ok(())
    } else {
        Err(format!("host not allowed: {host}"))
    }
}

fn request_from_init(url: String, init: &str) -> FetchRequest {
    let init: serde_json::Value = serde_json::from_str(init).unwrap_or(json!({}));
    let method = init
        .get("method")
        .and_then(|m| m.as_str())
        .unwrap_or("GET")
        .to_string();
    let headers = init
        .get("headers")
        .and_then(|h| h.as_object())
        .map(|h| {
            h.iter()
                .filter_map(|(k, v)| v.as_str().map(|v| (k.clone(), v.to_string())))
                .collect()
        })
        .unwrap_or_default();
    let body = init
        .get("body")
        .and_then(|b| b.as_str())
        .map(str::to_string);
    // `init.timeout` in milliseconds; a hung host must not use up the whole call budget.
    let timeout = init
        .get("timeout")
        .and_then(|t| t.as_f64())
        .filter(|t| *t > 0.0)
        .map(|t| Duration::from_millis(t as u64))
        .unwrap_or(FETCH_TIMEOUT);
    FetchRequest {
        url,
        method,
        headers,
        body,
        timeout: Some(timeout),
    }
}

async fn host_fetch(
    http: HttpClient,
    allow: Arc<RwLock<Vec<String>>>,
    url: String,
    init: String,
) -> String {
    let patterns = allow.read().map(|p| p.clone()).unwrap_or_default();
    let allowed = move |u: &url::Url| host_allowed(&patterns, u.as_str());
    match http
        .fetch_guarded(request_from_init(url, &init), &allowed)
        .await
    {
        Ok(r) => json!({ "status": r.status, "url": r.url, "headers": r.headers, "body": r.body })
            .to_string(),
        Err(e) => json!({ "error": { "kind": e.kind.as_str(), "message": e.message } }).to_string(),
    }
}

fn html_select(html: &str, css: &str) -> String {
    let Ok(selector) = scraper::Selector::parse(css) else {
        return "[]".into();
    };
    let doc = scraper::Html::parse_document(html);
    let nodes: Vec<serde_json::Value> = doc
        .select(&selector)
        .map(|el| {
            let attrs: serde_json::Map<String, serde_json::Value> =
                el.value().attrs().map(|(k, v)| (k.to_string(), json!(v))).collect();
            json!({ "text": el.text().collect::<String>(), "html": el.inner_html(), "attrs": attrs })
        })
        .collect();
    serde_json::Value::Array(nodes).to_string()
}

fn b64(s: &str, encode: bool) -> String {
    let engine = GeneralPurpose::new(
        &base64::alphabet::STANDARD,
        GeneralPurposeConfig::new().with_decode_padding_mode(DecodePaddingMode::Indifferent),
    );
    if encode {
        return engine.encode(s.as_bytes());
    }
    let url_safe = GeneralPurpose::new(
        &base64::alphabet::URL_SAFE,
        GeneralPurposeConfig::new().with_decode_padding_mode(DecodePaddingMode::Indifferent),
    );
    engine
        .decode(s.trim())
        .or_else(|_| url_safe.decode(s.trim()))
        .map(|b| String::from_utf8_lossy(&b).into_owned())
        .unwrap_or_default()
}

fn register_host(
    ctx: &Ctx<'_>,
    http: HttpClient,
    kv: KvStore,
    allow: Arc<RwLock<Vec<String>>>,
) -> rquickjs::Result<()> {
    let g = ctx.globals();
    let limit = Arc::new(Mutex::new(Bucket::new(
        FETCH_BURST,
        FETCH_PER_SEC,
        Instant::now(),
    )));
    g.set(
        "__host_fetch",
        Func::from(Async(move |url: String, init: String| {
            let http = http.clone();
            let allow = allow.clone();
            let limit = limit.clone();
            async move {
                let wait = limit.lock().unwrap().take(Instant::now());
                if !wait.is_zero() {
                    tokio::time::sleep(wait).await;
                }
                rquickjs::Result::Ok(host_fetch(http, allow, url, init).await)
            }
        })),
    )?;
    g.set(
        "__host_html_select",
        Func::from(|html: String, css: String| html_select(&html, &css)),
    )?;
    let kv_get = kv.clone();
    g.set(
        "__host_kv_get",
        Func::from(move |key: String| kv_get.get(&key)),
    )?;
    g.set(
        "__host_kv_set",
        Func::from(move |key: String, value: String, ttl: f64| kv.set(&key, &value, ttl)),
    )?;
    g.set(
        "__host_b64",
        Func::from(|s: String, encode: bool| b64(&s, encode)),
    )?;
    g.set(
        "__host_log",
        Func::from(|msg: String| tracing::info!(target: "extension", "{msg}")),
    )?;
    Ok(())
}

fn stringify<'js>(ctx: &Ctx<'js>, v: Value<'js>) -> Result<String, ExtError> {
    let s = ctx.json_stringify(v).map_err(js_err)?;
    match s {
        Some(s) => s.to_string().map_err(js_err),
        None => Ok("null".into()),
    }
}

fn load_module(
    ctx: Ctx<'_>,
    source: String,
    http: HttpClient,
    kv: KvStore,
    allow: Arc<RwLock<Vec<String>>>,
) -> Result<String, ExtError> {
    let caught = |e: rquickjs::CaughtError<'_>| ExtError::from_js(e.to_string());
    register_host(&ctx, http, kv, allow).map_err(js_err)?;
    ctx.eval::<(), _>(PRELUDE).catch(&ctx).map_err(caught)?;
    let declared = Module::declare(ctx.clone(), "extension", source)
        .catch(&ctx)
        .map_err(caught)?;
    let (module, promise) = declared.eval().catch(&ctx).map_err(caught)?;
    promise.finish::<()>().catch(&ctx).map_err(caught)?;
    let ns = module.namespace().catch(&ctx).map_err(caught)?;
    let manifest: Value = ns.get("manifest").catch(&ctx).map_err(caught)?;
    ctx.globals().set("__ext", ns).map_err(js_err)?;
    stringify(&ctx, manifest)
}

async fn run_call(
    ctx: Ctx<'_>,
    name: &'static str,
    args_json: String,
    base: String,
) -> Result<String, ExtError> {
    let caught = |e: rquickjs::CaughtError<'_>| ExtError::from_js(e.to_string());
    let g = ctx.globals();
    let ns: Object = g.get("__ext").catch(&ctx).map_err(caught)?;
    let f: Value = ns.get(name).catch(&ctx).map_err(caught)?;
    if !f.is_function() {
        return Err(ExtError::broken(format!(
            "extension does not export {name}"
        )));
    }
    let make: Function = g.get("__makeCtx").catch(&ctx).map_err(caught)?;
    let js_ctx: Value = make.call((base,)).catch(&ctx).map_err(caught)?;
    let args: Value = ctx.json_parse(args_json).catch(&ctx).map_err(caught)?;
    let caller: Function = g.get("__call").catch(&ctx).map_err(caught)?;
    let promise: Promise = caller.call((f, args, js_ctx)).catch(&ctx).map_err(caught)?;
    let result: Value = promise
        .into_future::<Value>()
        .await
        .catch(&ctx)
        .map_err(caught)?;
    stringify(&ctx, result)
}

pub struct JsExtension {
    _rt: AsyncRuntime,
    ctx: AsyncContext,
    manifest: Manifest,
    deadline: Arc<AtomicU64>,
    /// One call at a time: the interrupt deadline is per runtime, and a call that ends would
    /// otherwise clear the deadline of another one still running.
    calls: tokio::sync::Mutex<()>,
    timeout: Duration,
    base_index: Mutex<usize>,
}

impl JsExtension {
    pub async fn load(source: &str, http: HttpClient, kv: KvStore) -> Result<Self, ExtError> {
        Self::load_with(source, http, kv, CALL_TIMEOUT).await
    }

    pub async fn load_with(
        source: &str,
        http: HttpClient,
        kv: KvStore,
        timeout: Duration,
    ) -> Result<Self, ExtError> {
        let rt = AsyncRuntime::new().map_err(js_err)?;
        rt.set_memory_limit(MEMORY_LIMIT).await;
        let deadline = Arc::new(AtomicU64::new(0));
        let watched = deadline.clone();
        rt.set_interrupt_handler(Some(Box::new(move || {
            let d = watched.load(Ordering::Relaxed);
            d != 0 && now_ms() > d
        })))
        .await;
        let ctx = AsyncContext::full(&rt).await.map_err(js_err)?;
        let allow: Arc<RwLock<Vec<String>>> = Arc::new(RwLock::new(Vec::new()));

        deadline.store(now_ms() + timeout.as_millis() as u64, Ordering::Relaxed);
        let (source, allow_js) = (source.to_string(), allow.clone());
        let manifest_json = ctx
            .with(move |ctx| load_module(ctx, source, http, kv, allow_js))
            .await;
        deadline.store(0, Ordering::Relaxed);

        let manifest: Manifest = serde_json::from_str(&manifest_json?)
            .map_err(|e| ExtError::broken(format!("bad manifest: {e}")))?;
        if manifest.api_version != 1 {
            return Err(ExtError::broken(format!(
                "unsupported apiVersion {}",
                manifest.api_version
            )));
        }
        if manifest.domains.is_empty() {
            return Err(ExtError::broken("manifest.domains is empty"));
        }
        *allow.write().unwrap() = allowed_patterns(&manifest);
        Ok(Self {
            _rt: rt,
            ctx,
            manifest,
            deadline,
            calls: tokio::sync::Mutex::new(()),
            timeout,
            base_index: Mutex::new(0),
        })
    }

    pub fn manifest(&self) -> &Manifest {
        &self.manifest
    }

    async fn call_once(
        &self,
        name: &'static str,
        args_json: String,
        base: String,
    ) -> Result<String, ExtError> {
        let _turn = self.calls.lock().await;
        self.deadline.store(
            now_ms() + self.timeout.as_millis() as u64,
            Ordering::Relaxed,
        );
        let fut = self
            .ctx
            .async_with(async move |ctx| run_call(ctx, name, args_json, base).await);
        let res = tokio::time::timeout(self.timeout + Duration::from_secs(1), fut).await;
        self.deadline.store(0, Ordering::Relaxed);
        res.unwrap_or_else(|_| Err(ExtError::broken(format!("{name}: timed out"))))
    }

    /// Calls an exported function, moving to the next mirror domain when the current one is blocked.
    async fn call<T: DeserializeOwned>(
        &self,
        name: &'static str,
        args: serde_json::Value,
    ) -> Result<T, ExtError> {
        let domains = &self.manifest.domains;
        let start = *self.base_index.lock().unwrap();
        let args_json = args.to_string();
        let mut last = ExtError::broken("no domains");
        for i in 0..domains.len() {
            let idx = (start + i) % domains.len();
            match self
                .call_once(name, args_json.clone(), domains[idx].clone())
                .await
            {
                // Another mirror may sit in another region or behind another CDN.
                Err(e)
                    if matches!(
                        e.kind,
                        ErrorKind::Blocked | ErrorKind::GeoBlocked | ErrorKind::Challenge
                    ) =>
                {
                    last = e
                }
                Err(e) => return Err(e),
                Ok(json) => {
                    *self.base_index.lock().unwrap() = idx;
                    return serde_json::from_str(&json)
                        .map_err(|e| ExtError::broken(format!("{name}: bad result: {e}")));
                }
            }
        }
        Err(last)
    }

    pub async fn search(&self, q: &SearchQuery) -> Result<Vec<Match>, ExtError> {
        self.call("search", json!([q])).await
    }

    pub async fn episodes(&self, m: &Match) -> Result<EpisodeList, ExtError> {
        self.call("episodes", json!([m])).await
    }

    pub async fn resolve(
        &self,
        m: &Match,
        episode: i64,
        dub_id: &str,
    ) -> Result<Vec<Stream>, ExtError> {
        self.call("resolve", json!([m, episode, dub_id])).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;
    use crate::extensions::types::SearchQuery;
    use std::sync::{Arc, Mutex};
    use std::time::Instant;

    fn kv() -> KvStore {
        KvStore::new(
            Arc::new(Mutex::new(db::open_in_memory().unwrap())),
            "t".into(),
        )
    }

    fn source(domains: &str, body: &str) -> String {
        format!(
            r#"export const manifest = {{ id: "t", name: "T", version: "0.0.1", apiVersion: 1,
                 domains: [{domains}], allowedHosts: [] }};
               {body}"#
        )
    }

    fn query() -> SearchQuery {
        SearchQuery {
            shikimori_id: 52991,
            mal_id: 52991,
            names: vec!["Фрирен".into()],
            year: Some(2023),
            kind: Some("tv".into()),
            episodes: Some(28),
        }
    }

    async fn load(src: &str) -> JsExtension {
        JsExtension::load_with(
            src,
            HttpClient::new_allowing_private("ninari-test"),
            kv(),
            Duration::from_secs(1),
        )
        .await
        .unwrap()
    }

    async fn serve(router: axum::Router) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        format!("http://{addr}")
    }

    #[tokio::test]
    async fn reads_manifest_and_calls_search() {
        let ext = load(&source(
            r#""http://127.0.0.1:1""#,
            r#"export async function search(q, ctx) {
                 return [{ id: "1", title: q.names[0] + "|" + q.shikimoriId, confidence: "exact" }];
               }"#,
        ))
        .await;
        assert_eq!(ext.manifest().id, "t");
        let m = ext.search(&query()).await.unwrap();
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].title, "Фрирен|52991");
        assert_eq!(m[0].confidence, "exact");
    }

    #[tokio::test]
    async fn fetch_goes_through_host_to_allowed_domain() {
        let base = serve(
            axum::Router::new().route("/list", axum::routing::get(|| async { r#"{"eps":[1,2]}"# })),
        )
        .await;
        let ext = load(&source(
            &format!("\"{base}\""),
            r#"export async function episodes(m, ctx) {
                 const r = await ctx.fetch(ctx.baseUrl + "/list");
                 const j = await r.json();
                 return { dubs: [{ id: "d", name: "AniDub", type: "voice" }],
                          episodes: j.eps.map(n => ({ number: n, available: [{ dubId: "d", maxQuality: 720 }] })) };
               }"#,
        ))
        .await;
        let m = crate::extensions::types::Match {
            id: "1".into(),
            title: "x".into(),
            confidence: "exact".into(),
            data: serde_json::Value::Null,
        };
        let list = ext.episodes(&m).await.unwrap();
        assert_eq!(list.episodes.len(), 2);
        assert_eq!(list.episodes[1].number, 2);
        assert_eq!(list.dubs[0].name, "AniDub");
        assert_eq!(list.episodes[0].available[0].max_quality, Some(720));
    }

    #[tokio::test]
    async fn fetch_to_foreign_host_is_rejected() {
        let ext = load(&source(
            r#""https://example.org""#,
            r#"export async function search(q, ctx) { await ctx.fetch("https://evil.example/x"); return []; }"#,
        ))
        .await;
        let err = ext.search(&query()).await.unwrap_err();
        assert_eq!(err.kind, ErrorKind::Broken);
        assert!(err.message.contains("host not allowed"), "{}", err.message);
    }

    #[tokio::test]
    async fn fetch_does_not_follow_redirects_to_hosts_outside_the_manifest() {
        let base = serve(
            axum::Router::new()
                .route(
                    "/go",
                    axum::routing::get(|headers: axum::http::HeaderMap| async move {
                        let host = headers["host"].to_str().unwrap();
                        let port = host.rsplit(':').next().unwrap().to_string();
                        axum::response::Redirect::temporary(&format!(
                            "http://localhost:{port}/secret"
                        ))
                    }),
                )
                .route("/secret", axum::routing::get(|| async { "secret" })),
        )
        .await;
        let ext = load(&source(
            &format!("\"{base}\""),
            r#"export async function search(q, ctx) {
                 const r = await ctx.fetch(ctx.baseUrl + "/go");
                 return [{ id: "1", title: r.body, confidence: "exact" }];
               }"#,
        ))
        .await;
        let err = ext.search(&query()).await.unwrap_err();
        assert!(
            err.message.contains("host not allowed: localhost"),
            "{}",
            err.message
        );
    }

    #[tokio::test]
    async fn a_geo_blocked_mirror_moves_on_to_the_next_one() {
        let ext = load(&source(
            r#""https://blocked.example", "https://open.example""#,
            r#"export async function search(q, ctx) {
                 if (ctx.baseUrl.includes("blocked")) ctx.fail.geoBlocked();
                 return [{ id: "1", title: ctx.baseUrl, confidence: "exact" }];
               }"#,
        ))
        .await;
        let m = ext.search(&query()).await.unwrap();
        assert_eq!(m[0].title, "https://open.example");
    }

    #[tokio::test]
    async fn a_fetch_can_give_up_on_a_slow_host() {
        let base = serve(axum::Router::new().route(
            "/slow",
            axum::routing::get(|| async {
                tokio::time::sleep(Duration::from_secs(3)).await;
                "late"
            }),
        ))
        .await;
        let ext = load(&source(
            &format!("\"{base}\""),
            r#"export async function search(q, ctx) {
                 try { await ctx.fetch(ctx.baseUrl + "/slow", { timeout: 300 }); }
                 catch (e) { return [{ id: "1", title: String(e.message || e), confidence: "exact" }]; }
                 return [];
               }"#,
        ))
        .await;
        let started = Instant::now();
        let m = ext.search(&query()).await;
        assert!(
            started.elapsed() < Duration::from_millis(1500),
            "{:?}",
            started.elapsed()
        );
        let text = match m {
            Ok(m) => m[0].title.clone(),
            Err(e) => e.message,
        };
        assert!(!text.contains("late"), "{text}");
    }

    #[tokio::test]
    async fn pages_in_windows_1251_are_decoded() {
        let base = serve(axum::Router::new().route(
            "/cp1251",
            axum::routing::get(|| async {
                // "Привет" in windows-1251
                (
                    [("content-type", "text/html; charset=windows-1251")],
                    vec![0xCF_u8, 0xF0, 0xE8, 0xE2, 0xE5, 0xF2],
                )
            }),
        ))
        .await;
        let ext = load(&source(
            &format!("\"{base}\""),
            r#"export async function search(q, ctx) {
                 const r = await ctx.fetch(ctx.baseUrl + "/cp1251");
                 return [{ id: "1", title: await r.text(), confidence: "exact" }];
               }"#,
        ))
        .await;
        assert_eq!(ext.search(&query()).await.unwrap()[0].title, "Привет");
    }

    #[test]
    fn requests_are_limited_after_a_burst() {
        let t0 = std::time::Instant::now();
        let mut bucket = Bucket::new(20.0, 5.0, t0);
        for _ in 0..20 {
            assert_eq!(bucket.take(t0), Duration::ZERO);
        }
        assert_eq!(bucket.take(t0), Duration::from_millis(200));
        assert_eq!(bucket.take(t0), Duration::from_millis(400), "waits add up");
        let later = t0 + Duration::from_secs(10);
        assert_eq!(
            bucket.take(later),
            Duration::ZERO,
            "tokens come back over time"
        );
    }

    #[tokio::test]
    async fn infinite_loop_is_interrupted() {
        let ext = load(&source(
            r#""https://example.org""#,
            r#"export async function search(q, ctx) { while (true) {} }"#,
        ))
        .await;
        let started = Instant::now();
        let err = ext.search(&query()).await.unwrap_err();
        assert_eq!(err.kind, ErrorKind::Broken);
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    #[tokio::test]
    async fn overlapping_call_finishing_does_not_disarm_the_other_calls_timeout() {
        let base = serve(
            axum::Router::new()
                .route(
                    "/a",
                    axum::routing::get(|| async {
                        tokio::time::sleep(Duration::from_millis(100)).await;
                        "a"
                    }),
                )
                .route(
                    "/b",
                    axum::routing::get(|| async {
                        tokio::time::sleep(Duration::from_millis(400)).await;
                        "b"
                    }),
                ),
        )
        .await;
        let ext = load(&source(
            &format!("\"{base}\""),
            r#"export async function episodes(m, ctx) {
                 await ctx.fetch(ctx.baseUrl + "/a");
                 return { dubs: [], episodes: [] };
               }
               export async function search(q, ctx) {
                 await ctx.fetch(ctx.baseUrl + "/b");
                 const end = Date.now() + 3000;
                 while (Date.now() < end) {}
                 return [];
               }"#,
        ))
        .await;
        let m = crate::extensions::types::Match {
            id: "1".into(),
            title: "x".into(),
            confidence: "exact".into(),
            data: serde_json::Value::Null,
        };
        let q = query();
        let started = Instant::now();
        let (a, b) = tokio::join!(ext.episodes(&m), ext.search(&q));
        assert!(a.is_ok(), "{:?}", a.err());
        assert!(b.is_err(), "the spinning call ran to completion");
        assert!(started.elapsed() < Duration::from_millis(2500));
    }

    #[tokio::test]
    async fn fail_helpers_map_to_error_kinds() {
        let ext = load(&source(
            r#""https://example.org""#,
            r#"export async function search(q, ctx) { ctx.fail.notFound("нет такого"); }
               export async function episodes(m, ctx) { ctx.fail.geoBlocked(); }"#,
        ))
        .await;
        let err = ext.search(&query()).await.unwrap_err();
        assert_eq!(err.kind, ErrorKind::NotFound);
        assert!(err.message.contains("нет такого"));
        let m = crate::extensions::types::Match {
            id: "1".into(),
            title: "x".into(),
            confidence: "exact".into(),
            data: serde_json::Value::Null,
        };
        assert_eq!(
            ext.episodes(&m).await.unwrap_err().kind,
            ErrorKind::GeoBlocked
        );
    }

    #[tokio::test]
    async fn html_select_and_kv_work() {
        let ext = load(&source(
            r#""https://example.org""#,
            r#"export async function search(q, ctx) {
                 const nodes = ctx.html.select("<div><a href='/x'>t</a><a>u</a></div>", "a");
                 await ctx.kv.set("k", "v");
                 const v = await ctx.kv.get("k");
                 const missing = await ctx.kv.get("nope");
                 return [{ id: nodes[0].attrs.href, title: nodes.map(n => n.text).join(",") + v + String(missing),
                           confidence: ctx.base64.decode(ctx.base64.encode("exact")) }];
               }"#,
        ))
        .await;
        let m = ext.search(&query()).await.unwrap();
        assert_eq!(m[0].id, "/x");
        assert_eq!(m[0].title, "t,uvnull");
        assert_eq!(m[0].confidence, "exact");
    }

    /// Runs any extension bundle in the real sandbox against its live source:
    /// search -> episodes -> resolve, then fetches the start of the stream with its headers.
    ///
    /// NINARI_EXT_FILE=<dist/x.js> NINARI_SHIKI_ID=52991 NINARI_NAMES="Sousou no Frieren|Фрирен"
    /// [NINARI_EPISODE=1] [NINARI_DUB=<substring>]
    /// cargo test --lib live_extension -- --ignored --nocapture
    #[tokio::test]
    #[ignore = "network; needs NINARI_EXT_FILE"]
    async fn live_extension() {
        let file = std::env::var("NINARI_EXT_FILE").expect("NINARI_EXT_FILE");
        let id: i64 = std::env::var("NINARI_SHIKI_ID")
            .expect("NINARI_SHIKI_ID")
            .parse()
            .unwrap();
        let names: Vec<String> = std::env::var("NINARI_NAMES")
            .unwrap_or_default()
            .split('|')
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .collect();
        let episode: i64 = std::env::var("NINARI_EPISODE")
            .ok()
            .and_then(|e| e.parse().ok())
            .unwrap_or(1);
        let dub_filter = std::env::var("NINARI_DUB")
            .unwrap_or_default()
            .to_lowercase();
        let src = std::fs::read_to_string(&file).unwrap();
        let http = HttpClient::new("ninari/0.1.0 (+https://github.com/onokashino/ninari)");
        let ext = JsExtension::load(&src, http.clone(), kv())
            .await
            .expect("load");
        println!("manifest: {:?}", ext.manifest());
        let q = SearchQuery {
            shikimori_id: id,
            mal_id: id,
            names,
            year: None,
            kind: None,
            episodes: None,
        };
        let matches = ext.search(&q).await.expect("search");
        println!("matches: {matches:?}");
        let m = matches.first().expect("no match").clone();
        let list = ext.episodes(&m).await.expect("episodes");
        println!(
            "dubs ({}): {:?}",
            list.dubs.len(),
            list.dubs.iter().map(|d| &d.name).collect::<Vec<_>>()
        );
        println!("episodes: {}", list.episodes.len());
        let ep = list
            .episodes
            .iter()
            .find(|e| e.number == episode)
            .expect("episode not listed");
        let dub = list
            .dubs
            .iter()
            .filter(|d| ep.available.iter().any(|a| a.dub_id == d.id))
            .find(|d| dub_filter.is_empty() || d.name.to_lowercase().contains(&dub_filter))
            .expect("no dub for the episode");
        println!("resolving episode {episode} in {} ({})", dub.name, dub.id);
        let streams = ext.resolve(&m, episode, &dub.id).await.expect("resolve");
        for s in &streams {
            println!(
                "stream {:?} {:?} {} headers={:?}",
                s.kind, s.quality, s.url, s.headers
            );
        }
        let first = streams.first().expect("no streams");
        let mut req = http.inner().get(&first.url);
        for (k, v) in &first.headers {
            req = req.header(k.as_str(), v.as_str());
        }
        let resp = req.send().await.expect("stream request");
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        println!(
            "stream HTTP {status}, first bytes:\n{}",
            body.chars().take(400).collect::<String>()
        );
        assert!(status.is_success(), "stream answered {status}");
    }

    /// Loads the real AniLibria bundle and talks to the live API.
    /// Run with `NINARI_EXT_DIST=<extensions/dist> cargo test -- --ignored live_`.
    #[tokio::test]
    #[ignore = "network; needs NINARI_EXT_DIST"]
    async fn live_anilibria_bundle() {
        let dir = std::env::var("NINARI_EXT_DIST").expect("NINARI_EXT_DIST");
        let src = std::fs::read_to_string(format!("{dir}/anilibria.js")).unwrap();
        let ext = JsExtension::load(&src, HttpClient::new("ninari-test"), kv())
            .await
            .unwrap();
        let mut q = query();
        q.names = vec!["Sousou no Frieren".into()];
        let m = ext.search(&q).await.unwrap();
        assert_eq!(m[0].id, "9542");
        let list = ext.episodes(&m[0]).await.unwrap();
        assert_eq!(list.episodes.len(), 28);
        let streams = ext.resolve(&m[0], 1, "aniliberty").await.unwrap();
        assert_eq!(streams[0].quality, Some(1080));
        assert!(streams[0].url.contains(".m3u8"));
    }
}
