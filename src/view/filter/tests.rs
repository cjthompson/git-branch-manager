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
    let fs = FilterSet::parse("merge:merged");
    assert_eq!(fs.statuses, vec![MergeStatus::Merged]);
}

#[test]
fn parse_status_squash() {
    let fs = FilterSet::parse("merge:squash");
    assert_eq!(fs.statuses, vec![MergeStatus::SquashMerged]);
}

#[test]
fn parse_status_unmerged() {
    let fs = FilterSet::parse("merge:unmerged");
    assert_eq!(fs.statuses, vec![MergeStatus::Unmerged]);
}

#[test]
fn parse_status_possible_squash() {
    let fs = FilterSet::parse("merge:possible-squash");
    assert_eq!(fs.statuses, vec![MergeStatus::LikelySquashMerged]);
}

#[test]
fn parse_status_remote_merged() {
    let fs = FilterSet::parse("merge:remote-merged");
    assert_eq!(fs.statuses, vec![MergeStatus::RemoteMerged]);
}

#[test]
fn parse_status_local_merged() {
    let fs = FilterSet::parse("merge:local-merged");
    assert_eq!(fs.statuses, vec![MergeStatus::LocalMerged]);
}

#[test]
fn parse_status_remote_squash() {
    let fs = FilterSet::parse("merge:remote-squash");
    assert_eq!(fs.statuses, vec![MergeStatus::RemoteSquashMerged]);
}

#[test]
fn parse_status_local_squash() {
    let fs = FilterSet::parse("merge:local-squash");
    assert_eq!(fs.statuses, vec![MergeStatus::LocalSquashMerged]);
}

#[test]
fn parse_multiple_tokens() {
    let fs = FilterSet::parse("merge:merged pr:yes age:<7d");
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
    let fs = FilterSet::parse("feature merge:merged foo");
    assert_eq!(fs.text, "feature foo");
    assert_eq!(fs.statuses, vec![MergeStatus::Merged]);
}

#[test]
fn is_empty_default() {
    assert!(FilterSet::default().is_empty());
}

#[test]
fn is_empty_with_status() {
    let fs = FilterSet::parse("merge:merged");
    assert!(!fs.is_empty());
}

#[test]
fn toggle_token_adds() {
    let result = FilterSet::toggle_token("", "merge:merged");
    assert_eq!(result, "merge:merged");
}

#[test]
fn toggle_token_removes() {
    let result = FilterSet::toggle_token("merge:merged pr:yes", "merge:merged");
    assert_eq!(result, "pr:yes");
}

#[test]
fn toggle_token_adds_to_existing() {
    let result = FilterSet::toggle_token("pr:yes", "merge:merged");
    assert_eq!(result, "pr:yes merge:merged");
}

#[test]
fn has_token_true() {
    assert!(FilterSet::has_token("merge:merged pr:yes", "merge:merged"));
}

#[test]
fn has_token_false() {
    assert!(!FilterSet::has_token("merge:merged pr:yes", "merge:squash"));
}

#[test]
fn has_token_empty() {
    assert!(!FilterSet::has_token("", "merge:merged"));
}

#[test]
fn parse_invalid_age_suffix() {
    let fs = FilterSet::parse("age:<30x");
    assert!(fs.age_newer_secs.is_none());
    // "age:<30x" doesn't match any prefix, goes to text
    assert!(fs.text.contains("age:<30x"));
}

// --- token group helpers: exact key/label/token tuples ---

fn tuples(tokens: &[FilterTokenDef]) -> Vec<(char, &'static str, &'static str)> {
    tokens.iter().map(|t| (t.key, t.label, t.token)).collect()
}

#[test]
fn merge_tokens_exact() {
    assert_eq!(
        tuples(&super::merge_tokens()),
        vec![
            ('m', "Merged", "merge:merged"),
            ('s', "Squash-merged", "merge:squash"),
            ('r', "Remote-merged", "merge:remote-merged"),
            ('l', "Local-merged", "merge:local-merged"),
            ('R', "Remote-squash", "merge:remote-squash"),
            ('L', "Local-squash", "merge:local-squash"),
            ('c', "Cherry-picked", "merge:cherry"),
            ('C', "Remote cherry-picked", "merge:remote-cherry"),
            ('h', "Local cherry-picked", "merge:local-cherry"),
            ('u', "Unmerged", "merge:unmerged"),
            ('x', "Possible squash", "merge:possible-squash"),
        ]
    );
}

#[test]
fn pr_tokens_exact() {
    assert_eq!(
        tuples(&super::pr_tokens()),
        vec![('p', "Has PR", "pr:yes"), ('P', "No PR", "pr:no")]
    );
}

#[test]
fn sync_tokens_exact() {
    assert_eq!(
        tuples(&super::sync_tokens()),
        vec![('a', "Ahead", "sync:ahead"), ('b', "Behind", "sync:behind")]
    );
}

#[test]
fn age_tokens_exact() {
    assert_eq!(
        tuples(&super::age_tokens()),
        vec![
            ('1', "<7 days", "age:<7d"),
            ('2', "<30 days", "age:<30d"),
            ('3', ">30 days", "age:>30d"),
            ('4', ">90 days", "age:>90d"),
        ]
    );
}

// --- cross-view consistency: instantiate the real view defs and compare ---

fn subset<'a>(tokens: &'a [FilterTokenDef], prefixes: &[&str]) -> Vec<&'a str> {
    tokens
        .iter()
        .map(|t| t.token)
        .filter(|tok| prefixes.iter().any(|p| tok.starts_with(p)))
        .collect()
}

#[test]
fn branches_and_remotes_share_pr_and_sync_tokens() {
    let b = super::super::branches::BranchesViewDef.filter_tokens();
    let r = super::super::remotes::RemotesViewDef.filter_tokens();
    let kinds = ["pr:", "sync:"];
    assert_eq!(subset(&b, &kinds), subset(&r, &kinds));
    assert_eq!(
        subset(&b, &kinds),
        vec!["pr:yes", "pr:no", "sync:ahead", "sync:behind"]
    );
}

#[test]
fn branches_remotes_worktrees_share_merge_tokens() {
    let b = super::super::branches::BranchesViewDef.filter_tokens();
    let r = super::super::remotes::RemotesViewDef.filter_tokens();
    let w = super::super::worktrees::WorktreesViewDef.filter_tokens();
    let kinds = ["merge:"];
    assert_eq!(subset(&b, &kinds), subset(&r, &kinds));
    assert_eq!(subset(&b, &kinds), subset(&w, &kinds));
    assert_eq!(
        subset(&b, &kinds),
        vec![
            "merge:merged",
            "merge:squash",
            "merge:remote-merged",
            "merge:local-merged",
            "merge:remote-squash",
            "merge:local-squash",
            "merge:cherry",
            "merge:remote-cherry",
            "merge:local-cherry",
            "merge:unmerged",
            "merge:possible-squash",
        ]
    );
}

#[test]
fn all_views_share_age_tokens() {
    let b = super::super::branches::BranchesViewDef.filter_tokens();
    let r = super::super::remotes::RemotesViewDef.filter_tokens();
    let w = super::super::worktrees::WorktreesViewDef.filter_tokens();
    let t = super::super::tags::TagsViewDef.filter_tokens();
    let kinds = ["age:"];
    let expected = vec!["age:<7d", "age:<30d", "age:>30d", "age:>90d"];
    assert_eq!(subset(&b, &kinds), expected);
    assert_eq!(subset(&r, &kinds), expected);
    assert_eq!(subset(&w, &kinds), expected);
    assert_eq!(subset(&t, &kinds), expected);
}
