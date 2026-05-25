/*!
tool-call-budgets: per-tool call-count caps for AI agent runs.

Set a maximum number of times each tool may be called per run. Raises
`BudgetExceeded` when a tool hits its cap. Composes with `token-budget-pool`
and `llm-budget-window` for layered spending control.

```rust
use tool_call_budgets::{BudgetStore, BudgetExceeded};

let mut store = BudgetStore::new();
store.set_budget("search", 3);
store.check_and_record("search").unwrap();
store.check_and_record("search").unwrap();
store.check_and_record("search").unwrap();
let err = store.check_and_record("search").unwrap_err();
assert_eq!(err.tool_name, "search");
assert_eq!(err.limit, 3);
```
*/

use serde_json::Value;
use std::collections::HashMap;

/// Error raised when a tool exceeds its call budget.
#[derive(Debug, Clone, PartialEq)]
pub struct BudgetExceeded {
    pub tool_name: String,
    pub limit: usize,
    pub count: usize,
}

impl std::fmt::Display for BudgetExceeded {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "BudgetExceeded: tool '{}' called {} times, limit is {}",
            self.tool_name, self.count, self.limit
        )
    }
}

impl std::error::Error for BudgetExceeded {}

/// Per-tool call-count budget store.
#[derive(Debug, Default, Clone)]
pub struct BudgetStore {
    budgets: HashMap<String, usize>,
    counts: HashMap<String, usize>,
    /// Default budget applied to tools without an explicit entry.
    pub default_budget: Option<usize>,
}

