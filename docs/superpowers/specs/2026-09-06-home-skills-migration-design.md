# Home Skills Migration Design

## Goal

Make `~/skills` the single canonical root for the user's locally managed agent
skills, then make the supported agent directories point at it. The migration is
also the first real-world validation of `si setup`.

## Scope

`fleet-cli` and `android-cli` are explicitly retired: every copy or symlink in
the supplied source locations is removed and neither is collected into
`~/skills`. Cursor is a built-in target. The supplied dotfiles package can be
included in an initial setup without hand-editing machine configuration.

External, provider-managed skills remain foreign symlinks. They are not copied
into the canonical root and are not deleted by the migration.

## CLI design

`setup` accepts repeatable `--target ID=PATH` options and repeatable
`--ignore PATTERN` options:

```text
si setup ~/skills \
  --target dotfiles=~/dotfiles/agent-skills/.agents/skills \
  --ignore fleet-cli --ignore android-cli
```

On first setup these options are stored in local configuration before discovery.
On a later setup they merge additional targets and ignore patterns into the
existing configuration. A repeated value is a no-op. An existing target ID
with a different path is rejected. Ignored top-level skill names are omitted
from discovery, collection, and linking; ignoring never deletes content.

Built-in target discovery includes Cursor at `~/.cursor/skills` when it exists.

## Migration procedure

Before changes, create one timestamped archive containing the five supplied
skill roots. Remove the two explicitly retired named skills from each supplied
root, treating a symlink as a link-only deletion and a real directory as a
directory deletion. Then run the command above, review its aggregate plan, and
apply it. The postcondition is one real directory for each collected skill in
`~/skills` and managed links in supported targets.

Run `si status` and targeted filesystem checks after applying. Broken managed
links are repaired when their canonical skill exists; unrelated foreign or
external links remain reported rather than silently changed.

## Safety

No divergent physical directory is overwritten. The new setup flags merely
persist configuration and influence planning; `--yes` still cannot resolve a
divergent-content decision. The backup archive is retained after successful
migration.
