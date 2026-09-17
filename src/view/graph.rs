use crate::git::graph::{GraphEnrichmentUpdate, GraphLoadError, GraphSnapshot};

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
        match result {
            Ok(snapshot) => {
                let cursor = selected_oid
                    .as_deref()
                    .and_then(|oid| {
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
                    .unwrap_or(0);
                self.max_count = snapshot.max_count;
                self.include_remotes = snapshot.includes_remotes;
                self.snapshot = Some(snapshot);
                self.error = None;
                self.cursor = cursor;
                self.offset = 0;
                self.horizontal_offset = 0;
            }
            Err(error) => {
                self.snapshot = None;
                self.error = Some(error.to_string());
                self.cursor = 0;
                self.offset = 0;
                self.horizontal_offset = 0;
            }
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
    pub fn apply_squash_enrichment(&mut self, updates: &[GraphEnrichmentUpdate]) -> bool {
        let Some(snapshot) = self.snapshot.as_mut() else {
            return false;
        };
        let before = snapshot.clone();
        crate::git::graph::apply_squash_enrichment(snapshot, updates);
        *snapshot != before
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
                    is_possible_squash_merge: false,
                    fuzzy_squash_match: None,
                    ..GraphCommit::default()
                },
                GraphCommit {
                    oid: "2222222222222222222222222222222222222222".into(),
                    summary: "second".into(),
                    parents: vec![],
                    lane: Some(0),
                    branch: None,
                    refs: vec![],
                    is_possible_squash_merge: false,
                    fuzzy_squash_match: None,
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
}
