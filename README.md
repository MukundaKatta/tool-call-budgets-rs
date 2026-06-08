# tool-call-budgets

[![CI](https://github.com/MukundaKatta/tool-call-budgets-rs/actions/workflows/ci.yml/badge.svg)](https://github.com/MukundaKatta/tool-call-budgets-rs/actions/workflows/ci.yml)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](#license)

Per-tool call-count caps for AI agent runs. Set a maximum number of times each
tool may be called during a single run and stop runaway tool loops before they
hit your invoice.

The crate is tiny, dependency-light, and has no async or runtime requirements —
it is just a `HashMap`-backed counter with budget checks, so it drops into any
agent loop regardless of which LLM client or framework you use.

## Why

When an LLM-driven agent gets stuck, it often calls the same tool over and over
(retrying a failing `fetch_url`, re-running `search`, etc.). Each call may cost
money or hit a rate limit. `tool-call-budgets` lets you declare "this tool may
be called at most N times per run" and surfaces a typed error the moment a tool
would exceed its cap, so your loop can stop or degrade gracefully.

## Install

Add it to your `Cargo.toml`:

```toml
[dependencies]
tool-call-budgets = "0.1"
```

Or from the command line:

```sh
cargo add tool-call-budgets
```

## Usage

```rust
use tool_call_budgets::{BudgetStore, BudgetExceeded};

let mut store = BudgetStore::new();

// "search" may be called at most 3 times this run.
store.set_budget("search", 3);

// Every tool without an explicit budget gets this cap.
store.set_default_budget(10);

// Before invoking a tool, check-and-record atomically.
store.check_and_record("search").unwrap(); // count -> 1
store.check_and_record("search").unwrap(); // count -> 2
store.check_and_record("search").unwrap(); // count -> 3

// The 4th call would exceed the cap of 3.
match store.check_and_record("search") {
    Ok(()) => println!("call allowed"),
    Err(BudgetExceeded { tool_name, limit, count }) => {
        println!("'{tool_name}' is capped at {limit} (already used {count})");
    }
}

// Inspect remaining budget at any time.
assert_eq!(store.remaining("search"), Some(0));
assert_eq!(store.count("search"), 3);
```

### In an agent loop

```rust
use tool_call_budgets::BudgetStore;

let mut store = BudgetStore::new();
store.set_budget("fetch_url", 5);

# let tool_calls = ["fetch_url", "fetch_url"];
for tool_name in tool_calls {
    match store.check_and_record(tool_name) {
        Ok(()) => {
            // call the actual tool here
        }
        Err(e) => {
            eprintln!("budget hit, skipping: {e}");
            // break, fall back, or ask the model to stop
        }
    }
}
```

## API

`BudgetStore` is the single entry point. All counts and budgets are keyed by
tool name (`&str`).

| Method | Description |
| --- | --- |
| `BudgetStore::new()` | Create an empty store (no budgets, no counts). |
| `set_budget(tool, limit)` | Set the per-run call cap for a named tool. |
| `set_default_budget(limit)` | Cap every tool that has no explicit budget. |
| `check(tool)` | Return `Err(BudgetExceeded)` if the tool is at its cap; does **not** record. |
| `record(tool)` | Increment a tool's count with no budget check. |
| `check_and_record(tool)` | Check the budget and, if within limits, increment atomically. |
| `count(tool)` | Current call count for a tool (`0` if never called). |
| `remaining(tool)` | Remaining calls, or `None` if the tool has no budget. |
| `reset(tool)` | Reset a single tool's count to zero. |
| `reset_all()` | Reset every count to zero (budgets are kept). |
| `counts()` | Borrow the full `HashMap` of current counts. |
| `budgets()` | Borrow the full `HashMap` of configured budgets. |
| `to_json()` | Snapshot budgets, counts, and the default budget as a `serde_json::Value`. |

`default_budget` is also exposed as a public field if you prefer to read or set
it directly.

### Errors

`check` and `check_and_record` return `Result<(), BudgetExceeded>`.
`BudgetExceeded` implements `std::error::Error` and `Display`, and carries:

- `tool_name: String` — the tool that hit its cap
- `limit: usize` — the configured cap
- `count: usize` — the count at the time of the breach

### Semantics

- A tool with **no budget and no default** is never capped — calls always succeed.
- A budget of `0` means the tool may never be called: the first
  `check_and_record` fails immediately.
- An explicit `set_budget` always overrides the default budget for that tool.
- `record` bypasses budget checks; use it only when you intentionally want to
  count a call without enforcement.

## Composes with

`tool-call-budgets` covers *call counts*. For layered spending control it pairs
with `token-budget-pool` (token spend) and `llm-budget-window` (time-windowed
limits).

## License

Licensed under either of

- MIT license ([LICENSE-MIT](LICENSE-MIT))
- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))

at your option.
