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
      type: shell                    # shell | dnf | apt | pacman | flatpak | snap | cargo | pip | npm | compose
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
- Each channel must have `type` and `summary`. Most channel types also require `run`. The `compose` type instead requires its own source/parameter fields (see below) and forbids `run`.
- All shell snippets run with `set -e` injected by the tool. They are run as the user; sudo is invoked from inside `run` when needed and declared via `requires_sudo: true`.
- `version_check` is optional but required if any manifest references this spell with a version constraint.
- `version_hint` values are: `latest` (channel tracks upstream), `distro` (whatever the distro ships), `pinned` (the `run:` installs a fixed version).

---

## Compose channels

A spell can declare its desired state as "this Docker Compose project is up." Use this for self-hosted services (Immich, Linkwarden, Paperless, etc.) that publish a `docker-compose.yml` upstream.

```yaml
name: linkwarden
summary: Self-hosted bookmark and archive manager.
requires: [docker]
verify: docker compose -p linkwarden ps --status running -q | grep -q .
cast:
  default: compose
  channels:
    compose:
      type: compose
      summary: Official upstream compose, pinned to a release tag.
      repo: https://github.com/linkwarden/linkwarden
      ref: v2.9.3                       # release tag — bump to upgrade
      path: .                           # subdir holding docker-compose.yml
      compose_file: docker-compose.yml  # optional; defaults to docker-compose.yml
      parameters:
        POSTGRES_PASSWORD:
          summary: Database password (auto-generated, persisted in the service .env).
          kind: secret
        NEXTAUTH_SECRET:
          summary: Session signing secret (auto-generated).
          kind: secret
        NEXTAUTH_URL:
          summary: Public URL for the instance.
          default: http://localhost:3000
```

### Source

The compose source is fetched as a shallow git clone at `ref` and the `path` subtree is copied into the service runtime dir. Pinning to a release tag is intentional — upstream composes that reference `:latest` images or change layout silently are a known foot-gun. Bump `ref` to upgrade.

### Parameters

Each parameter declares one env var that lands in the materialized `.env` next to the compose file (which `docker compose` picks up via `${VAR}` interpolation **and** `env_file:`).

| Field | Meaning |
|---|---|
| `summary` | Optional; shown in prompts and `grimoire show`. |
| `default` | Used if the user hasn't set the parameter. |
| `kind: secret` | First cast generates a value via `openssl rand -hex 32` and persists it in the service `.env`. Subsequent casts reuse it. |
| `required: true` | Must be supplied (no default). Cast fails fast with an error if missing — interactive prompting is a future addition. |

Resolution order (highest precedence first): user override file (`~/.config/grimoire/services/<name>.env`), persisted secret in the service `.env`, declared `default`. Required parameters with no value abort the cast.

### Service runtime layout

```
~/.local/share/grimoire/services/<spell-name>/
  docker-compose.yml         # fetched from upstream
  .env                       # materialized parameters (incl. generated secrets)
  ... any sibling files from `path` ...
```

The compose project name passed to `docker compose -p` is the spell name. So `verify` is conventionally:

```bash
docker compose -p <spell-name> ps --status running -q | grep -q .
```

### Lifecycle

- `cast`: prepare the service dir, materialize `.env`, run `docker compose -p <name> -f <file> up -d --remove-orphans`.
- `verify`: spell-defined (typically the `docker compose ps` check above).
- Re-cast (e.g. after bumping `ref`) re-fetches the source and re-runs `up -d`. Existing volumes and persisted secrets are preserved.
- Teardown is not yet a first-class operation; today it's manual: `docker compose -p <name> down` from the service dir.

### Edge cases not covered in v1

