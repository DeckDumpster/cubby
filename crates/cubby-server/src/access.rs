//! Cloudflare Access JWT validation, for `/parent` and `/api/parent`.
//!
//! Copied and adapted from pokedumpster's Access verifier (not a dependency on it). RS256 only,
//! with issuer, audience, `exp` and `nbf` all checked, against the team's published JWKS. The
//! token comes from the `Cf-Access-Jwt-Assertion` header or the `CF_Authorization` cookie.
//!
//! Keys are cached for an hour. An unknown `kid` re-fetches, but at most once a minute, so a
//! fabricated token cannot be used to hammer the JWKS endpoint. A missing or invalid token is a
//! 401 with no fallthrough.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use axum::http::{StatusCode, header, request::Parts};
use jsonwebtoken::{Algorithm, DecodingKey, Validation, decode};
use serde::Deserialize;
use tokio::sync::RwLock;

use crate::error::ApiError;

pub const JWT_HEADER: &str = "cf-access-jwt-assertion";
pub const JWT_COOKIE_NAME: &str = "CF_Authorization";

const CACHE_TTL: Duration = Duration::from_secs(3600);
const REFRESH_RATE_LIMIT: Duration = Duration::from_secs(60);

#[derive(Clone, Debug, PartialEq)]
pub struct AccessConfig {
    /// Full team URL, e.g. `https://myteam.cloudflareaccess.com`; the expected `iss`.
    pub team_domain: String,
    /// The Access application's audience tag.
    pub aud: String,
    /// The JWKS endpoint. Explicit rather than derived, so tests can point at localhost.
    pub jwks_url: String,
}

#[derive(Deserialize)]
struct Jwk {
    kid: String,
    n: String,
    e: String,
}

#[derive(Deserialize)]
struct JwkSet {
    keys: Vec<Jwk>,
}

struct CacheState {
    keys: HashMap<String, DecodingKey>,
    fetched_at: Instant,
    last_refresh_attempt: Option<Instant>,
}

/// Proof that a request carried a valid Access JWT. Only [`AccessState::verify`] makes one.
#[derive(Clone, Debug)]
pub struct VerifiedIdentity {
    email: String,
}

impl VerifiedIdentity {
    /// The user's email, lowercased.
    pub fn email(&self) -> &str {
        &self.email
    }
}

#[derive(Deserialize)]
struct Claims {
    email: String,
}

pub struct AccessState {
    aud: String,
    iss: String,
    jwks_url: String,
    client: reqwest::Client,
    cache: RwLock<Option<CacheState>>,
}

impl AccessState {
    pub fn new(cfg: &AccessConfig) -> AccessState {
        AccessState {
            aud: cfg.aud.clone(),
            iss: cfg.team_domain.trim_end_matches('/').to_string(),
            jwks_url: cfg.jwks_url.clone(),
            client: reqwest::Client::builder()
                .timeout(Duration::from_secs(10))
                .build()
                .expect("a plain reqwest client builds"),
            cache: RwLock::new(None),
        }
    }

    async fn fetch(&self) -> Result<HashMap<String, DecodingKey>, String> {
        let set: JwkSet = async {
            self.client
                .get(&self.jwks_url)
                .send()
                .await?
                .error_for_status()?
                .json()
                .await
        }
        .await
        .map_err(|e: reqwest::Error| e.to_string())?;
        Ok(set
            .keys
            .into_iter()
            .filter_map(|j| {
                DecodingKey::from_rsa_components(&j.n, &j.e)
                    .ok()
                    .map(|k| (j.kid, k))
            })
            .collect())
    }

    /// Fetches the JWKS once, so a bad URL fails the start rather than the first login. A failed
    /// fetch or an empty key set is an error.
    pub async fn prime(&self) -> Result<(), String> {
        let keys = self
            .fetch()
            .await
            .map_err(|e| format!("JWKS fetch from {} failed: {e}", self.jwks_url))?;
        if keys.is_empty() {
            return Err(format!(
                "JWKS at {} returned no usable RSA keys",
                self.jwks_url
            ));
        }
        *self.cache.write().await = Some(CacheState {
            keys,
            fetched_at: Instant::now(),
            last_refresh_attempt: None,
        });
        Ok(())
    }

    async fn key(&self, kid: &str) -> Result<DecodingKey, ApiError> {
        {
            let guard = self.cache.read().await;
            if let Some(c) = guard.as_ref()
                && c.fetched_at.elapsed() < CACHE_TTL
                && let Some(k) = c.keys.get(kid)
            {
                return Ok(k.clone());
            }
        }
        let mut guard = self.cache.write().await;
        if let Some(c) = guard.as_ref()
            && c.fetched_at.elapsed() < CACHE_TTL
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
        match self.fetch().await {
            Ok(keys) => {
                let found = keys.get(kid).cloned();
                *guard = Some(CacheState {
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

    pub async fn verify(&self, token: &str) -> Result<VerifiedIdentity, ApiError> {
        let header = jsonwebtoken::decode_header(token)
            .map_err(|_| ApiError::unauthorized("invalid JWT"))?;
        let kid = header
            .kid
            .ok_or_else(|| ApiError::unauthorized("JWT missing kid"))?;
        let key = self.key(&kid).await?;
        let mut validation = Validation::new(Algorithm::RS256);
        validation.set_audience(&[&self.aud]);
        validation.set_issuer(&[&self.iss]);
        validation.validate_nbf = true;
        // jsonwebtoken checks `aud` and `iss` only when the token carries them; requiring them
        // stops a token that omits both from skipping both checks.
        validation.set_required_spec_claims(&["exp", "aud", "iss"]);
        let data = decode::<Claims>(token, &key, &validation)
            .map_err(|e| ApiError::unauthorized(format!("JWT rejected: {e}")))?;
        let email = data.claims.email.trim().to_lowercase();
        if email.is_empty() {
            return Err(ApiError::unauthorized("JWT carries no email"));
        }
        Ok(VerifiedIdentity { email })
    }
}

/// The JWT from the `Cf-Access-Jwt-Assertion` header, falling back to the `CF_Authorization`
/// cookie.
pub fn extract_token(parts: &Parts) -> Option<String> {
    if let Some(v) = parts.headers.get(JWT_HEADER)
        && let Ok(s) = v.to_str()
    {
        return Some(s.to_string());
    }
    for cookie in parts.headers.get_all(header::COOKIE) {
        for part in cookie.to_str().unwrap_or_default().split(';') {
            if let Some(v) = part
                .trim()
                .strip_prefix(JWT_COOKIE_NAME)
                .and_then(|s| s.strip_prefix('='))
            {
                return Some(v.to_string());
            }
        }
    }
    None
}

pub fn missing_token() -> ApiError {
    ApiError::new(StatusCode::UNAUTHORIZED, "missing Cloudflare Access token")
}
