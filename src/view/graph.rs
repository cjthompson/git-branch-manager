use crate::git::graph::{GraphLoadError, GraphRefKind, GraphRelationship, GraphSnapshot};

/// The default number of commits shown by the Graph tab. Larger histories are
/// opt-in so opening the tab remains responsive on large repositories.
pub const GRAPH_PAGE_SIZE: usize = 500;

/// State owned exclusively by the Graph tab. The graph and ref columns are
/// rendered from one row stream, so they intentionally share one cursor and
/// one scroll offset.
#[derive(Debug)]
pub struct GraphState {
    snapshot: Option<GraphSnapshot>,
    loading: bool,
    error: Option<String>,
    max_count: usize,
    include_remotes: bool,
    cursor: usize,
    offset: usize,
    horizontal_offset: usize,
    pending_target: Option<String>,
}

impl Default for GraphState {
    fn default() -> Self {
        Self::new()
    }
}

impl GraphState {
    pub fn new() -> Self {
        Self {
            snapshot: None,
            loading: false,
            error: None,
            max_count: GRAPH_PAGE_SIZE,
            include_remotes: false,
            cursor: 0,
            offset: 0,
            horizontal_offset: 0,
            pending_target: None,
        }
    }

    pub fn snapshot(&self) -> Option<&GraphSnapshot> {
        self.snapshot.as_ref()
    }

    pub fn is_loading(&self) -> bool {
        self.loading
    }

    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    pub fn max_count(&self) -> usize {
        self.max_count
    }

    pub fn includes_remotes(&self) -> bool {
        self.include_remotes
    }

    pub fn commit_offset(&self) -> usize {
        self.offset
    }

    pub fn commit_cursor(&self) -> usize {
        self.cursor
    }

    pub fn horizontal_offset(&self) -> usize {
        self.horizontal_offset
    }

    pub fn scroll_left(&mut self) {
        self.horizontal_offset = self.horizontal_offset.saturating_sub(1);
    }

    pub fn scroll_right(&mut self) {
        self.horizontal_offset = self.horizontal_offset.saturating_add(1);
    }

    pub fn clamp_horizontal_offset(&mut self, max_offset: usize) {
        self.horizontal_offset = self.horizontal_offset.min(max_offset);
    }

    pub fn begin_load(&mut self, max_count: usize, include_remotes: bool) {
        self.max_count = max_count.max(1);
        self.include_remotes = include_remotes;
        self.loading = true;
        self.error = None;
    }

    pub fn apply_result(&mut self, result: Result<GraphSnapshot, GraphLoadError>) {
        self.loading = false;
        let selected_oid = self.selected_commit().map(|commit| commit.oid.clone());
        let pending_target = self.pending_target.take();
        match result {
            Ok(snapshot) => {
                let target_cursor = pending_target
                    .as_deref()
                    .and_then(|target| cursor_for_target(&snapshot, target));
                let cursor = target_cursor
                    .or_else(|| {
                        selected_oid.as_deref().and_then(|oid| {
                            snapshot
                                .lines
                                .iter()
                                .filter_map(|line| line.commit_index)
                                .position(|commit_index| {
                                    snapshot
                                        .commits
                                        .get(commit_index)
                                        .is_some_and(|commit| commit.oid == oid)
                                })
                            })
                    })
                    .unwrap_or(0);
                self.max_count = snapshot.max_count;
                self.include_remotes = snapshot.includes_remotes;
                self.snapshot = Some(snapshot);
                self.error = None;
                self.cursor = cursor;
                self.offset = 0;
                self.horizontal_offset = 0;
                if target_cursor.is_none() {
                    self.pending_target = pending_target;
                }
            }
            Err(error) => {
                self.snapshot = None;
                self.error = Some(error.to_string());
                self.cursor = 0;
                self.offset = 0;
                self.horizontal_offset = 0;
                self.pending_target = pending_target;
            }
        }
    }

