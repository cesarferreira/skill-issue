Absolutely. I’d frame skillissue as a deliberately small, safe, local-first CLI: not a package manager, not a skill marketplace, just a tool that finds duplicated agent skills, centralizes them, and replaces the copies with symlinks.

skillissue

Your agents have a skill issue.

A local-first CLI that discovers duplicated AI-agent skills across tools such as Claude Code, Codex, Gemini CLI, Cursor, and generic .agents directories, moves them into one canonical directory, and symlinks every agent back to the canonical copy.

The goal is simple:

A skill should physically exist once, while every compatible agent can use it.

⸻

1. Problem

AI coding agents increasingly support reusable skills/instructions stored in filesystem directories.

Different agents use different locations:

~/.claude/skills/
~/.codex/skills/
~/.gemini/skills/
~/.agents/skills/
project/.claude/skills/
project/.agents/skills/
...

Users who use multiple agents frequently end up with the same skill copied into several locations.

Example:

~/.claude/skills/rust-cli/
~/.codex/skills/rust-cli/
~/.gemini/skills/rust-cli/

These directories begin identical but eventually drift.

Typical problems:

* Which copy is canonical?
* Did I update Claude’s version but forget Codex?
* Are two skills with the same name actually identical?
* How do I back all of these up?
* How do I move my skills to another machine?
* Which agent has which skills?
* Are any symlinks broken?
* Did an agent/plugin overwrite one of my managed skills?
* Why am I maintaining the same files five times?

The underlying problem is not package management.

It is filesystem duplication.

⸻

2. Product Vision

skillissue turns this:

Claude                     Codex
  │                          │
  ▼                          ▼
foo/                       foo/
bar/                       bar/
Gemini                     Agents
  │                          │
  ▼                          ▼
foo/                       foo/
bar/                       baz/

into this:

                ~/skills
                   │
         ┌─────────┼─────────┐
         │         │         │
        foo       bar       baz
         │
         │ symlinks
         │
   ┌─────┼──────┬───────┐
   ▼     ▼      ▼       ▼
Claude Codex  Gemini  .agents

The canonical directory can itself be a Git repository:

~/code/skills/
├── rust-cli/
├── pr-review/
├── android/
└── prd/

Git handles:

* backup
* history
* remote synchronization
* branches
* restoring a machine

skillissue handles:

* discovery
* deduplication
* migration
* linking
* verification

⸻

3. Product Principles

3.1 Local-first

No account.

No server.

No database required for core functionality.

Everything should be understandable from the filesystem.

⸻

3.2 Safe before clever

The tool moves user-authored files.

Data loss is unacceptable.

Any destructive filesystem operation must:

1. be explainable beforehand
2. have a dry-run representation
3. preserve conflicting versions
4. be recoverable whenever practical
5. never silently overwrite different content

⸻

3.3 Symlinks, not synchronization

skillissue should not continuously copy files between directories.

There should be exactly one real copy.

Everything else should point to it.

~/.claude/skills/foo
    ->
~/code/skills/foo

This eliminates synchronization as a problem entirely.

⸻

3.4 Git does backup

Do not build a proprietary backup system.

Instead:

skillissue git-status

may tell users whether their canonical skill directory:

* is a Git repository
* has uncommitted changes
* has an upstream
* has unpushed commits

But Git remains responsible for version control.

⸻

3.5 Human-readable state

Users should be able to inspect the filesystem and understand what happened.

Avoid obscure internal layouts.

A managed installation should simply look like:

~/.claude/skills/foo -> ~/code/skills/foo

⸻

4. Target User

Developers who:

* use multiple AI coding agents
* create or install reusable skills
* maintain personal skills across projects
* switch frequently between Claude Code, Codex, Gemini, Cursor, etc.
* want their skills backed up in Git
* prefer CLI tooling
* understand symlinks and filesystem concepts

The primary initial target is macOS/Linux developers.

