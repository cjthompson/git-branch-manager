use super::resolve_debug_log_path;
use std::cell::Cell;
use std::path::PathBuf;

#[test]
fn unset_disables_logging() {
    let calls = Cell::new(0u32);
    let result = resolve_debug_log_path(None, || {
        calls.set(calls.get() + 1);
        PathBuf::from("/stub/default.log")
    });
    assert_eq!(result, None);
    assert_eq!(calls.get(), 0);
}

#[test]
fn zero_disables_logging() {
    let calls = Cell::new(0u32);
    let result = resolve_debug_log_path(Some("0"), || {
        calls.set(calls.get() + 1);
        PathBuf::from("/stub/default.log")
    });
    assert_eq!(result, None);
    assert_eq!(calls.get(), 0);
}

#[test]
fn false_disables_logging() {
    let calls = Cell::new(0u32);
    let result = resolve_debug_log_path(Some("false"), || {
        calls.set(calls.get() + 1);
        PathBuf::from("/stub/default.log")
    });
    assert_eq!(result, None);
    assert_eq!(calls.get(), 0);
}

#[test]
fn empty_disables_logging() {
    let calls = Cell::new(0u32);
    let result = resolve_debug_log_path(Some(""), || {
        calls.set(calls.get() + 1);
        PathBuf::from("/stub/default.log")
    });
    assert_eq!(result, None);
    assert_eq!(calls.get(), 0);
}

#[test]
fn one_uses_default_path() {
    let calls = Cell::new(0u32);
    let result = resolve_debug_log_path(Some("1"), || {
        calls.set(calls.get() + 1);
        PathBuf::from("/stub/default.log")
    });
    assert_eq!(result, Some(PathBuf::from("/stub/default.log")));
    assert_eq!(calls.get(), 1);
}

#[test]
fn true_uppercase_uses_default_path() {
    let calls = Cell::new(0u32);
    let result = resolve_debug_log_path(Some("TRUE"), || {
        calls.set(calls.get() + 1);
        PathBuf::from("/stub/default.log")
    });
    assert_eq!(result, Some(PathBuf::from("/stub/default.log")));
    assert_eq!(calls.get(), 1);
}

#[test]
fn explicit_path_used_verbatim() {
    let calls = Cell::new(0u32);
    let result = resolve_debug_log_path(Some("/tmp/custom-gbm.log"), || {
        calls.set(calls.get() + 1);
        PathBuf::from("/stub/default.log")
    });
    assert_eq!(result, Some(PathBuf::from("/tmp/custom-gbm.log")));
    assert_eq!(calls.get(), 0);
}
