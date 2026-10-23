pub mod ssrf;

use std::sync::Arc;
use std::time::Duration;

use reqwest::header::LOCATION;
use serde::{Deserialize, Serialize};

use crate::error::AppError;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ErrorKind {
    Blocked,
    GeoBlocked,
    Challenge,
    NotFound,
    RateLimited,
    Broken,
}

impl ErrorKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            ErrorKind::Blocked => "Blocked",
            ErrorKind::GeoBlocked => "GeoBlocked",
            ErrorKind::Challenge => "Challenge",
            ErrorKind::NotFound => "NotFound",
            ErrorKind::RateLimited => "RateLimited",
            ErrorKind::Broken => "Broken",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "Blocked" => ErrorKind::Blocked,
            "GeoBlocked" => ErrorKind::GeoBlocked,
            "Challenge" => ErrorKind::Challenge,
            "NotFound" => ErrorKind::NotFound,
            "RateLimited" => ErrorKind::RateLimited,
            "Broken" => ErrorKind::Broken,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct NetError {
    pub kind: ErrorKind,
    pub message: String,
}

impl NetError {
    pub fn new(kind: ErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }
}

impl std::fmt::Display for NetError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.kind.as_str(), self.message)
    }
}

impl std::error::Error for NetError {}

impl From<NetError> for AppError {
    fn from(e: NetError) -> Self {
        AppError::new(e.kind.as_str(), e.message)
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct FetchRequest {
    pub url: String,
    pub method: String,
    pub headers: Vec<(String, String)>,
    pub body: Option<String>,
    /// Gives up on this request sooner than the client-wide timeout.
    #[serde(skip)]
    pub timeout: Option<Duration>,
}

impl FetchRequest {
    pub fn get(url: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            method: "GET".into(),
            ..Default::default()
        }
    }

    pub fn post_json(url: impl Into<String>, body: String) -> Self {
        Self {
            url: url.into(),
            method: "POST".into(),
            headers: vec![("Content-Type".into(), "application/json".into())],
            body: Some(body),
            timeout: None,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct FetchResponse {
    pub status: u16,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: String,
}

/// Classifies an HTTP status (and the start of the body for anti-bot pages) into an error kind.
/// Returns `None` for statuses that are not errors.
pub fn classify_status(status: u16, body: &str) -> Option<ErrorKind> {
    if status < 400 {
        return None;
    }
    if status == 403 || status == 503 {
        let head: String = body.chars().take(4096).collect::<String>().to_lowercase();
        if head.contains("ddos-guard") || head.contains("cf-chl") || head.contains("just a moment")
        {
            return Some(ErrorKind::Challenge);
        }
    }
    Some(match status {
        404 | 410 => ErrorKind::NotFound,
        451 => ErrorKind::GeoBlocked,
        429 => ErrorKind::RateLimited,
        _ => ErrorKind::Broken,
    })
}

fn classify_transport(e: &reqwest::Error) -> ErrorKind {
    if e.is_timeout() || e.is_connect() {
        ErrorKind::Blocked
    } else {
        ErrorKind::Broken
    }
}

const MAX_REDIRECTS: usize = 10;

/// Checks one hop of a guarded request; the error explains the refusal.
pub type HopCheck = dyn Fn(&url::Url) -> Result<(), String> + Sync;

#[derive(Clone)]
pub struct HttpClient {
    client: reqwest::Client,
    /// No automatic redirects and public addresses only; see `send_guarded`.
    guarded: reqwest::Client,
    user_agent: String,
    allow_private: bool,
}

impl HttpClient {
    pub fn new(user_agent: &str) -> Self {
        Self::build(user_agent, false)
    }

    /// For tests that serve upstreams on loopback.
    #[cfg(test)]
    pub fn new_allowing_private(user_agent: &str) -> Self {
        Self::build(user_agent, true)
    }

    fn build(user_agent: &str, allow_private: bool) -> Self {
        let builder = || {
            reqwest::Client::builder()
                .user_agent(user_agent)
                .connect_timeout(Duration::from_secs(10))
                .timeout(Duration::from_secs(30))
        };
        let mut guarded = builder().redirect(reqwest::redirect::Policy::none());
        if !allow_private {
            guarded = guarded.dns_resolver(Arc::new(ssrf::PublicResolver));
        }
        Self {
            client: builder().build().expect("reqwest client"),
            guarded: guarded.build().expect("reqwest client"),
            user_agent: user_agent.to_string(),
            allow_private,
        }
    }

    pub fn user_agent(&self) -> &str {
        &self.user_agent
    }

    pub fn inner(&self) -> &reqwest::Client {
        &self.client
    }

    /// Performs a request. HTTP error statuses are returned as responses so callers can
    /// inspect them; only transport failures and anti-bot challenges become errors.
    pub async fn fetch(&self, req: FetchRequest) -> Result<FetchResponse, NetError> {
        let resp = request(&self.client, &req)?
            .send()
            .await
            .map_err(|e| NetError::new(classify_transport(&e), format!("{}: {e}", req.url)))?;
        read_response(resp).await
    }

    /// `fetch` for URLs that come from extensions or streams: see `send_guarded`.
    pub async fn fetch_guarded(
        &self,
        req: FetchRequest,
        allowed: &HopCheck,
    ) -> Result<FetchResponse, NetError> {
        read_response(self.send_guarded(req, allowed).await?).await
    }

    /// Sends a request that may only reach public addresses and follows redirects itself, so
    /// that `allowed` and the address check apply to every hop, the first one included.
    pub async fn send_guarded(
        &self,
        mut req: FetchRequest,
        allowed: &HopCheck,
    ) -> Result<reqwest::Response, NetError> {
        let refused = |m: String| NetError::new(ErrorKind::Broken, m);
        let mut url =
            url::Url::parse(&req.url).map_err(|e| refused(format!("bad url {}: {e}", req.url)))?;
        for _ in 0..=MAX_REDIRECTS {
            if !matches!(url.scheme(), "http" | "https") {
                return Err(refused(format!("scheme not allowed: {}", url.scheme())));
            }
            allowed(&url).map_err(refused)?;
            if !self.allow_private && ssrf::literal_ip(&url).is_some_and(ssrf::is_forbidden_ip) {
                return Err(refused(format!("{url}: private address")));
            }
            req.url = url.to_string();
            let resp = request(&self.guarded, &req)?
                .send()
                .await
                .map_err(|e| NetError::new(classify_transport(&e), format!("{url}: {e}")))?;
            let status = resp.status();
            let location = resp
                .headers()
                .get(LOCATION)
                .filter(|_| status.is_redirection())
                .and_then(|v| v.to_str().ok())
                .map(str::to_string);
            let Some(location) = location else {
                return Ok(resp);
            };
            let next = url
                .join(&location)
                .map_err(|e| refused(format!("{url}: bad redirect: {e}")))?;
            let post = req.method.eq_ignore_ascii_case("POST");
            if status == 303 || (post && matches!(status.as_u16(), 301 | 302)) {
                req.method = "GET".into();
                req.body = None;
            }
            if next.host_str() != url.host_str() {
                req.headers.retain(|(k, _)| {
                    !k.eq_ignore_ascii_case("authorization") && !k.eq_ignore_ascii_case("cookie")
                });
            }
            url = next;
        }
        Err(refused(format!("{url}: too many redirects")))
    }

    /// Tries `path_and_query` against each base domain in order. Moves on when a domain is
    /// unreachable, shows an anti-bot challenge or answers with a server error; any other
    /// error status is returned as is. On success returns the response and the working base.
    pub async fn fetch_mirrored(
        &self,
        domains: &[String],
        path_and_query: &str,
        template: FetchRequest,
    ) -> Result<(FetchResponse, String), NetError> {
        let mut last = NetError::new(ErrorKind::Blocked, "no domains configured");
        for base in domains {
            let mut req = template.clone();
            req.url = format!("{}{}", base.trim_end_matches('/'), path_and_query);
            let mut tries = 0;
            let got = loop {
                let got = self.fetch(req.clone()).await;
                match &got {
                    // Shikimori allows a few requests a second; a page asks for many at once.
                    Ok(resp) if resp.status == 429 && tries < RATE_LIMIT_TRIES => {
                        tokio::time::sleep(rate_limit_wait(&resp.headers, tries)).await;
                        tries += 1;
                    }
                    _ => break got,
                }
            };
            match got {
                Ok(resp) => match classify_status(resp.status, &resp.body) {
                    None => return Ok((resp, base.clone())),
                    Some(ErrorKind::Broken) if resp.status >= 500 => {
                        last = NetError::new(
                            ErrorKind::Broken,
                            format!("{}: HTTP {}", resp.url, resp.status),
                        );
                    }
                    Some(kind) => {
                        return Err(NetError::new(
                            kind,
                            format!("{}: HTTP {}", resp.url, resp.status),
                        ))
                    }
                },
                Err(e) if matches!(e.kind, ErrorKind::Blocked | ErrorKind::Challenge) => last = e,
                Err(e) => return Err(e),
            }
        }
        Err(last)
    }
}

/// How many times a rate-limited request is asked again.
const RATE_LIMIT_TRIES: u32 = 3;

/// How long to wait before asking again after the `attempt`th 429: what Retry-After says, up to
/// 8 s, or 1, 2, 4 s.
fn rate_limit_wait(headers: &[(String, String)], attempt: u32) -> Duration {
    let asked = headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("retry-after"))
        .and_then(|(_, v)| v.trim().parse::<u64>().ok());
    match asked {
        Some(secs) => Duration::from_secs(secs.min(8)),
        None => Duration::from_secs(1 << attempt.min(3)),
    }
}

fn request(
    client: &reqwest::Client,
    req: &FetchRequest,
) -> Result<reqwest::RequestBuilder, NetError> {
    let method = if req.method.is_empty() {
        "GET"
    } else {
        req.method.as_str()
    };
    let method = reqwest::Method::from_bytes(method.to_uppercase().as_bytes())
        .map_err(|_| NetError::new(ErrorKind::Broken, format!("bad method {method}")))?;
    let mut builder = client.request(method, &req.url);
    for (k, v) in &req.headers {
        builder = builder.header(k.as_str(), v.as_str());
    }
    if let Some(body) = &req.body {
        builder = builder.body(body.clone());
    }
    if let Some(t) = req.timeout {
        builder = builder.timeout(t);
    }
    Ok(builder)
}

async fn read_response(resp: reqwest::Response) -> Result<FetchResponse, NetError> {
    let status = resp.status().as_u16();
    let url = resp.url().to_string();
    let headers = resp
        .headers()
        .iter()
        .filter_map(|(k, v)| {
            v.to_str()
                .ok()
                .map(|v| (k.as_str().to_string(), v.to_string()))
        })
        .collect();
    let body = resp
        .text()
        .await
        .map_err(|e| NetError::new(classify_transport(&e), format!("{url}: {e}")))?;
    if classify_status(status, &body) == Some(ErrorKind::Challenge) {
        return Err(NetError::new(ErrorKind::Challenge, url));
    }
    Ok(FetchResponse {
        status,
        url,
        headers,
        body,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::response::IntoResponse;

    #[test]
    fn classifies_status() {
        assert_eq!(classify_status(404, ""), Some(ErrorKind::NotFound));
        assert_eq!(classify_status(410, ""), Some(ErrorKind::NotFound));
        assert_eq!(classify_status(451, ""), Some(ErrorKind::GeoBlocked));
        assert_eq!(classify_status(429, ""), Some(ErrorKind::RateLimited));
        assert_eq!(
            classify_status(403, "<title>DDoS-Guard</title>"),
            Some(ErrorKind::Challenge)
        );
        assert_eq!(
            classify_status(503, "Just a moment... cf-chl"),
            Some(ErrorKind::Challenge)
        );
        assert_eq!(classify_status(403, "forbidden"), Some(ErrorKind::Broken));
        assert_eq!(classify_status(500, ""), Some(ErrorKind::Broken));
        assert_eq!(classify_status(200, "ok"), None);
        assert_eq!(classify_status(301, ""), None);
    }

    async fn serve(router: axum::Router) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        format!("http://{addr}")
    }

    #[tokio::test]
    async fn guarded_fetch_refuses_local_addresses() {
        let base =
            serve(axum::Router::new().route("/x", axum::routing::get(|| async { "x" }))).await;
        let port = base.rsplit(':').next().unwrap();
        let client = HttpClient::new("ninari-test");
        for url in [format!("{base}/x"), format!("http://localhost:{port}/x")] {
            let res = client
                .fetch_guarded(FetchRequest::get(&url), &|_| Ok(()))
                .await;
            assert!(res.is_err(), "{url} was fetched");
        }
    }

    #[tokio::test]
    async fn guarded_fetch_checks_every_redirect_hop() {
        let base = serve(
            axum::Router::new()
                .route(
                    "/start",
                    axum::routing::get(|| async { axum::response::Redirect::temporary("/next") }),
                )
                .route("/next", axum::routing::get(|| async { "next" })),
        )
        .await;
        let client = HttpClient::new_allowing_private("ninari-test");
        let ok = client
            .fetch_guarded(FetchRequest::get(format!("{base}/start")), &|_| Ok(()))
            .await
            .unwrap();
        assert_eq!(ok.body, "next");
        let refused = client
            .fetch_guarded(FetchRequest::get(format!("{base}/start")), &|u| {
                if u.path() == "/next" {
                    Err("hop refused".into())
                } else {
                    Ok(())
                }
            })
            .await
            .unwrap_err();
        assert!(
            refused.message.contains("hop refused"),
            "{}",
            refused.message
        );
    }

    #[tokio::test]
    async fn mirrored_skips_dead_domain() {
        let good =
            serve(axum::Router::new().route("/ping", axum::routing::get(|| async { "pong" })))
                .await;
        // Port 9 on loopback is closed: the connection is refused, which counts as Blocked.
        let dead = "http://127.0.0.1:9".to_string();
        let client = HttpClient::new("ninari-test");
        let (resp, base) = client
            .fetch_mirrored(&[dead, good.clone()], "/ping", FetchRequest::get(""))
            .await
            .unwrap();
        assert_eq!(resp.body, "pong");
        assert_eq!(base, good);
    }

    #[tokio::test]
    async fn mirrored_waits_out_a_rate_limit_and_asks_again() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let asked = Arc::new(AtomicUsize::new(0));
        let seen = asked.clone();
        let base = serve(axum::Router::new().route(
            "/x",
            axum::routing::get(move || {
                let n = seen.fetch_add(1, Ordering::SeqCst);
                async move {
                    if n < 2 {
                        (
                            axum::http::StatusCode::TOO_MANY_REQUESTS,
                            [("retry-after", "0")],
                            "slow down",
                        )
                            .into_response()
                    } else {
                        "ok".into_response()
                    }
                }
            }),
        ))
        .await;
        let client = HttpClient::new_allowing_private("ninari-test");
        let (resp, _) = client
            .fetch_mirrored(&[base], "/x", FetchRequest::get(""))
            .await
            .unwrap();
        assert_eq!(resp.body, "ok");
        assert_eq!(asked.load(Ordering::SeqCst), 3);
    }

    #[test]
    fn a_rate_limit_waits_as_asked_or_doubles() {
        let after = |v: &str| vec![("Retry-After".to_string(), v.to_string())];
        assert_eq!(rate_limit_wait(&after("3"), 0), Duration::from_secs(3));
        assert_eq!(rate_limit_wait(&after("600"), 0), Duration::from_secs(8));
        assert_eq!(rate_limit_wait(&[], 0), Duration::from_secs(1));
        assert_eq!(rate_limit_wait(&[], 2), Duration::from_secs(4));
    }

    #[tokio::test]
    async fn mirrored_returns_not_found_without_trying_next() {
        let a = serve(axum::Router::new()).await;
        let b = serve(axum::Router::new().route("/x", axum::routing::get(|| async { "b" }))).await;
        let client = HttpClient::new("ninari-test");
        let err = client
            .fetch_mirrored(&[a, b], "/x", FetchRequest::get(""))
            .await
            .unwrap_err();
        assert_eq!(err.kind, ErrorKind::NotFound);
    }
}
