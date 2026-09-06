# Sync Command Design

## Goal

Make `si sync` the sole public command for reconciling canonical skills into
configured agent directories. `si apply` is removed without a compatibility
alias.

## Public behavior

`si sync` creates missing managed links, repairs managed-link drift, replaces
identical physical duplicates, and removes stale managed links after a skill is
removed from the canonical directory. It never fetches, pulls, commits, or
pushes Git state.

## Implementation

Rename the Clap command variant and all user-facing command recommendations to
`sync`. The existing `apply` module and its planner types remain internal to
avoid an implementation-only refactor. Update README, PRD, specs, and CLI
tests so help exposes `sync` and rejects `apply`.
