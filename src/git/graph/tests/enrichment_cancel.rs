use super::*;

#[test]
fn enrichment_cancel_tracks_latest_generation() {
    let latest = Arc::new(AtomicU64::new(3));
    let cancel = EnrichmentCancel::new(Arc::clone(&latest), 3);
    assert!(!cancel.is_cancelled());
    latest.store(4, Ordering::Release);
    assert!(cancel.is_cancelled());
    assert!(!EnrichmentCancel::never().is_cancelled());
}

fn enrichment_git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap().trim().to_owned()
}

fn enrichment_fixture() -> (tempfile::TempDir, GraphSnapshot, CacheRoot) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path();
    enrichment_git(path, &["init", "-b", "main"]);
    enrichment_git(path, &["config", "user.name", "Test"]);
    enrichment_git(path, &["config", "user.email", "test@example.com"]);
    enrichment_git(path, &["commit", "--allow-empty", "-m", "root"]);
    enrichment_git(path, &["checkout", "-b", "feature"]);
    for index in 0..3 {
        let name = format!("feature-{index}.txt");
        std::fs::write(path.join(&name), format!("content {index}\n")).unwrap();
        enrichment_git(path, &["add", &name]);
        enrichment_git(path, &["commit", "-m", &name]);
    }
    enrichment_git(path, &["checkout", "main"]);
    enrichment_git(path, &["merge", "--squash", "feature"]);
    enrichment_git(path, &["commit", "-m", "squash landing"]);
    let cache_root = CacheRoot::at(path.join("cache"));
    let snapshot = load_graph(
        path,
        GraphLoadOptions {
            base_branch: Some("main".into()),
            cache_root: cache_root.clone(),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(
        enrichment_git(path, &["cherry", "main", "feature"])
            .lines()
            .count(),
        3
    );
    (dir, snapshot, cache_root)
}

fn assert_started_enrichment_cancelled(stage: EnrichmentStage) {
    let (dir, snapshot, cache_root) = enrichment_fixture();
    let latest = Arc::new(AtomicU64::new(1));
    let cancel = EnrichmentCancel::new(Arc::clone(&latest), 1);
    let (started_tx, started_rx) = mpsc::sync_channel(0);
    let (resume_tx, resume_rx) = mpsc::sync_channel(0);
    let resume_rx = Mutex::new(resume_rx);
    let observed = Arc::new(AtomicU64::new(0));
    let observed_worker = Arc::clone(&observed);
    let rx = spawn_enrichment_with_observer(
        snapshot,
        dir.path().to_path_buf(),
        Some("main".into()),
        1,
        cache_root,
        cancel,
        move |current| {
            if current == stage {
                observed_worker.fetch_add(1, Ordering::Relaxed);
                started_tx.send(()).unwrap();
                resume_rx.lock().unwrap().recv().unwrap();
            }
        },
    );
    let timeout = std::time::Duration::from_secs(60);
    started_rx
        .recv_timeout(timeout)
        .expect("worker reached cancellation boundary");
    latest.store(2, Ordering::Release);
    resume_tx.send(()).unwrap();
    assert_eq!(
        rx.recv_timeout(timeout),
        Err(mpsc::RecvTimeoutError::Disconnected)
    );
    assert_eq!(
        observed.load(Ordering::Relaxed),
        1,
        "no further work at this boundary"
    );
}

#[test]
fn started_enrichment_superseded_during_patch_dispatch_publishes_nothing() {
    assert_started_enrichment_cancelled(EnrichmentStage::PatchDispatch);
}

#[test]
fn started_enrichment_superseded_during_cherry_source_revwalk_publishes_nothing() {
    assert_started_enrichment_cancelled(EnrichmentStage::CherrySourceYield);
}