    /// Atomically replace a loaded snapshot produced by the incremental
    /// updater while retaining the user's viewport and selected commit.
    pub fn apply_incremental_result(&mut self, result: Result<GraphSnapshot, GraphLoadError>) {
        let selected_oid = self.selected_commit().map(|commit| commit.oid.clone());
        let pending_target = self.pending_target.clone();
        let old_offset = self.offset;
        let old_horizontal = self.horizontal_offset;
        match result {
            Ok(snapshot) => {
                let target_cursor = pending_target.as_deref().and_then(|target| cursor_for_target(&snapshot, target));
                let cursor = target_cursor.or_else(|| selected_oid.as_deref().and_then(|oid| {
                    snapshot.lines.iter().filter_map(|line| line.commit_index).position(|index| {
                        snapshot.commits.get(index).is_some_and(|commit| commit.oid == oid)
                    })
                })).unwrap_or_else(|| self.cursor.min(snapshot.commits.len().saturating_sub(1)));
                self.max_count = snapshot.max_count;
                self.include_remotes = snapshot.includes_remotes;
                self.snapshot = Some(snapshot);
                self.error = None;
                self.loading = false;
                self.cursor = cursor;
                self.offset = old_offset.min(cursor);
                self.horizontal_offset = old_horizontal;
                if target_cursor.is_some() { self.pending_target = None; }
            }
            Err(error) => {
                self.error = Some(error.to_string());
                self.loading = false;
            }
        }
    }

    /// Focus the commit identified by a live ref name or a full/abbreviated
    /// commit OID. If the snapshot is still loading, retain the target and
    /// apply it when the next structural snapshot arrives.
    pub fn focus_target(&mut self, target: impl Into<String>) -> bool {
        let target = target.into();
        let cursor = self
            .snapshot
            .as_ref()
            .and_then(|snapshot| cursor_for_target(snapshot, &target));
        if let Some(cursor) = cursor {
            self.cursor = cursor;
            self.pending_target = None;
            true
        } else {
            self.pending_target = Some(target);
            false
        }
    }

    pub fn set_include_remotes(&mut self, include_remotes: bool) {
        self.include_remotes = include_remotes;
    }

    /// Apply asynchronous squash-merge enrichment to the active snapshot.
    /// Returns true if the enrichment actually changed the snapshot. The
    /// caller is expected to have already verified the enrichment belongs
    /// to the current reload generation; this method does not compare
    /// `snapshot.generation` against any App-side epoch.
    pub fn apply_squash_enrichment(&mut self, updates: &[GraphRelationship]) -> bool {
        let Some(snapshot) = self.snapshot.as_mut() else {
            return false;
        };
        let before = snapshot.clone();
        crate::git::graph::apply_squash_enrichment(snapshot, updates);
        *snapshot != before
    }

    /// Surgically patch the snapshot by dropping refs with the given names.
    /// Used after ref-only actions like DeleteLocal, DeleteRemote, DeleteTag,
    /// etc., where the OID topology is unchanged but the ref set must be
    /// edited in place. App owns generation and freshness revisions.
    ///
    /// For each ref_name, scans the snapshot's commits to find the one that
    /// holds that ref, removes it, and decrements the corresponding ref
    /// counter. Stale branch labels on affected commits are cleared.
    ///
    /// Returns true if any ref was actually dropped (i.e., the snapshot was
    /// modified). Returns false if there is no snapshot loaded or if none of
    /// the requested refs were present.
    pub fn apply_ref_changes(&mut self, refs: &[(GraphRefKind, String)]) -> bool {
        let Some(snapshot) = self.snapshot.as_mut() else {
            return false;
        };
        if refs.is_empty() {
            return false;
        }

        let mut changed = false;
        let mut removed = Vec::new();
        for (kind, ref_name) in refs {
            for commit in &mut snapshot.commits {
                let old_len = commit.refs.len();
                commit.refs.retain(|reference| reference.kind != *kind || reference.name != *ref_name);
                if commit.refs.len() != old_len {
                    changed = true;
                    removed.push((*kind, ref_name.as_str()));
                    match kind {
                        GraphRefKind::LocalBranch => snapshot.ref_counts.local = snapshot.ref_counts.local.saturating_sub(1),
                        GraphRefKind::RemoteBranch => snapshot.ref_counts.remote = snapshot.ref_counts.remote.saturating_sub(1),
                        GraphRefKind::Tag => {}
                    }
                }
            }
        }
        for commit in &mut snapshot.commits {
            if commit.branch.as_ref().is_some_and(|label| removed.iter().any(|(kind, name)| label.kind == *kind && label.name == *name)) {
                commit.branch = None;
                changed = true;
            }
        }

        changed
    }

    /// Compatibility helper for callers that have a unique ref name. New
    /// mutation code should use `apply_ref_changes` with canonical identities.
    pub fn apply_ref_delta(&mut self, ref_names: Vec<String>) -> bool {
        let refs: Vec<_> = ref_names.into_iter().filter_map(|name| {
            let matches: Vec<_> = self.snapshot.as_ref()?.commits.iter().flat_map(|commit| &commit.refs)
                .filter(|reference| reference.name == name).collect();
            (matches.len() == 1).then(|| (matches[0].kind, name))
        }).collect();
        self.apply_ref_changes(&refs)
    }

