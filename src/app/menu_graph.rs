use super::*;

impl App {
    pub(super) fn build_graph_menu(&self) -> Vec<MenuItem> {
        let Some(commit) = self.graph.selected_commit() else {
            return vec![];
        };
        self.build_graph_menu_for(commit)
    }

    pub(super) fn build_graph_menu_for(&self, commit: &graph::GraphCommit) -> Vec<MenuItem> {
        let mut groups = Vec::new();
        for reference in &commit.refs {
            let items = match reference.kind {
                graph::GraphRefKind::LocalBranch => self
                    .branches
                    .items()
                    .iter()
                    .find(|branch| branch.name == reference.name)
                    .map(|branch| self.build_branch_menu_for(branch)),
                graph::GraphRefKind::RemoteBranch => self
                    .remotes
                    .items()
                    .iter()
                    .find(|branch| branch.full_ref == reference.name)
                    .map(|branch| self.build_remote_menu_for(branch)),
                graph::GraphRefKind::Tag => self
                    .tags
                    .items()
                    .iter()
                    .find(|tag| tag.name == reference.name)
                    .map(|tag| self.build_tag_menu_for(tag)),
            };
            if let Some(items) = items {
                let items: Vec<MenuItem> = items
                    .into_iter()
                    .filter(|item| item.action != BranchAction::JumpToGraph)
                    .collect();
                groups.push((reference.name.clone(), items));
            }
        }

        let has_multiple_refs = groups.len() > 1;
        groups
            .into_iter()
            .flat_map(|(reference, items)| {
                items.into_iter().map(move |mut item| {
                    item.label = format!("{reference}: {}", item.label);
                    if has_multiple_refs {
                        item.shortcut = None;
                    }
                    item
                })
            })
            .collect()
    }

    pub(super) fn refresh_open_graph_menu(&mut self) {
        if matches!(self.overlay, Some(Overlay::CommitDiff { .. })) {
            let Some(Overlay::CommitDiff {
                commit,
                details,
                items: old_items,
                cursor,
                file_cursor,
                focus,
                details_scroll,
                diff,
                scroll,
            }) = self.overlay.take()
            else {
                return;
            };
            let commit = self
                .graph
                .snapshot()
                .and_then(|snapshot| snapshot.commits.iter().find(|item| item.oid == details.oid))
                .cloned()
                .unwrap_or(commit);
            let items = self.build_graph_menu_for(&commit);
            let selected = old_items
                .get(cursor)
                .map(|item| (item.action, item.target.as_str(), item.remote.as_deref()));
            let cursor = selected
                .and_then(|(action, target, remote)| {
                    items.iter().position(|item| {
                        item.action == action
                            && item.target == target
                            && item.remote.as_deref() == remote
                    })
                })
                .filter(|cursor| items[*cursor].enabled)
                .or_else(|| first_enabled_index(&items))
                .unwrap_or(0);
            self.overlay = Some(Overlay::CommitDiff {
                commit,
                details,
                items,
                cursor,
                file_cursor,
                focus,
                details_scroll,
                diff,
                scroll,
            });
            return;
        }
        if matches!(self.overlay, Some(Overlay::CommitDetails { .. })) {
            let Some(Overlay::CommitDetails {
                commit,
                details,
                items: old_items,
                cursor,
                file_cursor,
                focus,
                scroll,
            }) = self.overlay.take()
            else {
                return;
            };
            let commit = self
                .graph
                .snapshot()
                .and_then(|snapshot| snapshot.commits.iter().find(|item| item.oid == details.oid))
                .cloned()
                .unwrap_or(commit);
            let items = self.build_graph_menu_for(&commit);
            let selected = old_items
                .get(cursor)
                .map(|item| (item.action, item.target.as_str(), item.remote.as_deref()));
            let cursor = selected
                .and_then(|(action, target, remote)| {
                    items.iter().position(|item| {
                        item.action == action
                            && item.target == target
                            && item.remote.as_deref() == remote
                    })
                })
                .filter(|cursor| items[*cursor].enabled)
                .or_else(|| first_enabled_index(&items))
                .unwrap_or(0);
            let focus = if first_enabled_index(&items).is_none() {
                CommitDetailsFocus::Files
            } else {
                focus
            };
            self.overlay = Some(Overlay::CommitDetails {
                commit,
                details,
                items,
                cursor,
                file_cursor,
                focus,
                scroll,
            });
            return;
        }
        let Some(old_commit) = (match self.overlay.as_ref() {
            Some(Overlay::InfoModal {
                row: InfoModalRow::GraphCommit(commit),
                ..
            }) => Some(commit.clone()),
            _ => None,
        }) else {
            return;
        };
        let commit = self.graph.selected_commit().cloned().unwrap_or(old_commit);
        let items = self.build_graph_menu_for(&commit);

        let Some(Overlay::InfoModal {
            items: old_items,
            cursor,
            info_cursor,
            focus,
            row: _,
        }) = self.overlay.take()
        else {
            unreachable!();
        };
        let selected = old_items
            .get(cursor)
            .map(|item| (item.action, item.target.as_str(), item.remote.as_deref()));
        let cursor = selected
            .and_then(|(action, target, remote)| {
                items.iter().position(|item| {
                    item.action == action
                        && item.target == target
                        && item.remote.as_deref() == remote
                })
            })
            .filter(|cursor| items[*cursor].enabled)
            .or_else(|| first_enabled_index(&items))
            .unwrap_or(0);
        let focus = if first_enabled_index(&items).is_none() {
            InfoModalFocus::Info
        } else {
            focus
        };
        let row = InfoModalRow::GraphCommit(commit);
        let info_cursor = info_cursor.min(row.info_field_count().saturating_sub(1));
        self.overlay = Some(Overlay::InfoModal {
            items,
            cursor,
            info_cursor,
            focus,
            row,
        });
    }
}
