//! Parent sign-in: OpenID Connect (authorization code + PKCE) against Pocket ID, in place of the
//! Cloudflare Access JWT.
//!
//! `/auth/login` sends the browser to the provider, `/auth/callback` exchanges the code, checks
//! the ID token (signature against the provider's JWKS, `iss`, `aud`, `exp`, `nonce`) and the
//! required group, and opens a server-side session held in memory behind an HttpOnly cookie.
//! `/auth/logout` ends it and the provider's session with it (RP-initiated logout), landing on
//! `/signed-out`, a public page that never starts a sign-in by itself. A restart signs everyone out, which costs a parent one redirect.
//!
//! Discovery and JWKS are fetched lazily and cached; an unknown `kid` re-fetches the JWKS, but at
//! most once a minute, so a fabricated token cannot be used to hammer the provider.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::Router;
use axum::extract::{Query, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use jsonwebtoken::jwk::Jwk;
use jsonwebtoken::{Algorithm, DecodingKey, Validation, decode};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use tokio::sync::RwLock;

use crate::AppState;
use crate::error::ApiError;

pub const SESSION_COOKIE: &str = "cubby_session";
pub const DEFAULT_GROUP: &str = "cubby-parents";
pub const DEFAULT_SESSION_TTL: Duration = Duration::from_secs(8 * 3600);
/// Where a sign-in lands when it was not asked to go somewhere particular.
const DEFAULT_NEXT: &str = "/parent/allow";

const KEY_TTL: Duration = Duration::from_secs(3600);
const REFRESH_RATE_LIMIT: Duration = Duration::from_secs(60);
const PENDING_TTL: Duration = Duration::from_secs(600);
const PENDING_CAP: usize = 1000;

#[derive(Clone, Debug, PartialEq)]
pub struct OidcConfig {
    /// The provider's issuer URL; discovery is `<issuer>/.well-known/openid-configuration` and the
    /// ID token's `iss` must equal it.
    pub issuer: String,
    pub client_id: String,
    pub client_secret: String,
    /// The public origin of this app, e.g. `https://cubby.example`; the redirect URI is
    /// `<base_url>/auth/callback`, and an `https` origin marks the session cookie `Secure`.
    pub base_url: String,
    /// The provider group a parent must be in.
    pub group: String,
    pub session_ttl: Duration,
}

#[derive(Deserialize, Clone)]
struct Discovery {
    issuer: String,
    authorization_endpoint: String,
    token_endpoint: String,
    jwks_uri: String,
    /// RP-initiated logout; absent when the provider has no single sign-out.
    #[serde(default)]
    end_session_endpoint: Option<String>,
}

struct KeyCache {
    keys: HashMap<String, DecodingKey>,
    fetched_at: Instant,
    last_refresh_attempt: Option<Instant>,
}

struct Pending {
    nonce: String,
    verifier: String,
    next: String,
    created: Instant,
}

struct Session {
    email: String,
    /// Handed back to the provider as `id_token_hint` when the parent signs out.
    id_token: String,
    expires: Instant,
}

#[derive(Deserialize)]
struct Claims {
    #[serde(default)]
    email: String,
    #[serde(default)]
    nonce: String,
    #[serde(default)]
    groups: Vec<String>,
}

pub struct OidcState {
    cfg: OidcConfig,
    issuer: String,
    client: reqwest::Client,
    discovery: RwLock<Option<Arc<Discovery>>>,
    keys: RwLock<Option<KeyCache>>,
    pending: Mutex<HashMap<String, Pending>>,
    sessions: Mutex<HashMap<String, Session>>,
}

fn random_hex(bytes: usize) -> String {
    let mut b = vec![0u8; bytes];
    getrandom::fill(&mut b).expect("the OS random source is unavailable");
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn base64url(data: &[u8]) -> String {
    const A: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = String::new();
    for c in data.chunks(3) {
        let n = c
            .iter()
            .enumerate()
            .fold(0u32, |n, (i, b)| n | (*b as u32) << (16 - 8 * i));
        for i in 0..=c.len() {
            out.push(A[(n >> (18 - 6 * i) & 63) as usize] as char);
        }
    }
    out
}

/// A same-site path or the default; never an absolute URL, so a sign-in cannot be bounced
/// somewhere else.
fn safe_next(next: Option<&str>) -> String {
    match next {
        Some(n) if n.starts_with('/') && !n.starts_with("//") && !n.contains('\\') => n.to_string(),
        _ => DEFAULT_NEXT.to_string(),
    }
}

/// The login URL that returns to `next` afterwards.
pub fn login_url(next: &str) -> String {
    let mut u = reqwest::Url::parse("http://x/auth/login").expect("static URL");
    u.query_pairs_mut().append_pair("next", next);
    format!("/auth/login?{}", u.query().unwrap_or_default())
}

impl OidcState {
    pub fn new(cfg: &OidcConfig) -> OidcState {
        OidcState {
            issuer: cfg.issuer.trim_end_matches('/').to_string(),
            cfg: cfg.clone(),
            client: reqwest::Client::builder()
                .timeout(Duration::from_secs(10))
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .expect("a plain reqwest client builds"),
            discovery: RwLock::new(None),
            keys: RwLock::new(None),
            pending: Mutex::new(HashMap::new()),
            sessions: Mutex::new(HashMap::new()),
        }
    }

    fn redirect_uri(&self) -> String {
        format!("{}/auth/callback", self.cfg.base_url.trim_end_matches('/'))
    }

    async fn discover(&self) -> Result<Arc<Discovery>, String> {
        if let Some(d) = self.discovery.read().await.as_ref() {
            return Ok(d.clone());
        }
        let url = format!("{}/.well-known/openid-configuration", self.issuer);
        let d: Discovery = async {
            self.client
                .get(&url)
                .send()
                .await?
                .error_for_status()?
                .json()
                .await
        }
        .await
        .map_err(|e: reqwest::Error| format!("OIDC discovery at {url} failed: {e}"))?;
        if d.issuer.trim_end_matches('/') != self.issuer {
            return Err(format!(
                "OIDC discovery at {url} names issuer {:?}, not {:?}",
                d.issuer, self.issuer
            ));
        }
        let d = Arc::new(d);
        *self.discovery.write().await = Some(d.clone());
        Ok(d)
    }

    async fn fetch_keys(&self, jwks_uri: &str) -> Result<HashMap<String, DecodingKey>, String> {
        let set: serde_json::Value = async {
            self.client
                .get(jwks_uri)
                .send()
                .await?
                .error_for_status()?
                .json()
                .await
        }
        .await
        .map_err(|e: reqwest::Error| e.to_string())?;
        Ok(set["keys"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|k| serde_json::from_value::<Jwk>(k.clone()).ok())
            .filter_map(|j| {
                let kid = j.common.key_id.clone()?;
                DecodingKey::from_jwk(&j).ok().map(|k| (kid, k))
            })
            .collect())
    }

    /// Discovers the provider and fetches its keys once, so a wrong issuer fails the start
    /// rather than the first login.
    pub async fn prime(&self) -> Result<(), String> {
        let d = self.discover().await?;
        let keys = self
            .fetch_keys(&d.jwks_uri)
            .await
            .map_err(|e| format!("JWKS fetch from {} failed: {e}", d.jwks_uri))?;
        if keys.is_empty() {
            return Err(format!("JWKS at {} returned no usable keys", d.jwks_uri));
        }
        *self.keys.write().await = Some(KeyCache {
            keys,
            fetched_at: Instant::now(),
            last_refresh_attempt: None,
        });
        Ok(())
    }

    async fn key(&self, d: &Discovery, kid: &str) -> Result<DecodingKey, ApiError> {
        {
            let guard = self.keys.read().await;
            if let Some(c) = guard.as_ref()
                && c.fetched_at.elapsed() < KEY_TTL
                && let Some(k) = c.keys.get(kid)
            {
                return Ok(k.clone());
            }
        }
        let mut guard = self.keys.write().await;
        if let Some(c) = guard.as_ref()
            && c.fetched_at.elapsed() < KEY_TTL
        {
            if let Some(k) = c.keys.get(kid) {
                return Ok(k.clone());
            }
            if c.last_refresh_attempt
                .is_some_and(|t| t.elapsed() < REFRESH_RATE_LIMIT)
            {
                return Err(ApiError::unauthorized("unknown signing key"));
            }
        }
        let now = Instant::now();
        match self.fetch_keys(&d.jwks_uri).await {
            Ok(keys) => {
                let found = keys.get(kid).cloned();
                *guard = Some(KeyCache {
                    keys,
                    fetched_at: now,
                    last_refresh_attempt: Some(now),
                });
                found.ok_or_else(|| ApiError::unauthorized("unknown signing key"))
            }
            Err(_) => {
                if let Some(c) = guard.as_mut() {
                    c.last_refresh_attempt = Some(now);
                }
                Err(ApiError::unauthorized("key refresh failed"))
            }
        }
    }

    async fn verify_id_token(
        &self,
        d: &Discovery,
        token: &str,
        nonce: &str,
    ) -> Result<Claims, ApiError> {
        let header = jsonwebtoken::decode_header(token)
            .map_err(|_| ApiError::unauthorized("invalid ID token"))?;
        if !matches!(
            header.alg,
            Algorithm::RS256 | Algorithm::ES256 | Algorithm::EdDSA
        ) {
            return Err(ApiError::unauthorized("ID token algorithm not accepted"));
        }
        let kid = header
            .kid
            .ok_or_else(|| ApiError::unauthorized("ID token missing kid"))?;
        let key = self.key(d, &kid).await?;
        let mut validation = Validation::new(header.alg);
        validation.set_audience(&[&self.cfg.client_id]);
        validation.set_issuer(&[&self.issuer]);
        validation.set_required_spec_claims(&["exp", "aud", "iss"]);
        let claims = decode::<Claims>(token, &key, &validation)
            .map_err(|e| ApiError::unauthorized(format!("ID token rejected: {e}")))?
            .claims;
        if claims.nonce != nonce {
            return Err(ApiError::unauthorized("ID token nonce mismatch"));
        }
        Ok(claims)
    }

    fn open_session(&self, email: &str, id_token: &str) -> String {
        let token = random_hex(32);
        let now = Instant::now();
        let mut s = self.sessions.lock().unwrap_or_else(|e| e.into_inner());
        s.retain(|_, v| v.expires > now);
        s.insert(
            token.clone(),
            Session {
                email: email.to_string(),
                id_token: id_token.to_string(),
                expires: now + self.cfg.session_ttl,
            },
        );
        token
    }

    /// The email of the live session behind `token`, if any.
    pub fn session_email(&self, token: &str) -> Option<String> {
        let mut s = self.sessions.lock().unwrap_or_else(|e| e.into_inner());
        match s.get(token) {
            Some(v) if v.expires > Instant::now() => Some(v.email.clone()),
            Some(_) => {
                s.remove(token);
                None
            }
            None => None,
        }
    }

    /// Drops the session and returns the ID token it was opened with.
    fn end_session(&self, token: &str) -> Option<String> {
        self.sessions
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(token)
            .map(|s| s.id_token)
    }

    fn cookie(&self, value: &str, max_age: u64) -> HeaderValue {
        let secure = if self.cfg.base_url.starts_with("https://") {
            "; Secure"
        } else {
            ""
        };
        HeaderValue::from_str(&format!(
            "{SESSION_COOKIE}={value}; Path=/; HttpOnly; SameSite=Lax; Max-Age={max_age}{secure}"
        ))
        .expect("a hex cookie value is a valid header")
    }
}

/// The session token in the request's `Cookie` header.
pub fn session_token(headers: &HeaderMap) -> Option<String> {
    for cookie in headers.get_all(header::COOKIE) {
        for part in cookie.to_str().unwrap_or_default().split(';') {
            if let Some(v) = part
                .trim()
                .strip_prefix(SESSION_COOKIE)
                .and_then(|s| s.strip_prefix('='))
                && !v.is_empty()
            {
                return Some(v.to_string());
            }
        }
    }
    None
}

fn redirect(to: &str, cookie: Option<HeaderValue>) -> Response {
    let mut res = (
        StatusCode::SEE_OTHER,
        [(
            header::LOCATION,
            HeaderValue::from_str(to).unwrap_or_else(|_| HeaderValue::from_static("/")),
        )],
    )
        .into_response();
    if let Some(c) = cookie {
        res.headers_mut().append(header::SET_COOKIE, c);
    }
    res
}

pub(crate) fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/auth/login", get(login))
        .route("/auth/callback", get(callback))
        .route("/auth/logout", get(logout).post(logout))
}

fn oidc(state: &AppState) -> Result<&OidcState, ApiError> {
    state.oidc.as_ref().ok_or_else(|| {
        ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "OIDC sign-in is not configured",
        )
    })
}

