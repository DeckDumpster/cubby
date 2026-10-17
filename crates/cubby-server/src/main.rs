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
    if let Err(e) = axum::serve(listener, app(&config))
        .with_graceful_shutdown(shutdown)
        .await
    {
        eprintln!("cubby-server: {e}");
        std::process::exit(1);
    }
}
