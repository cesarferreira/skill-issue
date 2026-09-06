<div align="center">
  <h1>skillissue</h1>

  <p><strong>Deduplicate agent skills with safe canonical symlinks</strong></p>

  <p>
    <img alt="License" src="https://img.shields.io/badge/license-MIT-green">
    <img alt="Rust" src="https://img.shields.io/badge/rust-1.85%2B-orange">
    <img alt="Edition" src="https://img.shields.io/badge/edition-2024-blue">
    <a href="https://crates.io/crates/skillissue"><img alt="crates.io" src="https://img.shields.io/crates/v/skillissue.svg"></a>
  </p>

  <p>
    <a href="#install">Install</a>
    &nbsp;·&nbsp;
    <a href="#quickstart">Quickstart</a>
  </p>
</div>

---

## Install

Requires [Rust](https://rustup.rs) **1.85+** and `~/.cargo/bin` on your `PATH`.

```bash
cargo install skillissue
```

Verify:

```bash
skillissue --help
```

<details>
<summary><strong>Build from source</strong> — for development or unreleased changes</summary>

```bash
git clone https://github.com/cesarferreira/skill-issue.git
cd skill-issue
cargo install --path . --locked
# or
make install-release
```

Debug install (faster compile, larger binary):

```bash
make install
```

Run without installing:

```bash
make build-release
./target/release/skillissue
```

</details>

<a id="quickstart"></a>
## Quickstart

```bash
# Choose the directory that will hold the real copies.
skillissue init ~/code/skills

# Inspect skills found in Claude Code, Codex, Gemini, and .agents.
skillissue scan

# Preview the exact migration without changing anything.
skillissue adopt --dry-run

# Run the guided migration. Destructive confirmations default to No.
skillissue

# Verify every canonical skill and symlink.
skillissue doctor
```

`skillissue` compares directory contents rather than timestamps. Identical
copies are moved into the canonical directory and their original locations are
replaced with symlinks. Divergent copies are never overwritten: the guided flow
asks which version to adopt or lets you keep each version under a separate
name.

Common follow-up operations:

```bash
skillissue diff rust-cli --content
skillissue targets add opencode ~/.config/opencode/skills
skillissue link rust-cli claude codex
skillissue link --all --target gemini
skillissue unlink rust-cli codex
```

Every mutating command supports `--dry-run`. Colour is disabled by `NO_COLOR`
or `--no-color`.

## License

MIT