#[derive(Deserialize)]
struct LoginQuery {
    next: Option<String>,
}

async fn login(
    State(state): State<Arc<AppState>>,
    Query(q): Query<LoginQuery>,
) -> Result<Response, ApiError> {
    let o = oidc(&state)?;
    let d = o
        .discover()
        .await
        .map_err(|e| ApiError::new(StatusCode::BAD_GATEWAY, e))?;
    let sid = random_hex(16);
    let nonce = random_hex(16);
    let verifier = random_hex(32);
    let challenge = base64url(&Sha256::digest(verifier.as_bytes()));
    {
        let mut p = o.pending.lock().unwrap_or_else(|e| e.into_inner());
        p.retain(|_, v| v.created.elapsed() < PENDING_TTL);
        if p.len() >= PENDING_CAP {
            return Err(ApiError::new(
                StatusCode::TOO_MANY_REQUESTS,
                "too many sign-ins in flight",
            ));
        }
        p.insert(
            sid.clone(),
            Pending {
                nonce: nonce.clone(),
                verifier,
                next: safe_next(q.next.as_deref()),
                created: Instant::now(),
            },
        );
    }
    let mut url = reqwest::Url::parse(&d.authorization_endpoint).map_err(|e| {
        ApiError::new(
            StatusCode::BAD_GATEWAY,
            format!("bad authorization endpoint: {e}"),
        )
    })?;
    url.query_pairs_mut()
        .append_pair("response_type", "code")
        .append_pair("client_id", &o.cfg.client_id)
        .append_pair("redirect_uri", &o.redirect_uri())
        .append_pair("scope", "openid email profile groups")
        .append_pair("state", &sid)
        .append_pair("nonce", &nonce)
        .append_pair("code_challenge", &challenge)
        .append_pair("code_challenge_method", "S256");
    state.count("cubby_parent_login", "started");
    Ok(redirect(url.as_str(), None))
}

