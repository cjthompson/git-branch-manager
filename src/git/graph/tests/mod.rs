mod cherry_scan;
mod confidence;
mod enrichment_cancel;
mod fixtures;
mod incremental;
mod loaders;
mod merge_scan;
mod relationships;

use super::*;
use super::backends::gleisbau_settings;
use super::cherry::{
    compute_cherry_pick_relationships, compute_cherry_pick_relationships_observed,
    compute_cherry_pick_relationships_with_bounds,
};
use super::load::spawn_enrichment_with_observer;
use super::refs::collect_ref_data;
use super::squash::{displayed_branch_relation, DisplayedBranchRelation};
use crate::git::cache::{BranchCache, CacheRoot};
use fixtures::{merge_pair_fixture, native_commit, native_pair_fixture};
use std::path::Path;
use std::process::Command;
use std::sync::Mutex;

fn squash(
    dest: &str,
    source: &str,
    names: &[&str],
    matching: RelationshipMatch,
) -> GraphRelationship {
    GraphRelationship {
        kind: RelationshipKind::SquashMerge,
        matching,
        destination_oid: dest.into(),
        destination_refs: vec!["main".into()],
        source_oid: source.into(),
        source_refs: names.iter().map(|n| n.to_string()).collect(),
    }
}

fn relationship_snapshot(commits: Vec<GraphCommit>) -> GraphSnapshot {
    GraphSnapshot {
        source: GraphSource::Gleisbau,
        commits,
        lines: vec![],
        ref_counts: GraphRefCounts::default(),
        max_count: 50,
        includes_remotes: false,
        generation: None,
    }
}

#[derive(Clone)]
struct ScanLog(Arc<Mutex<Vec<u8>>>);
impl std::io::Write for ScanLog {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[test]
fn cherry_scan_logs_only_actual_overflow_on_default_target() {
    // Isolate tracing's global callsite-interest cache from parallel tests.
    if std::env::var_os("GBM_P64_SCAN_LOG_CHILD").is_none() {
        let output = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "git::graph::tests::cherry_scan_logs_only_actual_overflow_on_default_target",
                "--nocapture",
                "--test-threads=1",
            ])
            .env("GBM_P64_SCAN_LOG_CHILD", "1")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        return;
    }
    let (dir, snapshot, root) = merge_pair_fixture();
    let cache = BranchCache::load_for_base(dir.path(), "main", &root);
    let bytes = Arc::new(Mutex::new(Vec::new()));
    let writer = ScanLog(Arc::clone(&bytes));
    let subscriber = tracing_subscriber::fmt()
        .with_env_filter("git_branch_manager=debug")
        .with_ansi(false)
        .without_time()
        .with_writer(move || writer.clone())
        .finish();
    tracing::subscriber::with_default(subscriber, || {
        let scan = |sources, destinations| {
            compute_cherry_pick_relationships_with_bounds(
                dir.path(),
                &snapshot,
                "main",
                &snapshot.commits[0].oid,
                &cache,
                &EnrichmentCancel::never(),
                sources,
                destinations,
            )
        };
        assert_eq!(scan(2, 3).len(), 1);
        assert!(
            !String::from_utf8(bytes.lock().unwrap().clone())
                .unwrap()
                .contains("scan_incomplete"),
            "exact exhaustion is not overflow"
        );
        bytes.lock().unwrap().clear();
        assert!(scan(1, 3).is_empty());
        assert_eq!(scan(2, 2).len(), 1);
    });
    let text = String::from_utf8(bytes.lock().unwrap().clone()).unwrap();
    assert!(text.contains("git_branch_manager::git::graph"));
    assert!(text.contains("scan_incomplete: source bound"));
    assert!(text.contains("scan_incomplete: destination bound"));
}
