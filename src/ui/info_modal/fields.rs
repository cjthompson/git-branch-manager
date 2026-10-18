use super::ref_fields::{branch_fields, remote_fields, tag_fields, worktree_fields};
use super::{InfoField, InfoModalRow};
use crate::git::graph::{GraphCommit, GraphRefKind};
use crate::types::{MergeStatus, PrStatus, PrimaryBranchCodeMatch, SquashConfidence};

pub(super) fn build_fields(row: &InfoModalRow) -> Vec<InfoField> {
    match row {
        InfoModalRow::GraphCommit(commit) => graph_commit_fields(commit),
        InfoModalRow::Branch(b) => branch_fields(b),
        InfoModalRow::Remote(r) => remote_fields(r),
        InfoModalRow::Tag(t) => tag_fields(t),
        InfoModalRow::Worktree(w) => worktree_fields(w),
        InfoModalRow::PrimaryBranchCodeCheck {
            display_ref,
            base_branch,
            primary_reference,
            primary_oid,
            outcome,
        } => {
            let evidence_ref = primary_reference.as_deref().unwrap_or(base_branch);
            let mut fields = Vec::new();
            if let (Some(reference), Some(oid)) = (primary_reference, primary_oid) {
                fields.push(InfoField {
                    label: "Primary",
                    value: format!("{reference} @ {oid}"),
                });
            }
            let value = match outcome {
                Ok(PrimaryBranchCodeMatch::Merged) => format!(
                    "The selected commit from {display_ref} is reachable in {evidence_ref}'s history. This is historical evidence and does not establish current-tip retention."
                ),
                Ok(PrimaryBranchCodeMatch::ContentEquivalent { commit_oid }) => format!(
                    "All changes from {display_ref}, relative to the shared ancestor, are represented at {} in {evidence_ref}'s history. The selected ref itself need not be merged; this historical evidence does not establish current-tip retention.",
                    short_oid(commit_oid)
                ),
                Ok(PrimaryBranchCodeMatch::NotFound) => format!(
                    "No available snapshot in {evidence_ref}'s history contained the complete change from {display_ref}. Rewritten or split changes may not be recognized; this does not prove absence."
                ),
                Err(error) => format!("Could not check {display_ref} in {base_branch}: {error}"),
            };
            fields.push(InfoField {
                label: "Result",
                value,
            });
            fields
        }
    }
}

fn short_oid(oid: &str) -> &str {
    oid.get(..7).unwrap_or(oid)
}

pub(super) fn graph_commit_fields(commit: &GraphCommit) -> Vec<InfoField> {
    let mut fields = vec![
        InfoField {
            label: "Commit",
            value: commit.oid.clone(),
        },
        InfoField {
            label: "Summary",
            value: commit.summary.clone(),
        },
    ];

    if !commit.parents.is_empty() {
        fields.push(InfoField {
            label: "Parents",
            value: commit.parents.join(", "),
        });
    }

    // NEW: Author row — single combined "Name <email>" format
    let has_author_name = !commit.author_name.is_empty();
    let has_author_email = !commit.author_email.is_empty();
    if has_author_name || has_author_email {
        let author_value = match (has_author_name, has_author_email) {
            (true, true) => format!("{} <{}>", commit.author_name, commit.author_email),
            (true, false) => commit.author_name.clone(),
            (false, true) => commit.author_email.clone(),
            (false, false) => unreachable!(),
        };
        fields.push(InfoField {
            label: "Author",
            value: author_value,
        });
    }

    // Date row — absolute local time + relative age, when the loader provided a date.
    if let Some(authored_at) = commit.authored_at.as_ref() {
        let local = crate::types::format_local_absolute(authored_at);
        let age = crate::types::format_age(authored_at);
        fields.push(InfoField {
            label: "Date",
            value: format!("{local} ({age})"),
        });
    }

    if let Some(branch) = &commit.branch {
        fields.push(InfoField {
            label: "Branch",
            value: branch.name.clone(),
        });
    }

    for (label, kind) in [
        ("Local Refs", GraphRefKind::LocalBranch),
        ("Remote Refs", GraphRefKind::RemoteBranch),
        ("Tags", GraphRefKind::Tag),
    ] {
        let names = commit
            .refs
            .iter()
            .filter(|reference| reference.kind == kind)
            .map(|reference| reference.name.as_str())
            .collect::<Vec<_>>();
        if !names.is_empty() {
            fields.push(InfoField {
                label,
                value: names.join(", "),
            });
        }
    }

    if commit.is_possible_squash_merge() {
        fields.push(InfoField {
            label: "Possible Squash Merge",
            value: "yes".into(),
        });
        if !commit.possible_squash_merge_sources().is_empty() {
            fields.push(InfoField {
                label: "Possible Squash Merge From",
                value: commit.possible_squash_merge_sources().join(", "),
            });
        }
    } else if !commit.is_cherry_picked_commit() {
        if let Some(confidence) = commit.squash_match_confidence() {
            fields.push(InfoField {
                label: "Possible Squash Merge (fuzzy)",
                value: format!("{}% similarity", confidence.similarity_percent),
            });
            if !confidence.sources.is_empty() {
                fields.push(InfoField {
                    label: "Possible Squash Merge From",
                    value: confidence.sources.join(", "),
                });
            }
        }
    }

    for relationship in commit.cherry_pick_sources() {
        fields.push(InfoField {
            label: "Cherry-picked From",
            value: format!(
                "{} ({})",
                short_oid(&relationship.source_oid),
                relationship.source_refs.join(", ")
            ),
        });
    }

    fields
}

pub(super) fn merge_status_str(status: &MergeStatus) -> &'static str {
    match status {
        MergeStatus::Merged => "Merged",
        MergeStatus::InSync => "In Sync",
        MergeStatus::SquashMerged => "Squash Merged",
        MergeStatus::LocalMerged => "Local Merged",
        MergeStatus::RemoteMerged => "Remote Merged",
        MergeStatus::LocalSquashMerged => "Local Squash Merged",
        MergeStatus::RemoteSquashMerged => "Remote Squash Merged",
        MergeStatus::LikelySquashMerged => "Possible Squash Merge",
        MergeStatus::CherryPicked => "Cherry Picked",
        MergeStatus::LocalCherryPicked => "Local Cherry Picked",
        MergeStatus::RemoteCherryPicked => "Remote Cherry Picked",
        MergeStatus::Unmerged => "Unmerged",
        MergeStatus::Pending => "Pending",
    }
}

pub(super) fn squash_confidence_str(confidence: &SquashConfidence) -> String {
    match confidence {
        SquashConfidence::MergeTreeConfirmed => "Merge-tree confirmed".to_string(),
        SquashConfidence::FuzzyMatch { similarity_percent } => {
            format!("Fuzzy match ({similarity_percent}%)")
        }
    }
}

pub(super) fn pr_status_str(status: &PrStatus) -> &'static str {
    match status {
        PrStatus::Draft => "Draft",
        PrStatus::Open => "Open",
        PrStatus::Merged => "Merged",
        PrStatus::Closed => "Closed",
    }
}