Windows support can follow later.

⸻

5. Non-Goals

skillissue is explicitly not:

* a skill marketplace
* a package registry
* an npm replacement
* a dependency resolver
* an agent manager
* a plugin manager
* a desktop app
* a cloud synchronization service
* a skill generator
* a skill installer from GitHub
* an MCP manager
* a prompt manager

Those may be useful products, but they make this product worse.

The core promise should remain:

Find my duplicated skills and make them one.

⸻

6. Terminology

Skill

A directory representing an agent skill.

Typically:

foo/
├── SKILL.md
├── scripts/
└── references/

The exact contents are opaque to skillissue.

⸻

Skill installation

A skill directory visible to an agent.

Example:

~/.claude/skills/foo

⸻

Canonical skill

The real physical copy managed by skillissue.

Example:

~/code/skills/foo

⸻

Target

An agent skill directory into which canonical skills can be linked.

Examples:

~/.claude/skills
~/.codex/skills
~/.gemini/skills
~/.agents/skills

⸻

Managed skill

A skill installation that is currently a symlink to the configured canonical directory.

⸻

Duplicate

Two or more physical skill directories whose contents are identical.

⸻

Conflict / divergence

Two skill directories with the same logical skill name but different contents.

⸻

7. Primary User Journey

First run:

skillissue init

Interactive flow:

skillissue
Where should your skills live?
  ~/skills
> ~/code/skills
  ~/.skills
  Custom...
Canonical directory:
  /Users/cesar/code/skills
Scanning known agent locations...
Found 23 skill installations across 4 agents.
Claude
  8 skills
Codex
  7 skills
Gemini
  5 skills
Agents
  3 skills
14 unique skills detected.
8 skills are duplicated.
2 skills have conflicting copies.
4 skills exist in only one location.
Continue? [Y/n]

Then:

rust-cli
  ~/.claude/skills/rust-cli
  ~/.codex/skills/rust-cli
  ~/.gemini/skills/rust-cli
✓ All 3 copies are identical.
Action:
> Adopt into ~/code/skills/rust-cli
  Skip

Result:

Moving:
~/.claude/skills/rust-cli
        ↓
~/code/skills/rust-cli
Creating links:
~/.claude/skills/rust-cli
  -> ~/code/skills/rust-cli
~/.codex/skills/rust-cli
  -> ~/code/skills/rust-cli
~/.gemini/skills/rust-cli
  -> ~/code/skills/rust-cli
✓ rust-cli adopted

⸻

8. CLI Design

The main interface should feel useful even without knowing any commands.

skillissue

should be equivalent to:

skillissue status

or launch an interactive dashboard if migration is required.

Core commands:

skillissue init
skillissue scan
skillissue adopt
skillissue status
skillissue doctor
skillissue diff
skillissue link
skillissue unlink
skillissue targets
skillissue config

Potential later commands:

skillissue git-status
skillissue export
skillissue restore

Avoid a giant command surface.

⸻

9. skillissue init

Purpose:

Configure the canonical skill directory.

Example:

skillissue init

Interactive:

Where should your canonical skills live?
> ~/code/skills

Then:

✓ Created /Users/cesar/code/skills
✓ Configuration saved
✓ Detected Claude Code
✓ Detected Codex
✓ Detected Gemini
Run:
  skillissue scan

Non-interactive:

skillissue init ~/code/skills

If the directory already exists, use it.

Never delete or restructure its contents automatically.

⸻

10. Configuration

Suggested location:

~/.config/skillissue/config.toml

Example:

root = "/Users/cesar/code/skills"
[targets.claude]
path = "/Users/cesar/.claude/skills"
enabled = true
[targets.codex]
path = "/Users/cesar/.codex/skills"
enabled = true
[targets.gemini]
path = "/Users/cesar/.gemini/skills"
enabled = true
[targets.agents]
path = "/Users/cesar/.agents/skills"
enabled = true

