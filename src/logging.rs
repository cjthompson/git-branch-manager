use tracing::info;

/// Initialise the optional debug-log subscriber.
///
/// Controlled by the `GBM_DEBUG` env var:
/// - `1` / `true` → log to `<cache_dir>/git-branch-manager/debug.log`
///   (macOS: `~/Library/Caches/git-branch-manager/debug.log`)
/// - any other non-empty value → treated as a file path
/// - unset / `0` / `false` / empty → logging disabled
pub(crate) fn init_debug_log() -> Option<tracing_appender::non_blocking::WorkerGuard> {
    use std::fs::OpenOptions;
    use tracing_subscriber::fmt::format::FmtSpan;
    use tracing_subscriber::EnvFilter;

    let path = match std::env::var("GBM_DEBUG").ok().as_deref() {
        None => return None,
        Some(v) if v == "1" || v.eq_ignore_ascii_case("true") => default_debug_log_path(),
        Some(v) if v == "0" || v.eq_ignore_ascii_case("false") || v.is_empty() => return None,
        Some(v) => std::path::PathBuf::from(v),
    };

    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }

    let log_file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .expect("failed to open debug log");
    let (non_blocking, guard) = tracing_appender::non_blocking(log_file);
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("git_branch_manager=debug"));

    tracing_subscriber::fmt()
        .pretty()
        .with_env_filter(filter)
        .with_writer(non_blocking)
        .with_ansi(false)
        .with_span_events(FmtSpan::CLOSE)
        .init();

    info!(path = %path.display(), "debug log initialized");
    Some(guard)
}

fn default_debug_log_path() -> std::path::PathBuf {
    dirs::cache_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("git-branch-manager")
        .join("debug.log")
}
