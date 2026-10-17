fn main() {
    let endpoint = std::env::var("CUBBY_OTLP_ENDPOINT").unwrap_or_default();
    cubby_worker::run_once(&endpoint);
}
