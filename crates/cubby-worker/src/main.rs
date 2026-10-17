fn main() {
    let config = cubby_worker::Config::from_env();
    if cubby_worker::run_once(&config) == "error" {
        std::process::exit(1);
    }
}
