//! Identity over HTTP: Access JWT verification, device pairing, tokens and revocation.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::PathBuf;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use cubby_db::Registry;
use cubby_db::identity as ident;
use cubby_server::{Config, app};
use http_body_util::BodyExt;
use jsonwebtoken::{Algorithm, EncodingKey, Header};
use rusqlite::Connection;
use serde_json::{Value, json};
use tower::ServiceExt;

const GOOD_PEM: &str = include_str!("fixtures/good.pem");
const OTHER_PEM: &str = include_str!("fixtures/other.pem");
const GOOD_N: &str = include_str!("fixtures/good.n");
const KID: &str = "test-key";
const ISS: &str = "https://fam.cloudflareaccess.com";
const AUD: &str = "aud-tag";
const PARENT: &str = "mum@example.com";

struct Env {
    router: axum::Router,
    dir: PathBuf,
    family_id: String,
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
}

fn temp_dir() -> PathBuf {
    let d = std::env::temp_dir().join(format!(
        "cubby-id-{}-{:?}-{}",
        std::process::id(),
        std::thread::current().id(),
        now()
    ));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    std::fs::write(d.join("index.html"), "<html>shell</html>").unwrap();
    d
}

async fn jwks_server() -> String {
    let jwks = json!({"keys": [{"kty": "RSA", "kid": KID, "alg": "RS256", "use": "sig",
        "n": GOOD_N.trim(), "e": "AQAB"}]});
    let router = axum::Router::new().route(
        "/jwks",
        axum::routing::get(move || {
            let j = jwks.clone();
            async move { axum::Json(j) }
        }),
    );
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/jwks", l.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(l, router).await.unwrap() });
    url
}

async fn env_with(access: bool, otlp: &str) -> Env {
    let dir = temp_dir();
    let jwks = if access {
        jwks_server().await
    } else {
        String::new()
    };
    let cfg = Config::from_lookup(|k| match k {
        "CUBBY_WEB_DIR" => Some(dir.display().to_string()),
        "CUBBY_DATA_DIR" => Some(dir.join("data").display().to_string()),
        "CUBBY_OTLP_ENDPOINT" => Some(otlp.into()),
        "CUBBY_ACCESS_TEAM_DOMAIN" if access => Some(ISS.into()),
        "CUBBY_ACCESS_AUD" if access => Some(AUD.into()),
        "CUBBY_ACCESS_JWKS_URL" if access => Some(jwks.clone()),
        _ => None,
    })
    .unwrap();
    let data = dir.join("data");
    let mut registry = Registry::open(&data).unwrap();
    let family_id = registry.create_family(PARENT).unwrap().id;
    drop(registry);
    let router = app(&cfg).unwrap();
    Env {
        router,
        dir,
        family_id,
    }
}

async fn env() -> Env {
    env_with(true, "").await
}

impl Env {
    fn family(&self) -> Connection {
        Registry::open(&self.dir.join("data"))
            .unwrap()
            .open_family(&self.family_id)
            .unwrap()
    }

    async fn call(&self, req: Request<Body>) -> (StatusCode, Value) {
        let res = self.router.clone().oneshot(req).await.unwrap();
        let status = res.status();
        let body = res.into_body().collect().await.unwrap().to_bytes();
        (status, serde_json::from_slice(&body).unwrap_or(Value::Null))
    }

    async fn parent_post(&self, path: &str, jwt: Option<&str>) -> (StatusCode, Value) {
        let mut r = Request::post(path);
        if let Some(j) = jwt {
            r = r.header("cf-access-jwt-assertion", j);
        }
        self.call(r.body(Body::empty()).unwrap()).await
    }

    async fn pair(&self, code: &str, ip: &str) -> (StatusCode, Value) {
        let r = Request::post("/api/child/pair")
            .header("content-type", "application/json")
            .header("cf-connecting-ip", ip)
            .body(Body::from(
                json!({"code": code, "label": "iPad"}).to_string(),
            ))
            .unwrap();
        self.call(r).await
    }

    async fn me(&self, token: Option<&str>) -> (StatusCode, Value) {
        let mut r = Request::get("/api/child/me");
        if let Some(t) = token {
            r = r.header("authorization", format!("Bearer {t}"));
        }
        self.call(r.body(Body::empty()).unwrap()).await
    }
}

fn claims(email: &str, aud: &str, iss: &str, exp: i64, nbf: i64) -> Value {
    json!({"email": email, "sub": "sub-1", "aud": aud, "iss": iss, "exp": exp, "nbf": nbf})
}

