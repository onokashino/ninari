//! Local HTTP proxy between mpv and video CDNs. It adds per-session headers (Referer etc.),
//! rewrites HLS playlists and DASH manifests so every request comes back through it, and
//! refuses to connect to private or loopback addresses.

pub mod dash;
pub mod hls;

use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use std::time::Duration;

use axum::body::Body;
use axum::extract::{Path, RawQuery, State};
use axum::http::header::{
    ACCEPT_RANGES, CONTENT_LENGTH, CONTENT_RANGE, CONTENT_TYPE, LOCATION, RANGE,
};
use axum::http::{HeaderMap, Response, StatusCode};
use axum::response::IntoResponse;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;

use crate::error::AppError;
use crate::net::ssrf::{ensure_public, PublicResolver};
use crate::net::HttpClient;
use dash::{is_dash, rewrite_mpd};
use futures_util::StreamExt;
use hls::{is_playlist, looks_like_playlist, rewrite_m3u8, Url};

const MAX_REDIRECTS: usize = 5;

#[derive(Clone)]
struct Session {
    headers: HashMap<String, String>,
}

struct Inner {
    client: reqwest::Client,
    sessions: RwLock<HashMap<String, Session>>,
    origin: String,
    allow_private: bool,
}

type Failure = (StatusCode, String);

pub struct StreamProxy {
    inner: Arc<Inner>,
}

fn file_name(upstream: &str) -> String {
    let name = Url::parse(upstream)
        .ok()
        .and_then(|u| {
            u.path_segments()
                .and_then(|mut s| s.next_back().map(str::to_string))
        })
        .unwrap_or_default();
    let clean: String = name
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
        .collect();
    if clean.is_empty() {
        "stream".into()
    } else {
        clean
    }
}

fn local_url(origin: &str, token: &str, upstream: &str) -> String {
    format!(
        "{origin}/s/{token}/{}/{}",
        URL_SAFE_NO_PAD.encode(upstream),
        file_name(upstream)
    )
}

impl StreamProxy {
    pub async fn start(http: HttpClient) -> Result<Self, AppError> {
        Self::start_with(http, false).await
    }

    /// `allow_private` exists for tests that serve the upstream on loopback.
    pub async fn start_with(http: HttpClient, allow_private: bool) -> Result<Self, AppError> {
        let mut client = reqwest::Client::builder()
            .user_agent(http.user_agent())
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(10))
            .read_timeout(Duration::from_secs(30));
        if !allow_private {
            // check_host resolves before connecting; this keeps the connection itself on the
            // same public addresses, so DNS rebinding cannot slip in between.
            client = client.dns_resolver(Arc::new(PublicResolver));
        }
        let client = client
            .build()
            .map_err(|e| AppError::new("Internal", e.to_string()))?;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let addr = listener.local_addr()?;
        let inner = Arc::new(Inner {
            client,
            sessions: RwLock::new(HashMap::new()),
            origin: format!("http://{addr}"),
            allow_private,
        });
        let router = axum::Router::new()
            .route("/s/{token}/{enc}/{name}", axum::routing::get(handle))
            .route("/d/{token}/{enc}/", axum::routing::get(handle_dir_root))
            .route("/d/{token}/{enc}/{*rest}", axum::routing::get(handle_dir))
            .with_state(inner.clone());
        tokio::spawn(async move {
            if let Err(e) = axum::serve(listener, router).await {
                tracing::error!("stream proxy stopped: {e}");
            }
        });
        Ok(Self { inner })
    }

    /// Registers a playback session and returns the local URL to hand to the player.
    pub fn open_session(&self, upstream: &str, headers: HashMap<String, String>) -> String {
        let token = hex::encode(rand::random::<[u8; 16]>());
        self.inner
            .sessions
            .write()
            .unwrap()
            .insert(token.clone(), Session { headers });
        local_url(&self.inner.origin, &token, upstream)
    }

    pub fn close_session(&self, token: &str) {
        self.inner.sessions.write().unwrap().remove(token);
    }

    /// Extracts the session token from a URL produced by `open_session`.
    pub fn token_of(local: &str) -> Option<String> {
        let rest = local.split("/s/").nth(1)?;
        rest.split('/').next().map(str::to_string)
    }
}

fn decode(enc: &str) -> Result<Url, Failure> {
    let bad = || (StatusCode::BAD_REQUEST, "bad url".to_string());
    let raw = URL_SAFE_NO_PAD.decode(enc).map_err(|_| bad())?;
    let upstream = String::from_utf8(raw).map_err(|_| bad())?;
    Url::parse(&upstream).map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))
}