impl BudgetStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the call limit for a named tool.
    pub fn set_budget(&mut self, tool_name: &str, limit: usize) {
        self.budgets.insert(tool_name.to_owned(), limit);
    }

    /// Set a default budget for all tools not explicitly configured.
    pub fn set_default_budget(&mut self, limit: usize) {
        self.default_budget = Some(limit);
    }

    /// Current call count for a tool.
    pub fn count(&self, tool_name: &str) -> usize {
        *self.counts.get(tool_name).unwrap_or(&0)
    }

    /// Remaining calls for a tool. `None` if no budget is set.
    pub fn remaining(&self, tool_name: &str) -> Option<usize> {
        let limit = self.limit_for(tool_name)?;
        let used = self.count(tool_name);
        Some(limit.saturating_sub(used))
    }

    fn limit_for(&self, tool_name: &str) -> Option<usize> {
        self.budgets
            .get(tool_name)
            .copied()
            .or(self.default_budget)
    }

    /// Check without recording. Returns `Err(BudgetExceeded)` if at limit.
    pub fn check(&self, tool_name: &str) -> Result<(), BudgetExceeded> {
        if let Some(limit) = self.limit_for(tool_name) {
            let count = self.count(tool_name);
            if count >= limit {
                return Err(BudgetExceeded {
                    tool_name: tool_name.to_owned(),
                    limit,
                    count,
                });
            }
        }
        Ok(())
    }

    /// Increment the count for a tool (no budget check).
    pub fn record(&mut self, tool_name: &str) {
        *self.counts.entry(tool_name.to_owned()).or_insert(0) += 1;
    }

    /// Check the budget and, if OK, increment the count atomically.
    pub fn check_and_record(&mut self, tool_name: &str) -> Result<(), BudgetExceeded> {
        self.check(tool_name)?;
        self.record(tool_name);
        Ok(())
    }

    /// Reset the count for a specific tool.
    pub fn reset(&mut self, tool_name: &str) {
        self.counts.remove(tool_name);
    }

    /// Reset all counts.
    pub fn reset_all(&mut self) {
        self.counts.clear();
    }

    /// All current counts.
    pub fn counts(&self) -> &HashMap<String, usize> {
        &self.counts
    }

    /// All configured budgets.
    pub fn budgets(&self) -> &HashMap<String, usize> {
        &self.budgets
    }

    /// Serialize current state to JSON.
    pub fn to_json(&self) -> Value {
        let budgets: serde_json::Map<String, Value> = self
            .budgets
            .iter()
            .map(|(k, v)| (k.clone(), Value::from(*v)))
            .collect();
        let counts: serde_json::Map<String, Value> = self
            .counts
            .iter()
            .map(|(k, v)| (k.clone(), Value::from(*v)))
            .collect();
        serde_json::json!({
            "budgets": budgets,
            "counts": counts,
            "default_budget": self.default_budget,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_budget_always_ok() {
        let mut s = BudgetStore::new();
        for _ in 0..100 {
            s.check_and_record("unlimited").unwrap();
        }
        assert_eq!(s.count("unlimited"), 100);
    }

    #[test]
    fn budget_exceeded_at_limit() {
        let mut s = BudgetStore::new();
        s.set_budget("t", 2);
        s.check_and_record("t").unwrap();
        s.check_and_record("t").unwrap();
        let err = s.check_and_record("t").unwrap_err();
        assert_eq!(err.tool_name, "t");
        assert_eq!(err.limit, 2);
        assert_eq!(err.count, 2);
    }

    #[test]
    fn check_does_not_increment() {
        let mut s = BudgetStore::new();
        s.set_budget("t", 5);
        for _ in 0..4 {
            s.check("t").unwrap();
        }
        assert_eq!(s.count("t"), 0);
    }

    #[test]
    fn remaining_decrements() {
        let mut s = BudgetStore::new();
        s.set_budget("t", 3);
        assert_eq!(s.remaining("t"), Some(3));
        s.record("t");
        assert_eq!(s.remaining("t"), Some(2));
        s.record("t");
        s.record("t");
        assert_eq!(s.remaining("t"), Some(0));
    }

    #[test]
    fn remaining_none_without_budget() {
        let s = BudgetStore::new();
        assert_eq!(s.remaining("t"), None);
    }

    #[test]
    fn reset_clears_count() {
        let mut s = BudgetStore::new();
        s.set_budget("t", 5);
        s.record("t");
        s.record("t");
        s.reset("t");
        assert_eq!(s.count("t"), 0);
        s.check_and_record("t").unwrap();
    }

    #[test]
    fn reset_all_clears_all() {
        let mut s = BudgetStore::new();
        s.set_budget("a", 5);
        s.set_budget("b", 5);
        s.record("a");
        s.record("b");
        s.reset_all();
        assert_eq!(s.count("a"), 0);
        assert_eq!(s.count("b"), 0);
    }

    #[test]
    fn default_budget_applies() {
        let mut s = BudgetStore::new();
        s.set_default_budget(2);
        s.check_and_record("any_tool").unwrap();
        s.check_and_record("any_tool").unwrap();
        assert!(s.check_and_record("any_tool").is_err());
    }

    #[test]
    fn explicit_budget_overrides_default() {
        let mut s = BudgetStore::new();
        s.set_default_budget(1);
        s.set_budget("special", 5);
        for _ in 0..5 {
            s.check_and_record("special").unwrap();
        }
        assert!(s.check_and_record("special").is_err());
    }

    #[test]
    fn zero_budget_immediately_fails() {
        let mut s = BudgetStore::new();
        s.set_budget("t", 0);
        let err = s.check_and_record("t").unwrap_err();
        assert_eq!(err.count, 0);
    }

    #[test]
    fn budget_exceeded_display() {
        let err = BudgetExceeded {
            tool_name: "search".to_string(),
            limit: 3,
            count: 3,
        };
        let s = err.to_string();
        assert!(s.contains("search"));
        assert!(s.contains("3"));
    }

    #[test]
    fn multiple_tools_independent() {
        let mut s = BudgetStore::new();
        s.set_budget("a", 1);
        s.set_budget("b", 2);
        s.check_and_record("a").unwrap();
        assert!(s.check("a").is_err());
        s.check_and_record("b").unwrap();
        s.check_and_record("b").unwrap();
        assert!(s.check("b").is_err());
    }

    #[test]
    fn counts_map_has_recorded_entries() {
        let mut s = BudgetStore::new();
        s.record("t");
        s.record("t");
        assert_eq!(s.counts()["t"], 2);
    }

    #[test]
    fn to_json_has_fields() {
        let mut s = BudgetStore::new();
        s.set_budget("t", 3);
        s.record("t");
        let j = s.to_json();
        assert_eq!(j["budgets"]["t"], 3);
        assert_eq!(j["counts"]["t"], 1);
    }
}