- **`url:` source** for projects that publish at an unversioned URL (e.g. Authentik's `goauthentik.io/docker-compose.yml`). Today only `repo + ref + path` is supported.
- **`override:` file** bundled in the spell to merge a `compose.override.yml` over the upstream compose. Useful for projects that hardcode insecure defaults (e.g. Paperless's `POSTGRES_PASSWORD: paperless`).
- **`env_template:`** declaring an upstream `.env.example` to inherit defaults from, instead of re-declaring every parameter in the spell.
- **AIO-style installers** (Nextcloud All-in-One): not compose at all — model these as a `shell` channel running `docker run nextcloud/all-in-one`.
- **Manifest parameter overrides** (`[parameters.linkwarden] NEXTAUTH_URL = ...`). For now use the per-user `.env` override file.

---

## Desktop entries

Many spells install software that doesn't auto-create a menu entry — raw AppImages dropped into `$SOFTWARE_DIR`, manually-extracted tarballs, GitHub-release binaries — leaving the user with a working binary on `$PATH` but nothing to launch from their app menu. A `desktop:` block fills the gap: grimoire writes `~/.local/share/applications/<spell>.desktop` and (optionally) installs a bundled icon.

```yaml
desktop:
  name: LM Studio                       # Name=
  exec: lmstudio %U                     # Exec= — field codes are passed through
  comment: Discover, download, and run local LLMs.   # Comment= — defaults to spell.summary
  icon: lmstudio                        # Icon= — see "Icon resolution" below
  categories: [Development, Science]    # Categories=
  terminal: false                       # Terminal=
  startup_wm_class: LM Studio           # StartupWMClass= — for window grouping
  mime_types: []                        # MimeType=
```

Reconciliation runs at the end of every successful cast (and on no-op already-cast runs), so adding or editing a `desktop:` block then re-running grimoire is enough to update the menu entry.

### Icon resolution

When `icon:` is a stem name (no path separators), grimoire looks in this order and copies the first match into `~/.local/share/icons/hicolor/`:

1. `~/.config/grimoire/icons/<name>.{svg,png}` — personal override
2. `<repo>/icons/<name>.{svg,png}` — bundled with grimoire

SVG files go to `hicolor/scalable/apps/`, PNG files to `hicolor/256x256/apps/`. After copying, grimoire runs `gtk-update-icon-cache` (best-effort; ignored if the binary isn't present). If no matching file is found, the value is still written verbatim to `Icon=` so a system theme icon can resolve it.

When `icon:` is an absolute path, it's written verbatim and no copy happens — useful for spells whose `after:` hook extracts an icon out of an AppImage or tarball.

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
  icons/                               # bundled .desktop icons; <spell>.{svg,png}
    lmstudio.png
  CONTRIBUTING.md
  SPEC.md
  Cargo.toml
```

Spells in `spells/` and icons in `icons/` are embedded into the binary at build time (`include_dir!`).
Each release ships with a curated set.

### Runtime (per machine)

```
~/.config/grimoire/
  manifest.toml                        # optional user-level manifest
  spells/                              # personal spells; override embedded by name match
    my-dotfiles.yaml
  icons/                               # personal icon overrides; win over embedded
    my-spell.png
  config.toml                          # optional: env exports for spells

~/.local/state/grimoire/
  cast.db                              # SQLite event log

~/.local/share/grimoire/services/      # one dir per cast `compose` channel
  <spell-name>/
    docker-compose.yml                 # fetched from upstream at the pinned ref
    .env                               # materialized parameters + generated secrets

~/.local/share/applications/           # XDG menu entries grimoire writes
  <spell-name>.desktop
~/.local/share/icons/hicolor/          # XDG icon tree grimoire copies into
  256x256/apps/<icon>.png
  scalable/apps/<icon>.svg
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
| `grimoire cast <spell> [<spell> ...]` (single or multi, with `--via` for single, `--dry-run`, `--recast`) | ✅ |
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
| `compose` channel: repo+ref+path source, parameters with default/secret, `.env` materialization, `docker compose up -d` | ✅ |
| `compose` channel: `url:` source for unversioned upstreams | not started |
| `compose` channel: spell-bundled `override:` file (compose.override.yml) | not started |
| `compose` channel: manifest parameter overrides + interactive prompt | not started |
| `desktop:` entries: `.desktop` materialization + bundled icons | ✅ |
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