fn respond(result: Result<Response<Body>, Failure>) -> Response<Body> {
    result.unwrap_or_else(|(status, msg)| {
        tracing::warn!("stream proxy: {msg}");
        (status, msg).into_response()
    })
}

/// One upstream URL: `/s/<token>/<url>/<name>`.
async fn handle(
    State(inner): State<Arc<Inner>>,
    Path((token, enc, _name)): Path<(String, String, String)>,
    headers: HeaderMap,
) -> Response<Body> {
    respond(async { forward(&inner, &token, decode(&enc)?, &headers).await }.await)
}

/// A folder prefix for DASH templates: `/d/<token>/<folder>/<path the player filled in>`.
async fn handle_dir(
    State(inner): State<Arc<Inner>>,
    Path((token, enc, rest)): Path<(String, String, String)>,
    RawQuery(query): RawQuery,
    headers: HeaderMap,
) -> Response<Body> {
    respond(
        async {
            let mut url = decode(&enc)?
                .join(&rest)
                .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
            if query.is_some() {
                url.set_query(query.as_deref());
            }
            forward(&inner, &token, url, &headers).await
        }
        .await,
    )
}

async fn handle_dir_root(
    state: State<Arc<Inner>>,
    Path((token, enc)): Path<(String, String)>,
    query: RawQuery,
    headers: HeaderMap,
) -> Response<Body> {
    handle_dir(state, Path((token, enc, String::new())), query, headers).await
}

async fn read_text(resp: reqwest::Response) -> Result<String, Failure> {
    resp.text()
        .await
        .map_err(|e| (StatusCode::BAD_GATEWAY, e.to_string()))
}

fn text_response(body: String, content_type: &str) -> Result<Response<Body>, Failure> {
    Response::builder()
        .status(StatusCode::OK)
        .header(CONTENT_TYPE, content_type)
        .body(Body::from(body))
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))
}

fn local_dir(origin: &str, token: &str, folder: &Url) -> String {
    format!(
        "{origin}/d/{token}/{}/",
        URL_SAFE_NO_PAD.encode(folder.as_str())
    )
}

/// Content types a CDN uses when it does not say what it sends; a playlist may hide behind them.
fn is_generic(content_type: Option<&str>) -> bool {
    content_type.is_none_or(|ct| {
        let ct = ct.to_ascii_lowercase();
        ct.starts_with("application/octet-stream")
            || ct.starts_with("text/plain")
            || ct.starts_with("binary/")
    })
}

async fn check_host(url: &Url) -> Result<(), Failure> {
    ensure_public(url).await.map_err(|e| {
        let status = if e.contains("private") {
            StatusCode::FORBIDDEN
        } else {
            StatusCode::BAD_GATEWAY
        };
        (status, e)
    })
}