    /// Apply repository-observed ref and HEAD metadata to a loaded snapshot.
    /// Topology is deliberately left untouched here; the caller can distinguish
    /// commits that require the structural updater by checking whether each
    /// resulting ref tip exists in the cached window.
    pub fn apply_repository_delta(&mut self, delta: &crate::git::graph::GraphRepositoryDelta) -> bool {
        let Some(snapshot) = self.snapshot.as_mut() else { return false; };
        let kind_name = |id: &crate::git::graph::GraphRefId| {
            let prefix = match id.kind {
                GraphRefKind::LocalBranch => "refs/heads/",
                GraphRefKind::RemoteBranch => "refs/remotes/",
                GraphRefKind::Tag => "refs/tags/",
            };
            id.full_name.strip_prefix(prefix).unwrap_or(&id.full_name).to_string()
        };
        let mut changed = false;
        for (id, _) in &delta.removed_refs {
            let name = kind_name(id);
            for commit in &mut snapshot.commits {
                let before = commit.refs.len();
                commit.refs.retain(|reference| reference.kind != id.kind || reference.name != name);
                changed |= before != commit.refs.len();
                if commit.branch.as_ref().is_some_and(|branch| branch.kind == id.kind && branch.name == name) {
                    commit.branch = None;
                    changed = true;
                }
            }
        }
        let changed_tips: Vec<_> = delta.moved_refs.iter().map(|(id, _, new)| (id.clone(), new.clone()))
            .chain(delta.added_refs.iter().cloned()).collect();
        for (id, new_oid) in changed_tips {
            let name = kind_name(&id);
            for commit in &mut snapshot.commits {
                let before = commit.refs.len();
                commit.refs.retain(|reference| reference.kind != id.kind || reference.name != name);
                changed |= before != commit.refs.len();
            }
            if let Some(commit) = snapshot.commits.iter_mut().find(|commit| commit.oid == new_oid) {
                commit.refs.push(crate::git::graph::GraphRef {
                    name,
                    kind: id.kind,
                    has_linked_worktree: delta.after.worktrees.values().any(|branch| branch.as_deref() == Some(id.full_name.strip_prefix("refs/heads/").unwrap_or(""))),
                    is_current: delta.after.head_ref.as_deref() == Some(id.full_name.as_str()),
                    tracking: None,
                });
                changed = true;
            }
        }
        if delta.head_changed {
            for commit in &mut snapshot.commits {
                for reference in &mut commit.refs {
                    if reference.kind == GraphRefKind::LocalBranch {
                        let full_name = format!("refs/heads/{}", reference.name);
                        let current = delta.after.head_ref.as_deref() == Some(full_name.as_str());
                        changed |= reference.is_current != current;
                        reference.is_current = current;
                    }
                }
            }
        }
        snapshot.ref_counts.local = delta.after.refs.keys().filter(|id| id.kind == GraphRefKind::LocalBranch).count();
        snapshot.ref_counts.remote = delta.after.refs.keys().filter(|id| id.kind == GraphRefKind::RemoteBranch).count();
        changed
    }

    pub fn apply_metadata_delta(&mut self, updates: &[(GraphRefKind, String, Option<bool>, Option<bool>)]) -> bool {
        let Some(snapshot) = self.snapshot.as_mut() else { return false; };
        let mut changed = false;
        for (kind, name, current, linked) in updates {
            for commit in &mut snapshot.commits {
                for reference in &mut commit.refs {
                    if reference.kind == *kind && (name.is_empty() || reference.name == *name) {
                        if let Some(value) = current { changed |= reference.is_current != *value; reference.is_current = *value; }
                        if let Some(value) = linked { changed |= reference.has_linked_worktree != *value; reference.has_linked_worktree = *value; }
                    }
                }
            }
        }
        changed
    }

    /// Increase the history window by one page and return the new limit.
    pub fn load_older_history(&mut self) -> usize {
        self.max_count = self.max_count.saturating_add(GRAPH_PAGE_SIZE);
        self.max_count
    }

    pub fn move_up(&mut self) {
        self.cursor = self.cursor.saturating_sub(1);
    }

    pub fn move_down(&mut self) {
        let count = self.commit_count();
        if count > 0 {
            self.cursor = (self.cursor + 1).min(count - 1);
        }
    }

    pub fn page_up(&mut self) {
        self.cursor = self.cursor.saturating_sub(20);
    }

    pub fn page_down(&mut self) {
        let count = self.commit_count();
        self.cursor = (self.cursor + 20).min(count.saturating_sub(1));
    }

    pub fn home(&mut self) {
        self.cursor = 0;
    }

