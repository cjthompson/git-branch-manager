use super::*;

#[test]
fn remotes_without_deleted_drops_only_the_recorded_remote_pair() {
    let items = vec![
        remote("origin/feature/local", "feature/local"),
        remote("upstream/feature/local", "feature/local"),
    ];
    let deleted = vec![("upstream".to_string(), "feature/local".to_string())];

    let kept = super::super::drain::remotes_without_deleted(&items, &deleted);

    assert_eq!(kept.len(), 1);
    assert_eq!(kept[0].full_ref, "origin/feature/local");
}
