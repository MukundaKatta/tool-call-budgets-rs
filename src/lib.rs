/*!
tool-call-budgets: per-tool call-count caps for a single AI agent run.

```rust
use tool_call_budgets::{ToolBudgets, BudgetError};

let mut budgets = ToolBudgets::new([("search", 3), ("fetch", 5)], true);
let mut ctx = budgets.run().unwrap();
ctx.record("search").unwrap();
ctx.record("search").unwrap();
ctx.record("search").unwrap();
let err = ctx.record("search");
assert!(matches!(err, Err(BudgetError::ToolBudgetExceeded { .. })));
let report = ctx.report();
assert_eq!(report.breach_count, 1);
```
*/

use std::collections::HashMap;

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

// ---- errors ---------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BudgetError {
    /// A tool exceeded its configured cap.
    ToolBudgetExceeded { tool: String, cap: u64, used: u64 },
    /// Strict mode: tool not registered in caps.
    UnknownTool(String),
    /// record() called after close().
    RunContextClosed,
    /// A run is already active on this ToolBudgets instance.
    AlreadyActive,
}

impl std::fmt::Display for BudgetError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BudgetError::ToolBudgetExceeded { tool, cap, .. } => {
                write!(f, "{tool} hit cap of {cap}")
            }
            BudgetError::UnknownTool(t) => {
                write!(f, "tool {t:?} is not in caps and strict=true")
            }
            BudgetError::RunContextClosed => write!(f, "RunContext is already closed"),
            BudgetError::AlreadyActive => {
                write!(f, "another RunContext is already active on this ToolBudgets")
            }
        }
    }
}

impl std::error::Error for BudgetError {}

// ---- BudgetReport ---------------------------------------------------------

/// Snapshot of a run's budget state.
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct BudgetReport {
    pub usage: HashMap<String, u64>,
    pub caps: HashMap<String, u64>,
    pub remaining: HashMap<String, u64>,
    pub aborted_on: Option<String>,
    pub breach_count: u64,
}

impl BudgetReport {
    /// Per-tool utilization fraction. cap=0 with any usage → f64::INFINITY.
    pub fn utilization(&self) -> HashMap<String, f64> {
        self.caps
            .iter()
            .map(|(tool, &cap)| {
                let used = *self.usage.get(tool).unwrap_or(&0);
                let u = if cap == 0 {
                    if used > 0 { f64::INFINITY } else { 0.0 }
                } else {
                    used as f64 / cap as f64
                };
                (tool.clone(), u)
            })
            .collect()
    }
}

// ---- RunContext ------------------------------------------------------------

/// Per-run accounting for tool call budgets.
///
/// Obtain one via [`ToolBudgets::run`]. Not thread-safe.
pub struct RunContext {
    caps: HashMap<String, u64>,
    usage: HashMap<String, u64>,
    strict: bool,
    aborted_on: Option<String>,
    breach_count: u64,
    pub(crate) closed: bool,
}

impl RunContext {
    fn new(caps: HashMap<String, u64>, strict: bool) -> Self {
        let usage = caps.keys().map(|k| (k.clone(), 0u64)).collect();
        Self {
            caps,
            usage,
            strict,
            aborted_on: None,
            breach_count: 0,
            closed: false,
        }
    }

    /// Record one call to `tool`. Returns new usage count or BudgetError.
    pub fn record(&mut self, tool: &str) -> Result<u64, BudgetError> {
        if self.closed {
            return Err(BudgetError::RunContextClosed);
        }
        if !self.caps.contains_key(tool) {
            if self.strict {
                return Err(BudgetError::UnknownTool(tool.to_owned()));
            }
            return Ok(0);
        }
        let new_count = self.usage[tool] + 1;
        let cap = self.caps[tool];
        if new_count > cap {
            self.breach_count += 1;
            if self.aborted_on.is_none() {
                self.aborted_on = Some(tool.to_owned());
            }
            *self.usage.get_mut(tool).unwrap() = new_count;
            return Err(BudgetError::ToolBudgetExceeded {
                tool: tool.to_owned(),
                cap,
                used: new_count,
            });
        }
        *self.usage.get_mut(tool).unwrap() = new_count;
        Ok(new_count)
    }

    /// Calls recorded so far for `tool`.
    pub fn used(&self, tool: &str) -> u64 {
        *self.usage.get(tool).unwrap_or(&0)
    }

    /// Calls remaining before breach. Floors at 0.
    pub fn remaining(&self, tool: &str) -> u64 {
        if let Some(&cap) = self.caps.get(tool) {
            let used = *self.usage.get(tool).unwrap_or(&0);
            cap.saturating_sub(used)
        } else {
            0
        }
    }