    pub fn end(&mut self) {
        self.cursor = self.commit_count().saturating_sub(1);
    }

    pub fn commit_count(&self) -> usize {
        self.snapshot
            .as_ref()
            .map(|snapshot| {
                snapshot
                    .lines
                    .iter()
                    .filter(|line| line.commit_index.is_some())
                    .count()
            })
            .unwrap_or(0)
    }

    pub fn selected_commit_line(&self) -> Option<usize> {
        self.snapshot.as_ref().and_then(|snapshot| {
            snapshot
                .lines
                .iter()
                .enumerate()
                .filter(|(_, line)| line.commit_index.is_some())
                .nth(self.cursor)
                .map(|(line_index, _)| line_index)
        })
    }

    pub fn selected_commit(&self) -> Option<&crate::git::graph::GraphCommit> {
        let commit_index = self
            .selected_commit_line()
            .and_then(|line_index| self.snapshot.as_ref()?.lines[line_index].commit_index)?;
        self.snapshot.as_ref()?.commits.get(commit_index)
    }

    /// Keep the active row inside the shared graph/ref viewport.
    pub fn ensure_visible(&mut self, rows: usize) {
        if let Some(line_index) = self.selected_commit_line() {
            if rows > 0 {
                if line_index < self.offset {
                    self.offset = line_index;
                } else if line_index >= self.offset + rows {
                    self.offset = line_index + 1 - rows;
                }
            }
        }
    }
}