The config should remain tiny.

⸻

11. Agent Discovery

skillissue scan should check known locations.

Initial built-in targets:

Claude Code
~/.claude/skills
Codex
~/.codex/skills
Gemini
~/.gemini/skills
Generic agents
~/.agents/skills

Potential future targets:

Cursor
OpenCode
Amp
Windsurf
project-local agent directories

A target should be represented internally approximately as:

struct Target {
    id: String,
    display_name: String,
    path: PathBuf,
}

No agent-specific logic should exist beyond locating its skill directory.

⸻

12. Custom Targets

Users must be able to add arbitrary skill locations.

Example:

skillissue targets add opencode ~/.config/opencode/skills

List:

skillissue targets

Output:

TARGET       PATH                           STATUS
claude       ~/.claude/skills               ✓
codex        ~/.codex/skills                ✓
gemini       ~/.gemini/skills               ✓
agents       ~/.agents/skills               ✓
opencode     ~/.config/opencode/skills      ✓

Remove:

skillissue targets remove opencode

Removing a target from config must not remove any filesystem data.

⸻

13. Scan

skillissue scan

Search all configured target directories.

Example:

Scanning skill locations...
Claude       11
Codex         9
Gemini        6
Agents        4
30 installations
17 unique skills
IDENTICAL DUPLICATES
rust-cli       claude codex gemini
prd            claude codex
github         claude codex agents
testing        codex gemini
CONFLICTS
android
  claude       9f4c2d
  codex        d71a0e
release
  claude       0c1ae1
  gemini       a884f0
UNIQUE
foo            claude
bar            agents
8 duplicates
2 conflicts
7 unique

scan is read-only.

Always.

⸻

14. Skill Identity

The initial definition of logical identity should simply be:

directory basename

So:

~/.claude/skills/foo
~/.codex/skills/foo

represent the same logical skill.

This is deliberately boring.

Metadata-based identity can be considered later.

⸻

15. Content Comparison

The tool must determine whether two skill directories are equivalent.

Naively comparing mtimes is insufficient.

Generate a deterministic directory fingerprint.

Pseudo algorithm:

walk files recursively
for each regular file:
    relative path
    file content hash
sort by relative path
hash:
    relative path
    content hash

Ignore:

.DS_Store
.git/

Potential configurable ignores later.

Example logical representation:

SKILL.md            sha256:...
scripts/foo.sh      sha256:...
references/bar.md   sha256:...

Then hash the entire manifest.

If fingerprints match:

identical

Otherwise:

diverged

⸻

16. Symlink Awareness

A scan must distinguish:

physical directory
managed symlink
foreign symlink
broken symlink

Example:

rust-cli
  Claude      ✓ managed
  Codex       ✓ managed
  Gemini      ⚠ physical duplicate

A symlink is considered managed if its resolved target is inside the configured canonical root.

⸻

17. Adopt

The central command:

skillissue adopt

It converts existing physical skills into canonical skills + symlinks.

It should operate skill-by-skill.

For identical copies:

rust-cli
Found identical copies:
  ~/.claude/skills/rust-cli
  ~/.codex/skills/rust-cli
  ~/.gemini/skills/rust-cli
Canonical destination:
  ~/code/skills/rust-cli
Proceed? [Y/n]

The operation:

1. choose source copy
2. move it to canonical root
3. verify move
4. remove remaining identical copies
5. create symlinks at every original location
6. verify every link

The source chosen should not matter if all fingerprints are equal.

⸻

18. Existing Canonical Skill

If:

~/code/skills/foo

already exists, compare it with discovered installations.

Identical:

foo
Canonical copy already exists.
✓ Claude matches
✓ Codex matches
Replace physical copies with links? [Y/n]

Different:

foo
⚠ Canonical copy differs from Claude.
Canonical
  8d91cf
Claude
  f82ab4