    /// Snapshot the current state as an immutable report.
    pub fn report(&self) -> BudgetReport {
        let remaining = self
            .caps
            .iter()
            .map(|(tool, &cap)| {
                let used = *self.usage.get(tool).unwrap_or(&0);
                (tool.clone(), cap.saturating_sub(used))
            })
            .collect();
        BudgetReport {
            usage: self.usage.clone(),
            caps: self.caps.clone(),
            remaining,
            aborted_on: self.aborted_on.clone(),
            breach_count: self.breach_count,
        }
    }

    /// Close the context. Further record() calls return RunContextClosed.
    pub fn close(&mut self) {
        self.closed = true;
    }

    pub fn is_closed(&self) -> bool {
        self.closed
    }
}

impl Drop for RunContext {
    fn drop(&mut self) {
        self.closed = true;
    }
}

// ---- ToolBudgets ----------------------------------------------------------

/// Configure per-tool call-count caps and hand out per-run contexts.
///
/// Reusable across runs; each [`ToolBudgets::run`] returns a fresh [`RunContext`].
pub struct ToolBudgets {
    caps: HashMap<String, u64>,
    strict: bool,
    active: bool,
}

impl ToolBudgets {
    /// Create with caps from any `(name, cap)` iterable.
    ///
    /// `strict=true` makes `record()` fail on unknown tool names.
    pub fn new<I, S>(caps: I, strict: bool) -> Self
    where
        I: IntoIterator<Item = (S, u64)>,
        S: Into<String>,
    {
        Self {
            caps: caps.into_iter().map(|(k, v)| (k.into(), v)).collect(),
            strict,
            active: false,
        }
    }

    /// Caps accessor.
    pub fn caps(&self) -> &HashMap<String, u64> {
        &self.caps
    }

    /// Open a fresh RunContext. Only one can be active at a time.
    pub fn run(&mut self) -> Result<RunContext, BudgetError> {
        if self.active {
            return Err(BudgetError::AlreadyActive);
        }
        self.active = true;
        Ok(RunContext::new(self.caps.clone(), self.strict))
    }

    /// Mark no active run (call after dropping the context if needed).
    pub fn release(&mut self) {
        self.active = false;
    }
}

