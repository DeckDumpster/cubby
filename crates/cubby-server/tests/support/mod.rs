#![allow(dead_code)]
//! A fake OIDC provider (discovery, token endpoint, JWKS) and the browser's half of the
//! authorization-code round trip, so tests sign in the way a parent does.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use axum::Json;
use axum::body::Body;
use axum::extract::{Form, State};
use axum::http::{Request, StatusCode, header};
use axum::routing::{get, post};
use jsonwebtoken::{Algorithm, EncodingKey, Header};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tower::ServiceExt;

const GOOD_PEM: &str = include_str!("../fixtures/good.pem");
const OTHER_PEM: &str = include_str!("../fixtures/other.pem");
const GOOD_N: &str = include_str!("../fixtures/good.n");
const KID: &str = "test-key";
pub const CLIENT_ID: &str = "cubby-client";
pub const CLIENT_SECRET: &str = "s3cret";
pub const BASE_URL: &str = "https://cubby.example";
pub const GROUP: &str = "cubby-parents";

pub fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
}

/// What the provider vouches for, and how it misbehaves for a negative test.
#[derive(Clone)]
pub struct User {
    pub email: String,
    pub groups: Vec<String>,
    pub signed_by_other_key: bool,
    pub aud: String,
    pub nonce: Option<String>,
    pub exp_in: i64,
}

impl User {
    pub fn parent(email: &str) -> User {
        User {
            email: email.into(),
            groups: vec![GROUP.into()],
            signed_by_other_key: false,
            aud: CLIENT_ID.into(),
            nonce: None,
            exp_in: 300,
        }
    }
}

struct Issued {
    user: User,
    nonce: String,
    challenge: String,
}

#[derive(Clone)]
pub struct Provider {
    pub issuer: String,
    codes: Arc<Mutex<HashMap<String, Issued>>>,
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

impl Provider {
    pub async fn start() -> Provider {
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let issuer = format!("http://{}", l.local_addr().unwrap());
        let p = Provider {
            issuer: issuer.clone(),
            codes: Arc::default(),
        };
        let disc = json!({
            "issuer": issuer,
            "authorization_endpoint": format!("{issuer}/authorize"),
            "token_endpoint": format!("{issuer}/token"),
            "jwks_uri": format!("{issuer}/jwks"),
            "end_session_endpoint": format!("{issuer}/logout"),
        });
        let jwks = json!({"keys": [{"kty": "RSA", "kid": KID, "alg": "RS256", "use": "sig",
            "n": GOOD_N.trim(), "e": "AQAB"}]});
        let router = axum::Router::new()
            .route(
                "/.well-known/openid-configuration",
                get(move || {
                    let d = disc.clone();
                    async move { Json(d) }
                }),
            )
            .route(
                "/jwks",
                get(move || {
                    let j = jwks.clone();
                    async move { Json(j) }
                }),
            )
            .route("/token", post(token))
            .with_state(p.clone());
        tokio::spawn(async move { axum::serve(l, router).await.unwrap() });
        p
    }

    /// The code the provider would hand the browser after the user authenticated as `user`.
    fn issue(&self, user: User, nonce: &str, challenge: &str) -> String {
        let code = format!("code-{}", self.codes.lock().unwrap().len());
        self.codes.lock().unwrap().insert(
            code.clone(),
            Issued {
                user,
                nonce: nonce.into(),
                challenge: challenge.into(),
            },
        );
        code
    }

