use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::rc::Rc;
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex};

use thiserror::Error;

/// An app-owned graph payload. It deliberately contains no repository handles
/// or Gleisbau values, so it can move across a background channel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphSnapshot {
    pub source: GraphSource,
    pub commits: Vec<GraphCommit>,
    pub lines: Vec<GraphLine>,
    pub ref_counts: GraphRefCounts,
    pub max_count: usize,
    pub includes_remotes: bool,
    /// Reload-generation tag. `None` for snapshots that bypass the enrichment
    /// channel (test fixtures, the CLI dump path). The App stamps every
    /// `load_graph` result with `Some(generation)` so it can match enrichment
    /// messages to the snapshot they belong to and drop stale ones.
    pub generation: Option<u64>,
}

/// One per-commit update produced by asynchronous squash-merge enrichment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphEnrichmentUpdate {
    pub oid: String,
    pub is_possible_squash_merge: bool,
    pub fuzzy_squash_match: Option<FuzzySquashMatch>,
}

/// Channel message carrying the full set of squash-merge enrichment updates
/// for a given `GraphSnapshot` reload generation. Delivered as a single
/// batch rather than per-commit so the App applies it atomically.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphEnrichmentMsg {
    pub generation: u64,
    pub updates: Vec<GraphEnrichmentUpdate>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GraphSource {
    Gleisbau,
    GitCliFallback { cause: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphCommit {
    pub oid: String,
    pub summary: String,
    pub parents: Vec<String>,
    pub lane: Option<usize>,
    /// The live branch whose first-parent history owns this commit. The base
    /// branch claims its chain first so retained merged refs cannot relabel it.
    pub branch: Option<GraphBranchLabel>,
    pub refs: Vec<GraphRef>,
    /// True when this displayed base-branch commit has the same stable Git
    /// patch ID as the aggregate patch of a displayed, non-merged local branch.
    pub is_possible_squash_merge: bool,
    /// Set when this commit's diff is a *near*-match (not exact) for a
    /// displayed branch tip's aggregate diff, per the Option 6 fuzzy/possible
    /// tier (`git::fuzzy_match`). Never set on a commit that already has
    /// `is_possible_squash_merge == true` — Option 6 is additive and defers
    /// to the exact-match tier.
    pub fuzzy_squash_match: Option<FuzzySquashMatch>,
}

/// A fuzzy (non-exact) possible-squash-merge signal for a single base commit,
/// scored against the best-matching displayed branch tip. See
/// `docs/plans/2026-08-29-squash-merge-test-scenarios.md` "New: Option 6".
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FuzzySquashMatch {
    /// Jaccard similarity as an integer percent (0-100), rounded from the raw
    /// f32 score. Stored as u8 rather than f32 so GraphCommit/FuzzySquashMatch
    /// can keep deriving Eq (f32 has no Eq impl).
    pub similarity_percent: u8,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphBranchLabel {
    pub name: String,
    pub target_oid: String,
    pub kind: GraphRefKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphLine {
    pub graph: String,
    pub commit_index: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphRef {
    pub name: String,
    pub kind: GraphRefKind,
    pub has_linked_worktree: bool,
    pub tracking: Option<GraphRefTracking>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphRefTracking {
    pub ahead: u32,
    pub behind: u32,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct GraphRefCounts {
    pub local: usize,
    pub remote: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GraphRefKind {
    LocalBranch,
    RemoteBranch,
    Tag,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphLoadOptions {
    pub max_count: usize,
    pub include_remotes: bool,
    pub line_style: GraphLineStyle,
    pub base_branch: Option<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum GraphLineStyle {
    #[default]
    Thin,
    Round,
}

impl GraphLineStyle {
    pub fn from_symbol_name(name: &str) -> Self {
        if name == "powerline" {
            Self::Round
        } else {
            Self::Thin
        }
    }
}

impl Default for GraphLoadOptions {
    fn default() -> Self {
        Self {
            max_count: 500,
            include_remotes: false,
            line_style: GraphLineStyle::default(),
            base_branch: None,
        }
    }
}

#[derive(Debug, Error)]
pub enum GraphLoadError {
    #[error("Gleisbau failed ({gleisbau}) and the git CLI fallback failed ({fallback})")]
    Both { gleisbau: String, fallback: String },
}

pub fn load_graph(
    repo_path: &Path,
    options: GraphLoadOptions,
) -> Result<GraphSnapshot, GraphLoadError> {
    let gleisbau_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        load_with_gleisbau(repo_path, options.clone())
    }));

    match gleisbau_result {
        Ok(Ok(snapshot)) => Ok(snapshot),
        Ok(Err(cause)) => load_with_git_cli(repo_path, options, cause.clone()).map_err(|fallback| {
            GraphLoadError::Both {
                gleisbau: cause,
                fallback,
            }
        }),
        Err(_) => {
            let cause = "Gleisbau panicked while building the graph".to_string();
            load_with_git_cli(repo_path, options, cause.clone()).map_err(|fallback| {
                GraphLoadError::Both {
                    gleisbau: cause,
                    fallback,
                }
            })
        }
    }
}

/// Synchronous, single-call helper for tests: loads the structural snapshot
/// and applies squash-merge enrichment on the calling thread, returning the
/// fully annotated snapshot. The app uses `spawn_possible_squash_enrichment`
/// instead so the Graph view becomes usable before annotation completes.
#[doc(hidden)]
pub fn load_graph_with_squash_annotations(
    repo_path: &Path,
    options: GraphLoadOptions,
) -> Result<GraphSnapshot, GraphLoadError> {
    let mut snapshot = load_graph(repo_path, options.clone())?;
    let updates =
        compute_possible_squash_updates(repo_path, &snapshot, options.base_branch.as_deref());
    apply_squash_enrichment(&mut snapshot, &updates);
    Ok(snapshot)
}

/// Spawn a background thread that computes squash-merge enrichment for the
/// given snapshot and returns a receiver for the single batched result. The
/// caller (App) is expected to compare `msg.generation` against its current
/// reload generation before applying — enrichment that completes after a
/// newer load started must NOT overwrite the newer snapshot.
pub fn spawn_possible_squash_enrichment(
    snapshot: GraphSnapshot,
    repo_path: PathBuf,
    requested_base: Option<String>,
    generation: u64,
) -> Receiver<GraphEnrichmentMsg> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let updates = compute_possible_squash_updates(
            &repo_path,
            &snapshot,
            requested_base.as_deref(),
        );
        let _ = tx.send(GraphEnrichmentMsg {
            generation,
            updates,
        });
    });
    rx
}

/// Load a graph in a worker thread. The receiver carries only the owned
/// [`GraphSnapshot`] result, never a repository or Gleisbau type.
pub fn spawn_graph_loader(
    repo_path: PathBuf,
    options: GraphLoadOptions,
) -> Receiver<Result<GraphSnapshot, GraphLoadError>> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(load_graph(&repo_path, options));
    });
    rx
}

fn load_with_gleisbau(
    repo_path: &Path,
    options: GraphLoadOptions,
) -> Result<GraphSnapshot, String> {
    let ref_data = collect_ref_data(
        repo_path,
        options.include_remotes,
        options.base_branch.as_deref(),
    )?;
    let settings = gleisbau_settings(options.include_remotes, options.line_style)?;
    let repository =
        gleisbau::Repository::open(repo_path).map_err(|error| error.message().to_string())?;
    let graph = gleisbau::graph::Builder::new()
        .with_repository(repository)
        .with_settings(Rc::clone(&settings))
        .with_max_count(options.max_count)
        .build()?;
    let heights = vec![1; graph.layout.commit_count()];
    let rendered = gleisbau::print::unicode::print_graph_terminal(
        &settings,
        &graph.tracks,
        &graph.layout,
        &heights,
    );
    let mut line_to_commit = HashMap::new();
    for (relative_index, line_index) in rendered.commit2line.iter().copied().enumerate() {
        line_to_commit.insert(
            line_index,
            graph.layout.commit_index_start() + relative_index,
        );
    }

    let mut commits = graph
        .tracks
        .commits
        .iter()
        .map(|info| {
            let oid = info.oid.to_string();
            let summary = graph
                .repository
                .find_commit(info.oid)
                .map_err(|error| error.message().to_string())?
                .summary()
                .map_err(|error| error.message().to_string())?
                .unwrap_or_default()
                .to_string();
            let lane = info
                .branch_trace
                .and_then(|trace| graph.layout.track_visual(trace))
                .and_then(|visual| visual.column);

            Ok(GraphCommit {
                oid: oid.clone(),
                summary,
                parents: info.parents.iter().map(ToString::to_string).collect(),
                lane,
                branch: None,
                refs: ref_data.refs_by_oid.get(&oid).cloned().unwrap_or_default(),
                is_possible_squash_merge: false,
                fuzzy_squash_match: None,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    assign_graph_branch_labels(
        &mut commits,
        &ref_data.branch_labels,
        ref_data.base_branch.as_deref(),
    );
    Ok(GraphSnapshot {
        source: GraphSource::Gleisbau,
        commits,
        lines: rendered
            .graph_lines
            .into_iter()
            .enumerate()
            .map(|(index, graph)| GraphLine {
                graph,
                commit_index: line_to_commit.get(&index).copied(),
            })
            .collect(),
        ref_counts: ref_data.ref_counts,
        max_count: options.max_count,
        includes_remotes: options.include_remotes,
        generation: None,
    })
}

fn load_with_git_cli(
    repo_path: &Path,
    options: GraphLoadOptions,
    cause: String,
) -> Result<GraphSnapshot, String> {
    let ref_data = collect_ref_data(
        repo_path,
        options.include_remotes,
        options.base_branch.as_deref(),
    )?;
    let max_count = format!("--max-count={}", options.max_count);
    let mut command = Command::new("git");
    command.current_dir(repo_path).args([
        "-c",
        "color.ui=false",
        "log",
        "--graph",
        "--topo-order",
        "--decorate",
        "--oneline",
        "--no-color",
        "--format=%x1e%H%x1f%P%x1f%s",
        &max_count,
    ]);
    if options.include_remotes {
        // Gleisbau uses `push_glob("*")` when remotes are included, which
        // covers tag-only and other non-head refs as well as heads/remotes.
        command.arg("--all");
    } else {
        command.arg("--branches");
    }
    let output = command
        .output()
        .map_err(|error| format!("failed to run git log: {error}"))?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
    }

    let mut commits = Vec::new();
    let mut lines = Vec::new();
    for line in String::from_utf8_lossy(&output.stdout).lines() {
        let Some(record_start) = line.find('\x1e') else {
            lines.push(GraphLine {
                graph: line.to_string(),
                commit_index: None,
            });
            continue;
        };

        let graph = line[..record_start].to_string();
        let fields: Vec<_> = line[record_start + 1..].splitn(3, '\x1f').collect();
        if fields.len() != 3 || fields[0].is_empty() {
            return Err(format!("could not parse git log record: {line}"));
        }

        let oid = fields[0].to_string();
        let commit_index = commits.len();
        let lane = graph
            .chars()
            .position(|character| character == '*')
            .map(|index| index / 2);
        commits.push(GraphCommit {
            oid: oid.clone(),
            summary: fields[2].to_string(),
            parents: fields[1].split_whitespace().map(str::to_string).collect(),
            lane,
            branch: None,
            refs: ref_data.refs_by_oid.get(&oid).cloned().unwrap_or_default(),
            is_possible_squash_merge: false,
            fuzzy_squash_match: None,
        });
        lines.push(GraphLine {
            graph,
            commit_index: Some(commit_index),
        });
    }
    assign_graph_branch_labels(
        &mut commits,
        &ref_data.branch_labels,
        ref_data.base_branch.as_deref(),
    );
    Ok(GraphSnapshot {
        source: GraphSource::GitCliFallback { cause },
        commits,
        lines,
        ref_counts: ref_data.ref_counts,
        max_count: options.max_count,
        includes_remotes: options.include_remotes,
        generation: None,
    })
}

/// Match only work represented by this bounded snapshot. Both base commits and
/// local branch tips are selected from `snapshot.commits`, so increasing
/// repository history or ref age cannot make Graph startup scan beyond the
/// configured `max_count` window.
///
/// Returns a list of per-commit updates that the caller applies to the
/// snapshot via `apply_squash_enrichment`. This split lets the App run the
/// heavy work on a background thread and the result stays purely owned
/// (no `&mut` over the App's snapshot) so it can be applied at any time.
pub fn compute_possible_squash_updates(
    repo_path: &Path,
    snapshot: &GraphSnapshot,
    requested_base: Option<&str>,
) -> Vec<GraphEnrichmentUpdate> {
    let base_branch = requested_base.map(str::to_string).or_else(|| {
        let repository = git2::Repository::open(repo_path).ok()?;
        crate::git::branch::detect_base_branch(&repository, None).ok()
    });
    let Some(base_branch) = base_branch else {
        return Vec::new();
    };
    let Some(base_tip) = snapshot.commits.iter().find_map(|commit| {
        commit
            .refs
            .iter()
            .any(|reference| {
                reference.kind == GraphRefKind::LocalBranch && reference.name == base_branch
            })
            .then(|| commit.oid.clone())
    }) else {
        return Vec::new();
    };

    let mut jobs = snapshot
        .commits
        .iter()
        .filter(|commit| {
            commit.parents.len() == 1
                && commit.branch.as_ref().is_some_and(|branch| {
                    branch.kind == GraphRefKind::LocalBranch && branch.name == base_branch
                })
        })
        .map(|commit| PatchJob {
            target: PatchTarget::BaseCommit(commit.oid.clone()),
            old_oid: commit.parents[0].clone(),
            new_oid: commit.oid.clone(),
        })
        .collect::<Vec<_>>();

    let displayed_branch_tips = snapshot
        .commits
        .iter()
        .filter(|commit| {
            commit.refs.iter().any(|reference| {
                reference.kind == GraphRefKind::LocalBranch && reference.name != base_branch
            })
        })
        .map(|commit| commit.oid.clone())
        .collect::<HashSet<_>>();

    for tip in displayed_branch_tips {
        let merge_base = match displayed_branch_relation(&snapshot.commits, &base_tip, &tip) {
            DisplayedBranchRelation::Diverged { merge_base } => merge_base,
            DisplayedBranchRelation::RegularlyMerged | DisplayedBranchRelation::Ineligible => {
                continue;
            }
        };
        jobs.push(PatchJob {
            target: PatchTarget::BranchTip,
            old_oid: merge_base,
            new_oid: tip,
        });
    }

    let mut base_oids_by_patch = HashMap::<String, Vec<String>>::new();
    let mut branch_patch_ids = HashSet::new();
    // Retained alongside patch IDs so pairs whose patch IDs don't exactly
    // match can still be scored by the Option 6 fuzzy tier below, without a
    // second `git diff` subprocess per job.
    let mut base_diffs = Vec::<(String, Vec<u8>)>::new();
    let mut branch_diffs = Vec::<Vec<u8>>::new();
    for result in load_patch_ids(repo_path, jobs) {
        match result.target {
            PatchTarget::BaseCommit(oid) => {
                if let Some(patch_id) = result.patch_id {
                    base_oids_by_patch
                        .entry(patch_id)
                        .or_default()
                        .push(oid.clone());
                }
                if let Some(diff_text) = result.diff_text {
                    base_diffs.push((oid, diff_text));
                }
            }
            PatchTarget::BranchTip => {
                if let Some(patch_id) = result.patch_id {
                    branch_patch_ids.insert(patch_id);
                }
                if let Some(diff_text) = result.diff_text {
                    branch_diffs.push(diff_text);
                }
            }
        }
    }

    let matching_base_oids = branch_patch_ids
        .iter()
        .filter_map(|patch_id| base_oids_by_patch.get(patch_id))
        .flatten()
        .collect::<HashSet<_>>();

    // Option 6: for base commits that didn't get an exact patch-id match,
    // score their diff against every displayed branch tip's diff and keep
    // the best fuzzy classification, if any clears the threshold.
    let mut fuzzy_by_oid: HashMap<String, FuzzySquashMatch> = HashMap::new();
    for (oid, base_diff) in &base_diffs {
        if matching_base_oids.contains(oid) {
            continue;
        }
        let best_percent = branch_diffs
            .iter()
            .filter_map(|branch_diff| crate::git::fuzzy_match::score(branch_diff, base_diff))
            .filter_map(|fuzzy_score| crate::git::fuzzy_match::classify(&fuzzy_score))
            .max();
        let Some(percent) = best_percent else {
            continue;
        };
        fuzzy_by_oid.insert(
            oid.clone(),
            FuzzySquashMatch {
                similarity_percent: percent,
            },
        );
    }

    snapshot
        .commits
        .iter()
        .filter(|commit| {
            commit.branch.as_ref().is_some_and(|branch| {
                branch.kind == GraphRefKind::LocalBranch && branch.name == base_branch
            })
        })
        .map(|commit| GraphEnrichmentUpdate {
            oid: commit.oid.clone(),
            is_possible_squash_merge: matching_base_oids.contains(&commit.oid),
            fuzzy_squash_match: fuzzy_by_oid.get(&commit.oid).cloned(),
        })
        .collect()
}

/// Apply a batch of squash-merge enrichment updates to a snapshot. Unknown
/// OIDs are ignored — they may belong to a newer snapshot (stale enrichment)
/// or to commits the structural snapshot didn't include.
pub fn apply_squash_enrichment(snapshot: &mut GraphSnapshot, updates: &[GraphEnrichmentUpdate]) {
    for update in updates {
        if let Some(commit) = snapshot
            .commits
            .iter_mut()
            .find(|commit| commit.oid == update.oid)
        {
            commit.is_possible_squash_merge = update.is_possible_squash_merge;
            commit.fuzzy_squash_match = update.fuzzy_squash_match.clone();
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
enum DisplayedBranchRelation {
    RegularlyMerged,
    Diverged { merge_base: String },
    Ineligible,
}

/// Resolve ancestry only through commits already owned by the snapshot. A
/// merge base outside the displayed window is intentionally ineligible rather
/// than triggering an unbounded repository walk during Graph startup.
fn displayed_branch_relation(
    commits: &[GraphCommit],
    base_tip: &str,
    branch_tip: &str,
) -> DisplayedBranchRelation {
    let commits_by_oid = commits
        .iter()
        .map(|commit| (commit.oid.as_str(), commit))
        .collect::<HashMap<_, _>>();
    let base_ancestors = displayed_ancestors(&commits_by_oid, base_tip);
    if base_ancestors.contains(branch_tip) {
        return DisplayedBranchRelation::RegularlyMerged;
    }
    let branch_ancestors = displayed_ancestors(&commits_by_oid, branch_tip);
    let common = base_ancestors
        .intersection(&branch_ancestors)
        .copied()
        .collect::<HashSet<_>>();
    if common.is_empty() {
        return DisplayedBranchRelation::Ineligible;
    }

    let ancestors_by_common = common
        .iter()
        .map(|oid| (*oid, displayed_ancestors(&commits_by_oid, oid)))
        .collect::<HashMap<_, _>>();
    let mut best = common.iter().copied().filter(|candidate| {
        !common.iter().any(|other| {
            other != candidate
                && ancestors_by_common
                    .get(other)
                    .is_some_and(|ancestors| ancestors.contains(candidate))
        })
    });
    let Some(merge_base) = best.next() else {
        return DisplayedBranchRelation::Ineligible;
    };
    if best.next().is_some() {
        return DisplayedBranchRelation::Ineligible;
    }
    DisplayedBranchRelation::Diverged {
        merge_base: merge_base.to_string(),
    }
}

fn displayed_ancestors<'a>(
    commits_by_oid: &HashMap<&'a str, &'a GraphCommit>,
    start: &str,
) -> HashSet<&'a str> {
    let Some(start) = commits_by_oid.get(start) else {
        return HashSet::new();
    };
    let mut ancestors = HashSet::new();
    let mut pending = vec![start.oid.as_str()];
    while let Some(oid) = pending.pop() {
        if !ancestors.insert(oid) {
            continue;
        }
        if let Some(commit) = commits_by_oid.get(oid) {
            pending.extend(
                commit
                    .parents
                    .iter()
                    .filter_map(|parent| commits_by_oid.get(parent.as_str()))
                    .map(|parent| parent.oid.as_str()),
            );
        }
    }
    ancestors
}

/// Like the established squash loader, Graph patch matching uses four workers
/// regardless of CPU count. Each worker runs at most one Git subprocess at a
/// time, so both worker and active-subprocess concurrency are capped at four.
const GRAPH_PATCH_WORKER_COUNT: usize = 4;

struct PatchJob {
    target: PatchTarget,
    old_oid: String,
    new_oid: String,
}

enum PatchTarget {
    BaseCommit(String),
    BranchTip,
}

struct PatchResult {
    target: PatchTarget,
    patch_id: Option<String>,
    /// Raw `git diff` stdout for this job, retained alongside the patch-id so
    /// `annotate_possible_squash_merges` can run the Option 6 fuzzy-match tier
    /// on pairs whose patch IDs don't exactly match, without a second `git
    /// diff` subprocess per job. `Some` whenever the diff subprocess produced
    /// non-empty output (regardless of whether patch-id computation itself
    /// succeeded); `None` for empty/net-zero diffs or subprocess failures.
    diff_text: Option<Vec<u8>>,
}

fn load_patch_ids(repo_path: &Path, jobs: Vec<PatchJob>) -> Vec<PatchResult> {
    if jobs.is_empty() {
        return Vec::new();
    }

    let worker_count = jobs.len().min(GRAPH_PATCH_WORKER_COUNT);
    let queue = Arc::new(Mutex::new(
        jobs.into_iter().collect::<std::collections::VecDeque<_>>(),
    ));
    let (tx, rx) = mpsc::channel();
    let mut handles = Vec::with_capacity(worker_count);
    for _ in 0..worker_count {
        let queue = Arc::clone(&queue);
        let tx = tx.clone();
        let repo_path = repo_path.to_path_buf();
        handles.push(std::thread::spawn(move || {
            while let Some(job) = next_patch_job(&queue) {
                let (patch_id, diff_text) = compute_patch(&repo_path, &job.old_oid, &job.new_oid);
                if tx
                    .send(PatchResult {
                        target: job.target,
                        patch_id,
                        diff_text,
                    })
                    .is_err()
                {
                    break;
                }
            }
        }));
    }
    drop(tx);

    let results = rx.into_iter().collect();
    for handle in handles {
        let _ = handle.join();
    }
    results
}

fn next_patch_job(queue: &Mutex<std::collections::VecDeque<PatchJob>>) -> Option<PatchJob> {
    queue.lock().ok()?.pop_front()
}

/// Run a single `git diff` between `old_oid` and `new_oid`, then feed its
/// output through `git patch-id --stable`. Returns `(patch_id, diff_text)`:
/// `diff_text` is the raw diff bytes (for the Option 6 fuzzy-match tier),
/// `patch_id` is the stable patch identity (for the existing exact-match
/// tier). Both share the single `git diff` invocation below rather than
/// re-running it per caller.
fn compute_patch(repo_path: &Path, old_oid: &str, new_oid: &str) -> (Option<String>, Option<Vec<u8>>) {
    let diff = Command::new("git")
        .current_dir(repo_path)
        .args([
            "diff",
            "--binary",
            "--full-index",
            "--no-ext-diff",
            "--no-textconv",
            old_oid,
            new_oid,
            "--",
        ])
        .stdin(Stdio::null())
        .output();
    let Ok(diff) = diff else {
        return (None, None);
    };
    if !diff.status.success() || diff.stdout.is_empty() {
        return (None, None);
    }
    let diff_text = Some(diff.stdout.clone());

    let patch_id = (|| {
        let mut patch_id = Command::new("git")
            .current_dir(repo_path)
            .args(["patch-id", "--stable"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .ok()?;
        patch_id.stdin.as_mut()?.write_all(&diff.stdout).ok()?;
        let output = patch_id.wait_with_output().ok()?;
        if !output.status.success() {
            return None;
        }
        String::from_utf8_lossy(&output.stdout)
            .split_whitespace()
            .next()
            .map(str::to_string)
    })();

    (patch_id, diff_text)
}

fn gleisbau_settings(
    include_remotes: bool,
    line_style: GraphLineStyle,
) -> Result<Rc<gleisbau::settings::Settings>, String> {
    use gleisbau::print::format::CommitFormat;
    use gleisbau::settings::{
        BranchOrder, BranchSettings, BranchSettingsDef, Characters, MergePatterns, Settings,
    };

    let characters = match line_style {
        GraphLineStyle::Thin => Characters::thin(),
        GraphLineStyle::Round => Characters::round(),
    };

    Ok(Rc::new(Settings {
        reverse_commit_order: false,
        debug: false,
        compact: true,
        colored: false,
        include_remote: include_remotes,
        format: CommitFormat::OneLine,
        wrapping: None,
        characters,
        branch_order: BranchOrder::ShortestFirst(true),
        branches: BranchSettings::from(BranchSettingsDef::none())
            .map_err(|error| error.to_string())?,
        merge_patterns: MergePatterns::default(),
    }))
}

#[derive(Default)]
struct RefData {
    refs_by_oid: HashMap<String, Vec<GraphRef>>,
    ref_counts: GraphRefCounts,
    branch_labels: HashMap<String, GraphBranchLabel>,
    base_branch: Option<String>,
}

fn collect_ref_data(
    repo_path: &Path,
    include_remotes: bool,
    requested_base: Option<&str>,
) -> Result<RefData, String> {
    let repository =
        git2::Repository::open(repo_path).map_err(|error| error.message().to_string())?;
    let mut data = RefData::default();
    let base_branch = requested_base
        .map(str::to_string)
        .or_else(|| crate::git::branch::detect_base_branch(&repository, None).ok());
    data.base_branch = base_branch.clone();
    let linked_worktrees = crate::git::worktree::list_worktrees(repo_path)
        .into_iter()
        .filter(|worktree| !worktree.is_main)
        .filter_map(|worktree| worktree.branch)
        .collect::<HashSet<_>>();
    let mut local_refs = Vec::new();
    let mut remote_refs = Vec::new();

    for branch in repository
        .branches(Some(git2::BranchType::Local))
        .map_err(|error| error.message().to_string())?
    {
        let (branch, _) = branch.map_err(|error| error.message().to_string())?;
        let Some(name) = branch
            .name()
            .map_err(|error| error.message().to_string())?
            .map(str::to_string)
        else {
            continue;
        };
        let Some(target) = branch.get().target() else {
            continue;
        };
        let target_oid = target.to_string();
        local_refs.push((name.clone(), target_oid.clone()));
        data.ref_counts.local += 1;
        let label = GraphBranchLabel {
            name: name.clone(),
            target_oid: target_oid.clone(),
            kind: GraphRefKind::LocalBranch,
        };
        data.branch_labels.insert(name.clone(), label);
    }

    for branch in repository
        .branches(Some(git2::BranchType::Remote))
        .map_err(|error| error.message().to_string())?
    {
        let (branch, _) = branch.map_err(|error| error.message().to_string())?;
        let Some(name) = branch
            .name()
            .map_err(|error| error.message().to_string())?
            .map(str::to_string)
        else {
            continue;
        };
        if name.ends_with("/HEAD") {
            continue;
        }
        let Some(target) = branch.get().target() else {
            continue;
        };
        let target_oid = target.to_string();
        remote_refs.push((name.clone(), target_oid.clone()));
        if include_remotes {
            data.ref_counts.remote += 1;
            data.branch_labels.insert(
                name.clone(),
                GraphBranchLabel {
                    name: name.clone(),
                    target_oid: target_oid.clone(),
                    kind: GraphRefKind::RemoteBranch,
                },
            );
        }
    }

    for (name, target_oid) in &local_refs {
        let tracking = remote_refs
            .iter()
            .filter(|(remote_name, _)| remote_short_name(remote_name) == name)
            .find_map(|(_remote_name, remote_oid)| {
                let (ahead, behind) = repository
                    .graph_ahead_behind(
                        git2::Oid::from_str(target_oid).ok()?,
                        git2::Oid::from_str(remote_oid).ok()?,
                    )
                    .ok()?;
                Some(GraphRefTracking {
                    ahead: ahead.try_into().unwrap_or(u32::MAX),
                    behind: behind.try_into().unwrap_or(u32::MAX),
                })
            });
        insert_ref(
            &mut data.refs_by_oid,
            target_oid,
            GraphRef {
                name: name.clone(),
                kind: GraphRefKind::LocalBranch,
                has_linked_worktree: linked_worktrees.contains(name),
                tracking,
            },
        );
    }

    if include_remotes {
        for (name, target_oid) in &remote_refs {
            insert_ref(
                &mut data.refs_by_oid,
                target_oid,
                GraphRef {
                    name: name.clone(),
                    kind: GraphRefKind::RemoteBranch,
                    has_linked_worktree: false,
                    tracking: None,
                },
            );
        }
    }

    for name in repository
        .tag_names(None)
        .map_err(|error| error.message().to_string())?
        .iter()
        .filter_map(|name| name.ok().flatten())
    {
        let Ok(target) = repository
            .revparse_single(&format!("refs/tags/{name}"))
            .and_then(|object| object.peel_to_commit())
        else {
            continue;
        };
        insert_ref(
            &mut data.refs_by_oid,
            &target.id().to_string(),
            GraphRef {
                name: name.to_string(),
                kind: GraphRefKind::Tag,
                has_linked_worktree: false,
                tracking: None,
            },
        );
    }

    Ok(data)
}

fn remote_short_name(name: &str) -> &str {
    name.split_once('/').map(|(_, short)| short).unwrap_or(name)
}

fn assign_graph_branch_labels(
    commits: &mut [GraphCommit],
    branch_labels: &HashMap<String, GraphBranchLabel>,
    base_branch: Option<&str>,
) {
    assign_first_parent_branch_labels(commits, branch_labels, base_branch);
    assign_merge_subject_branch_labels(commits);
}

fn assign_first_parent_branch_labels(
    commits: &mut [GraphCommit],
    branch_labels: &HashMap<String, GraphBranchLabel>,
    base_branch: Option<&str>,
) {
    let index_by_oid = commits
        .iter()
        .enumerate()
        .map(|(index, commit)| (commit.oid.clone(), index))
        .collect::<HashMap<_, _>>();
    let mut labels = branch_labels.values().cloned().collect::<Vec<_>>();
    labels.sort_by(|left, right| {
        let priority = |label: &GraphBranchLabel| {
            if label.kind == GraphRefKind::LocalBranch && Some(label.name.as_str()) == base_branch {
                0
            } else {
                match label.kind {
                    GraphRefKind::LocalBranch => 1,
                    GraphRefKind::RemoteBranch => 2,
                    GraphRefKind::Tag => 3,
                }
            }
        };
        priority(left)
            .cmp(&priority(right))
            .then_with(|| left.name.cmp(&right.name))
    });
    for label in labels {
        let mut oid = label.target_oid.clone();
        let mut visited = HashSet::new();
        while visited.insert(oid.clone()) {
            let Some(&index) = index_by_oid.get(&oid) else {
                break;
            };
            if commits[index].branch.is_none() {
                commits[index].branch = Some(label.clone());
            }
            let Some(parent) = commits[index].parents.first() else {
                break;
            };
            oid = parent.clone();
        }
    }
}

fn assign_merge_subject_branch_labels(commits: &mut [GraphCommit]) {
    let index_by_oid = commits
        .iter()
        .enumerate()
        .map(|(index, commit)| (commit.oid.clone(), index))
        .collect::<HashMap<_, _>>();

    for merge_index in 0..commits.len() {
        let Some(branch_name) = parse_merge_branch_name(&commits[merge_index].summary) else {
            continue;
        };
        let Some(first_parent) = commits[merge_index].parents.first() else {
            continue;
        };
        let Some(second_parent) = commits[merge_index].parents.get(1) else {
            continue;
        };

        let receiving_branch_history = first_parent_history(commits, &index_by_oid, first_parent);
        let label = GraphBranchLabel {
            name: branch_name,
            target_oid: second_parent.clone(),
            kind: GraphRefKind::LocalBranch,
        };
        let mut oid = second_parent.clone();
        let mut visited = HashSet::new();
        while visited.insert(oid.clone()) {
            if receiving_branch_history.contains(&oid) {
                break;
            }
            let Some(&index) = index_by_oid.get(&oid) else {
                break;
            };
            if commits[index].branch.is_none() {
                commits[index].branch = Some(label.clone());
            }
            let Some(parent) = commits[index].parents.first() else {
                break;
            };
            oid = parent.clone();
        }
    }
}

fn first_parent_history(
    commits: &[GraphCommit],
    index_by_oid: &HashMap<String, usize>,
    start_oid: &str,
) -> HashSet<String> {
    let mut history = HashSet::new();
    let mut oid = start_oid.to_string();
    while history.insert(oid.clone()) {
        let Some(&index) = index_by_oid.get(&oid) else {
            break;
        };
        let Some(parent) = commits[index].parents.first() else {
            break;
        };
        oid = parent.clone();
    }
    history
}

fn parse_merge_branch_name(summary: &str) -> Option<String> {
    let marker = "Merge branch '";
    let start = summary.find(marker)? + marker.len();
    let rest = &summary[start..];
    let end = rest.find('\'')?;
    let name = &rest[..end];
    (!name.is_empty()).then(|| name.to_string())
}

fn insert_ref(refs_by_oid: &mut HashMap<String, Vec<GraphRef>>, oid: &str, reference: GraphRef) {
    refs_by_oid
        .entry(oid.to_string())
        .or_default()
        .push(reference);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn commit(oid: &str, parents: &[&str]) -> GraphCommit {
        GraphCommit {
            oid: oid.into(),
            summary: oid.into(),
            parents: parents.iter().map(|parent| (*parent).into()).collect(),
            lane: None,
            branch: None,
            refs: Vec::new(),
            is_possible_squash_merge: false,
            fuzzy_squash_match: None,
        }
    }

    #[test]
    fn round_line_style_uses_gleisbau_round_characters() {
        use gleisbau::settings::Characters;

        let settings = gleisbau_settings(false, GraphLineStyle::Round).unwrap();

        assert_eq!(settings.characters.chars, Characters::round().chars);
    }

    #[test]
    fn symbol_sets_only_select_rounded_lines_for_powerline() {
        assert_eq!(
            GraphLineStyle::from_symbol_name("powerline"),
            GraphLineStyle::Round
        );
        assert_eq!(
            GraphLineStyle::from_symbol_name("unicode"),
            GraphLineStyle::Thin
        );
        assert_eq!(
            GraphLineStyle::from_symbol_name("ascii"),
            GraphLineStyle::Thin
        );
    }

    #[test]
    fn displayed_history_bounds_branch_relationships() {
        let commits = vec![
            commit("base-tip", &["base-parent"]),
            commit("branch-tip", &["shared"]),
            commit("base-parent", &["shared"]),
            commit("shared", &["root"]),
            commit("root", &[]),
        ];
        assert_eq!(
            displayed_branch_relation(&commits, "base-tip", "branch-tip"),
            DisplayedBranchRelation::Diverged {
                merge_base: "shared".into()
            }
        );

        let regular = vec![
            commit("base-tip", &["branch-tip"]),
            commit("branch-tip", &["root"]),
            commit("root", &[]),
        ];
        assert_eq!(
            displayed_branch_relation(&regular, "base-tip", "branch-tip"),
            DisplayedBranchRelation::RegularlyMerged
        );

        let truncated = vec![
            commit("base-tip", &["missing-base-parent"]),
            commit("branch-tip", &["missing-branch-parent"]),
        ];
        assert_eq!(
            displayed_branch_relation(&truncated, "base-tip", "branch-tip"),
            DisplayedBranchRelation::Ineligible
        );
    }
}
