# grimoire — declarative state for personal Linux desktops

> Status: design draft, partial implementation. Foundation (parsing, validation, listing) is in. Execution (`cast`, `scry`) is stubbed.

**grimoire** turns "the things you need installed and configured to do X" into
single-file YAML "spells" you can share, version, and cast on demand. Each
spell declares a state to reach (`verify`), how to reach it
(`cast.channels[*].run`), and optional version metadata.

It is **not** a system configuration manager (Ansible, NixOS): no inventory,
no host-level state, no rollbacks. It is intentionally one-machine, one-user,
shell-based, and editable by hand.

---

## Vocabulary

| Term | Meaning |
|---|---|
| **spell** | A YAML file declaring one desired state (e.g. `rust.yaml`). |
| **grimoire** | The collection of available spells (bundled into the binary; extended by the user's personal spell directory). |
| **channel** | A specific way to install/reach the state (e.g. `flatpak`, `dnf`, `upstream`, `rustup`). A spell has one or more channels; one is the default. |
| **cast** | The act of running a channel to reach the state. Idempotent — `verify` runs first; if it passes, cast is a no-op. |
| **scry** | A read-only check: report state without changing anything. |
| **manifest** | A list of spells (with optional version constraints) that a project or machine wants. Lives at `<project>/.grimoire.toml` or `~/.config/grimoire/manifest.toml`. |

---

## Spell schema

```yaml
# REQUIRED
name: rust                           # ^[a-z][a-z0-9-]+$, unique across the grimoire
summary: One-line description shown in listings.
verify: |                            # bash; exits 0 iff state is reached
  command -v cargo && command -v rustc

cast:
  default: rustup                    # which channel to use unless overridden
  channels:
    rustup:                          # channel key — used as `--via rustup`
      type: shell                    # shell | dnf | flatpak | snap | cargo | pip | npm
      summary: Upstream rustup (rolling stable)
      run: |
        curl --proto '=https' -sSf https://sh.rustup.rs | sh -s -- -y
      version_hint: latest           # latest | distro | pinned (informational; helps picking)
      requires_sudo: false
    dnf:
      type: dnf
      summary: Fedora packages
      run: sudo dnf install -y rust cargo rustfmt clippy
      version_hint: distro
      requires_sudo: true

# OPTIONAL
description: |
  Long-form explanation, shown in `grimoire show` and the GUI detail view.
homepage: https://www.rust-lang.org
maintainer: contributor@example.com
category: development                # free-form, used for GUI grouping
tags: [language, toolchain]
distros: [fedora, ubuntu, arch]      # omit ⇒ universal; tool checks /etc/os-release

version_check: rustc --version | awk '{print $2}'
                                     # bash; prints current installed version on stdout.
                                     # Empty/non-zero exit ⇒ not installed (same as verify=false).

requires: ["java >= 17"]             # other spells; same constraint syntax as manifests
system_requires: [cc, pkg-config]    # bare binaries that must be in PATH before cast.
                                     # Missing binaries are installed via the distro
                                     # package manager (dnf/apt/pacman) — not modeled
                                     # as spells. Known: cc, pkg-config, make, curl,
                                     # wget, git.
provides:
  binaries: [cargo, rustc, rustup]   # informational; useful for cross-spell reasoning later

before: |                            # pre-cast hook; runs before any channel
  : # no-op
after: |                             # post-cast hook; runs after the chosen channel
  rustup component add rustfmt clippy 2>/dev/null || true
```

### Field rules

- `name`, `summary`, `verify`, `cast.default`, and at least one `cast.channels[*]` are required.
- Each channel must have `type` and `run`.
- All shell snippets run with `set -e` injected by the tool. They are run as the user; sudo is invoked from inside `run` when needed and declared via `requires_sudo: true`.
- `version_check` is optional but required if any manifest references this spell with a version constraint.
- `version_hint` values are: `latest` (channel tracks upstream), `distro` (whatever the distro ships), `pinned` (the `run:` installs a fixed version).

---

## Constraint syntax

Used in manifest entries and `requires`. Comma-separated comparators (PEP-440 / Cargo flavor):

```
<spell-name>[ <op><version>[, <op><version>]* ]
```

Operators: `>=`, `>`, `<=`, `<`, `==`, `!=`. Multiple comparators combine with implicit AND.

```
rust >= 1.95
java >= 18, < 20
android-studio == 2025.1.2.12
bun
```

Versions compare semver-first (dot-separated numerics + optional pre-release after `-`),
falling back to lexical for non-numeric components. Dotted dates like `2025.1.2.12`
compare correctly because each component is numeric.

---

## Manifest schema

### Project manifest — `<project>/.grimoire.toml`

```toml
spells = [
  "rust >= 1.95",
  "bun >= 1.0",
  "android-studio",
  "java >= 18, < 20",
]

# Optional per-spell channel overrides for this project
[overrides.rust]
channel = "rustup"
```

### User manifest — `~/.config/grimoire/manifest.toml`

Same schema; meant for desktop staples that aren't project-specific. Optional —
many users may only ever use project manifests.

```toml
spells = [
  "discord",
  "blender",
  "firefox",
]
```

### Profiles (optional)

Either manifest may declare named profiles to switch between sets:

```toml
spells = []                          # base set; can be empty

[profiles.work]
spells = ["rust", "slack", "my-dotfiles"]

[profiles.gaming]
spells = ["lutris", "discord", "steam"]
```

Selected with `grimoire cast --profile work`.

---

## State machine

Each spell-on-this-machine has exactly one state at any moment:

| State | Meaning | `cast` default action |
|---|---|---|
| **missing** | `verify` fails; never cast successfully. | Cast it. |
| **cast** | `verify` passes; `version_check` (if any) satisfies the active constraint. | No-op. |
| **outdated** | `verify` passes; `version_check` does not satisfy the active constraint. | Cast it (recasting same channel). |
| **drifted** | Was last recorded as `cast`; `verify` now fails. | **Skip; report.** Use `--recast` to force. |
| **invalid** | `verify` exited with an error other than 0/1, or the spell file failed validation. | **Skip; report.** Human review. |

Auto-recast on drift is intentionally off — the user decides.

---

## Update detection

Only the **currently-installed channel** (or, if not installed, the
**recommended channel**) is probed for updates. Channel-type-specific:

| Channel type | Update probe |
|---|---|
| `dnf` | `dnf check-update <pkg>` (exit 100 ⇒ update available) |
| `flatpak` | `flatpak remote-info <remote> <ref>` and compare commit/version |
| `snap` | `snap refresh --list` |
| `cargo` | `cargo install --list` + crate registry lookup |
| `shell` | None by default. Spell may declare `update_check` snippet for custom probes (e.g. parse upstream release feed). [Reserved schema field, not yet implemented.] |

Probes run only on `grimoire scry --check-updates` (or scheduled cron). They
never run during `cast`.

---

## CLI reference

```
grimoire cast [<spell> ...]            Cast missing + outdated spells. With no
                                       args: read manifest from cwd (project)
                                       or user manifest if no .grimoire.toml.
  --via <channel>                      Override channel for the named spell.
  --profile <name>                     Use a named profile from manifest.
  --manifest <path>                    Use the manifest at <path>; bypasses discovery.
  --recast                             Force recast even if state is drifted/invalid.
  --dry-run                            Print plan, don't execute.

grimoire scry [<spell> ...]            Report state. Reads manifest like cast.
  --manifest <path>                    Use the manifest at <path>; bypasses discovery.
  --check-updates                      Run update probes on cast/outdated spells.
  --explain <spell>                    Show why a spell is in its current state.

grimoire ls                            List spells available in the grimoire.
  --personal                           Only personal spells.
  --embedded                           Only embedded (shareable) spells.
  --category <name>                    Filter by category.

grimoire show <spell>                  Render a spell's full schema for inspection.

grimoire scribe <name>                 Scaffold a new personal spell from a
                                       template into ~/.config/grimoire/spells/.

grimoire diff                          Show plan: what cast would change.

grimoire log                           Tail the cast event log.
```

---

## Layout

### Repo (the tool itself)

```
github.com/<org>/grimoire/
  src/                                 # Rust source
  spells/                              # bundled, shareable spells (PR target)
    rust.yaml
    android-studio.yaml
    blender.yaml
    discord.yaml
    bun.yaml
    java.yaml
    ...
  schema/                              # JSON Schema for spells + manifests
    spell.schema.json
    manifest.schema.json
  CONTRIBUTING.md
  SPEC.md
  Cargo.toml
```

Spells in `spells/` are embedded into the binary at build time (`include_dir!`).
Each release ships with a curated set.

### Runtime (per machine)

```
~/.config/grimoire/
  manifest.toml                        # optional user-level manifest
  spells/                              # personal spells; override embedded by name match
    my-dotfiles.yaml
  config.toml                          # optional: env exports for spells

~/.local/state/grimoire/
  cast.db                              # SQLite event log
```

### `config.toml`

```toml
# Exported into every shell snippet's environment before it runs.
# Spells should reference these with a shell default so they keep working
# without config: ${SOFTWARE_DIR:-$HOME/Software}
[env]
SOFTWARE_DIR = "/home/me/Apps"
DEVELOPMENT_DIR = "/home/me/dev"
```

---

## Schema validation

- `schema/spell.schema.json` and `schema/manifest.schema.json` are JSON Schema definitions, source of truth for fields and types. *(Planned; today's validation is the Rust struct + `deny_unknown_fields`.)*
- CI on the canonical repo runs:
  - JSON Schema validation of every `spells/*.yaml`.
  - `bash -n` on every shell snippet.
  - A smoke test that `name` matches the filename.
  - `grimoire show` parses the spell.
- The runtime tool validates loaded spells (embedded + personal) at startup and refuses bad spells with a clear error.

---

## Implementation status

| Area | Status |
|---|---|
| Spell schema parsing + validation | ✅ |
| Manifest schema parsing + validation | ✅ |
| Constraint parser + version compare | ✅ |
| Embedded + personal spell merge with override | ✅ |
| `grimoire ls` | ✅ |
| `grimoire show` | ✅ |
| `grimoire cast <spell>` (single, with `--via`, `--dry-run`, `--recast`) | ✅ |
| `grimoire scry [<spell>]` (single + all-spells; missing/cast/drifted) | ✅ |
| `cast.db` SQLite event log + drift detection | ✅ |
| Manifest discovery (`.grimoire.toml` walk-up + user manifest fallback) | ✅ |
| `grimoire cast` (no args; manifest-driven, multi-spell) | ✅ |
| `grimoire scry` w/ manifest constraint check (outdated state) | ✅ |
| `--profile` flag for cast and scry | ✅ |
| Per-spell channel override from manifest (`[overrides.X] channel = ...`) | ✅ |
| Pre-commit hook (fmt + clippy) | ✅ |
| `requires:` resolution (transitive deps, topo sort, cycle detect) | ✅ |
| User config (`~/.config/grimoire/config.toml`) with `[env]` exports | ✅ |
| JSON Schema files (`schema/spell.schema.json`, `schema/manifest.schema.json`) | ✅ |
| Schema-validation integration test (every shipped spell) | ✅ |
| `grimoire scribe` (template scaffold) | 🚧 stub |
| `grimoire diff` / `log` | not started |
| Update probes (`--check-updates`) | not started |
| fili / moi integration | deferred |

---

## Open follow-ups

- `update_check` shell snippet for `type: shell` channels.
- fili integration: discover `.grimoire.toml` during scan and tag project entries with their declared spells.
- moi integration: tool surfaced inside agent sessions.
- A `grimoire publish` flow that opens a PR in the canonical spells repo from a personal spell.
- Multi-user / shared-machine scenarios. Out of scope for v1.
- Rollback semantics. Out of scope for v1; the audit log in `cast.db` provides forensics.
