use std::collections::HashMap;
use std::path::Path;
use std::process::Command;
use std::rc::Rc;

use chrono::{DateTime, TimeZone, Utc};

use super::model::{GraphCommit, GraphLine, GraphSource};
use super::refs::{assign_graph_branch_labels, collect_ref_data};
use super::state::{GraphLineStyle, GraphLoadOptions};
use super::GraphSnapshot;

pub(super) fn load_with_gleisbau(
    repo_path: &Path,
    options: GraphLoadOptions,
) -> Result<GraphSnapshot, String> {
    let ref_data = collect_ref_data(
        repo_path,
        options.include_remotes,
        options.base_branch.as_deref(),
    )?;
    let settings = gleisbau_settings(
        options.include_remotes,
        options.line_style,
        options.base_branch.as_deref(),
    )?;
    let repository =
        gleisbau::Repository::open(repo_path).map_err(|error| error.message().to_string())?;
    let mut graph = gleisbau::graph::Builder::new()
        .with_repository(repository)
        .with_settings(Rc::clone(&settings))
        .with_max_count(options.max_count)
        .build()?;

    // Gleisbau intentionally strips "origin/" before applying branch order
    // patterns, so ^main$ puts both main and origin/main in the same order
    // group. When those refs diverge, ShortestFirst can then place the remote
    // track in column zero. Keep the remote commits in the graph, but make
    // the matching remote base track non-matching for this layout pass.
    let remote_base_index = if options.include_remotes {
        options.base_branch.as_deref().and_then(|base| {
            let remote_name = format!("origin/{base}");
            graph
                .tracks
                .all_branches
                .iter()
                .position(|branch| branch.is_remote && branch.name == remote_name)
        })
    } else {
        None
    };
    let layout = if let Some(index) = remote_base_index {
        let original_name = graph.tracks.all_branches[index].name.clone();
        let layout_name = format!("__gbm_remote_base__/{original_name}");
        graph.tracks.all_branches[index].name = layout_name;
        let layout = gleisbau::layout::layout_track_range(
            &graph.tracks,
            0..graph.tracks.commits.len(),
            &settings,
        );
        graph.tracks.all_branches[index].name = original_name;
        layout?
    } else {
        graph.layout
    };

    let heights = vec![1; layout.commit_count()];
    let rendered = gleisbau::print::unicode::print_graph_terminal(
        &settings,
        &graph.tracks,
        &layout,
        &heights,
    );
    let mut line_to_commit = HashMap::new();
    for (relative_index, line_index) in rendered.commit2line.iter().copied().enumerate() {
        line_to_commit.insert(
            line_index,
            layout.commit_index_start() + relative_index,
        );
    }

    let mut commits = graph
        .tracks
        .commits
        .iter()
        .map(|info| {
            let oid = info.oid.to_string();
            let commit = graph
                .repository
                .find_commit(info.oid)
                .map_err(|error| error.message().to_string())?;
            let summary = commit
                .summary()
                .map_err(|error| error.message().to_string())?
                .unwrap_or_default()
                .to_string();
            let author = commit.author();
            let author_time = author.when();
            let authored_at = Utc.timestamp_opt(author_time.seconds(), 0).single();
            let lane = info
                .branch_trace
                .and_then(|trace| layout.track_visual(trace))
                .and_then(|visual| visual.column);

            Ok(GraphCommit {
                oid: oid.clone(),
                summary,
                parents: info.parents.iter().map(ToString::to_string).collect(),
                lane,
                branch: None,
                refs: ref_data.refs_by_oid.get(&oid).cloned().unwrap_or_default(),
                relationships: vec![],
                author_name: author.name().unwrap_or("").to_string(),
                author_email: author.email().unwrap_or("").to_string(),
                authored_at,
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

pub(super) fn load_with_git_cli(
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
        // NUL cannot occur in Git commit headers, while control bytes such as
        // unit separator are valid in author names and email addresses.
        "--format=%x1e%H%x00%P%x00%an%x00%ae%x00%aI%x00%s",
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
        let fields: Vec<_> = line[record_start + 1..].splitn(6, '\0').collect();
        if fields.len() != 6 || fields[0].is_empty() {
            return Err(format!("could not parse git log record: {line}"));
        }

        let oid = fields[0].to_string();
        let commit_index = commits.len();
        let lane = graph
            .chars()
            .position(|character| character == '*')
            .map(|index| index / 2);
        let authored_at = DateTime::parse_from_rfc3339(fields[4])
            .ok()
            .map(|dt| dt.with_timezone(&Utc));
        commits.push(GraphCommit {
            oid: oid.clone(),
            summary: fields[5].to_string(),
            parents: fields[1].split_whitespace().map(str::to_string).collect(),
            lane,
            branch: None,
            refs: ref_data.refs_by_oid.get(&oid).cloned().unwrap_or_default(),
            relationships: vec![],
            author_name: fields[2].to_string(),
            author_email: fields[3].to_string(),
            authored_at,
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

pub(super) fn gleisbau_settings(
    include_remotes: bool,
    line_style: GraphLineStyle,
    base_branch: Option<&str>,
) -> Result<Rc<gleisbau::settings::Settings>, String> {
    use gleisbau::print::format::CommitFormat;
    use gleisbau::settings::{
        BranchOrder, BranchSettings, BranchSettingsDef, Characters, MergePatterns, Settings,
    };

    let characters = match line_style {
        GraphLineStyle::Thin => Characters::thin(),
        GraphLineStyle::Round => Characters::round(),
    };

    let mut def = BranchSettingsDef::none();
    if let Some(name) = base_branch {
        let pattern = format!("^{}$", regex::escape(name));
        def.persistence.insert(0, pattern.clone());
        def.order.insert(0, pattern);
    }

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
        branches: BranchSettings::from(def).map_err(|error| error.to_string())?,
        merge_patterns: MergePatterns::default(),
    }))
}
