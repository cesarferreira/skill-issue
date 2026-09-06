# Changelog

All notable changes to this project will be documented in this file.

## [0.2.0] - 2026-09-06

### 🚀 Features

- Discover project-local Claude and `.agents` skills with `scan --project`.
- Report canonical-root Git health with `status --git`.
- Restore every canonical skill into configured targets.
- Emit JSON from read commands and generate shell completions.
- Support configurable relative symlinks and fingerprint ignore patterns.

### 🛡️ Safety

- Preserve permissions, timestamps, and extended attributes during cross-filesystem copies.
- Support explicit non-interactive confirmation with `--yes` while conflicts still require a TTY.
- Detect recovery artifacts left by interrupted migrations.

## [0.1.0] - 2026-09-06

### 🚀 Features

- Discover and fingerprint skills across built-in and custom agent targets.
- Safely adopt physical copies into one canonical directory with verified symlinks.
- Diagnose divergence, broken links, unsafe topology, and interrupted migrations.
- Preview all filesystem mutations with dry-run operation plans.
