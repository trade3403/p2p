use std::{env, fs, path::PathBuf};

/// Keep the non-blocking writer alive for the entire application lifetime.
pub fn init() {
    let base = env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(env::temp_dir);
    let directory = base.join("P2PRemote").join("logs");

    if let Err(error) = fs::create_dir_all(&directory) {
        #[cfg(debug_assertions)]
        eprintln!("P2P Remote could not create its log directory: {error}");
        return;
    }

    let appender = tracing_appender::rolling::daily(&directory, "p2p-remote.log");
    let (writer, guard) = tracing_appender::non_blocking(appender);
    let _guard = Box::leak(Box::new(guard));

    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));

    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(writer)
        .with_ansi(false)
        .try_init();
}
