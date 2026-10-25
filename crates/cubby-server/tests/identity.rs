//! Identity over HTTP: OIDC session guard, device pairing, tokens and revocation.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::PathBuf;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use cubby_db::Registry;
use cubby_db::identity as ident;
use cubby_server::{Config, app};
use http_body_util::BodyExt;
use rusqlite::Connection;
use serde_json::{Value, json};
use tower::ServiceExt;

mod support;
use support::Provider;

const PARENT: &str = "mum@example.com";

struct Env {
    router: axum::Router,
    dir: PathBuf,
    family_id: String,
    provider: Provider,
    parent_cookie: std::sync::Mutex<Option<String>>,
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

async fn env_with(access: bool, otlp: &str) -> Env {
    env_build(access, otlp, true).await
}

async fn env_build(access: bool, otlp: &str, provision: bool) -> Env {
    let dir = temp_dir();
    let provider = Provider::start().await;
    let cfg = Config::from_lookup(|k| match k {
        "CUBBY_WEB_DIR" => Some(dir.display().to_string()),
        "CUBBY_DATA_DIR" => Some(dir.join("data").display().to_string()),
        "CUBBY_OTLP_ENDPOINT" => Some(otlp.into()),
        k if access => provider.env(k),
        _ => None,
    })
    .unwrap();
    let data = dir.join("data");
    let mut registry = Registry::open(&data).unwrap();
    let family_id = if provision {
        registry.create_family(PARENT).unwrap().id
    } else {
        String::new()
    };
    drop(registry);
    let router = app(&cfg).unwrap();
    Env {
        router,
        dir,
        family_id,
        provider,
        parent_cookie: Default::default(),
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

    /// Signs `email` in through the fake provider and returns the session cookie.
    async fn cookie(&self, email: &str) -> String {
        support::session_cookie(&self.router, &self.provider, email).await
    }

    async fn parent_session(&self) -> String {
        let cached = self.parent_cookie.lock().unwrap().clone();
        match cached {
            Some(c) => c,
            None => {
                let c = self.cookie(PARENT).await;
                *self.parent_cookie.lock().unwrap() = Some(c.clone());
                c
            }
        }
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
            r = r.header("cookie", j);
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

const PAIRING: &str = "/api/parent/children/c1/pairing";

fn seed(e: &Env) {
    ident::add_child(&e.family(), "c1", "Ada", "Europe/London").unwrap();
}

#[tokio::test]
async fn a_signed_in_parent_is_accepted() {
    let e = env().await;
    seed(&e);
    let (s, body) = e
        .parent_post(PAIRING, Some(&e.parent_session().await))
        .await;
    assert_eq!(s, StatusCode::OK, "{body}");
    assert_eq!(body["code"].as_str().unwrap().len(), 6);
    assert!(body["expires_at"].as_i64().unwrap() > now());
}

#[tokio::test]
async fn a_verified_stranger_is_forbidden() {
    let e = env().await;
    seed(&e);
    let o = support::sign_in(&e.router, &e.provider, "stranger@example.com").await;
    assert_eq!(o.status, StatusCode::FORBIDDEN);
    let (s, _) = e
        .parent_post(PAIRING, Some(&e.cookie("MUM@Example.com").await))
        .await;
    assert_eq!(s, StatusCode::OK, "email match ignores case");
}

#[tokio::test]
async fn parent_routes_are_closed_when_oidc_is_not_configured() {
    let e = env_with(false, "").await;
    seed(&e);
    let (s, _) = e.parent_post(PAIRING, Some("cubby_session=anything")).await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn a_partial_oidc_configuration_is_refused_at_startup() {
    let r = Config::from_lookup(|k| (k == "CUBBY_OIDC_CLIENT_ID").then(|| "x".into()));
    assert!(r.is_err());
}

#[tokio::test]
async fn pairing_an_unknown_child_is_not_found() {
    let e = env().await;
    let (s, _) = e
        .parent_post(
            "/api/parent/children/ghost/pairing",
            Some(&e.parent_session().await),
        )
        .await;
    assert_eq!(s, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn a_paired_device_is_revoked_and_gets_401_at_once() {
    let e = env().await;
    seed(&e);
    let jwt = e.parent_session().await;
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
    let (_, code) = e
        .parent_post(PAIRING, Some(&e.parent_session().await))
        .await;
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
    let (_, code) = e
        .parent_post(PAIRING, Some(&e.parent_session().await))
        .await;
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
         INSERT INTO budgets(child_id,video_equivalents,cutoff_mid_play) VALUES ('c1',1,0);",
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

#[tokio::test]
async fn play_beat_and_http_telemetry_reach_the_otlp_sink() {
    let f = cubby_otel::fake::FakeSink::start();
    let e = env_with(true, &f.addr).await;
    seed(&e);
    let conn = e.family();
    conn.execute_batch(
        "INSERT INTO videos(youtube_id,title,duration_s) VALUES ('v1','one',600);
         INSERT INTO allow_policies(id,kind,youtube_id,added_at) VALUES ('p','video','v1',0);
         INSERT INTO allowed_videos(youtube_id,child_id,policy_id,state,changed_at) VALUES ('v1',NULL,'p','approved',0);",
    )
    .unwrap();
    let code = ident::new_pairing_code(&conn, "c1", now())
        .unwrap()
        .unwrap();
    let token = ident::redeem_pairing_code(&conn, &e.family_id, &code.code, None, now())
        .unwrap()
        .token;

    let (_, v) = child_post(&e, "/api/child/play", &token, json!({"youtube_id": "v1"})).await;
    let id = v["grant_id"].as_str().unwrap().to_string();
    let plays = f.wait_for("cubby_plays");
    assert_eq!(plays[0].label("outcome"), Some("granted"));
    assert_eq!(plays[0].label("child"), Some("c1"));

    let (_, v) = child_post(&e, "/api/child/play", &token, json!({"youtube_id": "nope"})).await;
    assert_eq!(v["refused"], "not-allowed");
    let refused = loop {
        let r: Vec<_> = f
            .wait_for("cubby_plays")
            .into_iter()
            .filter(|s| s.label("outcome") == Some("refused"))
            .collect();
        if !r.is_empty() {
            break r;
        }
    };
    assert_eq!(refused[0].label("reason"), Some("not-allowed"));

    // Move the grant's last beat back so the next beat measures 60 watched seconds.
    conn.execute(
        "UPDATE play_grants SET last_beat_at = last_beat_at - 60",
        [],
    )
    .unwrap();
    let beat = format!("/api/child/grant/{id}/beat");
    child_post(&e, &beat, &token, json!({"position_s": 60})).await;
    let watch = f.wait_for("cubby_watch_seconds");
    assert!(watch[0].value > 0.0 && watch[0].label("child") == Some("c1"));
    let charge = f.wait_for("cubby_charge");
    assert!(charge[0].value > 0.0 && charge[0].label("child") == Some("c1"));

    assert!(!f.wait_for("cubby_http_request_seconds").is_empty());
}

#[tokio::test]
async fn the_family_cli_creates_a_family_an_access_request_is_then_accepted_for() {
    let e = env_build(true, "", false).await;
    let data = e.dir.join("data");
    let refused = support::sign_in(&e.router, &e.provider, PARENT).await;
    assert_eq!(refused.status, StatusCode::FORBIDDEN, "no family yet");
    assert!(refused.cookie.is_none());

    let args = |a: &[&str]| a.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    let mut out = Vec::new();
    cubby_server::family_cli::run(&data, &args(&["create", "Mum@Example.com"]), &mut out).unwrap();
    let line = String::from_utf8(out).unwrap();
    assert!(line.trim_end().ends_with("\tmum@example.com"), "{line}");

    let fam = Registry::open(&data)
        .unwrap()
        .lookup(PARENT)
        .unwrap()
        .unwrap();
    ident::add_child(
        &Registry::open(&data).unwrap().open_family(&fam.id).unwrap(),
        "c1",
        "Ada",
        "Europe/London",
    )
    .unwrap();
    let jwt = e.parent_session().await;
    let (s, body) = e.parent_post(PAIRING, Some(&jwt)).await;
    assert_eq!(s, StatusCode::OK, "{body}");

    let mut out = Vec::new();
    cubby_server::family_cli::run(&data, &args(&["list"]), &mut out).unwrap();
    assert!(String::from_utf8(out).unwrap().contains(&fam.id));

    let mut out = Vec::new();
    cubby_server::family_cli::run(
        &data,
        &args(&["add-parent", PARENT, "Kim@Gmail.com"]),
        &mut out,
    )
    .unwrap();
    assert_eq!(
        String::from_utf8(out).unwrap(),
        format!("{}\tkim@gmail.com\n", fam.id)
    );
    let (s, _) = e
        .parent_post(PAIRING, Some(&e.cookie("kim@gmail.com").await))
        .await;
    assert_eq!(s, StatusCode::OK, "a co-parent reaches the same family");
    let mut out = Vec::new();
    cubby_server::family_cli::run(&data, &args(&["list"]), &mut out).unwrap();
    assert_eq!(
        String::from_utf8(out).unwrap(),
        format!("{}\tkim@gmail.com,{PARENT}\n", fam.id)
    );
    assert!(
        cubby_server::family_cli::run(
            &data,
            &args(&["add-parent", &fam.id, PARENT]),
            &mut Vec::new()
        )
        .is_err(),
        "an email already a parent is refused"
    );
    cubby_server::family_cli::run(
        &data,
        &args(&["add-parent", &fam.id, "by-id@example.com"]),
        &mut Vec::new(),
    )
    .unwrap();
    assert!(
        cubby_server::family_cli::run(
            &data,
            &args(&["add-parent", "nobody@example.com", "z@z.z"]),
            &mut Vec::new()
        )
        .is_err()
    );
    assert!(
        cubby_server::family_cli::run(&data, &args(&["create", PARENT]), &mut Vec::new()).is_err(),
        "a second create for the same email is refused"
    );
    assert!(cubby_server::family_cli::run(&data, &args(&["bogus"]), &mut Vec::new()).is_err());
}

#[tokio::test]
async fn bootstrap_creates_a_family_only_into_an_empty_registry() {
    let e = env_build(true, "", false).await;
    let data = e.dir.join("data");
    assert!(cubby_server::family_cli::bootstrap(&data, PARENT).unwrap());
    assert!(!cubby_server::family_cli::bootstrap(&data, PARENT).unwrap());
    assert!(!cubby_server::family_cli::bootstrap(&data, "other@example.com").unwrap());
    assert_eq!(Registry::open(&data).unwrap().list().unwrap().len(), 1);
}

#[tokio::test]
async fn a_paired_device_browses_home_search_and_next_over_http() {
    let e = env().await;
    seed(&e);
    let conn = e.family();
    conn.execute_batch(
        "INSERT INTO videos(youtube_id,title,duration_s) VALUES ('v1','Blue train',600),('v2','Red train',300),('v3','Hidden train',300);
         INSERT INTO allow_policies(id,kind,youtube_id,added_at) VALUES ('p1','video','v1',0),('p2','video','v2',0);
         INSERT INTO allowed_videos(youtube_id,child_id,policy_id,state,changed_at) VALUES ('v1',NULL,'p1','approved',0),('v2',NULL,'p2','approved',0);
         INSERT INTO budgets(child_id,video_equivalents,cutoff_mid_play) VALUES ('c1',2,0);",
    )
    .unwrap();
    let code = ident::new_pairing_code(&conn, "c1", now())
        .unwrap()
        .unwrap();
    let token = ident::redeem_pairing_code(&conn, &e.family_id, &code.code, None, now())
        .unwrap()
        .token;
    let get = |path: &'static str, token: Option<String>| {
        let e = &e;
        async move {
            let mut r = Request::get(path);
            if let Some(t) = token {
                r = r.header("authorization", format!("Bearer {t}"));
            }
            e.call(r.body(Body::empty()).unwrap()).await
        }
    };
    for p in [
        "/api/child/home",
        "/api/child/search?q=train",
        "/api/child/next?after=v1",
    ] {
        assert_eq!(get(p, None).await.0, StatusCode::UNAUTHORIZED, "{p}");
    }
    let (s, v) = get("/api/child/home", Some(token.clone())).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["remaining"]["videos"], 2.0);
    let all = v["sections"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["key"] == "all")
        .unwrap()["tiles"]
        .as_array()
        .unwrap()
        .len();
    assert_eq!(all, 2);
    let (s, v) = get("/api/child/search?q=train", Some(token.clone())).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["tiles"].as_array().unwrap().len(), 2);
    let (s, v) = get("/api/child/next?after=v1", Some(token)).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["tiles"][0]["youtube_id"], "v2");
}
