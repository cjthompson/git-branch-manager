use super::*;

/// Find additional pairs between displayed branches without walking every branch pair.
/// Subjects only select candidates; exact patch identity still confirms each pair.
/// Base-history pairs remain the responsibility of the existing bounded scan.
pub(in crate::git::graph) fn compute_cross_branch_cherry_relationships(
    repo_path: &Path,
    snapshot: &GraphSnapshot,
    base_tip: &str,
    cache: &BranchCache,
    cancel: &EnrichmentCancel,
    observer: &EnrichmentObserver,
) -> Vec<GraphRelationship> {
    let (Ok(repo), Ok(base)) = (
        git2::Repository::open(repo_path),
        git2::Oid::from_str(base_tip),
    ) else {
        return Vec::new();
    };
    let mut subjects = HashMap::<_, Vec<_>>::new();
    let mut seen = HashSet::new();
    for (index, commit) in snapshot.commits.iter().enumerate() {
        if cancel.is_cancelled() {
            return Vec::new();
        }
        if commit.parents.len() != 1 {
            continue;
        }
        let mut names = commit
            .refs
            .iter()
            .filter(|r| is_source_ref(r, "", snapshot.includes_remotes))
            .map(|r| r.name.clone())
            .collect::<Vec<_>>();
        if let Some(branch) = &commit.branch {
            if branch.kind == crate::git::graph::GraphRefKind::LocalBranch
                || (snapshot.includes_remotes
                    && branch.kind == crate::git::graph::GraphRefKind::RemoteBranch)
            {
                names.push(branch.name.clone());
            }
        }
        names.sort();
        names.dedup();
        if names.is_empty() || !seen.insert(&commit.oid) {
            continue;
        }
        let Ok(oid) = git2::Oid::from_str(&commit.oid) else {
            continue;
        };
        let Ok(native) = repo.find_commit(oid) else {
            continue;
        };
        let subject = native
            .message_bytes()
            .split(|byte| *byte == b'\n')
            .next()
            .unwrap_or_default();
        if subject.is_empty() {
            continue;
        }
        subjects
            .entry(subject.to_vec())
            .or_default()
            .push((index, commit, names));
    }
    let mut patches = CherryPatchIds {
        repo: &repo,
        cache,
        cancel,
        observer,
        memo: HashMap::new(),
        pending: Vec::new(),
    };
    let mut relationships = RelationshipAccumulator::default();
    for group in subjects.values().filter(|group| group.len() > 1) {
        let mut candidates = Vec::new();
        for (index, commit, names) in group {
            if cancel.is_cancelled() {
                return Vec::new();
            }
            let Ok(oid) = git2::Oid::from_str(&commit.oid) else {
                continue;
            };
            if oid == base || repo.graph_descendant_of(base, oid).unwrap_or(true) {
                continue;
            }
            let Ok(native) = repo.find_commit(oid) else {
                continue;
            };
            candidates.push((
                native.time().seconds(),
                std::cmp::Reverse(*index),
                oid,
                names,
            ));
        }
        if candidates.len() < 2 {
            continue;
        }
        // Committer time orders source before destination. On equal timestamps,
        // the graph's newest-first order provides a deterministic fallback.
        candidates.sort();
        if !patches.load(&candidates.iter().map(|c| c.2).collect::<Vec<_>>()) {
            return Vec::new();
        }
        let mut buckets = HashMap::<_, Vec<_>>::new();
        for (_, _, oid, names) in &candidates {
            let Some(Some(patch)) = patches.memo.get(oid) else {
                continue;
            };
            buckets.entry(patch).or_default().push((*oid, names));
        }
        for matches in buckets.values() {
            for (position, (destination, destination_refs)) in matches.iter().enumerate() {
                for (source, source_refs) in &matches[..position] {
                    if cancel.is_cancelled() {
                        return Vec::new();
                    }
                    if repo
                        .graph_descendant_of(*destination, *source)
                        .unwrap_or(true)
                        || repo
                            .graph_descendant_of(*source, *destination)
                            .unwrap_or(true)
                    {
                        continue;
                    }
                    relationships.insert(&GraphRelationship {
                        kind: RelationshipKind::CherryPick,
                        matching: RelationshipMatch::Exact,
                        source_oid: source.to_string(),
                        source_refs: source_refs.to_vec(),
                        destination_oid: destination.to_string(),
                        destination_refs: destination_refs.to_vec(),
                    });
                }
            }
        }
        patches.flush();
    }
    relationships.into_relationships()
}