fn sign(pem: &str, alg: Algorithm, kid: Option<&str>, claims: &Value) -> String {
    let mut h = Header::new(alg);
    h.kid = kid.map(str::to_string);
    let key = match alg {
        Algorithm::RS256 => EncodingKey::from_rsa_pem(pem.as_bytes()).unwrap(),
        _ => EncodingKey::from_secret(b"secret"),
    };
    jsonwebtoken::encode(&h, claims, &key).unwrap()
}

fn good_jwt(email: &str) -> String {
    sign(
        GOOD_PEM,
        Algorithm::RS256,
        Some(KID),
        &claims(email, AUD, ISS, now() + 300, now() - 10),
    )
}

const PAIRING: &str = "/api/parent/children/c1/pairing";

fn seed(e: &Env) {
    ident::add_child(&e.family(), "c1", "Ada", "Europe/London").unwrap();
}

#[tokio::test]
async fn a_valid_access_jwt_from_a_parent_is_accepted() {
    let e = env().await;
    seed(&e);
    let (s, body) = e.parent_post(PAIRING, Some(&good_jwt(PARENT))).await;
    assert_eq!(s, StatusCode::OK, "{body}");
    assert_eq!(body["code"].as_str().unwrap().len(), 6);
    assert!(body["expires_at"].as_i64().unwrap() > now());
}

#[tokio::test]
async fn the_access_cookie_is_accepted_like_the_header() {
    let e = env().await;
    seed(&e);
    let r = Request::post(PAIRING)
        .header(
            "cookie",
            format!("a=b; CF_Authorization={}", good_jwt(PARENT)),
        )
        .body(Body::empty())
        .unwrap();
    assert_eq!(e.call(r).await.0, StatusCode::OK);
}

#[tokio::test]
async fn a_forged_or_invalid_jwt_is_refused() {
    let e = env().await;
    seed(&e);
    let ok =
        |email: &str, aud: &str, iss: &str, exp: i64, nbf: i64| claims(email, aud, iss, exp, nbf);
    let (t, past) = (now() + 300, now() - 10);
    let cases: Vec<(&str, Option<String>)> = vec![
        ("missing", None),
        ("garbage", Some("not.a.jwt".into())),
        (
            "signed by another key",
            Some(sign(
                OTHER_PEM,
                Algorithm::RS256,
                Some(KID),
                &ok(PARENT, AUD, ISS, t, past),
            )),
        ),
        (
            "expired",
            Some(sign(
                GOOD_PEM,
                Algorithm::RS256,
                Some(KID),
                &ok(PARENT, AUD, ISS, now() - 3600, past - 7200),
            )),
        ),
        (
            "not yet valid",
            Some(sign(
                GOOD_PEM,
                Algorithm::RS256,
                Some(KID),
                &ok(PARENT, AUD, ISS, t + 3600, now() + 1800),
            )),
        ),
        (
            "wrong audience",
            Some(sign(
                GOOD_PEM,
                Algorithm::RS256,
                Some(KID),
                &ok(PARENT, "other", ISS, t, past),
            )),
        ),
        (
            "wrong issuer",
            Some(sign(
                GOOD_PEM,
                Algorithm::RS256,
                Some(KID),
                &ok(PARENT, AUD, "https://evil.example", t, past),
            )),
        ),
        (
            "unknown kid",
            Some(sign(
                GOOD_PEM,
                Algorithm::RS256,
                Some("nope"),
                &ok(PARENT, AUD, ISS, t, past),
            )),
        ),
        (
            "no kid",
            Some(sign(
                GOOD_PEM,
                Algorithm::RS256,
                None,
                &ok(PARENT, AUD, ISS, t, past),
            )),
        ),
        (
            "HS256",
            Some(sign(
                "",
                Algorithm::HS256,
                Some(KID),
                &ok(PARENT, AUD, ISS, t, past),
            )),
        ),
        (
            "no audience or issuer",
            Some(sign(
                GOOD_PEM,
                Algorithm::RS256,
                Some(KID),
                &json!({"email": PARENT, "sub": "s", "exp": t}),
            )),
        ),
        (
            "empty email",
            Some(sign(
                GOOD_PEM,
                Algorithm::RS256,
                Some(KID),
                &ok("", AUD, ISS, t, past),
            )),
        ),
    ];
    for (name, jwt) in cases {
        let (s, _) = e.parent_post(PAIRING, jwt.as_deref()).await;
        assert_eq!(s, StatusCode::UNAUTHORIZED, "{name}");
    }
}