No files were changed.
Run:
  skillissue diff foo

Never overwrite.

⸻

19. Conflict Handling

This is one of the most important features.

Example:

skillissue adopt android

Result:

⚠ android has divergent copies.
Claude
  ~/.claude/skills/android
  fingerprint: 4a81cd
Codex
  ~/.codex/skills/android
  fingerprint: f0193a
Gemini
  ~/.gemini/skills/android
  fingerprint: 4a81cd
Claude and Gemini match.
Codex differs.
Choose canonical version:
> Claude / Gemini
  Codex
  View diff
  Keep both
  Skip

No automatic conflict resolution.

⸻

20. Diff

skillissue diff android

Should show directory-level differences.

Example:

Claude ↔ Codex
M SKILL.md
A references/gradle.md
D scripts/build.sh

Optional verbose mode:

skillissue diff android --content

This can invoke an internal textual diff.

Potential future option:

skillissue diff android --tool delta

But MVP should have no external dependency.

⸻

21. Keep Both

During a conflict, the user may choose:

Keep both

The tool should avoid inventing semantic merges.

Potential destination:

~/code/skills/android/
~/code/skills/android-codex/

Ask the user for the second name.

Then links can preserve the current state.

Example:

Claude -> android
Gemini -> android
Codex  -> android-codex

⸻

22. Status

skillissue status

This should become the everyday command.

Example:

skillissue
Canonical directory
  ~/code/skills
15 skills
4 targets
42 installations
✓ 13 fully managed
⚠  1 duplicate
⚠  1 divergent
✓  39 healthy symlinks
⚠  1 broken symlink
Issues
android
  Codex contains a physical copy
release
  Gemini differs from canonical
foo
  Claude symlink is broken
Run:
  skillissue doctor

When everything is healthy:

15 skills
4 targets
43 links
✓ No skill issues.

Yes, that line is mandatory.

⸻

23. Doctor

skillissue doctor

Checks:

* canonical root exists
* canonical root is readable
* targets exist
* skill symlinks resolve
* links point into canonical root
* canonical targets exist
* no duplicate physical copies exist
* no name collisions exist
* no target contains a diverged physical copy
* no circular symlinks exist

Example:

Checking skillissue...
✓ Canonical directory exists
✓ 15 canonical skills readable
✓ Claude target healthy
✓ Codex target healthy
✓ Gemini target healthy
⚠ 2 issues found
1. ~/.codex/skills/android
   physical copy shadows canonical skill
2. ~/.claude/skills/foo
   broken symlink
Run:
  skillissue doctor --fix

⸻

24. Doctor Fix

skillissue doctor --fix

Only automatically fix issues that are unambiguous.

Safe:

broken link whose canonical target exists
missing link to known canonical skill
duplicate physical directory identical to canonical

Unsafe:

physical directory differs from canonical

Unsafe cases must require interactive resolution.

⸻

25. Link

Manually expose a canonical skill to one or more agents.

Example:

skillissue link rust-cli claude codex gemini

Or:

skillissue link rust-cli --all

Result:

✓ Claude
  ~/.claude/skills/rust-cli
    -> ~/code/skills/rust-cli
✓ Codex
  ~/.codex/skills/rust-cli
    -> ~/code/skills/rust-cli

If destination exists and differs:

Error: ~/.claude/skills/rust-cli already exists.
Nothing changed.
Run:
  skillissue diff rust-cli

⸻

26. Unlink

skillissue unlink rust-cli codex

Removes only the symlink.

Never removes the canonical skill.

Example:

Removed:
~/.codex/skills/rust-cli
Canonical skill remains:
~/code/skills/rust-cli

⸻

27. Linking All Canonical Skills

Useful for a new agent installation:

skillissue link --all --target claude

Or potentially:

skillissue link-all claude

Prefer the former to reduce command count.

This allows a fresh machine setup like:

git clone git@github.com:cesarferreira/skills ~/code/skills
skillissue init ~/code/skills
skillissue link --all --target claude
skillissue link --all --target codex

⸻

28. Migration Safety

Migration is the highest-risk operation.

Before changing files, build a complete operation plan.

Example:

PLAN
MOVE
  ~/.claude/skills/foo
    -> ~/code/skills/foo
REMOVE IDENTICAL DUPLICATES
  ~/.codex/skills/foo
  ~/.gemini/skills/foo
CREATE LINKS
  ~/.claude/skills/foo
  ~/.codex/skills/foo
  ~/.gemini/skills/foo
3 locations affected.
Proceed? [y/N]

Default should be No for destructive interactive actions.

⸻

29. Dry Run

Every mutating command should support:

--dry-run

Example:

skillissue adopt --dry-run

Output must be the exact operations that would happen.

No filesystem mutations.

⸻

30. Transaction-Like Migration

For adoption, avoid:

delete duplicates
then move source
then discover something failed

Prefer:

validate everything
create temporary backup/staging state
move canonical copy
verify
replace one location at a time
verify each link
clean temporary copies last

The operation should fail as safely as possible.

⸻

31. Temporary Backup During Migration

For risky migrations, consider temporary relocation rather than immediate deletion.

Example:

~/.cache/skillissue/migrations/<uuid>/

Identical duplicates can be moved there during migration.

After all symlinks verify successfully, delete the temporary copies.

If migration fails:

restore them

This is not long-term backup.

It is transactional safety.

⸻

32. Filesystem Edge Cases

Handle:

* paths containing spaces
* Unicode filenames
* relative symlinks
* absolute symlinks
* dangling symlinks
* symlink loops
* permissions errors
* read-only directories
* missing parent directories
* canonical root inside a target directory
* target directory inside canonical root
* canonical skill already symlinked elsewhere
* name collisions
* case-insensitive macOS filesystems

Potential dangerous topology must fail early.

⸻

33. Symlink Strategy

Prefer relative symlinks when practical.

Example:

~/.claude/skills/foo -> ../../../code/skills/foo

Benefits:

* potentially survives home directory relocation
* easier machine migration

However, absolute symlinks are simpler and easier to inspect.

For MVP, use absolute canonical paths.

Consider relative links later.

⸻

34. Canonical Root Rules

Canonical directory:

~/code/skills

contains immediate child directories:

~/code/skills/foo
~/code/skills/bar
~/code/skills/baz

Each child is one canonical skill.

Do not introduce hidden nesting such as:

~/code/skills/.skillissue/packages/foo/v3/

Keep it boring.

⸻

35. Metadata

Ideally skillissue should require no metadata inside skill directories.

Do not mutate users’ skills by inserting:

.skillissue.json

The filesystem should remain compatible with every agent.

Global state belongs in:

~/.config/skillissue/

⸻

36. State Database

MVP should avoid requiring one.

Everything necessary can mostly be derived from:

* config
* filesystem
* symlink destinations
* fingerprints

If historical migration metadata is useful later:

~/.local/state/skillissue/

may contain non-essential records.

Deleting it must not break the installation.

⸻

37. Git Awareness

If the canonical root contains:

.git/

status can optionally display:

Git
✓ repository
⚠ 3 uncommitted changes
✓ upstream configured
⚠ 2 commits ahead of origin/main

Do not automatically commit or push in MVP.

Potential command:

skillissue git-status

But this could simply be included under:

skillissue status --git

⸻

38. Machine Restore Story

The recovery workflow should be one of the product’s strongest selling points.

New Mac:

git clone git@github.com:me/skills ~/code/skills
skillissue init ~/code/skills
skillissue link --all --target claude
skillissue link --all --target codex
skillissue link --all --target gemini

Potential future:

skillissue restore

which asks:

Detected:
✓ Claude
✓ Codex
✓ Gemini
Link all 17 skills into detected agents?
[Y/n]

