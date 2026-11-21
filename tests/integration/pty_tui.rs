//! Runs the real binary inside a pseudo-terminal so `terminal::run_app` (raw
//! mode, alternate screen, mouse capture, teardown) executes end to end.
//!
//! GBM_CONFIG_DIR and HOME point at tempdirs, so the real config is never
//! read or written.

use std::io::{Read, Write};
use std::path::Path;
use std::sync::mpsc::{self, RecvTimeoutError};
use std::time::{Duration, Instant};

use portable_pty::{native_pty_system, Child, CommandBuilder, PtySize};

use crate::common::*;

const WAIT: Duration = Duration::from_secs(15);
const EXIT_WAIT: Duration = Duration::from_secs(10);
const POLL: Duration = Duration::from_millis(50);
/// crossterm asks for the cursor position at startup and fails after ~2 s without a reply.
const CURSOR_QUERY: &[u8] = b"\x1b[6n";
const CURSOR_REPLY: &[u8] = b"\x1b[1;1R";

/// Kills the child if the test unwinds before the child has been reaped.
struct KillOnDrop(Box<dyn Child + Send + Sync>);

impl Drop for KillOnDrop {
    fn drop(&mut self) {
        if let Ok(None) = self.0.try_wait() {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
}

struct Pty {
    rx: mpsc::Receiver<Vec<u8>>,
    /// Dropping it writes a newline and VEOF to the pty, so it must outlive the exit wait.
    writer: Box<dyn Write + Send>,
    out: Vec<u8>,
    cursor_replies: usize,
}

impl Pty {
    /// Drains the pty master on a thread. Read errors (EIO after the child exits) end the stream.
    fn new(mut reader: Box<dyn Read + Send>, writer: Box<dyn Write + Send>) -> Self {
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let mut buf = [0u8; 4096];
            while let Ok(n) = reader.read(&mut buf) {
                if n == 0 || tx.send(buf[..n].to_vec()).is_err() {
                    break;
                }
            }
        });
        Self { rx, writer, out: Vec::new(), cursor_replies: 0 }
    }

    /// Accumulates output until `needle` appears or `WAIT` elapses. Checks `out`
    /// before blocking so a needle that arrived with an earlier chunk is not missed.
    fn wait_for(&mut self, needle: &[u8]) -> bool {
        let deadline = Instant::now() + WAIT;
        loop {
            if contains(&self.out, needle) {
                return true;
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return false;
            }
            match self.rx.recv_timeout(remaining) {
                Ok(chunk) => {
                    self.out.extend_from_slice(&chunk);
                    self.answer_cursor_queries();
                }
                Err(RecvTimeoutError::Timeout | RecvTimeoutError::Disconnected) => return false,
            }
        }
    }

    fn answer_cursor_queries(&mut self) {
        let queries = self.out.windows(CURSOR_QUERY.len()).filter(|w| *w == CURSOR_QUERY).count();
        while self.cursor_replies < queries {
            self.writer.write_all(CURSOR_REPLY).expect("reply to cursor query");
            self.writer.flush().expect("flush cursor reply");
            self.cursor_replies += 1;
        }
    }

    fn dump(&self) -> String {
        String::from_utf8_lossy(&self.out).into_owned()
    }
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|window| window == needle)
}

/// Builds the child with a cleared environment so nothing leaks from the user's shell.
fn tui_command(cwd: &Path, cache_root: &Path, config_dir: &Path, home: &Path, xdg: &Path) -> CommandBuilder {
    let mut cmd = CommandBuilder::new(env!("CARGO_BIN_EXE_git-branch-manager"));
    cmd.cwd(cwd);
    cmd.env_clear();
    cmd.env("PATH", std::env::var_os("PATH").expect("PATH is set"));
    if let Some(profile) = std::env::var_os("LLVM_PROFILE_FILE") {
        cmd.env("LLVM_PROFILE_FILE", profile);
    }
    cmd.env("TERM", "xterm-256color");
    cmd.env("GBM_CONFIG_DIR", config_dir);
    cmd.env("GBM_CACHE_DIR", cache_root);
    cmd.env("HOME", home);
    cmd.env("XDG_CONFIG_HOME", xdg.join("config"));
    cmd.env("XDG_CACHE_HOME", xdg.join("cache"));
    cmd.env("XDG_DATA_HOME", xdg.join("data"));
    cmd.env("XDG_STATE_HOME", xdg.join("state"));
    cmd.env("GIT_CONFIG_NOSYSTEM", "1");
    cmd.env("GIT_CONFIG_GLOBAL", "/dev/null");
    cmd
}

#[test]
fn tui_starts_renders_and_quits_cleanly_in_pty() {
    let (tmp, _repo) = setup_test_repo();
    run_git(tmp.path(), &["branch", "pty-feature"]);

    let config_dir = tempfile::tempdir().expect("config tempdir");
    let home = tempfile::tempdir().expect("home tempdir");
    let xdg = tempfile::tempdir().expect("xdg tempdir");
    for sub in ["config", "cache", "data", "state"] {
        std::fs::create_dir_all(xdg.path().join(sub)).expect("create xdg subdir");
    }

    let pair = native_pty_system()
        .openpty(PtySize { rows: 40, cols: 120, pixel_width: 0, pixel_height: 0 })
        .expect("openpty");
    let cmd = tui_command(
        tmp.path(),
        tmp.cache_root(),
        config_dir.path(),
        home.path(),
        xdg.path(),
    );
    let mut child = KillOnDrop(pair.slave.spawn_command(cmd).expect("spawn TUI in pty"));
    drop(pair.slave);

    let reader = pair.master.try_clone_reader().expect("pty reader");
    let writer = pair.master.take_writer().expect("pty writer");
    let mut pty = Pty::new(reader, writer);

    assert!(pty.wait_for(b"\x1b[?1049h"), "alt screen not entered:\n{}", pty.dump());
    assert!(pty.wait_for(b"pty-feature"), "branch row not rendered:\n{}", pty.dump());

    pty.writer.write_all(b"q").expect("send q");
    pty.writer.flush().expect("flush q");

    assert!(pty.wait_for(b"\x1b[?1049l"), "alt screen not left:\n{}", pty.dump());
    assert!(pty.wait_for(b"\x1b[?1000l"), "mouse capture not disabled:\n{}", pty.dump());

    let deadline = Instant::now() + EXIT_WAIT;
    let status = loop {
        if let Some(status) = child.0.try_wait().expect("try_wait") {
            break status;
        }
        if Instant::now() >= deadline {
            let _ = child.0.kill();
            panic!("TUI did not exit within {EXIT_WAIT:?} of 'q':\n{}", pty.dump());
        }
        std::thread::sleep(POLL);
    };
    assert!(status.success(), "TUI exited with {status:?}:\n{}", pty.dump());
}
