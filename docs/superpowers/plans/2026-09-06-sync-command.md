# Sync Command Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the public `si apply` command with `si sync` without compatibility support.

**Architecture:** Rename only the Clap command variant and user-facing wording. Keep the existing `apply` module as an internal implementation detail.

**Tech Stack:** Rust, clap, assert_cmd, Markdown documentation.

---

### Task 1: Rename the public CLI command

**Files:**
- Modify: `src/cli.rs`
- Modify: `src/lib.rs`
- Test: `tests/cli.rs`

- [ ] **Step 1: Write failing command-surface tests**

```rust
assert!(help.contains("  sync"));
assert!(!help.contains("  apply"));
cargo_bin_cmd!("si").arg("apply").assert().failure();
```

- [ ] **Step 2: Run the focused test**

Run: `cargo test --test cli help_lists_only_the_simple_public_workflows`

Expected: FAIL because help still presents `apply`.

- [ ] **Step 3: Rename the Clap variant**

```rust
Sync,
```

Dispatch `Command::Sync` to `apply::run`, and change status recommendations
from `si apply` to `si sync`.

- [ ] **Step 4: Verify command behavior**

Run: `cargo test --test cli`

Expected: PASS; `si sync` is accepted and `si apply` is rejected.

### Task 2: Rename user-facing documentation

**Files:**
- Modify: `README.md`
- Modify: `prd.md`
- Modify: `docs/superpowers/specs/2026-09-06-simple-cli-design.md`

- [ ] **Step 1: Replace public command examples and references**

Use `si sync` for every local reconciliation workflow. Preserve the statement
that Git remote operations are outside `si`.

- [ ] **Step 2: Confirm no public `si apply` reference remains**

Run: `rg -n 'si apply|`apply` reconciles|## `si apply`' README.md prd.md docs/superpowers/specs`

Expected: no matches.

### Task 3: Full verification and commit

**Files:**
- Modify: `src/cli.rs`, `src/lib.rs`, `tests/cli.rs`, documentation files

- [ ] **Step 1: Run checks**

Run: `cargo fmt --check && cargo test && cargo clippy -- -D warnings && git diff --check`

Expected: all commands exit 0.

- [ ] **Step 2: Commit**

```bash
git add src/cli.rs src/lib.rs tests/cli.rs README.md prd.md docs/superpowers
git commit -m "feat: rename apply command to sync"
```