async fn forward(
    inner: &Inner,
    token: &str,
    mut url: Url,
    headers: &HeaderMap,
) -> Result<Response<Body>, Failure> {
    let forbidden = |m: &str| (StatusCode::FORBIDDEN, m.to_string());
    let session = inner
        .sessions
        .read()
        .unwrap()
        .get(token)
        .cloned()
        .ok_or_else(|| forbidden("unknown session"))?;

    for _ in 0..=MAX_REDIRECTS {
        if !matches!(url.scheme(), "http" | "https") {
            return Err(forbidden("scheme not allowed"));
        }
        if !inner.allow_private {
            check_host(&url).await?;
        }
        let mut req = inner.client.get(url.as_str());
        for (k, v) in &session.headers {
            req = req.header(k.as_str(), v.as_str());
        }
        if let Some(range) = headers.get(RANGE) {
            req = req.header(RANGE, range);
        }
        let resp = req
            .send()
            .await
            .map_err(|e| (StatusCode::BAD_GATEWAY, format!("{url}: {e}")))?;
        let status = resp.status();
        if status.is_redirection() {
            let location = resp
                .headers()
                .get(LOCATION)
                .and_then(|v| v.to_str().ok())
                .unwrap_or_default();
            url = url
                .join(location)
                .map_err(|e| (StatusCode::BAD_GATEWAY, e.to_string()))?;
            continue;
        }
        let content_type = resp
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .map(str::to_string);
        let rewrite_hls = |body: &str, url: &Url| {
            let rewritten = rewrite_m3u8(body, url, &|u: &Url| {
                local_url(&inner.origin, token, u.as_str())
            });
            text_response(rewritten, "application/vnd.apple.mpegurl")
        };
        let ct = content_type.as_deref();
        if status.is_success() && is_playlist(ct, url.as_str()) {
            return rewrite_hls(&read_text(resp).await?, &url);
        }
        if status.is_success() && is_dash(ct, url.as_str()) {
            let body = read_text(resp).await?;
            let rewritten = rewrite_mpd(&body, &url, &|folder: &Url| {
                local_dir(&inner.origin, token, folder)
            });
            return text_response(rewritten, "application/dash+xml");
        }
        let mut builder = Response::builder().status(status.as_u16());
        for name in [CONTENT_TYPE, CONTENT_LENGTH, CONTENT_RANGE, ACCEPT_RANGES] {
            if let Some(v) = resp.headers().get(&name) {
                builder = builder.header(name, v.clone());
            }
        }
        let mut stream = resp.bytes_stream();
        let mut first = None;
        if status.is_success() && is_generic(ct) {
            // Peek at the first chunk: a playlist without a telling type or extension still
            // has to be rewritten, or its segments would bypass the proxy.
            first = stream.next().await;
            if let Some(Ok(head)) = &first {
                if looks_like_playlist(head) {
                    let mut all = head.to_vec();
                    while let Some(chunk) = stream.next().await {
                        let chunk = chunk.map_err(|e| (StatusCode::BAD_GATEWAY, e.to_string()))?;
                        all.extend_from_slice(&chunk);
                    }
                    return rewrite_hls(&String::from_utf8_lossy(&all), &url);
                }
            }
        }
        return builder
            .body(Body::from_stream(
                futures_util::stream::iter(first).chain(stream),
            ))
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()));
    }
    Err((StatusCode::BAD_GATEWAY, "too many redirects".into()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::{HeaderMap, StatusCode};
    use axum::response::IntoResponse;
    use std::collections::HashMap;

    async fn upstream() -> String {
        async fn playlist() -> impl IntoResponse {
            (
                [("content-type", "application/vnd.apple.mpegurl")],
                "#EXTM3U\n#EXTINF:4,\nseg0.ts\n#EXT-X-ENDLIST\n",
            )
        }
        async fn segment(headers: HeaderMap) -> axum::response::Response {
            if headers.get("referer").and_then(|v| v.to_str().ok())
                != Some("https://anilibria.top/")
            {
                return StatusCode::FORBIDDEN.into_response();
            }
            if headers.get("range").and_then(|v| v.to_str().ok()) == Some("bytes=0-3") {
                return (
                    StatusCode::PARTIAL_CONTENT,
                    [
                        ("content-range", "bytes 0-3/10"),
                        ("content-type", "video/mp2t"),
                    ],
                    "abcd",
                )
                    .into_response();
            }
            ([("content-type", "video/mp2t")], "abcdefghij").into_response()
        }
        let router = axum::Router::new()
            .route("/v/index.m3u8", axum::routing::get(playlist))
            .route("/v/seg0.ts", axum::routing::get(segment));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        format!("http://{addr}")
    }

    fn referer() -> HashMap<String, String> {
        HashMap::from([("Referer".to_string(), "https://anilibria.top/".to_string())])
    }

    #[tokio::test]
    async fn proxies_playlist_and_ranged_segment_with_session_headers() {
        let up = upstream().await;
        let proxy = StreamProxy::start_with(HttpClient::new("ninari-test"), true)
            .await
            .unwrap();
        let local = proxy.open_session(&format!("{up}/v/index.m3u8"), referer());
        assert!(local.starts_with("http://127.0.0.1:"));

        let client = reqwest::Client::new();
        let playlist = client
            .get(&local)
            .send()
            .await
            .unwrap()
            .text()
            .await
            .unwrap();
        let seg_line = playlist
            .lines()
            .find(|l| !l.starts_with('#') && !l.is_empty())
            .unwrap();
        assert!(
            seg_line.starts_with("http://127.0.0.1:"),
            "segment must go through proxy: {playlist}"
        );

        let resp = client
            .get(seg_line)
            .header("Range", "bytes=0-3")
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status().as_u16(), 206);
        assert_eq!(resp.headers()["content-range"], "bytes 0-3/10");
        assert_eq!(resp.text().await.unwrap(), "abcd");
    }

    async fn serve(router: axum::Router) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        format!("http://{addr}")
    }

    async fn get_text(url: &str) -> String {
        reqwest::get(url).await.unwrap().text().await.unwrap()
    }

    #[tokio::test]
    async fn dash_manifest_and_templated_segments_go_through_the_proxy() {
        let up = serve(
            axum::Router::new()
                .route(
                    "/dash/manifest.mpd",
                    axum::routing::get(|| async {
                        (
                            [("content-type", "application/dash+xml")],
                            r#"<MPD><Period><AdaptationSet><SegmentTemplate media="seg-$Number$.m4s"/></AdaptationSet></Period></MPD>"#,
                        )
                    }),
                )
                .route("/dash/seg-1.m4s", axum::routing::get(|| async { "segment-1" })),
        )
        .await;
        let proxy = StreamProxy::start_with(HttpClient::new("ninari-test"), true)
            .await
            .unwrap();
        let local = proxy.open_session(&format!("{up}/dash/manifest.mpd"), HashMap::new());
        let mpd = get_text(&local).await;
        let base = mpd
            .split("<BaseURL>")
            .nth(1)
            .and_then(|s| s.split("</BaseURL>").next())
            .unwrap_or_else(|| panic!("no base in {mpd}"));
        assert!(base.starts_with("http://127.0.0.1:"), "{base}");
        assert_eq!(get_text(&format!("{base}seg-1.m4s")).await, "segment-1");
    }

    #[tokio::test]
    async fn a_playlist_sent_as_plain_bytes_is_still_rewritten() {
        let up = serve(axum::Router::new().route(
            "/api/stream",
            axum::routing::get(|| async {
                (
                    [("content-type", "application/octet-stream")],
                    "#EXTM3U\n#EXTINF:4,\nseg0.ts\n#EXT-X-ENDLIST\n",
                )
            }),
        ))
        .await;
        let proxy = StreamProxy::start_with(HttpClient::new("ninari-test"), true)
            .await
            .unwrap();
        let local = proxy.open_session(&format!("{up}/api/stream"), HashMap::new());
        let playlist = get_text(&local).await;
        let seg = playlist.lines().find(|l| !l.starts_with('#')).unwrap();
        assert!(
            seg.starts_with("http://127.0.0.1:") && !seg.starts_with(&up),
            "{playlist}"
        );
    }

    /// Plays a real HLS master through the proxy with the bundled mpv and prints the picked
    /// video size. `NINARI_MASTER` (url) and `NINARI_REFERER` choose the stream:
    /// `cargo test --lib live_master_through_proxy -- --ignored --nocapture`
    #[tokio::test]
    #[ignore]
    async fn live_master_through_proxy() {
        let master = std::env::var("NINARI_MASTER").expect("NINARI_MASTER");
        let referer = std::env::var("NINARI_REFERER").unwrap_or_default();
        let proxy = StreamProxy::start(HttpClient::new("ninari-test"))
            .await
            .unwrap();
        let mut headers = HashMap::new();
        if !referer.is_empty() {
            headers.insert("Referer".to_string(), referer);
        }
        let local = proxy.open_session(&master, headers);
        let mpv = if cfg!(windows) {
            "../app/binaries/mpv-x86_64-pc-windows-msvc.exe"
        } else {
            "mpv"
        };
        let out = tokio::process::Command::new(mpv)
            .args([
                "--no-config",
                "--vo=null",
                "--ao=null",
                "--frames=24",
                &local,
            ])
            .output()
            .await
            .unwrap();
        let text = String::from_utf8_lossy(&out.stdout);
        let vo = text
            .lines()
            .find(|l| l.contains("VO:"))
            .unwrap_or("no video");
        println!("{vo}");
    }

    #[tokio::test]
    async fn refuses_private_addresses() {
        let up = upstream().await;
        let proxy = StreamProxy::start_with(HttpClient::new("ninari-test"), false)
            .await
            .unwrap();
        let local = proxy.open_session(&format!("{up}/v/index.m3u8"), referer());
        let resp = reqwest::get(&local).await.unwrap();
        assert_eq!(resp.status().as_u16(), 403);
    }

    #[tokio::test]
    async fn rejects_unknown_or_closed_session() {
        let up = upstream().await;
        let proxy = StreamProxy::start_with(HttpClient::new("ninari-test"), true)
            .await
            .unwrap();
        let local = proxy.open_session(&format!("{up}/v/index.m3u8"), referer());
        let token = local.split('/').nth(4).unwrap().to_string();
        proxy.close_session(&token);
        assert_eq!(reqwest::get(&local).await.unwrap().status().as_u16(), 403);
        let forged = local.replace(&token, "deadbeef");
        assert_eq!(reqwest::get(&forged).await.unwrap().status().as_u16(), 403);
    }
}