fn cursor_for_target(snapshot: &GraphSnapshot, target: &str) -> Option<usize> {
    if target.is_empty() {
        return None;
    }

    snapshot
        .lines
        .iter()
        .filter_map(|line| line.commit_index)
        .enumerate()
        .find_map(|(cursor, commit_index)| {
            let commit = snapshot.commits.get(commit_index)?;
            let is_ref = commit.refs.iter().any(|reference| reference.name == target);
            let is_oid = commit.oid == target || commit.oid.starts_with(target);
            (is_ref || is_oid).then_some(cursor)
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::graph::{GraphCommit, GraphLine, GraphRef, GraphRefKind};

    fn snapshot() -> GraphSnapshot {
        GraphSnapshot {
            source: crate::git::graph::GraphSource::Gleisbau,
            commits: vec![
                GraphCommit {
                    oid: "1111111111111111111111111111111111111111".into(),
                    summary: "first".into(),
                    parents: vec![],
                    lane: Some(0),
                    branch: None,
                    refs: vec![GraphRef {
                        name: "main".into(),
                        kind: GraphRefKind::LocalBranch,
                        has_linked_worktree: false,
                        is_current: false,
                        tracking: None,
                    }],
                    relationships: vec![],
                    ..GraphCommit::default()
                },
                GraphCommit {
                    oid: "2222222222222222222222222222222222222222".into(),
                    summary: "second".into(),
                    parents: vec![],
                    lane: Some(0),
                    branch: None,
                    refs: vec![],
                    relationships: vec![],
                    ..GraphCommit::default()
                },
            ],
            lines: vec![
                GraphLine {
                    graph: "*".into(),
                    commit_index: Some(0),
                },
                GraphLine {
                    graph: "|".into(),
                    commit_index: None,
                },
                GraphLine {
                    graph: "*".into(),
                    commit_index: Some(1),
                },
            ],
            ref_counts: Default::default(),
            max_count: 500,
            includes_remotes: false,
            generation: None,
        }
    }

    #[test]
    fn graph_navigation_uses_one_row_cursor() {
        let mut state = GraphState::new();
        state.apply_result(Ok(snapshot()));
        state.move_down();
        assert_eq!(state.commit_cursor(), 1);
    }

    #[test]
    fn graph_options_and_older_history_update_state() {
        let mut state = GraphState::new();
        assert_eq!(state.max_count(), 500);
        assert_eq!(state.load_older_history(), 1000);
        state.set_include_remotes(true);
        assert!(state.includes_remotes());
        state.begin_load(state.max_count(), state.includes_remotes());
        assert!(state.is_loading());
    }

    #[test]
    fn graph_and_ref_columns_share_one_scroll_offset() {
        let mut state = GraphState::new();
        let mut snapshot = snapshot();
        snapshot.lines = (0..6)
            .map(|index| GraphLine {
                graph: "*".into(),
                commit_index: Some(index % 2),
            })
            .collect();
        state.apply_result(Ok(snapshot));

        for _ in 0..4 {
            state.move_down();
        }
        state.ensure_visible(3);

        assert_eq!(state.commit_offset(), 2);
    }

    #[test]
    fn graph_horizontal_scroll_is_independent_and_resets_on_result() {
        let mut state = GraphState::new();
        state.apply_result(Ok(snapshot()));
        state.move_down();
        state.scroll_right();
        state.scroll_right();

        assert_eq!(state.commit_cursor(), 1);
        assert_eq!(state.horizontal_offset(), 2);

        state.scroll_left();
        state.scroll_left();
        state.scroll_left();
        assert_eq!(state.horizontal_offset(), 0);

        state.scroll_right();
        state.apply_result(Ok(snapshot()));
        assert_eq!(state.horizontal_offset(), 0);

        state.scroll_right();
        state.apply_result(Err(GraphLoadError::Both {
            gleisbau: "failed".into(),
            fallback: "failed".into(),
        }));
        assert_eq!(state.horizontal_offset(), 0);
    }

    #[test]
    fn graph_reload_retains_selected_commit_when_it_is_still_present() {
        let mut state = GraphState::new();
        state.apply_result(Ok(snapshot()));
        state.move_down();
        assert_eq!(state.selected_commit().unwrap().summary, "second");

        state.begin_load(500, false);
        state.apply_result(Ok(snapshot()));

        assert_eq!(state.commit_cursor(), 1);
        assert_eq!(state.selected_commit().unwrap().summary, "second");
    }

    #[test]
    fn graph_focus_target_selects_refs_and_abbreviated_oids() {
        let mut state = GraphState::new();
        state.apply_result(Ok(snapshot()));

        assert!(state.focus_target("main"));
        assert_eq!(state.commit_cursor(), 0);
        assert!(state.focus_target("2222222"));
        assert_eq!(state.commit_cursor(), 1);
    }

    #[test]
    fn graph_focus_target_waits_for_a_later_snapshot_when_loading() {
        let mut state = GraphState::new();
        state.begin_load(500, false);
        assert!(!state.focus_target("feature/new"));

        let mut loaded = snapshot();
        loaded.commits[1].refs.push(GraphRef {
            name: "feature/new".into(),
            kind: GraphRefKind::LocalBranch,
            has_linked_worktree: false,
            is_current: false,
            tracking: None,
        });
        state.apply_result(Ok(loaded));

        assert_eq!(state.commit_cursor(), 1);
    }

    #[test]
    fn graph_apply_ref_delta_removes_single_ref_cleanly() {
        let mut state = GraphState::new();
        let mut snap = snapshot();
        // Give the first commit a "feature" ref so we can drop it.
        snap.commits[0].refs.push(GraphRef {
            name: "feature/x".into(),
            kind: GraphRefKind::LocalBranch,
            has_linked_worktree: false,
            is_current: false,
            tracking: None,
        });
        snap.ref_counts.local = 2;
        snap.generation = Some(1);
        state.apply_result(Ok(snap));

        let changed = state.apply_ref_delta(vec!["feature/x".to_string()]);

        assert!(changed, "apply_ref_delta should report a change");

        let snap_after = state.snapshot().unwrap();
        let commit = &snap_after.commits[0];
        assert!(
            !commit.refs.iter().any(|r| r.name == "feature/x"),
            "feature/x should be removed"
        );
        assert_eq!(snap_after.ref_counts.local, 1, "local ref count should decrement");
    }

    #[test]
    fn graph_apply_ref_delta_leaves_generation_ownership_to_app() {
        let mut state = GraphState::new();
        let mut snap = snapshot();
        snap.commits[0].refs.push(GraphRef {
            name: "feature/y".into(),
            kind: GraphRefKind::LocalBranch,
            has_linked_worktree: false,
            is_current: false,
            tracking: None,
        });
        snap.generation = Some(1);
        state.apply_result(Ok(snap));

        let before_gen = state
            .snapshot()
            .unwrap()
            .generation
            .expect("snapshot should have a generation after apply_result");

        state.apply_ref_delta(vec!["feature/y".to_string()]);

        let after_gen = state.snapshot().unwrap().generation.unwrap();
        assert_eq!(after_gen, before_gen, "GraphState must not own App revisions");
    }

    #[test]
    fn graph_apply_ref_delta_returns_false_when_no_snapshot() {
        let mut state = GraphState::new();
        let changed =
            state.apply_ref_delta(vec!["anything".to_string()]);
        assert!(!changed, "no snapshot loaded → no change");
    }

    #[test]
    fn graph_apply_ref_delta_returns_false_when_ref_missing() {
        let mut state = GraphState::new();
        state.apply_result(Ok(snapshot()));

        let changed =
            state.apply_ref_delta(vec!["does/not/exist".to_string()]);
        assert!(!changed, "missing ref → no change");
    }
}
