//! The axum server: /healthz, identity (OIDC session for parents, bearer tokens for paired child
//! devices), the built SPA shell, and request-latency telemetry.

mod error;
pub mod family_cli;
mod identity;
pub mod oidc;
mod parent;
pub mod play;
mod ratelimit;

pub mod child;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use axum::extract::{MatchedPath, Request, State};
use axum::http::StatusCode;
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use cubby_otel::MetricsSink;
use serde_json::json;
use tower_http::services::{ServeDir, ServeFile};

pub use oidc::OidcConfig;

pub const DEFAULT_PORT: u16 = 8790;

#[derive(Clone, Debug, PartialEq)]
pub struct Config {
    pub bind: String,
    pub port: u16,
    pub web_dir: PathBuf,
    /// `host:port` of an OTLP/HTTP receiver; empty disables the push.
    pub otlp_endpoint: String,
    /// Directory holding the family database.
    pub data_dir: PathBuf,
    /// Pocket ID sign-in settings; without them every parent route refuses.
    pub oidc: Option<OidcConfig>,
    /// YouTube Data API key; with it, allowing a link ingests its metadata before the request
    /// returns. Empty leaves that to the worker's timer.
    pub yt_api_key: String,
}

impl Config {
    /// Reads `CUBBY_BIND` (127.0.0.1), `CUBBY_PORT` (8790), `CUBBY_WEB_DIR` (web/build),
    /// `CUBBY_OTLP_ENDPOINT` (empty), `CUBBY_DATA_DIR` (data), `CUBBY_OIDC_ISSUER` /
    /// `CUBBY_OIDC_CLIENT_ID` / `CUBBY_OIDC_CLIENT_SECRET` / `CUBBY_OIDC_BASE_URL`, which are set
    /// together or not at all, `CUBBY_OIDC_GROUP` (cubby-parents) and `CUBBY_SESSION_TTL_SECS`
    /// (28800).
    pub fn from_env() -> Result<Config, String> {
        Config::from_lookup(|k| std::env::var(k).ok())
    }

    pub fn from_lookup(get: impl Fn(&str) -> Option<String>) -> Result<Config, String> {
        let port = match get("CUBBY_PORT") {
            Some(p) => p
                .trim()
                .parse()
                .map_err(|_| format!("CUBBY_PORT {p:?} is not a port"))?,
            None => DEFAULT_PORT,
        };
        let var = |k: &str| get(k).filter(|v| !v.trim().is_empty());
        let oidc_vars = [
            "CUBBY_OIDC_ISSUER",
            "CUBBY_OIDC_CLIENT_ID",
            "CUBBY_OIDC_CLIENT_SECRET",
            "CUBBY_OIDC_BASE_URL",
        ];
        let session_ttl = match var("CUBBY_SESSION_TTL_SECS") {
            Some(t) => std::time::Duration::from_secs(
                t.trim()
                    .parse()
                    .map_err(|_| format!("CUBBY_SESSION_TTL_SECS {t:?} is not a number"))?,
            ),
            None => oidc::DEFAULT_SESSION_TTL,
        };
        let oidc = match oidc_vars.map(var) {
            [None, None, None, None] => None,
            [
                Some(issuer),
                Some(client_id),
                Some(client_secret),
                Some(base_url),
            ] => Some(OidcConfig {
                issuer,
                client_id,
                client_secret,
                base_url,
                group: var("CUBBY_OIDC_GROUP").unwrap_or_else(|| oidc::DEFAULT_GROUP.into()),
                session_ttl,
            }),
            vals => {
                let missing: Vec<_> = oidc_vars
                    .iter()
                    .zip(&vals)
                    .filter(|(_, v)| v.is_none())
                    .map(|(n, _)| *n)
                    .collect();
                return Err(format!(
                    "OIDC sign-in is partly configured; also set {}",
                    missing.join(", ")
                ));
            }
        };
        Ok(Config {
            oidc,
            yt_api_key: get("CUBBY_YT_API_KEY")
                .unwrap_or_default()
                .trim()
                .to_string(),
            data_dir: get("CUBBY_DATA_DIR")
                .unwrap_or_else(|| "data".into())
                .into(),
            bind: get("CUBBY_BIND").unwrap_or_else(|| "127.0.0.1".into()),
            port,
            web_dir: get("CUBBY_WEB_DIR")
                .unwrap_or_else(|| "web/build".into())
                .into(),
            otlp_endpoint: get("CUBBY_OTLP_ENDPOINT").unwrap_or_default(),
        })
    }
}

