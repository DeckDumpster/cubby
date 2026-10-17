#![allow(dead_code)]
//! The parent API over HTTP, end to end: what a parent allows or blocks decides what a paired
//! child device may play.

use std::path::PathBuf;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use cubby_db::Registry;
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

impl Env {
    async fn parent(&self, method: &str, path: &str, body: Option<Value>) -> (StatusCode, Value) {
        let mut r = Request::builder()
            .method(method)
            .uri(path)
            .header("cf-access-jwt-assertion", good_jwt(PARENT));
        let b = match body {
            Some(b) => {
                r = r.header("content-type", "application/json");
                Body::from(b.to_string())
            }
            None => Body::empty(),
        };
        self.call(r.body(b).unwrap()).await
    }

    async fn child_play(&self, token: &str, vid: &str) -> Value {
        let r = Request::post("/api/child/play")
            .header("authorization", format!("Bearer {token}"))
            .header("content-type", "application/json")
            .body(Body::from(json!({"youtube_id": vid}).to_string()))
            .unwrap();
        self.call(r).await.1
    }

    /// Makes a child and pairs a device for it; returns (child id, device token).
    async fn child_with_device(&self) -> (String, String) {
        let (s, c) = self
            .parent(
                "POST",
                "/api/parent/children",
                Some(json!({"name": "Ada", "tz": "UTC"})),
            )
            .await;
        assert_eq!(s, StatusCode::OK, "{c}");
        let id = c["id"].as_str().unwrap().to_string();
        let (_, code) = self
            .parent("POST", &format!("/api/parent/children/{id}/pairing"), None)
            .await;
        let (s, paired) = self.pair(code["code"].as_str().unwrap(), "9.9.9.9").await;
        assert_eq!(s, StatusCode::OK, "{paired}");
        (id, paired["token"].as_str().unwrap().to_string())
    }

    fn videos(&self) {
        self.family()
            .execute_batch(
                "INSERT INTO videos(youtube_id,title,channel_id,channel_title,duration_s,fetched_at) VALUES
                 ('v1','one','ch1','Chan',600,1),('v2','two','ch1','Chan',600,2),
                 ('w1','other','ch2','Other',600,3);",
            )
            .unwrap();
    }
}

fn granted(v: &Value) -> bool {
    v.get("grant_id").is_some()
}

