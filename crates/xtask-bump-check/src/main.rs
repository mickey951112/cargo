mod xtask;

fn main() {
    env_logger::init_from_env("CARGO_LOG");
    setup_logger();

    let cli = xtask::cli();
    let matches = cli.get_matches();

    let mut config = cargo::util::config::Config::default().unwrap_or_else(|e| {
        let mut eval = cargo::core::shell::Shell::new();
        cargo::exit_with_error(e.into(), &mut eval)
    });
    if let Err(e) = xtask::exec(&matches, &mut config) {
        cargo::exit_with_error(e, &mut config.shell())
    }
}

// In sync with `src/bin/cargo/main.rs@setup_logger`.
fn setup_logger() {
    let env = tracing_subscriber::EnvFilter::from_env("CARGO_LOG");

    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(env)
        .init();
}
