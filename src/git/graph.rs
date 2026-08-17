use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::rc::Rc;
use std::sync::mpsc::{self, Receiver};

use thiserror::Error;

/// An app-owned graph payload. It deliberately contains no repository handles
/// or Gleisbau values, so it can move across a background channel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphSnapshot {
    pub source: GraphSource,
    pub commits: Vec<GraphCommit>,
    pub lines: Vec<GraphLine>,
    pub sidebar: GraphSidebar,
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
    pub refs: Vec<GraphRef>,
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
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GraphRefKind {
    LocalBranch,
    RemoteBranch,
    Tag,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct GraphSidebar {
    pub local_branches: Vec<GraphSidebarRef>,
    pub remote_branches: Vec<GraphSidebarRef>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphSidebarRef {
    pub name: String,
    pub target_oid: String,
    pub lane: Option<usize>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GraphLoadOptions {
    pub max_count: usize,
    pub include_remotes: bool,
    pub line_style: GraphLineStyle,
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
        load_with_gleisbau(repo_path, options)
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
    let ref_data = collect_ref_data(repo_path, options.include_remotes)?;
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

    let commits = graph
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
                refs: ref_data.refs_by_oid.get(&oid).cloned().unwrap_or_default(),
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    let lanes_by_oid = commits
        .iter()
        .map(|commit| (commit.oid.clone(), commit.lane))
        .collect();

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
        sidebar: ref_data.sidebar.with_lanes(&lanes_by_oid),
        max_count: options.max_count,
        includes_remotes: options.include_remotes,
    })
}

fn load_with_git_cli(
    repo_path: &Path,
    options: GraphLoadOptions,
    cause: String,
) -> Result<GraphSnapshot, String> {
    let ref_data = collect_ref_data(repo_path, options.include_remotes)?;
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
            refs: ref_data.refs_by_oid.get(&oid).cloned().unwrap_or_default(),
        });
        lines.push(GraphLine {
            graph,
            commit_index: Some(commit_index),
        });
    }
    let lanes_by_oid = commits
        .iter()
        .map(|commit| (commit.oid.clone(), commit.lane))
        .collect();

    Ok(GraphSnapshot {
        source: GraphSource::GitCliFallback { cause },
        commits,
        lines,
        sidebar: ref_data.sidebar.with_lanes(&lanes_by_oid),
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
    sidebar: GraphSidebar,
}

fn collect_ref_data(repo_path: &Path, include_remotes: bool) -> Result<RefData, String> {
    let repository =
        git2::Repository::open(repo_path).map_err(|error| error.message().to_string())?;
    let mut data = RefData::default();

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
        insert_ref(
            &mut data.refs_by_oid,
            &target_oid,
            GraphRef {
                name: name.clone(),
                kind: GraphRefKind::LocalBranch,
            },
        );
        data.sidebar.local_branches.push(GraphSidebarRef {
            name,
            target_oid,
            lane: None,
        });
    }

    if include_remotes {
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
            insert_ref(
                &mut data.refs_by_oid,
                &target_oid,
                GraphRef {
                    name: name.clone(),
                    kind: GraphRefKind::RemoteBranch,
                },
            );
            data.sidebar.remote_branches.push(GraphSidebarRef {
                name,
                target_oid,
                lane: None,
            });
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
            },
        );
    }

    data.sidebar
        .local_branches
        .sort_by(|left, right| left.name.cmp(&right.name));
    data.sidebar
        .remote_branches
        .sort_by(|left, right| left.name.cmp(&right.name));
    Ok(data)
}

fn insert_ref(refs_by_oid: &mut HashMap<String, Vec<GraphRef>>, oid: &str, reference: GraphRef) {
    refs_by_oid
        .entry(oid.to_string())
        .or_default()
        .push(reference);
}

impl GraphSidebar {
    fn with_lanes(mut self, lanes_by_oid: &HashMap<String, Option<usize>>) -> Self {
        for branch in self
            .local_branches
            .iter_mut()
            .chain(self.remote_branches.iter_mut())
        {
            branch.lane = lanes_by_oid.get(&branch.target_oid).copied().flatten();
        }
        self
    }
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
