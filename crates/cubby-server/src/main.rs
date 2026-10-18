use cubby_server::{Config, app};

#[tokio::main]
async fn main() {
    let config = match Config::from_env() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("cubby-server: {e}");
            std::process::exit(2);
        }
    };
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("family") {
        if let Err(e) =
            cubby_server::family_cli::run(&config.data_dir, &args[1..], &mut std::io::stdout())
        {
            eprintln!("cubby-server: {e}");
            std::process::exit(1);
        }
        return;
    }
    if let Some(email) = std::env::var("CUBBY_BOOTSTRAP_PARENT")
        .ok()
        .filter(|v| !v.trim().is_empty())
    {
        match cubby_server::family_cli::bootstrap(&config.data_dir, &email) {
            Ok(true) => eprintln!("cubby-server: bootstrapped a family for {}", email.trim()),
            Ok(false) => {}
            Err(e) => {
                eprintln!("cubby-server: {e}");
                std::process::exit(1);
            }
        }
    }
    if let Err(e) = cubby_server::check_oidc(&config).await {
        eprintln!("cubby-server: {e}");
        std::process::exit(1);
    }
    let router = match app(&config) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("cubby-server: {e}");
            std::process::exit(2);
        }
    };
    let addr = format!("{}:{}", config.bind, config.port);
    let listener = match tokio::net::TcpListener::bind(&addr).await {
        Ok(l) => l,
        Err(e) => {
            eprintln!("cubby-server: bind {addr}: {e}");
            std::process::exit(1);
        }
    };
    eprintln!("cubby-server: listening on {addr}");
    let shutdown = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    if let Err(e) = axum::serve(
        listener,
        router.into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown)
    .await
    {
        eprintln!("cubby-server: {e}");
        std::process::exit(1);
    }
}