⸻

39. Project-Level Skills

MVP should focus on user-level/global skill directories.

Project-level discovery can follow.

Future:

skillissue scan --project .

detecting:

./.claude/skills
./.agents/skills

But project skills raise additional questions:

* should every project share them?
* are they actually project-specific?
* should project-local skill names collide with globals?

Do not let this complexity delay MVP.

⸻

40. UX

The CLI should have strong visual hierarchy.

Example:

skillissue scan
Scanning...
  Claude    ~/.claude/skills    12
  Codex     ~/.codex/skills      9
  Gemini    ~/.gemini/skills     7
28 installations
14 skills
✓ 10 identical
⚠  3 duplicated
✗  1 divergent

Use colour when stdout is a TTY.

Respect:

NO_COLOR

Support:

--no-color

⸻

41. Interactive UX

Use:

* arrow-key selection
* space for multi-select
* enter to confirm
* q / Esc to cancel

Rust crates worth considering:

dialoguer
inquire
ratatui

MVP probably does not need a full-screen TUI.

A polished interactive CLI is enough.

Later:

skillissue ui

could offer a richer TUI if genuinely useful.

⸻

42. Machine-Readable Output

All read commands should eventually support:

--json

Example:

skillissue scan --json

Useful for shell scripts and agents.

Potential structure:

{
  "skills": [
    {
      "name": "rust-cli",
      "status": "duplicate",
      "installations": [
        {
          "target": "claude",
          "path": "/Users/cesar/.claude/skills/rust-cli",
          "fingerprint": "..."
        }
      ]
    }
  ]
}

⸻

43. Exit Codes

Useful for scripting.

Suggested:

0 healthy / successful
1 generic error
2 issues detected
3 conflicting skills detected
4 invalid configuration

For:

skillissue doctor

healthy:

0

issues:

2

This makes it useful in dotfiles/bootstrap scripts.

⸻

44. Logging

Default output should be human-friendly.

Verbose:

-v

Debug:

-vv

Potential:

RUST_LOG=debug skillissue scan

But don’t force users to understand Rust logging conventions.

⸻

45. Performance

Typical user:

5-100 skills
1-5 agents

Scanning should feel instantaneous.

Hashing can happen concurrently.

Avoid hashing identical files repeatedly within a single process.

Potential optimization:

size + mtime cache

later.

MVP can simply hash correctly.

Correctness matters more than optimizing 50 Markdown directories.

⸻

46. Proposed Rust Architecture

Suggested modules:

src/
├── main.rs
├── cli.rs
├── config.rs
├── targets.rs
├── discovery.rs
├── fingerprint.rs
├── model.rs
├── scan.rs
├── adoption.rs
├── linking.rs
├── doctor.rs
├── diff.rs
├── fs.rs
└── ui.rs

Important separation:

filesystem discovery
        ↓
domain model
        ↓
operation planning
        ↓
user confirmation
        ↓
filesystem mutation

Do not mix scanning and mutation.

⸻

47. Core Domain Model

Something approximately like:

struct SkillInstallation {
    name: String,
    target: TargetId,
    path: PathBuf,
    kind: InstallationKind,
    fingerprint: Option<Fingerprint>,
}
enum InstallationKind {
    Physical,
    ManagedSymlink,
    ForeignSymlink,
    BrokenSymlink,
}
struct SkillGroup {
    name: String,
    canonical: Option<CanonicalSkill>,
    installations: Vec<SkillInstallation>,
}
enum SkillStatus {
    Unique,
    IdenticalDuplicate,
    Divergent,
    Managed,
    Broken,
}

⸻

48. Operations Model

Do not directly mutate files while deciding what to do.

Generate operations:

enum FsOperation {
    Move {
        from: PathBuf,
        to: PathBuf,
    },
    RemoveDirectory {
        path: PathBuf,
    },
    CreateSymlink {
        target: PathBuf,
        link: PathBuf,
    },
}

