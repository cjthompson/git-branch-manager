use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::rc::Rc;
use std::sync::mpsc::{self, Receiver};

use crate::types::MergeStatus;
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
    /// The live branch whose visual track owns this commit. This is derived
    /// from the graph layout, not from generic reachability, so a merged side
    /// branch does not get mislabeled as the base branch.
    pub branch: Option<GraphBranchLabel>,
    pub refs: Vec<GraphRef>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphBranchLabel {
    pub name: String,
    pub target_oid: String,
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
    pub status: Option<GraphRefStatus>,
    pub tracking: Option<GraphRefTracking>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphRefStatus {
    pub merge_status: MergeStatus,
    pub ahead: Option<u32>,
    pub behind: Option<u32>,
    pub is_current: bool,
    pub is_base: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphRefTracking {
    pub remote_name: String,
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
        Ok(Err(cause)) => {
            load_with_git_cli(repo_path, options, cause.clone()).map_err(|fallback| {
                GraphLoadError::Both {
                    gleisbau: cause,
                    fallback,
                }
            })
        }
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
    let mut local_branch_labels = ref_data
        .local_branch_labels
        .values()
        .cloned()
        .collect::<Vec<_>>();
    local_branch_labels.sort_by(|left, right| left.name.cmp(&right.name));
    let mut branch_labels_by_trace = HashMap::new();
    for label in local_branch_labels {
        let Some(trace) = graph
            .tracks
            .commits
            .iter()
            .find(|info| info.oid.to_string() == label.target_oid)
            .and_then(|info| info.branch_trace)
        else {
            continue;
        };
        branch_labels_by_trace.entry(trace).or_insert(label);
    }

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

            let branch = info
                .branch_trace
                .and_then(|trace| branch_labels_by_trace.get(&trace).cloned());

            Ok(GraphCommit {
                oid: oid.clone(),
                summary,
                parents: info.parents.iter().map(ToString::to_string).collect(),
                lane,
                branch,
                refs: ref_data.refs_by_oid.get(&oid).cloned().unwrap_or_default(),
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    assign_graph_branch_labels(&mut commits, &ref_data.branch_labels);
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
        });
        lines.push(GraphLine {
            graph,
            commit_index: Some(commit_index),
        });
    }
    assign_graph_branch_labels(&mut commits, &ref_data.branch_labels);
    Ok(GraphSnapshot {
        source: GraphSource::GitCliFallback { cause },
        commits,
        lines,
        ref_counts: ref_data.ref_counts,
        max_count: options.max_count,
        includes_remotes: options.include_remotes,
    })
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
    local_branch_labels: HashMap<String, GraphBranchLabel>,
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
    let local_statuses = base_branch
        .as_deref()
        .and_then(|base| crate::git::branch::list_branches(&repository, base).ok())
        .unwrap_or_default()
        .into_iter()
        .map(|branch| (branch.name.clone(), branch))
        .collect::<HashMap<_, _>>();
    let remote_statuses = if include_remotes {
        collect_remote_statuses(repo_path, &repository, base_branch.as_deref())
    } else {
        HashMap::new()
    };
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
        };
        data.branch_labels.insert(name.clone(), label.clone());
        data.local_branch_labels.insert(name.clone(), label);
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
                },
            );
        }
    }

    for (name, target_oid) in &local_refs {
        let status = local_statuses.get(name).map(graph_local_status);
        let tracking = remote_refs
            .iter()
            .filter(|(remote_name, _)| remote_short_name(remote_name) == name)
            .find_map(|(remote_name, remote_oid)| {
                let (ahead, behind) = repository
                    .graph_ahead_behind(
                        git2::Oid::from_str(target_oid).ok()?,
                        git2::Oid::from_str(remote_oid).ok()?,
                    )
                    .ok()?;
                Some(GraphRefTracking {
                    remote_name: remote_name.clone(),
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
                status,
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
                    status: remote_statuses.get(name).cloned(),
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
                status: None,
                tracking: None,
            },
        );
    }

    Ok(data)
}

fn graph_local_status(branch: &crate::types::BranchInfo) -> GraphRefStatus {
    GraphRefStatus {
        merge_status: branch.merge_status,
        ahead: branch.ahead,
        behind: branch.behind,
        is_current: branch.is_current,
        is_base: branch.is_base,
    }
}

fn collect_remote_statuses(
    repo_path: &Path,
    repository: &git2::Repository,
    base_branch: Option<&str>,
) -> HashMap<String, GraphRefStatus> {
    let Some(base_branch) = base_branch else {
        return HashMap::new();
    };
    let remotes = crate::git::branch::list_remote_branches_phase1(repository, base_branch)
        .unwrap_or_default();
    let mut statuses = remotes
        .iter()
        .map(|remote| {
            (
                remote.full_ref.clone(),
                GraphRefStatus {
                    merge_status: remote.merge_status,
                    ahead: remote.ahead,
                    behind: remote.behind,
                    is_current: false,
                    is_base: remote.is_base,
                },
            )
        })
        .collect::<HashMap<_, _>>();
    let receiver = crate::git::branch::spawn_remote_enricher(
        repo_path.to_path_buf(),
        base_branch.to_string(),
        remotes,
    );
    while let Ok(result) = receiver.recv() {
        if let Some(status) = statuses.get_mut(&result.full_ref) {
            status.merge_status = result.merge_status;
            status.ahead = result.ahead;
            status.behind = result.behind;
        }
    }
    statuses
}

fn remote_short_name(name: &str) -> &str {
    name.split_once('/').map(|(_, short)| short).unwrap_or(name)
}

fn assign_graph_branch_labels(
    commits: &mut [GraphCommit],
    branch_labels: &HashMap<String, GraphBranchLabel>,
) {
    assign_first_parent_branch_labels(commits, branch_labels);
    assign_merge_subject_branch_labels(commits);
}

fn assign_first_parent_branch_labels(
    commits: &mut [GraphCommit],
    branch_labels: &HashMap<String, GraphBranchLabel>,
) {
    let index_by_oid = commits
        .iter()
        .enumerate()
        .map(|(index, commit)| (commit.oid.clone(), index))
        .collect::<HashMap<_, _>>();
    let mut labels = branch_labels.values().cloned().collect::<Vec<_>>();
    labels.sort_by(|left, right| left.name.cmp(&right.name));
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
}
