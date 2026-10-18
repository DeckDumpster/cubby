//! Parent sign-in over HTTP against a fake OIDC provider: the authorization-code round trip,
//! the refusals (group, family, forged or mismatched ID token), session expiry and logout.

use std::path::PathBuf;

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use cubby_db::Registry;
use cubby_server::Config;
use http_body_util::BodyExt;
use tower::ServiceExt;

mod support;
use support::{Provider, User, now, sign_in, sign_in_as};

const PARENT: &str = "mum@example.com";

struct Env {
    router: axum::Router,
    provider: Provider,
    config: Config,
}

async fn env_ttl(ttl: Option<&str>) -> Env {
    let dir: PathBuf = std::env::temp_dir().join(format!(
        "cubby-oidc-{}-{:?}-{}",
        std::process::id(),
        std::thread::current().id(),
        now()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("index.html"), "<html>shell</html>").unwrap();
    let provider = Provider::start().await;
    let config = Config::from_lookup(|k| match k {
        "CUBBY_WEB_DIR" => Some(dir.display().to_string()),
        "CUBBY_DATA_DIR" => Some(dir.join("data").display().to_string()),
        "CUBBY_SESSION_TTL_SECS" => ttl.map(String::from),
        k => provider.env(k),
    })
    .unwrap();
    Registry::open(&dir.join("data"))
        .unwrap()
        .create_family(PARENT)
        .unwrap();
    let router = cubby_server::app(&config).unwrap();
    Env {
        router,
        provider,
        config,
    }
}

async fn env() -> Env {
    env_ttl(None).await
}

impl Env {
    async fn get(&self, path: &str, cookie: Option<&str>) -> (StatusCode, axum::http::HeaderMap) {
        let mut r = Request::get(path);
        if let Some(c) = cookie {
            r = r.header(header::COOKIE, c);
        }
        let res = self
            .router
            .clone()
            .oneshot(r.body(Body::empty()).unwrap())
            .await
            .unwrap();
        let (status, headers) = (res.status(), res.headers().clone());
        let _ = res.into_body().collect().await;
        (status, headers)
    }

    async fn api_status(&self, cookie: Option<&str>) -> StatusCode {
        self.get("/api/parent/children", cookie).await.0
    }
}

#[tokio::test]
async fn a_sign_in_round_trip_opens_a_session_the_parent_api_accepts() {
    let e = env().await;
    assert_eq!(e.api_status(None).await, StatusCode::UNAUTHORIZED);
    let o = sign_in(&e.router, &e.provider, "Mum@Example.com").await;
    assert_eq!(o.status, StatusCode::SEE_OTHER, "{}", o.body);
    assert_eq!(o.location.as_deref(), Some("/parent/allow"));
    let cookie = o.cookie.expect("a session cookie");
    assert_eq!(e.api_status(Some(&cookie)).await, StatusCode::OK);
}

#[tokio::test]
async fn the_session_cookie_is_httponly_lax_and_secure_on_an_https_origin() {
    let e = env().await;
    let o = sign_in(&e.router, &e.provider, PARENT).await;
    let set = o.set_cookie.unwrap();
    for attr in [
        "HttpOnly",
        "SameSite=Lax",
        "Secure",
        "Path=/",
        "Max-Age=28800",
    ] {
        assert!(set.contains(attr), "{attr} missing from {set}");
    }
}

#[tokio::test]
async fn login_returns_to_a_same_site_next_and_ignores_an_offsite_one() {
    let e = env().await;
    let o = sign_in_as(
        &e.router,
        &e.provider,
        User::parent(PARENT),
        Some("/parent/history?child=c1"),
    )
    .await;
    assert_eq!(o.location.as_deref(), Some("/parent/history?child=c1"));
    for bad in ["https://evil.example/", "//evil.example"] {
        let o = sign_in_as(&e.router, &e.provider, User::parent(PARENT), Some(bad)).await;
        assert_eq!(o.location.as_deref(), Some("/parent/allow"), "{bad}");
    }
}

#[tokio::test]
async fn a_user_outside_the_required_group_is_forbidden_and_gets_no_session() {
    let e = env().await;
    let mut u = User::parent(PARENT);
    u.groups = vec!["family-friends".into()];
    let o = sign_in_as(&e.router, &e.provider, u, None).await;
    assert_eq!(o.status, StatusCode::FORBIDDEN);
    assert!(o.cookie.is_none());
    let mut u = User::parent(PARENT);
    u.groups = vec![];
    assert_eq!(
        sign_in_as(&e.router, &e.provider, u, None).await.status,
        StatusCode::FORBIDDEN
    );
}

#[tokio::test]
async fn a_user_who_is_in_the_group_but_no_familys_parent_is_forbidden() {
    let e = env().await;
    let o = sign_in(&e.router, &e.provider, "stranger@example.com").await;
    assert_eq!(o.status, StatusCode::FORBIDDEN);
    assert!(o.body["error"].as_str().unwrap().contains("not a parent"));
    assert!(o.cookie.is_none());
}

#[tokio::test]
async fn a_forged_or_mismatched_id_token_opens_no_session() {
    let e = env().await;
    let mut cases = Vec::new();
    let mut u = User::parent(PARENT);
    u.signed_by_other_key = true;
    cases.push(("signed by another key", u));
    let mut u = User::parent(PARENT);
    u.aud = "someone-elses-client".into();
    cases.push(("wrong audience", u));
    let mut u = User::parent(PARENT);
    u.nonce = Some("replayed-nonce".into());
    cases.push(("wrong nonce", u));
    let mut u = User::parent(PARENT);
    u.exp_in = -3600;
    cases.push(("expired token", u));
    for (name, u) in cases {
        let o = sign_in_as(&e.router, &e.provider, u, None).await;
        assert_eq!(o.status, StatusCode::UNAUTHORIZED, "{name}: {}", o.body);
        assert!(o.cookie.is_none(), "{name}");
    }
}

#[tokio::test]
async fn an_unknown_or_replayed_state_is_refused() {
    let e = env().await;
    let o = support::callback(&e.router, "code-x", "never-issued").await;
    assert_eq!(o.status, StatusCode::BAD_REQUEST);
    assert!(o.cookie.is_none());
}

#[tokio::test]
async fn an_expired_session_is_refused() {
    let e = env_ttl(Some("1")).await;
    let cookie = sign_in(&e.router, &e.provider, PARENT)
        .await
        .cookie
        .unwrap();
    assert_eq!(e.api_status(Some(&cookie)).await, StatusCode::OK);
    tokio::time::sleep(std::time::Duration::from_millis(1300)).await;
    assert_eq!(e.api_status(Some(&cookie)).await, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn logout_ends_the_session_and_clears_the_cookie() {
    let e = env().await;
    let cookie = sign_in(&e.router, &e.provider, PARENT)
        .await
        .cookie
        .unwrap();
    let (s, headers) = e.get("/auth/logout", Some(&cookie)).await;
    assert_eq!(s, StatusCode::SEE_OTHER);
    let to = reqwest::Url::parse(headers[header::LOCATION].to_str().unwrap()).unwrap();
    assert_eq!(to.path(), "/logout", "ends the provider's session: {to}");
    let q: std::collections::HashMap<_, _> = to.query_pairs().into_owned().collect();
    assert!(q["id_token_hint"].split('.').count() == 3, "{to}");
    assert_eq!(q["post_logout_redirect_uri"], format!("{}/signed-out", support::BASE_URL));
    let set = headers[header::SET_COOKIE].to_str().unwrap();
    assert!(
        set.starts_with("cubby_session=;") && set.contains("Max-Age=0"),
        "{set}"
    );
    assert_eq!(e.api_status(Some(&cookie)).await, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn a_garbage_or_foreign_cookie_is_refused() {
    let e = env().await;
    for c in [
        "cubby_session=deadbeef",
        "CF_Authorization=abc",
        "cubby_session=",
    ] {
        assert_eq!(e.api_status(Some(c)).await, StatusCode::UNAUTHORIZED, "{c}");
    }
}

#[tokio::test]
async fn a_parent_page_without_a_session_redirects_to_sign_in() {
    let e = env().await;
    let (s, h) = e.get("/parent/history?child=c1", None).await;
    assert_eq!(s, StatusCode::SEE_OTHER);
    assert_eq!(
        h[header::LOCATION],
        "/auth/login?next=%2Fparent%2Fhistory%3Fchild%3Dc1"
    );
    assert_eq!(e.get("/parent", None).await.0, StatusCode::SEE_OTHER);
    let cookie = sign_in(&e.router, &e.provider, PARENT)
        .await
        .cookie
        .unwrap();
    assert_eq!(
        e.get("/parent/history", Some(&cookie)).await.0,
        StatusCode::OK
    );
    assert_eq!(
        e.get("/watch", None).await.0,
        StatusCode::OK,
        "the child app stays open"
    );
}

#[tokio::test]
async fn sign_in_is_unavailable_without_oidc_configuration() {
    let cfg = Config::from_lookup(|_| None).unwrap();
    assert!(cfg.oidc.is_none());
    let dir = std::env::temp_dir().join(format!("cubby-oidc-none-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let cfg = Config {
        data_dir: dir.join("data"),
        ..cfg
    };
    let router = cubby_server::app(&cfg).unwrap();
    let res = router
        .oneshot(Request::get("/auth/login").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::SERVICE_UNAVAILABLE);
}

#[tokio::test]
async fn startup_discovery_checks_the_issuer() {
    let e = env().await;
    cubby_server::check_oidc(&e.config).await.unwrap();
    let mut wrong = e.config.clone();
    wrong.oidc.as_mut().unwrap().issuer =
        format!("{}/", e.provider.issuer.replace("127.0.0.1", "localhost"));
    assert!(cubby_server::check_oidc(&wrong).await.is_err());
}

#[tokio::test]
async fn a_partial_oidc_configuration_is_refused() {
    let r =
        Config::from_lookup(|k| (k == "CUBBY_OIDC_ISSUER").then(|| "https://id.example".into()));
    let msg = r.unwrap_err();
    assert!(msg.contains("CUBBY_OIDC_CLIENT_SECRET"), "{msg}");
}
