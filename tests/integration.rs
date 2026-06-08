//! Integration tests that exercise the public API exactly as an external
//! crate would (no access to private internals).

use tool_call_budgets::{BudgetExceeded, BudgetStore};

#[test]
fn agent_loop_stops_at_cap() {
    let mut store = BudgetStore::new();
    store.set_budget("search", 3);

    let mut allowed = 0usize;
    let mut blocked = 0usize;

    // Simulate an agent that wants to call "search" five times.
    for _ in 0..5 {
        match store.check_and_record("search") {
            Ok(()) => allowed += 1,
            Err(BudgetExceeded {
                tool_name, limit, ..
            }) => {
                assert_eq!(tool_name, "search");
                assert_eq!(limit, 3);
                blocked += 1;
            }
        }
    }

    assert_eq!(allowed, 3);
    assert_eq!(blocked, 2);
    assert_eq!(store.count("search"), 3);
    assert_eq!(store.remaining("search"), Some(0));
    assert!(store.at_budget("search"));
}

#[test]
fn default_and_explicit_budgets_coexist() {
    let mut store = BudgetStore::new();
    store.set_default_budget(1);
    store.set_budget("special", 3);

    // Default-budgeted tool: one call then blocked.
    store.check_and_record("ordinary").unwrap();
    assert!(store.check_and_record("ordinary").is_err());

    // Explicit budget overrides the default.
    for _ in 0..3 {
        store.check_and_record("special").unwrap();
    }
    assert!(store.check_and_record("special").is_err());
    assert!(store.at_budget("special"));
    assert!(store.at_budget("ordinary"));
}

#[test]
fn unbudgeted_tool_never_blocks() {
    let mut store = BudgetStore::new();
    for _ in 0..50 {
        store.check_and_record("free").unwrap();
    }
    assert_eq!(store.count("free"), 50);
    assert_eq!(store.remaining("free"), None);
    assert!(!store.at_budget("free"));
}

#[test]
fn reset_allows_reuse_within_same_store() {
    let mut store = BudgetStore::new();
    store.set_budget("t", 1);
    store.check_and_record("t").unwrap();
    assert!(store.check_and_record("t").is_err());

    store.reset("t");
    assert_eq!(store.count("t"), 0);
    store.check_and_record("t").unwrap();
}

#[test]
fn total_calls_aggregates_across_tools() {
    let mut store = BudgetStore::new();
    store.record("a");
    store.record("a");
    store.record("b");
    assert_eq!(store.total_calls(), 3);

    store.reset_all();
    assert_eq!(store.total_calls(), 0);
}

#[test]
fn json_snapshot_round_trips_fields() {
    let mut store = BudgetStore::new();
    store.set_budget("search", 5);
    store.set_default_budget(2);
    store.check_and_record("search").unwrap();

    let json = store.to_json();
    assert_eq!(json["budgets"]["search"], 5);
    assert_eq!(json["counts"]["search"], 1);
    assert_eq!(json["default_budget"], 2);
}

#[test]
fn zero_budget_blocks_first_call() {
    let mut store = BudgetStore::new();
    store.set_budget("forbidden", 0);
    let err = store.check_and_record("forbidden").unwrap_err();
    assert_eq!(err.count, 0);
    assert_eq!(err.limit, 0);
    assert!(store.at_budget("forbidden"));
}