/// Shared by every handler and middleware.
pub(crate) struct AppState {
    pub registry: std::sync::Mutex<cubby_db::Registry>,
    pub oidc: Option<oidc::OidcState>,
    pub sink: Arc<MetricsSink>,
    pub pair_limiter: ratelimit::RateLimiter,
    pub ingest: Option<Ingest>,
}

/// Runs one metadata pass over a family's database; called when a parent allows a link.
pub type Ingest = Arc<dyn Fn(&rusqlite::Connection) + Send + Sync>;

impl AppState {
    /// Fire-and-forget counter push; never blocks or fails the request.
    pub(crate) fn count(self: &Arc<Self>, name: &'static str, outcome: &'static str) {
        if self.sink.enabled() {
            let state = self.clone();
            tokio::task::spawn_blocking(move || {
                state.sink.push_count(name, &[("outcome", outcome)])
            });
        }
    }
}

/// Discovers the OIDC provider and fetches its keys once so a wrong issuer fails the start; a
/// no-op without OIDC.
pub async fn check_oidc(config: &Config) -> Result<(), String> {
    match &config.oidc {
        Some(cfg) => oidc::OidcState::new(cfg).prime().await,
        None => Ok(()),
    }
}

pub fn app(config: &Config) -> Result<Router, String> {
    let ingest: Option<Ingest> = (!config.yt_api_key.is_empty()).then(|| {
        let key = config.yt_api_key.clone();
        let thumbs = config.data_dir.join("thumbs");
        Arc::new(move |conn: &rusqlite::Connection| {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_secs() as i64);
            let transport = cubby_yt::HttpTransport::new(&key);
            cubby_worker::run_pass(conn, &transport, now, Some(&thumbs));
        }) as Ingest
    });
    app_with_ingest(config, ingest)
}

/// [`app`] with the allow-time metadata pass supplied by the caller (tests pass a fixture one).
pub fn app_with_ingest(config: &Config, ingest: Option<Ingest>) -> Result<Router, String> {
    let registry = cubby_db::Registry::open(&config.data_dir)
        .map_err(|e| format!("open registry in {}: {e}", config.data_dir.display()))?;
    let sink = Arc::new(MetricsSink::new(
        Some(&config.otlp_endpoint),
        "cubby-server",
    ));
    let state = Arc::new(AppState {
        registry: std::sync::Mutex::new(registry),
        oidc: config.oidc.as_ref().map(oidc::OidcState::new),
        sink: sink.clone(),
        pair_limiter: ratelimit::RateLimiter::new(5, 60),
        ingest,
    });
    let spa =
        ServeDir::new(&config.web_dir).fallback(ServeFile::new(config.web_dir.join("index.html")));
    Ok(Router::new()
        .route("/healthz", get(healthz))
        .merge(identity::routes(state.clone()))
        .route("/api/{*rest}", axum::routing::any(api_not_found))
        .nest_service("/thumbs", ServeDir::new(config.data_dir.join("thumbs")))
        .fallback_service(spa)
        .layer(middleware::from_fn_with_state(state, parent_page_login))
        .layer(middleware::from_fn_with_state(sink, observe))
        .layer(middleware::from_fn(capture_log_line))
        .layer(middleware::from_fn(cache_control)))
}

/// Thumbnails and the versioned `/assets/icons` are cached for a year; no `/api/*` response is ever cached (errors and 401s included), and every HTML response (the SPA shell
/// included) is `no-store`, so a CDN that caches by extension can never keep it as a static file; the immutable `/_app` assets keep the long cache they are built with. Without
/// this a browser heuristically caches a header-less response forever.
async fn cache_control(req: Request, next: Next) -> Response {
    let is_api = req.uri().path().starts_with("/api/") || req.uri().path().starts_with("/auth/");
    let is_thumb = req.uri().path().starts_with("/thumbs/");
    let is_icon = req.uri().path().starts_with("/assets/icons/");
    let is_manifest = req.uri().path() == "/assets/manifest.webmanifest";
    let mut res = next.run(req).await;
    let value = if is_api {
        Some("no-store")
    } else if is_thumb {
        // Thumbnails are named for their video and never rewritten; a miss must not stick.
        Some(if res.status().is_success() {
            "public, max-age=31536000, immutable"
        } else {
            "no-store"
        })
    } else if res
        .headers()
        .get(axum::http::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|t| t.starts_with("text/html"))
    {
        Some("no-store")
    } else if (is_icon || is_manifest) && res.status().is_success() {
        // Icons carry a version in their name and are never rewritten; the manifest names them, so it may change.
        Some(if is_icon {
            "public, max-age=31536000, immutable"
        } else {
            "public, max-age=86400"
        })
    } else {
        None
    };
    if let Some(v) = value {
        res.headers_mut().insert(
            axum::http::header::CACHE_CONTROL,
            axum::http::HeaderValue::from_static(v),
        );
    }
    res
}