// ---- tests ----------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn budgets(caps: &[(&str, u64)]) -> ToolBudgets {
        ToolBudgets::new(caps.iter().map(|(k, v)| (*k, *v)), true)
    }

    #[test]
    fn basic_record_and_report() {
        let mut b = budgets(&[("search", 3), ("fetch", 5)]);
        let mut ctx = b.run().unwrap();
        assert_eq!(ctx.record("search").unwrap(), 1);
        assert_eq!(ctx.record("search").unwrap(), 2);
        assert_eq!(ctx.record("search").unwrap(), 3);
        let r = ctx.report();
        assert_eq!(r.usage["search"], 3);
        assert_eq!(r.remaining["search"], 0);
        assert_eq!(r.breach_count, 0);
        assert!(r.aborted_on.is_none());
    }

    #[test]
    fn breach_raises() {
        let mut b = budgets(&[("search", 2)]);
        let mut ctx = b.run().unwrap();
        ctx.record("search").unwrap();
        ctx.record("search").unwrap();
        let err = ctx.record("search").unwrap_err();
        assert!(matches!(
            err,
            BudgetError::ToolBudgetExceeded { tool, cap: 2, used: 3 } if tool == "search"
        ));
        let r = ctx.report();
        assert_eq!(r.breach_count, 1);
        assert_eq!(r.aborted_on.as_deref(), Some("search"));
    }

    #[test]
    fn cap_zero_means_no_calls() {
        let mut b = budgets(&[("forbidden", 0)]);
        let mut ctx = b.run().unwrap();
        let err = ctx.record("forbidden").unwrap_err();
        assert!(matches!(err, BudgetError::ToolBudgetExceeded { cap: 0, .. }));
    }

    #[test]
    fn unknown_tool_strict() {
        let mut b = budgets(&[("search", 5)]);
        let mut ctx = b.run().unwrap();
        let err = ctx.record("ghost").unwrap_err();
        assert!(matches!(err, BudgetError::UnknownTool(t) if t == "ghost"));
    }

    #[test]
    fn unknown_tool_non_strict() {
        let mut b = ToolBudgets::new([("search", 5u64)], false);
        let mut ctx = b.run().unwrap();
        assert_eq!(ctx.record("ghost").unwrap(), 0);
    }

    #[test]
    fn closed_context_errors() {
        let mut b = budgets(&[("search", 5)]);
        let mut ctx = b.run().unwrap();
        ctx.close();
        assert!(matches!(ctx.record("search"), Err(BudgetError::RunContextClosed)));
    }

    #[test]
    fn only_one_active_run() {
        let mut b = budgets(&[("search", 5)]);
        let _ctx = b.run().unwrap();
        assert!(matches!(b.run(), Err(BudgetError::AlreadyActive)));
    }

    #[test]
    fn remaining_counts_down() {
        let mut b = budgets(&[("a", 4)]);
        let mut ctx = b.run().unwrap();
        assert_eq!(ctx.remaining("a"), 4);
        ctx.record("a").unwrap();
        assert_eq!(ctx.remaining("a"), 3);
        ctx.record("a").unwrap();
        ctx.record("a").unwrap();
        ctx.record("a").unwrap();
        assert_eq!(ctx.remaining("a"), 0);
    }

    #[test]
    fn used_returns_count() {
        let mut b = budgets(&[("b", 10)]);
        let mut ctx = b.run().unwrap();
        assert_eq!(ctx.used("b"), 0);
        ctx.record("b").unwrap();
        ctx.record("b").unwrap();
        assert_eq!(ctx.used("b"), 2);
    }

    #[test]
    fn multiple_tools_independent() {
        let mut b = budgets(&[("a", 2), ("b", 3)]);
        let mut ctx = b.run().unwrap();
        ctx.record("a").unwrap();
        ctx.record("a").unwrap();
        ctx.record("b").unwrap();
        assert!(matches!(ctx.record("a"), Err(BudgetError::ToolBudgetExceeded { .. })));
        ctx.record("b").unwrap();
        ctx.record("b").unwrap();
        let r = ctx.report();
        assert_eq!(r.usage["a"], 3); // one breach still tracked
        assert_eq!(r.usage["b"], 3);
        assert_eq!(r.breach_count, 1);
    }

    #[test]
    fn report_remaining_floors_at_zero() {
        let mut b = budgets(&[("x", 1)]);
        let mut ctx = b.run().unwrap();
        ctx.record("x").unwrap();
        let _ = ctx.record("x"); // breach
        let r = ctx.report();
        assert_eq!(r.remaining["x"], 0);
    }

    #[test]
    fn utilization_normal() {
        let mut b = budgets(&[("a", 4)]);
        let mut ctx = b.run().unwrap();
        ctx.record("a").unwrap();
        ctx.record("a").unwrap();
        let r = ctx.report();
        let u = r.utilization();
        assert!((u["a"] - 0.5).abs() < 1e-9);
    }

    #[test]
    fn utilization_zero_cap_no_usage() {
        let mut b = budgets(&[("a", 0)]);
        let ctx = b.run().unwrap();
        let r = ctx.report();
        let u = r.utilization();
        assert_eq!(u["a"], 0.0);
    }

    #[test]
    fn utilization_zero_cap_with_breach() {
        let mut b = budgets(&[("a", 0)]);
        let mut ctx = b.run().unwrap();
        let _ = ctx.record("a"); // breach
        let r = ctx.report();
        let u = r.utilization();
        assert_eq!(u["a"], f64::INFINITY);
    }

    #[test]
    fn release_allows_new_run() {
        let mut b = budgets(&[("a", 5)]);
        {
            let mut ctx = b.run().unwrap();
            ctx.record("a").unwrap();
            ctx.close();
        }
        b.release();
        let mut ctx2 = b.run().unwrap();
        assert_eq!(ctx2.record("a").unwrap(), 1); // fresh context
    }

    #[test]
    fn breach_count_accumulates_with_multiple_calls() {
        let mut b = ToolBudgets::new([("a", 0u64)], true);
        let mut ctx = b.run().unwrap();
        for _ in 0..3 {
            let _ = ctx.record("a");
        }
        let r = ctx.report();
        assert_eq!(r.breach_count, 3);
        assert_eq!(r.aborted_on.as_deref(), Some("a")); // set on first breach
    }

    #[test]
    fn empty_caps() {
        let mut b = ToolBudgets::new::<Vec<(&str, u64)>, _>(vec![], true);
        let mut ctx = b.run().unwrap();
        assert!(matches!(ctx.record("x"), Err(BudgetError::UnknownTool(_))));
        let r = ctx.report();
        assert_eq!(r.breach_count, 0); // strict error before breach recorded
    }

    #[test]
    fn used_unknown_tool_returns_zero() {
        let mut b = budgets(&[("a", 5)]);
        let ctx = b.run().unwrap();
        assert_eq!(ctx.used("never_seen"), 0);
    }

    #[test]
    fn remaining_unknown_tool_returns_zero() {
        let mut b = budgets(&[("a", 5)]);
        let ctx = b.run().unwrap();
        assert_eq!(ctx.remaining("never_seen"), 0);
    }

    #[test]
    fn report_clone() {
        let mut b = budgets(&[("a", 3)]);
        let mut ctx = b.run().unwrap();
        ctx.record("a").unwrap();
        let r1 = ctx.report();
        ctx.record("a").unwrap();
        let r2 = ctx.report();
        assert_eq!(r1.usage["a"], 1);
        assert_eq!(r2.usage["a"], 2);
    }
}
