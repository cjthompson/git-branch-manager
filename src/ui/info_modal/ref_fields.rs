use super::fields::{merge_status_str, pr_status_str, squash_confidence_str};
use super::InfoField;
use crate::types::{
    BranchInfo, ChangedFileKind, RemoteBranchInfo, TagInfo, TrackingStatus, WorktreeInfo,
};

pub(super) fn branch_fields(b: &BranchInfo) -> Vec<InfoField> {
    let mut fields = vec![
        InfoField {
            label: "Name",
            value: b.name.clone(),
        },
        InfoField {
            label: "Current",
            value: if b.is_current { "yes" } else { "no" }.to_string(),
        },
        InfoField {
            label: "Base",
            value: if b.is_base { "yes" } else { "no" }.to_string(),
        },
    ];

    match &b.tracking {
        TrackingStatus::Tracked { remote_ref, gone } => {
            fields.push(InfoField {
                label: "Remote",
                value: remote_ref.clone(),
            });
            if *gone {
                fields.push(InfoField {
                    label: "Tracking",
                    value: "gone".to_string(),
                });
            }
        }
        TrackingStatus::Local => {
            fields.push(InfoField {
                label: "Tracking",
                value: "local".to_string(),
            });
        }
    }

    if let Some(ahead) = b.ahead {
        fields.push(InfoField {
            label: "Ahead",
            value: ahead.to_string(),
        });
    }

    if let Some(behind) = b.behind {
        fields.push(InfoField {
            label: "Behind",
            value: behind.to_string(),
        });
    }

    fields.push(InfoField {
        label: "Merge Status",
        value: merge_status_str(&b.merge_status).to_string(),
    });

    if let Some(confidence) = &b.squash_confidence {
        fields.push(InfoField {
            label: "Confidence",
            value: squash_confidence_str(confidence),
        });
    }

    fields.push(InfoField {
        label: "Base Branch",
        value: b.base_branch.clone(),
    });

    if let Some(hash) = &b.merge_base_commit {
        fields.push(InfoField {
            label: "Merge Base",
            value: hash.clone(),
        });
    }

    fields.push(InfoField {
        label: "Last Commit",
        value: b.age_display(),
    });

    if let Some(pr) = &b.pr {
        fields.push(InfoField {
            label: "PR",
            value: format!("#{} ({})", pr.number, pr_status_str(&pr.status)),
        });
    }

    fields
}

pub(super) fn remote_fields(r: &RemoteBranchInfo) -> Vec<InfoField> {
    let mut fields = vec![
        InfoField {
            label: "Full Ref",
            value: r.full_ref.clone(),
        },
        InfoField {
            label: "Remote",
            value: r.remote.clone(),
        },
        InfoField {
            label: "Short Name",
            value: r.short_name.clone(),
        },
        InfoField {
            label: "Has Local",
            value: if r.has_local { "yes" } else { "no" }.to_string(),
        },
        InfoField {
            label: "Base",
            value: if r.is_base { "yes" } else { "no" }.to_string(),
        },
    ];

    if let Some(ahead) = r.ahead {
        fields.push(InfoField {
            label: "Ahead",
            value: ahead.to_string(),
        });
    }

    if let Some(behind) = r.behind {
        fields.push(InfoField {
            label: "Behind",
            value: behind.to_string(),
        });
    }

    fields.push(InfoField {
        label: "Merge Status",
        value: merge_status_str(&r.merge_status).to_string(),
    });

    if let Some(confidence) = &r.squash_confidence {
        fields.push(InfoField {
            label: "Confidence",
            value: squash_confidence_str(confidence),
        });
    }

    fields.push(InfoField {
        label: "Last Commit",
        value: r.age_display(),
    });

    if let Some(pr) = &r.pr {
        fields.push(InfoField {
            label: "PR",
            value: format!("#{} ({})", pr.number, pr_status_str(&pr.status)),
        });
    }

    fields
}

pub(super) fn tag_fields(t: &TagInfo) -> Vec<InfoField> {
    let mut fields = vec![
        InfoField {
            label: "Name",
            value: t.name.clone(),
        },
        InfoField {
            label: "Commit Hash",
            value: t.commit_hash.clone(),
        },
        InfoField {
            label: "Date",
            value: t.age_display(),
        },
        InfoField {
            label: "Annotated",
            value: if t.is_annotated { "yes" } else { "no" }.to_string(),
        },
    ];

    if let Some(msg) = &t.message {
        fields.push(InfoField {
            label: "Message",
            value: msg.clone(),
        });
    }

    fields
}

pub(super) fn worktree_fields(w: &WorktreeInfo) -> Vec<InfoField> {
    let mut fields = vec![
        InfoField {
            label: "Path",
            value: w.path.to_string_lossy().to_string(),
        },
        InfoField {
            label: "Main",
            value: if w.is_main { "yes" } else { "no" }.to_string(),
        },
        InfoField {
            label: "Commit Hash",
            value: w.commit_hash.clone(),
        },
    ];

    if let Some(branch) = &w.branch {
        fields.push(InfoField {
            label: "Branch",
            value: branch.clone(),
        });
    }

    fields.push(InfoField {
        label: "Status",
        value: w.wt_status.summary(),
    });

    if let Some(ahead) = w.ahead {
        fields.push(InfoField {
            label: "Ahead",
            value: ahead.to_string(),
        });
    }

    if let Some(behind) = w.behind {
        fields.push(InfoField {
            label: "Behind",
            value: behind.to_string(),
        });
    }

    fields.push(InfoField {
        label: "Merge Status",
        value: merge_status_str(&w.merge_status).to_string(),
    });

    fields.push(InfoField {
        label: "Last Commit",
        value: w.age_display(),
    });

    if let Some(pr_status) = &w.pr {
        fields.push(InfoField {
            label: "PR",
            value: pr_status_str(pr_status).to_string(),
        });
    }

    for kind in [
        ChangedFileKind::Staged,
        ChangedFileKind::Modified,
        ChangedFileKind::Untracked,
    ] {
        let mut first_in_group = true;
        for file in w
            .wt_status
            .changed_files
            .iter()
            .filter(|file| file.kind == kind)
        {
            fields.push(InfoField {
                label: if first_in_group { kind.label() } else { "" },
                value: format!("{} ({})", file.path, file.kind.label()),
            });
            first_in_group = false;
        }
    }

    fields
}
