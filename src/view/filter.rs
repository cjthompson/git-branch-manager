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
                "status:merged" => fs.statuses.push(MergeStatus::Merged),
                "status:squash" => fs.statuses.push(MergeStatus::SquashMerged),
                "status:unmerged" => fs.statuses.push(MergeStatus::Unmerged),
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
mod tests {
    use super::*;
    use crate::types::MergeStatus;

    #[test]
    fn parse_empty_query() {
        let fs = FilterSet::parse("");
        assert!(fs.statuses.is_empty());
        assert!(!fs.pr_yes);
        assert!(!fs.sync_ahead);
        assert!(fs.text.is_empty());
    }

    #[test]
    fn parse_status_merged() {
        let fs = FilterSet::parse("status:merged");
        assert_eq!(fs.statuses, vec![MergeStatus::Merged]);
    }

    #[test]
    fn parse_status_squash() {
        let fs = FilterSet::parse("status:squash");
        assert_eq!(fs.statuses, vec![MergeStatus::SquashMerged]);
    }

    #[test]
    fn parse_status_unmerged() {
        let fs = FilterSet::parse("status:unmerged");
        assert_eq!(fs.statuses, vec![MergeStatus::Unmerged]);
    }

    #[test]
    fn parse_multiple_tokens() {
        let fs = FilterSet::parse("status:merged pr:yes age:<7d");
        assert_eq!(fs.statuses, vec![MergeStatus::Merged]);
        assert!(fs.pr_yes);
        assert!(fs.age_newer_secs.is_some());
    }

    #[test]
    fn parse_age_newer() {
        let fs = FilterSet::parse("age:<30d");
        assert_eq!(fs.age_newer_secs, Some(30 * 86400));
    }

    #[test]
    fn parse_age_older() {
        let fs = FilterSet::parse("age:>90d");
        assert_eq!(fs.age_older_secs, Some(90 * 86400));
    }

    #[test]
    fn parse_age_weeks() {
        let fs = FilterSet::parse("age:<2w");
        assert_eq!(fs.age_newer_secs, Some(2 * 7 * 86400));
    }

    #[test]
    fn parse_age_months() {
        let fs = FilterSet::parse("age:>3m");
        assert_eq!(fs.age_older_secs, Some(3 * 30 * 86400));
    }

    #[test]
    fn parse_age_years() {
        let fs = FilterSet::parse("age:>1y");
        assert_eq!(fs.age_older_secs, Some(365 * 86400));
    }

    #[test]
    fn parse_sync_tokens() {
        let fs = FilterSet::parse("sync:ahead sync:behind");
        assert!(fs.sync_ahead);
        assert!(fs.sync_behind);
    }

    #[test]
    fn parse_pr_tokens() {
        let fs = FilterSet::parse("pr:no");
        assert!(fs.pr_no);
        assert!(!fs.pr_yes);
    }

    #[test]
    fn parse_text_tokens() {
        let fs = FilterSet::parse("feature status:merged foo");
        assert_eq!(fs.text, "feature foo");
        assert_eq!(fs.statuses, vec![MergeStatus::Merged]);
    }

    #[test]
    fn is_empty_default() {
        assert!(FilterSet::default().is_empty());
    }

    #[test]
    fn is_empty_with_status() {
        let fs = FilterSet::parse("status:merged");
        assert!(!fs.is_empty());
    }

    #[test]
    fn toggle_token_adds() {
        let result = FilterSet::toggle_token("", "status:merged");
        assert_eq!(result, "status:merged");
    }

    #[test]
    fn toggle_token_removes() {
        let result = FilterSet::toggle_token("status:merged pr:yes", "status:merged");
        assert_eq!(result, "pr:yes");
    }

    #[test]
    fn toggle_token_adds_to_existing() {
        let result = FilterSet::toggle_token("pr:yes", "status:merged");
        assert_eq!(result, "pr:yes status:merged");
    }

    #[test]
    fn has_token_true() {
        assert!(FilterSet::has_token(
            "status:merged pr:yes",
            "status:merged"
        ));
    }

    #[test]
    fn has_token_false() {
        assert!(!FilterSet::has_token(
            "status:merged pr:yes",
            "status:squash"
        ));
    }

    #[test]
    fn has_token_empty() {
        assert!(!FilterSet::has_token("", "status:merged"));
    }

    #[test]
    fn parse_invalid_age_suffix() {
        let fs = FilterSet::parse("age:<30x");
        assert!(fs.age_newer_secs.is_none());
        // "age:<30x" doesn't match any prefix, goes to text
        assert!(fs.text.contains("age:<30x"));
    }
}