#[tokio::test]
async fn allow_a_channel_block_one_video_and_the_channel_stays_allowed() {
    let e = env().await;
    e.videos();
    let (child, tok) = e.child_with_device().await;
    assert!(
        !granted(&e.child_play(&tok, "v1").await),
        "nothing is allowed yet"
    );

    let (s, p) = e
        .parent(
            "POST",
            "/api/parent/policies",
            Some(json!({"url": "https://www.youtube.com/channel/ch1"})),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{p}");
    assert!(granted(&e.child_play(&tok, "v1").await));
    assert!(granted(&e.child_play(&tok, "v2").await));
    assert!(!granted(&e.child_play(&tok, "w1").await));

    let (s, _) = e
        .parent("POST", "/api/parent/videos/v1/block", Some(json!({})))
        .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(e.child_play(&tok, "v1").await["refused"], "not-allowed");
    assert!(
        granted(&e.child_play(&tok, "v2").await),
        "the channel stays allowed"
    );
    let (_, pols) = e.parent("GET", "/api/parent/policies", None).await;
    assert_eq!(pols.as_array().unwrap().len(), 1);

    let (s, _) = e
        .parent("POST", "/api/parent/videos/v1/unblock", Some(json!({})))
        .await;
    assert_eq!(s, StatusCode::OK);
    assert!(granted(&e.child_play(&tok, "v1").await));
    let _ = child;
}

#[tokio::test]
async fn approving_a_more_from_channel_request_adds_a_policy_and_opens_the_channel() {
    let e = env().await;
    e.videos();
    let (child, tok) = e.child_with_device().await;
    e.parent(
        "POST",
        "/api/parent/policies",
        Some(json!({"url": "https://youtu.be/v1", "child_id": child})),
    )
    .await;
    assert!(!granted(&e.child_play(&tok, "v2").await));
    e.family()
        .execute(
            "INSERT INTO requests(id,child_id,kind,youtube_id,created_at) VALUES ('r1',?1,'more-from-channel','v1',1)",
            [&child],
        )
        .unwrap();

    let (_, inbox) = e.parent("GET", "/api/parent/requests", None).await;
    assert_eq!(inbox.as_array().unwrap().len(), 1);
    assert_eq!(inbox[0]["channel_title"], "Chan");

    let (s, b) = e
        .parent(
            "POST",
            "/api/parent/requests/r1/resolve",
            Some(json!({"action": "allow-channel"})),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert!(granted(&e.child_play(&tok, "v2").await));
    let (_, pols) = e.parent("GET", "/api/parent/policies", None).await;
    assert_eq!(pols.as_array().unwrap().len(), 2);
    let (_, inbox) = e.parent("GET", "/api/parent/requests", None).await;
    assert!(inbox.as_array().unwrap().is_empty());
    let (s, _) = e
        .parent(
            "POST",
            "/api/parent/requests/r1/resolve",
            Some(json!({"action": "dismiss"})),
        )
        .await;
    assert_eq!(s, StatusCode::NOT_FOUND, "a resolved request is gone");
}

#[tokio::test]
async fn the_review_list_shows_what_policies_added_and_approving_clears_it() {
    let e = env().await;
    e.videos();
    e.parent(
        "POST",
        "/api/parent/policies",
        Some(json!({"url": "youtube.com/channel/ch1"})),
    )
    .await;
    let (_, all) = e.parent("GET", "/api/parent/review", None).await;
    assert_eq!(all.as_array().unwrap().len(), 2);
    assert_eq!(all[0]["policy_youtube_id"], "ch1");
    let (_, none) = e
        .parent("GET", "/api/parent/review?channel=ch2", None)
        .await;
    assert!(none.as_array().unwrap().is_empty());
    e.parent("POST", "/api/parent/videos/v1/approve", Some(json!({})))
        .await;
    let (_, left) = e.parent("GET", "/api/parent/review", None).await;
    assert_eq!(left.as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn a_url_that_is_not_youtube_is_refused_and_a_preview_names_the_policy() {
    let e = env().await;
    let (s, _) = e
        .parent(
            "POST",
            "/api/parent/policies",
            Some(json!({"url": "https://example.com/x"})),
        )
        .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    let (s, p) = e
        .parent(
            "POST",
            "/api/parent/policies/preview",
            Some(json!({"url": "https://www.youtube.com/@Bluey"})),
        )
        .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(
        (p["kind"].as_str(), p["youtube_id"].as_str()),
        (Some("channel"), Some("@Bluey"))
    );
}

#[tokio::test]
async fn removing_a_policy_stops_play() {
    let e = env().await;
    e.videos();
    let (_, tok) = e.child_with_device().await;
    let (_, p) = e
        .parent(
            "POST",
            "/api/parent/policies",
            Some(json!({"url": "youtube.com/channel/ch1"})),
        )
        .await;
    let id = p["id"].as_str().unwrap();
    assert!(granted(&e.child_play(&tok, "v1").await));
    let (s, _) = e
        .parent("DELETE", &format!("/api/parent/policies/{id}"), None)
        .await;
    assert_eq!(s, StatusCode::OK);
    assert!(!granted(&e.child_play(&tok, "v1").await));
    let (s, _) = e
        .parent("DELETE", &format!("/api/parent/policies/{id}"), None)
        .await;
    assert_eq!(s, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn children_budgets_devices_and_history_are_reachable() {
    let e = env().await;
    e.videos();
    let (child, tok) = e.child_with_device().await;
    let (s, b) = e
        .parent(
            "PUT",
            &format!("/api/parent/children/{child}/budget"),
            Some(
                json!({"video_equivalents": 2.0, "minutes_cap": 45, "window_s": 86400,
                "cutoff_mid_play": false, "daily_windows": [["00:00", "23:59"]]}),
            ),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{b}");
    let (s, _) = e
        .parent(
            "PUT",
            &format!("/api/parent/children/{child}/budget"),
            Some(json!({"daily_windows": [["25:00", "19:00"]]})),
        )
        .await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "bad windows are refused");
    let (s, _) = e
        .parent(
            "PUT",
            &format!("/api/parent/children/{child}"),
            Some(json!({"name": "Ada", "avatar": "x", "tz": "Not/AZone"})),
        )
        .await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "bad tz is refused");

    let (_, kids) = e.parent("GET", "/api/parent/children", None).await;
    assert_eq!(kids[0]["budget"]["minutes_cap"], 45);
    assert_eq!(kids[0]["budget"]["daily_windows"][0][0], "00:00");
    assert_eq!(kids[0]["devices"].as_array().unwrap().len(), 1);

    e.parent(
        "POST",
        "/api/parent/policies",
        Some(json!({"url": "youtube.com/channel/ch1"})),
    )
    .await;
    assert!(granted(&e.child_play(&tok, "v1").await));
    let (_, h) = e
        .parent(
            "GET",
            &format!("/api/parent/children/{child}/history"),
            None,
        )
        .await;
    assert_eq!(h[0]["title"], "one");

    let dev = kids[0]["devices"][0]["id"].as_str().unwrap();
    e.parent("POST", &format!("/api/parent/devices/{dev}/revoke"), None)
        .await;
    let (_, kids) = e.parent("GET", "/api/parent/children", None).await;
    assert!(kids[0]["devices"][0]["revoked_at"].is_i64());
}

#[tokio::test]
async fn the_parent_api_needs_a_parent() {
    let e = env().await;
    let r = Request::get("/api/parent/policies")
        .body(Body::empty())
        .unwrap();
    assert_eq!(e.call(r).await.0, StatusCode::UNAUTHORIZED);
}
