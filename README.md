# tool-call-budgets

Per-tool call-count caps for AI agent runs. Stops runaway tool loops before they hit your invoice.

## Usage

```rust
use tool_call_budgets::{ToolBudgets, BudgetError};

let mut budgets = ToolBudgets::new([
    ("search", 5u64),
    ("fetch_url", 10),
], true);

let mut ctx = budgets.run().unwrap();

ctx.record("search").unwrap();
ctx.record("search").unwrap();

let report = ctx.report();
println!("search used: {}", report.usage["search"]);
println!("search remaining: {}", report.remaining["search"]);

// Breaches raise an error
for _ in 0..4 {
    let _ = ctx.record("search"); // 3rd–5th are fine, 6th breaches
}
```

## Features

- Per-tool caps as a `HashMap<String, u64>`
- `record()` returns the new count or `BudgetError::ToolBudgetExceeded`
- `strict` mode raises `BudgetError::UnknownTool` for unregistered tools
- `BudgetReport` snapshot with usage, caps, remaining, breach_count
- Optional `serde` feature for JSON serialization of reports

## License

MIT OR Apache-2.0
