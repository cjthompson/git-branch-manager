use std::collections::HashMap;
use std::path::Path;

use thiserror::Error;

use crate::git::cache::CacheRoot;

use super::model::{GraphRefId, GraphRefKind};

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct GraphRepositoryState {
    pub refs: HashMap<GraphRefId, String>,
    /// Non-branch/tag history roots (for example refs/stash and custom refs).
    /// Full remote-inclusive loaders use these as traversal roots too.
    pub additional_roots: HashMap<String, String>,
    pub head_ref: Option<String>,
    pub head_oid: Option<String>,
    pub worktrees: HashMap<String, Option<String>>,
    pub tracking: HashMap<String, (String, Option<u32>, Option<u32>)>,
}

impl GraphRepositoryState {
    pub fn capture(repo_path: &Path) -> Result<Self, String> {
        Self::capture_with_head(repo_path, repo_path)
    }

    pub fn capture_with_head(repo_path: &Path, head_repo_path: &Path) -> Result<Self, String> {
        let repo = git2::Repository::open(repo_path).map_err(|error| error.message().to_string())?;
        let head_repo = git2::Repository::open(head_repo_path).map_err(|error| error.message().to_string())?;
        let mut state = Self::default();
        let references = repo.references().map_err(|error| error.message().to_string())?;
        for reference in references {
            let reference = reference.map_err(|error| error.message().to_string())?;
            let Ok(full_name) = reference.name().map(str::to_string) else { continue };
            let kind = if full_name.starts_with("refs/heads/") {
                GraphRefKind::LocalBranch
            } else if full_name.starts_with("refs/remotes/") {
                GraphRefKind::RemoteBranch
            } else if full_name.starts_with("refs/tags/") {
                GraphRefKind::Tag
            } else {
                if let Ok(commit) = reference.peel_to_commit() {
                    state.additional_roots.insert(full_name, commit.id().to_string());
                }
                continue
            };
            let oid = reference.peel_to_commit().map(|commit| commit.id().to_string()).ok();
            if let Some(oid) = oid {
                state.refs.insert(GraphRefId { kind, full_name }, oid);
            }
        }
        for branch in repo.branches(Some(git2::BranchType::Local)).map_err(|error| error.message().to_string())? {
            let (branch, _) = branch.map_err(|error| error.message().to_string())?;
            let Some(name) = branch.name().map_err(|error| error.message().to_string())?.map(str::to_string) else { continue };
            let Ok(upstream) = branch.upstream() else { continue };
            let Some(upstream_name) = upstream.name().ok().flatten().map(str::to_string) else { continue };
            let tracking = match (branch.get().target(), upstream.get().target()) {
                (Some(local), Some(remote)) => repo.graph_ahead_behind(local, remote).ok().map(|(ahead, behind)| (Some(ahead.try_into().unwrap_or(u32::MAX)), Some(behind.try_into().unwrap_or(u32::MAX)))),
                _ => None,
            }.unwrap_or((None, None));
            state.tracking.insert(name, (upstream_name, tracking.0, tracking.1));
        }
        if let Ok(head) = head_repo.head() {
            state.head_ref = head.name().ok().map(str::to_string);
            state.head_oid = head.peel_to_commit().ok().map(|commit| commit.id().to_string());
        }
        for worktree in crate::git::worktree::try_list_worktrees(repo_path)? {
            let path = std::fs::canonicalize(&worktree.path).unwrap_or(worktree.path);
            state.worktrees.insert(path.to_string_lossy().into_owned(), worktree.branch);
        }
        Ok(state)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphRepositoryDelta {
    pub before: GraphRepositoryState,
    pub after: GraphRepositoryState,
    pub added_refs: Vec<(GraphRefId, String)>,
    pub removed_refs: Vec<(GraphRefId, String)>,
    pub moved_refs: Vec<(GraphRefId, String, String)>,
    pub head_changed: bool,
    pub worktrees_changed: bool,
    pub tracking_changed: bool,
    pub additional_roots_changed: bool,
}

impl GraphRepositoryDelta {
    pub fn between(before: GraphRepositoryState, after: GraphRepositoryState) -> Self {
        let mut added_refs = Vec::new();
        let mut removed_refs = Vec::new();
        let mut moved_refs = Vec::new();
        for (id, old_oid) in &before.refs {
            match after.refs.get(id) {
                None => removed_refs.push((id.clone(), old_oid.clone())),
                Some(new_oid) if new_oid != old_oid => moved_refs.push((id.clone(), old_oid.clone(), new_oid.clone())),
                _ => {}
            }
        }
        for (id, oid) in &after.refs {
            if !before.refs.contains_key(id) { added_refs.push((id.clone(), oid.clone())); }
        }
        Self {
            head_changed: before.head_ref != after.head_ref || before.head_oid != after.head_oid,
            worktrees_changed: before.worktrees != after.worktrees,
            tracking_changed: before.tracking != after.tracking,
            additional_roots_changed: before.additional_roots != after.additional_roots,
            before,
            after,
            added_refs,
            removed_refs,
            moved_refs,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphLoadOptions {
    pub max_count: usize,
    pub include_remotes: bool,
    pub line_style: GraphLineStyle,
    pub base_branch: Option<String>,
    pub cache_root: CacheRoot,
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
            cache_root: CacheRoot::from_env(),
        }
    }
}

#[derive(Debug, Error)]
pub enum GraphLoadError {
    #[error("Gleisbau failed ({gleisbau}) and the git CLI fallback failed ({fallback})")]
    Both { gleisbau: String, fallback: String },
}
