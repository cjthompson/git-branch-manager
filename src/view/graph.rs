use crate::git::graph::{GraphLoadError, GraphSidebarRef, GraphSnapshot};

/// The default number of commits shown by the Graph tab. Larger histories are
/// opt-in so opening the tab remains responsive on large repositories.
pub const GRAPH_PAGE_SIZE: usize = 500;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GraphPane {
    Commits,
    Sidebar,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GraphRefScope {
    Local,
    Remote,
}

/// State owned exclusively by the Graph tab. It intentionally does not reuse
/// `ListState`: graph lines, commit rows, and ref rows have different scroll
/// and selection semantics than the sortable table views.
#[derive(Debug)]
pub struct GraphState {
    snapshot: Option<GraphSnapshot>,
    loading: bool,
    error: Option<String>,
    max_count: usize,
    include_remotes: bool,
    focus: GraphPane,
    commit_cursor: usize,
    commit_offset: usize,
    sidebar_cursor: usize,
    sidebar_offset: usize,
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
            focus: GraphPane::Commits,
            commit_cursor: 0,
            commit_offset: 0,
            sidebar_cursor: 0,
            sidebar_offset: 0,
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

    pub fn focus(&self) -> GraphPane {
        self.focus
    }

    pub fn commit_offset(&self) -> usize {
        self.commit_offset
    }

    pub fn sidebar_offset(&self) -> usize {
        self.sidebar_offset
    }

    pub fn commit_cursor(&self) -> usize {
        self.commit_cursor
    }

    pub fn sidebar_cursor(&self) -> usize {
        self.sidebar_cursor
    }

    pub fn begin_load(&mut self, max_count: usize, include_remotes: bool) {
        self.max_count = max_count.max(1);
        self.include_remotes = include_remotes;
        self.loading = true;
        self.error = None;
    }

    pub fn apply_result(&mut self, result: Result<GraphSnapshot, GraphLoadError>) {
        self.loading = false;
        match result {
            Ok(snapshot) => {
                self.max_count = snapshot.max_count;
                self.include_remotes = snapshot.includes_remotes;
                self.snapshot = Some(snapshot);
                self.error = None;
                self.commit_cursor = 0;
                self.commit_offset = 0;
                self.sidebar_cursor = 0;
                self.sidebar_offset = 0;
            }
            Err(error) => {
                self.snapshot = None;
                self.error = Some(error.to_string());
                self.commit_cursor = 0;
                self.commit_offset = 0;
                self.sidebar_cursor = 0;
                self.sidebar_offset = 0;
            }
        }
    }

    pub fn set_include_remotes(&mut self, include_remotes: bool) {
        self.include_remotes = include_remotes;
    }

    /// Increase the history window by one page and return the new limit.
    pub fn load_older_history(&mut self) -> usize {
        self.max_count = self.max_count.saturating_add(GRAPH_PAGE_SIZE);
        self.max_count
    }

    pub fn focus_left(&mut self) {
        self.focus = GraphPane::Commits;
    }

    pub fn focus_right(&mut self) {
        self.focus = GraphPane::Sidebar;
    }

    pub fn move_up(&mut self) {
        match self.focus {
            GraphPane::Commits => self.commit_cursor = self.commit_cursor.saturating_sub(1),
            GraphPane::Sidebar => self.sidebar_cursor = self.sidebar_cursor.saturating_sub(1),
        }
    }

    pub fn move_down(&mut self) {
        match self.focus {
            GraphPane::Commits => {
                let count = self.commit_count();
                if count > 0 {
                    self.commit_cursor = (self.commit_cursor + 1).min(count - 1);
                }
            }
            GraphPane::Sidebar => {
                let count = self.sidebar_len();
                if count > 0 {
                    self.sidebar_cursor = (self.sidebar_cursor + 1).min(count - 1);
                }
            }
        }
    }

    pub fn page_up(&mut self) {
        match self.focus {
            GraphPane::Commits => self.commit_cursor = self.commit_cursor.saturating_sub(20),
            GraphPane::Sidebar => self.sidebar_cursor = self.sidebar_cursor.saturating_sub(10),
        }
    }

    pub fn page_down(&mut self) {
        match self.focus {
            GraphPane::Commits => {
                let count = self.commit_count();
                self.commit_cursor = (self.commit_cursor + 20).min(count.saturating_sub(1));
            }
            GraphPane::Sidebar => {
                let count = self.sidebar_len();
                self.sidebar_cursor = (self.sidebar_cursor + 10).min(count.saturating_sub(1));
            }
        }
    }

    pub fn home(&mut self) {
        match self.focus {
            GraphPane::Commits => self.commit_cursor = 0,
            GraphPane::Sidebar => self.sidebar_cursor = 0,
        }
    }

    pub fn end(&mut self) {
        match self.focus {
            GraphPane::Commits => self.commit_cursor = self.commit_count().saturating_sub(1),
            GraphPane::Sidebar => self.sidebar_cursor = self.sidebar_len().saturating_sub(1),
        }
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

    pub fn sidebar_len(&self) -> usize {
        self.snapshot
            .as_ref()
            .map(|snapshot| {
                snapshot.sidebar.local_branches.len() + snapshot.sidebar.remote_branches.len()
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
                .nth(self.commit_cursor)
                .map(|(line_index, _)| line_index)
        })
    }

    pub fn selected_commit(&self) -> Option<&crate::git::graph::GraphCommit> {
        let commit_index = self
            .selected_commit_line()
            .and_then(|line_index| self.snapshot.as_ref()?.lines[line_index].commit_index)?;
        self.snapshot.as_ref()?.commits.get(commit_index)
    }

    pub fn sidebar_ref(&self, index: usize) -> Option<(&GraphSidebarRef, GraphRefScope)> {
        let snapshot = self.snapshot.as_ref()?;
        if let Some(reference) = snapshot.sidebar.local_branches.get(index) {
            return Some((reference, GraphRefScope::Local));
        }
        let remote_index = index.checked_sub(snapshot.sidebar.local_branches.len())?;
        snapshot
            .sidebar
            .remote_branches
            .get(remote_index)
            .map(|reference| (reference, GraphRefScope::Remote))
    }

    pub fn selected_sidebar_ref(&self) -> Option<(&GraphSidebarRef, GraphRefScope)> {
        self.sidebar_ref(self.sidebar_cursor)
    }

    /// Keep the active row inside the viewport. The renderer calls this after
    /// it knows the current pane heights.
    pub fn ensure_visible(&mut self, graph_rows: usize, sidebar_rows: usize) {
        if let Some(line_index) = self.selected_commit_line() {
            if graph_rows > 0 {
                if line_index < self.commit_offset {
                    self.commit_offset = line_index;
                } else if line_index >= self.commit_offset + graph_rows {
                    self.commit_offset = line_index + 1 - graph_rows;
                }
            }
        }

        if sidebar_rows > 0 {
            if self.sidebar_cursor < self.sidebar_offset {
                self.sidebar_offset = self.sidebar_cursor;
            } else if self.sidebar_cursor >= self.sidebar_offset + sidebar_rows {
                self.sidebar_offset = self.sidebar_cursor + 1 - sidebar_rows;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::graph::{GraphCommit, GraphLine, GraphRef, GraphRefKind, GraphSidebar};

    fn snapshot() -> GraphSnapshot {
        GraphSnapshot {
            source: crate::git::graph::GraphSource::Gleisbau,
            commits: vec![
                GraphCommit {
                    oid: "1111111111111111111111111111111111111111".into(),
                    summary: "first".into(),
                    parents: vec![],
                    lane: Some(0),
                    refs: vec![GraphRef {
                        name: "main".into(),
                        kind: GraphRefKind::LocalBranch,
                    }],
                },
                GraphCommit {
                    oid: "2222222222222222222222222222222222222222".into(),
                    summary: "second".into(),
                    parents: vec![],
                    lane: Some(0),
                    refs: vec![],
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
            sidebar: GraphSidebar {
                local_branches: vec![GraphSidebarRef {
                    name: "main".into(),
                    target_oid: "1111111111111111111111111111111111111111".into(),
                    lane: Some(0),
                }],
                remote_branches: vec![],
            },
            max_count: 500,
            includes_remotes: false,
        }
    }

    #[test]
    fn graph_focus_and_navigation_are_independent_of_list_state() {
        let mut state = GraphState::new();
        state.apply_result(Ok(snapshot()));
        assert_eq!(state.focus(), GraphPane::Commits);
        state.focus_right();
        state.move_down();
        assert_eq!(state.sidebar_cursor(), 0);
        state.focus_left();
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
}