#[tokio::test]
async fn a_verified_stranger_is_forbidden() {
    let e = env().await;
    seed(&e);
    let (s, _) = e
        .parent_post(PAIRING, Some(&good_jwt("stranger@example.com")))
        .await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    let (s, _) = e
        .parent_post(PAIRING, Some(&good_jwt("MUM@Example.com")))
        .await;
    assert_eq!(s, StatusCode::OK, "email match ignores case");
}

#[tokio::test]
async fn parent_routes_are_closed_when_access_is_not_configured() {
    let e = env_with(false, "").await;
    seed(&e);
    let (s, _) = e.parent_post(PAIRING, Some(&good_jwt(PARENT))).await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn a_partial_access_configuration_is_refused_at_startup() {
    let r = Config::from_lookup(|k| (k == "CUBBY_ACCESS_AUD").then(|| "x".into()));
    assert!(r.is_err());
}

#[tokio::test]
async fn pairing_an_unknown_child_is_not_found() {
    let e = env().await;
    let (s, _) = e
        .parent_post(
            "/api/parent/children/ghost/pairing",
            Some(&good_jwt(PARENT)),
        )
        .await;
    assert_eq!(s, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn a_paired_device_is_revoked_and_gets_401_at_once() {
    let e = env().await;
    seed(&e);
    let jwt = good_jwt(PARENT);
    let (_, code) = e.parent_post(PAIRING, Some(&jwt)).await;
    let (s, paired) = e.pair(code["code"].as_str().unwrap(), "1.1.1.1").await;
    assert_eq!(s, StatusCode::OK, "{paired}");
    let token = paired["token"].as_str().unwrap();
    let (s, me) = e.me(Some(token)).await;
    assert_eq!((s, me["name"].as_str()), (StatusCode::OK, Some("Ada")));

    let device = paired["device_id"].as_str().unwrap();
    let (s, _) = e
        .parent_post(&format!("/api/parent/devices/{device}/revoke"), None)
        .await;
    assert_eq!(
        s,
        StatusCode::UNAUTHORIZED,
        "revoking needs the parent's JWT"
    );
    assert_eq!(e.me(Some(token)).await.0, StatusCode::OK);

    let (s, _) = e
        .parent_post(&format!("/api/parent/devices/{device}/revoke"), Some(&jwt))
        .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(e.me(Some(token)).await.0, StatusCode::UNAUTHORIZED);
    let (s, _) = e
        .parent_post("/api/parent/devices/ghost/revoke", Some(&jwt))
        .await;
    assert_eq!(s, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn child_calls_without_a_valid_bearer_token_get_401() {
    let e = env().await;
    assert_eq!(e.me(None).await.0, StatusCode::UNAUTHORIZED);
    assert_eq!(e.me(Some("bogus")).await.0, StatusCode::UNAUTHORIZED);
    let r = Request::get("/api/child/me")
        .header("authorization", "Basic abc")
        .body(Body::empty())
        .unwrap();
    assert_eq!(e.call(r).await.0, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn a_pairing_code_is_single_use_over_http() {
    let e = env().await;
    seed(&e);
    let (_, code) = e.parent_post(PAIRING, Some(&good_jwt(PARENT))).await;
    let code = code["code"].as_str().unwrap();
    assert_eq!(e.pair(code, "2.2.2.2").await.0, StatusCode::OK);
    assert_eq!(e.pair(code, "2.2.2.2").await.0, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn an_expired_pairing_code_is_refused() {
    let e = env().await;
    seed(&e);
    // Plant a code issued 11 minutes ago.
    let c = ident::new_pairing_code(&e.family(), "c1", now() - 660)
        .unwrap()
        .unwrap();
    assert_eq!(e.pair(&c.code, "3.3.3.3").await.0, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn pairing_is_rate_limited_to_five_attempts_a_minute_per_ip() {
    let e = env().await;
    seed(&e);
    for i in 0..5 {
        assert_eq!(
            e.pair("ZZZZZZ", "4.4.4.4").await.0,
            StatusCode::BAD_REQUEST,
            "{i}"
        );
    }
    let (s, _) = e.pair("ZZZZZZ", "4.4.4.4").await;
    assert_eq!(s, StatusCode::TOO_MANY_REQUESTS);
    let (_, code) = e.parent_post(PAIRING, Some(&good_jwt(PARENT))).await;
    let (s, _) = e.pair(code["code"].as_str().unwrap(), "4.4.4.4").await;
    assert_eq!(
        s,
        StatusCode::TOO_MANY_REQUESTS,
        "a good code does not bypass the limit"
    );
    let (s, _) = e.pair(code["code"].as_str().unwrap(), "5.5.5.5").await;
    assert_eq!(s, StatusCode::OK, "another address is unaffected");
}

#[tokio::test]
async fn a_rejected_device_token_pushes_a_device_auth_sample() {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = l.local_addr().unwrap().to_string();
    let sink = std::thread::spawn(move || {
        let mut all = String::new();
        for _ in 0..2 {
            let (mut s, _) = l.accept().unwrap();
            let mut got = Vec::new();
            let mut buf = [0u8; 4096];
            while !got.windows(4).any(|w| w == b"\r\n\r\n") || got.len() < 200 {
                let n = s.read(&mut buf).unwrap();
                if n == 0 {
                    break;
                }
                got.extend_from_slice(&buf[..n]);
            }
            s.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                .unwrap();
            all.push_str(&String::from_utf8_lossy(&got));
        }
        all
    });
    let e = env_with(true, &addr).await;
    assert_eq!(e.me(Some("bogus")).await.0, StatusCode::UNAUTHORIZED);
    let got = tokio::task::spawn_blocking(move || sink.join().unwrap())
        .await
        .unwrap();
    assert!(got.contains("cubby_device_auth"), "{got}");
    assert!(got.contains("rejected"), "{got}");
}

#[tokio::test]
async fn a_token_naming_an_unregistered_family_is_401_and_makes_no_file() {
    let e = env().await;
    let ghost = "01ARZ3NDEKTSV4RRFFQ69G5FAV";
    assert_eq!(
        e.me(Some(&format!("{ghost}.abc"))).await.0,
        StatusCode::UNAUTHORIZED
    );
    assert!(
        !e.dir
            .join("data")
            .join(format!("family-{ghost}.sqlite"))
            .exists()
    );
}

async fn child_post(e: &Env, path: &str, token: &str, body: Value) -> (StatusCode, Value) {
    let r = Request::post(path)
        .header("content-type", "application/json")
        .header("authorization", format!("Bearer {token}"))
        .body(Body::from(body.to_string()))
        .unwrap();
    e.call(r).await
}

#[tokio::test]
async fn a_paired_device_plays_beats_and_ends_over_http() {
    let e = env().await;
    seed(&e);
    let conn = e.family();
    conn.execute_batch(
        "INSERT INTO videos(youtube_id,title,duration_s) VALUES ('v1','one',600);
         INSERT INTO allow_policies(id,kind,youtube_id,added_at) VALUES ('p','video','v1',0);
         INSERT INTO allowed_videos(youtube_id,child_id,policy_id,state,changed_at) VALUES ('v1',NULL,'p','approved',0);
         INSERT INTO budgets(child_id,video_equivalents,window_s,cutoff_mid_play) VALUES ('c1',1,86400,0);",
    )
    .unwrap();
    let code = ident::new_pairing_code(&conn, "c1", now())
        .unwrap()
        .unwrap();
    let token = ident::redeem_pairing_code(&conn, &e.family_id, &code.code, None, now())
        .unwrap()
        .token;

    let r = Request::post("/api/child/play")
        .header("content-type", "application/json")
        .body(Body::from(json!({"youtube_id": "v1"}).to_string()))
        .unwrap();
    assert_eq!(e.call(r).await.0, StatusCode::UNAUTHORIZED);

    let (s, v) = child_post(&e, "/api/child/play", &token, json!({"youtube_id": "v1"})).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let id = v["grant_id"].as_str().unwrap().to_string();
    assert_eq!(v["remaining"]["videos"], 1.0);

    let beat = format!("/api/child/grant/{id}/beat");
    let (s, v) = child_post(
        &e,
        &beat,
        &token,
        json!({"position_s": 0, "state": "paused"}),
    )
    .await;
    assert_eq!((s, &v["stop"]), (StatusCode::OK, &json!(false)));

    let end = format!("/api/child/grant/{id}/end");
    let (s, _) = child_post(&e, &end, &token, json!({"reason": "left"})).await;
    assert_eq!(s, StatusCode::OK);
    let (s, _) = child_post(&e, &beat, &token, json!({"position_s": 1})).await;
    assert_eq!(s, StatusCode::CONFLICT);

    let (s, v) = child_post(&e, "/api/child/play", &token, json!({"youtube_id": "nope"})).await;
    assert_eq!(
        (s, v["refused"].as_str()),
        (StatusCode::OK, Some("not-allowed"))
    );
}