Then:

discover
→ classify
→ build OperationPlan
→ render plan
→ confirm
→ execute
→ verify

This architecture makes --dry-run almost free.

⸻

49. Fingerprinting

Use a cryptographic hash such as:

BLAKE3

It’s fast, simple, and well-supported in Rust.

For every directory:

BLAKE3(
  "SKILL.md"
  BLAKE3(file)
  "references/foo.md"
  BLAKE3(file)
  ...
)

Paths must be sorted.

Do not hash:

mtime
permissions
inode
absolute path

Two copied skills should compare identical.

⸻

50. Security

Because skills can contain scripts:

skillissue must never execute skill contents.

It only:

* reads files
* hashes files
* moves directories
* creates/removes symlinks

Treat every discovered skill as untrusted filesystem data.

⸻

51. MVP

The first usable release should support only:

✓ configuration of canonical root
✓ built-in Claude/Codex/Gemini/.agents targets
✓ custom targets
✓ scanning
✓ content fingerprints
✓ duplicate detection
✓ divergence detection
✓ adopt identical copies
✓ interactive conflict selection
✓ symlink creation
✓ status
✓ doctor
✓ dry run
✓ safe migration

That’s already a very useful product.

⸻

52. V0.1 Commands

Ship:

skillissue init
skillissue scan
skillissue adopt
skillissue status
skillissue doctor
skillissue diff
skillissue link
skillissue unlink
skillissue targets
skillissue config

No more.

⸻

53. V0.1 Acceptance Test

Given:

~/.claude/skills/foo/
~/.codex/skills/foo/
~/.gemini/skills/foo/

where all three are identical.

And:

root = ~/code/skills

Running:

skillissue adopt foo

must result in:

~/code/skills/foo/             REAL DIRECTORY
~/.claude/skills/foo           SYMLINK -> canonical
~/.codex/skills/foo            SYMLINK -> canonical
~/.gemini/skills/foo           SYMLINK -> canonical

All original files must remain accessible.

All fingerprints must still match.

Running:

skillissue doctor

must return:

✓ No skill issues.

⸻

54. V0.2

Potential additions:

project-local skill scanning
git awareness
restore command
JSON output
relative symlinks
ignore patterns
shell completions

⸻

55. V0.3+

Only after real usage validates the need:

remote skill sources
GitHub skill installation
skill updates
skill collections/profiles
agent-specific inclusion rules
TUI dashboard

Be extremely careful here.

This is the point where skillissue risks becoming Kitter.

The differentiator should remain simplicity.

⸻

56. README Pitch

skillissue
Your agents have a skill issue.
Claude has one copy.
Codex has another.
Gemini has a third.
They started identical.
Now you have no idea which one is current.
skillissue finds them, moves your skills into one canonical
directory, and replaces every duplicate with a symlink.
One skill.
One copy.
Every agent.

Example:

$ skillissue scan
Found rust-cli in:
  ~/.claude/skills/rust-cli
  ~/.codex/skills/rust-cli
  ~/.gemini/skills/rust-cli
✓ All copies identical.
$ skillissue adopt rust-cli
Moved to:
  ~/code/skills/rust-cli
Linked:
  Claude ✓
  Codex  ✓
  Gemini ✓
No skill issues.

⸻

57. Tagline

Primary:

Your agents have a skill issue.

Secondary:

One copy of every skill. Symlinked everywhere.

Or:

Stop copy-pasting your agent skills.

⸻

58. The Core Product Test

Whenever considering a feature, ask:

Does this help the user maintain one canonical copy of their skills across multiple agents?

If not, it probably doesn’t belong in skillissue.

The ideal implementation should remain small enough that a developer can understand the entire product in an afternoon.

The filesystem is the database.

Git is the backup.

Symlinks are the synchronization protocol.

skillissue is the glue.