#[derive(Deserialize)]
struct CallbackQuery {
    code: Option<String>,
    state: Option<String>,
    error: Option<String>,
}

#[derive(Deserialize)]
struct TokenResponse {
    id_token: String,
}

async fn callback(
    State(state): State<Arc<AppState>>,
    Query(q): Query<CallbackQuery>,
) -> Result<Response, ApiError> {
    let o = oidc(&state)?;
    let pending = q
        .state
        .as_deref()
        .and_then(|s| {
            o.pending
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .remove(s)
        })
        .filter(|p| p.created.elapsed() < PENDING_TTL);
    let Some(pending) = pending else {
        state.count("cubby_parent_login", "bad_state");
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "unknown or expired sign-in; start again at /parent",
        ));
    };
    if let Some(e) = q.error {
        state.count("cubby_parent_login", "provider_error");
        return Err(ApiError::new(
            StatusCode::FORBIDDEN,
            format!("sign-in refused by the provider: {e}"),
        ));
    }
    let Some(code) = q.code else {
        state.count("cubby_parent_login", "bad_state");
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "no authorization code",
        ));
    };
    let d = o
        .discover()
        .await
        .map_err(|e| ApiError::new(StatusCode::BAD_GATEWAY, e))?;
    let redirect_uri = o.redirect_uri();
    let tokens: TokenResponse = async {
        o.client
            .post(&d.token_endpoint)
            .form(&[
                ("grant_type", "authorization_code"),
                ("code", code.as_str()),
                ("redirect_uri", redirect_uri.as_str()),
                ("client_id", o.cfg.client_id.as_str()),
                ("client_secret", o.cfg.client_secret.as_str()),
                ("code_verifier", pending.verifier.as_str()),
            ])
            .send()
            .await?
            .error_for_status()?
            .json()
            .await
    }
    .await
    .map_err(|e: reqwest::Error| {
        state.count("cubby_parent_login", "exchange_failed");
        ApiError::new(
            StatusCode::BAD_GATEWAY,
            format!("code exchange failed: {}", e.without_url()),
        )
    })?;
    let claims = match o
        .verify_id_token(&d, &tokens.id_token, &pending.nonce)
        .await
    {
        Ok(c) => c,
        Err(e) => {
            state.count("cubby_parent_login", "invalid");
            return Err(e);
        }
    };
    let email = claims.email.trim().to_lowercase();
    if email.is_empty() {
        state.count("cubby_parent_login", "invalid");
        return Err(ApiError::unauthorized("ID token carries no email"));
    }
    if !claims.groups.iter().any(|g| g == &o.cfg.group) {
        state.count("cubby_parent_login", "missing_group");
        return Err(ApiError::new(
            StatusCode::FORBIDDEN,
            format!("not in the {} group", o.cfg.group),
        ));
    }
    let family = {
        let registry = state.registry.lock().map_err(ApiError::internal)?;
        registry.lookup(&email).map_err(ApiError::internal)?
    };
    if family.is_none() {
        state.count("cubby_parent_login", "not_a_parent");
        return Err(ApiError::new(
            StatusCode::FORBIDDEN,
            "not a parent of any family",
        ));
    }
    let token = o.open_session(&email, &tokens.id_token);
    state.count("cubby_parent_login", "ok");
    Ok(redirect(
        &pending.next,
        Some(o.cookie(&token, o.cfg.session_ttl.as_secs())),
    ))
}

async fn logout(State(state): State<Arc<AppState>>, headers: HeaderMap) -> Response {
    let Some(o) = state.oidc.as_ref() else {
        return redirect("/", None);
    };
    let id_token = session_token(&headers).and_then(|t| o.end_session(&t));
    // Ending only our own session would leave the provider's, which signs the parent straight
    // back in; so the provider's session ends too, and it returns the browser to /signed-out.
    let signed_out = format!("{}/signed-out", o.cfg.base_url.trim_end_matches('/'));
    let to = match o.discover().await {
        Ok(d) => d
            .end_session_endpoint
            .as_deref()
            .and_then(|e| reqwest::Url::parse(e).ok())
            .map(|mut u| {
                {
                    let mut q = u.query_pairs_mut();
                    if let Some(t) = &id_token {
                        q.append_pair("id_token_hint", t);
                    }
                    q.append_pair("client_id", &o.cfg.client_id)
                        .append_pair("post_logout_redirect_uri", &signed_out);
                }
                u.to_string()
            })
            .unwrap_or_else(|| "/signed-out".into()),
        Err(_) => "/signed-out".into(),
    };
    redirect(&to, Some(o.cookie("", 0)))
}
