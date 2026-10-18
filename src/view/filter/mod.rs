use crate::types::MergeStatus;

/// Parsed filter state from a query string
#[derive(Debug, Default, Clone)]
pub struct FilterSet {
    pub statuses: Vec<MergeStatus>,
    pub pr_yes: bool,
    pub pr_no: bool,
    pub sync_ahead: bool,
    pub sync_behind: bool,
    pub age_newer_secs: Option<i64>,
    pub age_older_secs: Option<i64>,
    pub text: String,
}

/// Defines which filter tokens are available in a view
#[derive(Debug, Clone)]
pub struct FilterTokenDef {
    pub key: char,
    pub label: &'static str,
    pub token: &'static str,
}

impl FilterSet {
    pub fn parse(query: &str) -> Self {
        let mut fs = Self::default();
        let mut text_parts = Vec::new();

        for token in query.split_whitespace() {
            match token {
                "merge:merged" => fs.statuses.push(MergeStatus::Merged),
                "merge:squash" => fs.statuses.push(MergeStatus::SquashMerged),
                "merge:remote-merged" => fs.statuses.push(MergeStatus::RemoteMerged),
                "merge:local-merged" => fs.statuses.push(MergeStatus::LocalMerged),
                "merge:remote-squash" => fs.statuses.push(MergeStatus::RemoteSquashMerged),
                "merge:local-squash" => fs.statuses.push(MergeStatus::LocalSquashMerged),
                "merge:cherry" => fs.statuses.push(MergeStatus::CherryPicked),
                "merge:remote-cherry" => fs.statuses.push(MergeStatus::RemoteCherryPicked),
                "merge:local-cherry" => fs.statuses.push(MergeStatus::LocalCherryPicked),
                "merge:unmerged" => fs.statuses.push(MergeStatus::Unmerged),
                "merge:possible-squash" => fs.statuses.push(MergeStatus::LikelySquashMerged),
                "pr:yes" => fs.pr_yes = true,
                "pr:no" => fs.pr_no = true,
                "sync:ahead" => fs.sync_ahead = true,
                "sync:behind" => fs.sync_behind = true,
                t if t.starts_with("age:<") => {
                    if let Some(secs) = parse_age_secs(&t[5..]) {
                        fs.age_newer_secs = Some(secs);
                    } else {
                        text_parts.push(t);
                    }
                }
                t if t.starts_with("age:>") => {
                    if let Some(secs) = parse_age_secs(&t[5..]) {
                        fs.age_older_secs = Some(secs);
                    } else {
                        text_parts.push(t);
                    }
                }
                other => text_parts.push(other),
            }
        }

        fs.text = text_parts.join(" ");
        fs
    }

    pub fn is_empty(&self) -> bool {
        self.statuses.is_empty()
            && !self.pr_yes
            && !self.pr_no
            && !self.sync_ahead
            && !self.sync_behind
            && self.age_newer_secs.is_none()
            && self.age_older_secs.is_none()
            && self.text.is_empty()
    }

    pub fn toggle_token(query: &str, token: &str) -> String {
        if Self::has_token(query, token) {
            query
                .split_whitespace()
                .filter(|&t| t != token)
                .collect::<Vec<_>>()
                .join(" ")
        } else {
            let mut result = query.to_string();
            if !result.is_empty() {
                result.push(' ');
            }
            result.push_str(token);
            result
        }
    }

    pub fn has_token(query: &str, token: &str) -> bool {
        query.split_whitespace().any(|t| t == token)
    }
}

/// Returns the three merge-status filter tokens (merged / squash / unmerged).
/// Used by the Branches, Remotes, and Worktrees views.
pub fn merge_tokens() -> Vec<FilterTokenDef> {
    vec![
        FilterTokenDef {
            key: 'm',
            label: "Merged",
            token: "merge:merged",
        },
        FilterTokenDef {
            key: 's',
            label: "Squash-merged",
            token: "merge:squash",
        },
        FilterTokenDef {
            key: 'r',
            label: "Remote-merged",
            token: "merge:remote-merged",
        },
        FilterTokenDef {
            key: 'l',
            label: "Local-merged",
            token: "merge:local-merged",
        },
        FilterTokenDef {
            key: 'R',
            label: "Remote-squash",
            token: "merge:remote-squash",
        },
        FilterTokenDef {
            key: 'L',
            label: "Local-squash",
            token: "merge:local-squash",
        },
        FilterTokenDef {
            key: 'c',
            label: "Cherry-picked",
            token: "merge:cherry",
        },
        FilterTokenDef {
            key: 'C',
            label: "Remote cherry-picked",
            token: "merge:remote-cherry",
        },
        FilterTokenDef {
            key: 'h',
            label: "Local cherry-picked",
            token: "merge:local-cherry",
        },
        FilterTokenDef {
            key: 'u',
            label: "Unmerged",
            token: "merge:unmerged",
        },
        FilterTokenDef {
            key: 'x',
            label: "Possible squash",
            token: "merge:possible-squash",
        },
    ]
}

/// Returns the two PR filter tokens (has-PR / no-PR). Used by Branches and Remotes.
pub fn pr_tokens() -> Vec<FilterTokenDef> {
    vec![
        FilterTokenDef {
            key: 'p',
            label: "Has PR",
            token: "pr:yes",
        },
        FilterTokenDef {
            key: 'P',
            label: "No PR",
            token: "pr:no",
        },
    ]
}

/// Returns the two sync filter tokens (ahead / behind). Used by Branches and Remotes.
pub fn sync_tokens() -> Vec<FilterTokenDef> {
    vec![
        FilterTokenDef {
            key: 'a',
            label: "Ahead",
            token: "sync:ahead",
        },
        FilterTokenDef {
            key: 'b',
            label: "Behind",
            token: "sync:behind",
        },
    ]
}

/// Returns the four age filter tokens (<7d / <30d / >30d / >90d). Used by all four views.
pub fn age_tokens() -> Vec<FilterTokenDef> {
    vec![
        FilterTokenDef {
            key: '1',
            label: "<7 days",
            token: "age:<7d",
        },
        FilterTokenDef {
            key: '2',
            label: "<30 days",
            token: "age:<30d",
        },
        FilterTokenDef {
            key: '3',
            label: ">30 days",
            token: "age:>30d",
        },
        FilterTokenDef {
            key: '4',
            label: ">90 days",
            token: "age:>90d",
        },
    ]
}

fn parse_age_secs(s: &str) -> Option<i64> {
    let (num_str, multiplier) = if let Some(n) = s.strip_suffix('d') {
        (n, 86400i64)
    } else if let Some(n) = s.strip_suffix('w') {
        (n, 7 * 86400)
    } else if let Some(n) = s.strip_suffix('m') {
        (n, 30 * 86400)
    } else if let Some(n) = s.strip_suffix('y') {
        (n, 365 * 86400)
    } else {
        return None;
    };
    num_str.parse::<i64>().ok().map(|n| n * multiplier)
}

#[cfg(test)]
mod tests;