    /// The lookup a [`cubby_server::Config`] reads, with the OIDC variables pointing here.
    pub fn env(&self, k: &str) -> Option<String> {
        match k {
            "CUBBY_OIDC_ISSUER" => Some(self.issuer.clone()),
            "CUBBY_OIDC_CLIENT_ID" => Some(CLIENT_ID.into()),
            "CUBBY_OIDC_CLIENT_SECRET" => Some(CLIENT_SECRET.into()),
            "CUBBY_OIDC_BASE_URL" => Some(BASE_URL.into()),
            _ => None,
        }
    }
}

async fn token(
    State(p): State<Provider>,
    Form(f): Form<HashMap<String, String>>,
) -> Result<Json<Value>, StatusCode> {
    let get = |k: &str| f.get(k).map(String::as_str).unwrap_or_default();
    if get("client_id") != CLIENT_ID
        || get("client_secret") != CLIENT_SECRET
        || get("grant_type") != "authorization_code"
        || get("redirect_uri") != format!("{BASE_URL}/auth/callback")
    {
        return Err(StatusCode::UNAUTHORIZED);
    }
    let issued = p
        .codes
        .lock()
        .unwrap()
        .remove(get("code"))
        .ok_or(StatusCode::BAD_REQUEST)?;
    if base64url(&Sha256::digest(get("code_verifier").as_bytes())) != issued.challenge {
        return Err(StatusCode::BAD_REQUEST);
    }
    let u = issued.user;
    let claims = json!({
        "iss": p.issuer, "aud": u.aud, "sub": "sub-1", "email": u.email, "groups": u.groups,
        "nonce": u.nonce.unwrap_or(issued.nonce),
        "iat": now(), "exp": now() + u.exp_in,
    });
    let mut h = Header::new(Algorithm::RS256);
    h.kid = Some(KID.into());
    let pem = if u.signed_by_other_key {
        OTHER_PEM
    } else {
        GOOD_PEM
    };
    let id_token = jsonwebtoken::encode(
        &h,
        &claims,
        &EncodingKey::from_rsa_pem(pem.as_bytes()).unwrap(),
    )
    .unwrap();
    Ok(Json(
        json!({"id_token": id_token, "access_token": "x", "token_type": "Bearer"}),
    ))
}

pub struct Outcome {
    pub status: StatusCode,
    pub body: Value,
    /// `cubby_session=<token>`, ready to send back as a `Cookie` header.
    pub cookie: Option<String>,
    pub location: Option<String>,
    /// The whole `Set-Cookie` line of the session cookie, attributes included.
    pub set_cookie: Option<String>,
}

async fn outcome_of(res: axum::response::Response) -> Outcome {
    {
        let status = res.status();
        let location = res
            .headers()
            .get(header::LOCATION)
            .map(|v| v.to_str().unwrap().to_string());
        let set_cookie = res
            .headers()
            .get_all(header::SET_COOKIE)
            .iter()
            .filter_map(|v| v.to_str().ok())
            .find(|c| c.starts_with("cubby_session=") && !c.starts_with("cubby_session=;"))
            .map(str::to_string);
        let cookie = set_cookie
            .as_ref()
            .map(|c| c.split(';').next().unwrap().to_string());
        use http_body_util::BodyExt;
        let bytes = res.into_body().collect().await.unwrap().to_bytes();
        Outcome {
            status,
            body: serde_json::from_slice(&bytes).unwrap_or(Value::Null),
            cookie,
            location,
            set_cookie,
        }
    }
}

/// The browser's walk: `/auth/login`, the provider's redirect back with a code, `/auth/callback`.
pub async fn sign_in_as(
    router: &axum::Router,
    provider: &Provider,
    user: User,
    next: Option<&str>,
) -> Outcome {
    let uri = match next {
        Some(n) => format!(
            "/auth/login?next={}",
            reqwest::Url::parse_with_params("http://x/", [("n", n)])
                .unwrap()
                .query()
                .unwrap()
                .trim_start_matches("n=")
        ),
        None => "/auth/login".into(),
    };
    let res = router
        .clone()
        .oneshot(Request::get(uri).body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::SEE_OTHER, "login must redirect");
    let to = res.headers()[header::LOCATION]
        .to_str()
        .unwrap()
        .to_string();
    let url = reqwest::Url::parse(&to).unwrap();
    assert!(
        to.starts_with(&format!("{}/authorize", provider.issuer)),
        "{to}"
    );
    let q: HashMap<_, _> = url.query_pairs().into_owned().collect();
    assert_eq!(q["response_type"], "code");
    assert_eq!(q["client_id"], CLIENT_ID);
    assert_eq!(q["redirect_uri"], format!("{BASE_URL}/auth/callback"));
    assert_eq!(q["scope"], "openid email profile groups");
    assert_eq!(q["code_challenge_method"], "S256");
    let code = provider.issue(user, &q["nonce"], &q["code_challenge"]);
    callback(router, &code, &q["state"]).await
}

pub async fn callback(router: &axum::Router, code: &str, state: &str) -> Outcome {
    let u = reqwest::Url::parse_with_params(
        "http://x/auth/callback",
        [("code", code), ("state", state)],
    )
    .unwrap();
    let res = router
        .clone()
        .oneshot(
            Request::get(format!("{}?{}", u.path(), u.query().unwrap()))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    outcome_of(res).await
}

pub async fn sign_in(router: &axum::Router, provider: &Provider, email: &str) -> Outcome {
    sign_in_as(router, provider, User::parent(email), None).await
}

/// Signs `email` in and returns the session cookie; panics if the sign-in is refused.
pub async fn session_cookie(router: &axum::Router, provider: &Provider, email: &str) -> String {
    let o = sign_in(router, provider, email).await;
    assert_eq!(
        o.status,
        StatusCode::SEE_OTHER,
        "sign-in refused: {}",
        o.body
    );
    o.cookie.expect("a session cookie")
}
