pub mod branch;
pub mod cache;
pub mod github;
pub mod merge_detection;
pub mod operations;
pub mod pr_loader;
pub mod squash_loader;
pub mod status;
pub mod tags;
pub mod worktree;

pub fn log_timing(label: &str, elapsed: std::time::Duration) {
    use std::io::Write;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let total_secs = now.as_secs();
    let h = (total_secs / 3600) % 24;
    let m = (total_secs / 60) % 60;
    let s = total_secs % 60;
    let ms = now.subsec_millis();
    let line = format!(
        "[{h:02}:{m:02}:{s:02}.{ms:03}] TIMING: {label}: {}ms\n",
        elapsed.as_millis()
    );
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open("/tmp/gbm-timing.log")
    {
        let _ = f.write_all(line.as_bytes());
    }
}