/// A browser asking for a `/parent` page with no live session goes to sign in rather than seeing
/// a shell whose every call is a 401.
async fn parent_page_login(
    State(state): State<Arc<AppState>>,
    req: Request,
    next: Next,
) -> Response {
    let path = req.uri().path();
    if let Some(oidc) = state.oidc.as_ref()
        && req.method() == axum::http::Method::GET
        && (path == "/parent" || path.starts_with("/parent/"))
        && oidc::session_token(req.headers())
            .and_then(|t| oidc.session_email(&t))
            .is_none()
    {
        let back = req
            .uri()
            .path_and_query()
            .map_or(path, |p| p.as_str())
            .to_string();
        return axum::response::Redirect::to(&oidc::login_url(&back)).into_response();
    }
    next.run(req).await
}

/// An unmounted `/api/*` path is a JSON 404, never the SPA shell with a 200.
async fn api_not_found() -> impl IntoResponse {
    (StatusCode::NOT_FOUND, Json(json!({"error": "not found"})))
}

async fn healthz() -> impl IntoResponse {
    (StatusCode::OK, Json(json!({"ok": true})))
}

/// Who a request was made by, filled by the guards (`device:<id>` or `parent:<email>`) so the
/// request log can name it. Never a token.
#[derive(Clone, Default)]
pub(crate) struct Actor(Arc<std::sync::Mutex<Option<String>>>);

impl Actor {
    pub(crate) fn set(&self, who: String) {
        if let Ok(mut g) = self.0.lock() {
            *g = Some(who);
        }
    }

    fn get(&self) -> Option<String> {
        self.0.lock().ok().and_then(|g| g.clone())
    }
}

/// Where request log lines go: stderr, which journald collects.
pub type LogSink = Arc<dyn Fn(&str) + Send + Sync>;

/// Writes one JSON line per request: method, route template (never the raw path, which can carry
/// ids), status, latency, the actor and, for a refusal, its message. No header, body or token.
async fn log_request(sink: Option<&LogSink>, mut req: Request, next: Next) -> Response {
    let route = req
        .extensions()
        .get::<MatchedPath>()
        .map_or_else(|| "spa".to_string(), |m| m.as_str().to_string());
    let method = req.method().to_string();
    let actor = Actor::default();
    req.extensions_mut().insert(actor.clone());
    let start = Instant::now();
    let res = next.run(req).await;
    let mut line = json!({
        "ts": SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs()),
        "method": method,
        "route": route,
        "status": res.status().as_u16(),
        "ms": start.elapsed().as_millis() as u64,
    });
    if let Some(who) = actor.get() {
        line["actor"] = who.into();
    }
    if let Some(cause) = res.extensions().get::<error::ErrorCause>() {
        line["error"] = cause.0.clone().into();
    }
    let text = format!("cubby-server: request {line}");
    match sink {
        Some(f) => f(&text),
        None => eprintln!("{text}"),
    }
    res
}

async fn capture_log_line(req: Request, next: Next) -> Response {
    let sink = req.extensions().get::<LogSink>().cloned();
    log_request(sink.as_ref(), req, next).await
}

/// Same as [`app`], but request lines go to `log` instead of stderr (tests read them).
pub fn app_with_log(config: &Config, log: LogSink) -> Result<Router, String> {
    Ok(app(config)?.layer(axum::Extension(log)))
}

async fn observe(State(sink): State<Arc<MetricsSink>>, req: Request, next: Next) -> Response {
    let route = req
        .extensions()
        .get::<MatchedPath>()
        .map_or_else(|| "spa".to_string(), |m| m.as_str().to_string());
    let start = Instant::now();
    let res = next.run(req).await;
    if sink.enabled() {
        let seconds = start.elapsed().as_secs_f64();
        let status = res.status().as_u16().to_string();
        tokio::task::spawn_blocking(move || {
            let labels = [("route", route.as_str()), ("status", status.as_str())];
            sink.push_seconds("cubby_http_request_seconds", seconds, &labels);
            sink.push_count("cubby_http_requests", &labels);
        });
    }
    res
}
