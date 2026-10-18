use crate::git::graph::GraphSnapshot;

use super::{GraphState, GRAPH_PAGE_SIZE};

impl GraphState {
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

pub(super) fn cursor_for_target(snapshot: &GraphSnapshot, target: &str) -> Option<usize> {
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
