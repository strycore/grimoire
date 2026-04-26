# grimoire

Declarative state for personal Linux desktops — share, version, and cast YAML spells.

> Status: alpha. 33 spells, runs on Fedora / Debian-Ubuntu / Arch families. Cast / scry / requires / manifests / multi-distro all work; `scribe`, update probes, and an AUR channel type are not yet implemented.

## What it is

You declare the states a machine should reach to do a task — "Rust dev", "Blender", "Android dev" — as single-file YAML *spells*. Each spell has a `verify` (how to check if the state is reached) and one or more `cast.channels` (how to reach it: flatpak, dnf, upstream installer, etc.). A canonical, contributable collection ships embedded in the binary; you can drop personal spells into `~/.config/grimoire/spells/` to extend or override.

A *manifest* — usually committed to a project repo as `.grimoire.toml` — declares which spells (with optional version constraints) the project needs:

```toml
spells = [
  "rust >= 1.95",
  "bun >= 1.0",
  "java >= 18, < 20",
  "android-studio",
]
```

Then `grimoire cast` ensures the machine reaches that state — runs `verify` first, picks the right channel for the current distro (dnf on Fedora, apt on Debian/Ubuntu, pacman on Arch, or the universal `shell` / `flatpak` / `cargo` / `npm` channels), and skips spells that already verify.

See [SPEC.md](./SPEC.md) for the full design.

## Install

Pre-built binary from the latest GitHub release (Linux x86_64 and aarch64; no Rust toolchain needed):

```sh
curl -fsSL https://raw.githubusercontent.com/strycore/grimoire/main/install.sh | bash
```

Pin a version, or change the install dir:

```sh
curl -fsSL .../install.sh | VERSION=v0.1.0 bash
curl -fsSL .../install.sh | GRIMOIRE_INSTALL_DIR=$HOME/bin bash
```

The script downloads the release tarball, verifies its SHA-256, and drops the `grimoire` binary into `~/.local/bin`. Make sure that's on your `$PATH`.

## Updating

```sh
grimoire upgrade           # download and replace if a newer release exists
grimoire upgrade --check   # just report; don't download
```

`upgrade` queries the GitHub releases API, downloads the tarball matching your platform, verifies the SHA-256, and atomically replaces the running binary via `rename(2)`. It refuses to run when the binary lives in a directory you can't write to (e.g. `/usr/bin/grimoire` from a distro package — let your package manager handle those) or when it looks like a `cargo` dev build.

Every command also tail-prints a one-line nag (`✨ grimoire X.Y.Z is available …`) when a newer release exists. The check is cached weekly in `~/.local/state/grimoire/update-check.json`, so it hits the network at most once every seven days, never during the hot path of a command. The nag goes to stderr and is suppressed entirely when stderr isn't an interactive terminal — pipelines, cron jobs, and `journald`-captured runs stay clean.

## Build from source

```sh
cargo build --release
./target/release/grimoire --help
```

Requires the Rust toolchain (rustup or distro-packaged) and a C compiler (`rusqlite` is configured with `bundled`, which compiles SQLite at build time).

## What works today

### Single-spell

```sh
grimoire ls                              # list available spells
grimoire ls --category development
grimoire show rust                       # render a spell

grimoire scry rust                       # status of one spell + version
grimoire cast rust                       # cast (idempotent — no-op if already cast)
grimoire cast rust --via dnf             # pick a non-default channel
grimoire cast rust --recast              # force re-run even if verify passes
grimoire cast rust --recast --dry-run    # show what would run, don't execute
```

### Manifest-driven (multi-spell)

Drop a `.grimoire.toml` at the root of a project:

```toml
spells = [
  "rust >= 1.95",
  "bun >= 1.0",
  "java >= 18, < 20",
  "android-studio",
]

[overrides.rust]
channel = "rustup"

[profiles.work]
spells = ["rust", "slack"]
```

Then from anywhere inside the project tree:

```sh
grimoire scry                  # report state of every spell in the manifest
grimoire scry --profile work   # status for a named profile
grimoire cast                  # cast everything missing or outdated
grimoire cast --dry-run        # show the plan
grimoire cast --profile work   # cast a profile's set
```

`grimoire` walks up from cwd looking for `.grimoire.toml`, falls back to
`~/.config/grimoire/manifest.toml` if there's no project manifest.

### Personal-preference paths (`config.toml`)

A spell can reference an env var like `${SOFTWARE_DIR:-$HOME/Software}`.
Set the override in `~/.config/grimoire/config.toml`:

```toml
[env]
SOFTWARE_DIR = "/home/me/Apps"
```

The variable is exported into every shell snippet that runs. With no config,
the spell's shell default kicks in — out-of-the-box behavior never depends on
config existing.

### Dependencies (`requires:` and `system_requires:`)

A spell can declare other spells it needs (`requires:`) and bare system binaries it expects in `$PATH` (`system_requires:`):

```yaml
name: rust
requires: []
system_requires: [cc, pkg-config, make]

name: android-studio
requires: ["java >= 17"]
```

`grimoire cast android-studio` (or `scry`) walks the spell graph: `java` gets checked/cast first, in topological order. Constraints from multiple dependents are AND-merged. Cycles are detected and refused.

`system_requires` entries are bare binary names (e.g. `cc`, `pkg-config`, `make`, `curl`, `wget`, `git`) — missing ones are installed via the active distro's package manager before the cast runs, without being modeled as full spells.

### Multi-distro

Channel types carry an implicit distro applicability:

| `type:` | Applies on |
|---|---|
| `dnf` | Fedora-family (RHEL, CentOS, Rocky, AlmaLinux, OpenSUSE) |
| `apt` | Debian-family (Debian, Ubuntu, Mint, Pop!_OS, elementary, Raspbian) |
| `pacman` | Arch-family (Arch, Manjaro, EndeavourOS, Garuda, CachyOS, Artix) |
| `shell`, `flatpak`, `snap`, `cargo`, `pip`, `npm` | universal |

`/etc/os-release` is read once at startup; cast picks `cast.default` if it applies on this distro, otherwise the first applicable channel in alphabetical key order. Per-channel `distros: [...]` overrides the implicit list. A spell with no applicable channel reports as **unsupported** rather than being silently failed.

### State machine

Each spell on this machine is in one of:

- **cast** — `verify` passes (and any constraint is satisfied)
- **outdated** — `verify` passes but `version_check` doesn't satisfy the constraint
- **missing** — `verify` fails, no record of a previous cast
- **drifted** — was cast successfully before; `verify` now fails. Surfaced for review; never auto-recast
- **invalid** — `verify` exited with an unexpected error
- **unsupported** — no channel applies on this distro

Cast events are logged to `~/.local/state/grimoire/cast.db` (SQLite) so drift detection survives across sessions.

### Login-shell snippets

Every shell snippet (`verify`, `version_check`, `before`, `after`, channel `run:`) executes under `bash -l` with `set -e` and `set -o pipefail` injected. Login mode sources `~/.bash_profile` / `~/.profile` so installers like `rustup`, `bun`, and `uv` make their `$PATH` additions visible immediately to subsequent snippets — no need for spells to manually `source $HOME/.cargo/env` and friends.

### Next

`scribe` (template scaffold for personal spells), update probes (`grimoire scry --check-updates`), and an `aur` channel type for Arch users would round out the canonical spell set.

## Contributing

After cloning, point git at the in-repo hooks once:

```sh
git config core.hooksPath .githooks
```

The `pre-commit` hook runs `cargo fmt --all --check` and `cargo clippy --all-targets -- -D warnings` whenever Rust sources are staged. Skip with `git commit --no-verify` only in emergencies.

CI (`.github/workflows/ci.yml`) runs the same gate plus `cargo test --all-targets` on every PR and push to `main`.

## Releasing

Two ways to trigger `.github/workflows/release.yml`:

1. **Tag push** (preferred for real releases): bump `version` in `Cargo.toml`, commit, then
   ```sh
   git tag v0.1.0
   git push --tags
   ```
2. **Manual dispatch**: GitHub → Actions → *Release* → *Run workflow*, supplying a tag like `v0.1.0`. The tag is created at the selected commit if it doesn't already exist. Useful for re-cutting a release without local git access.

Either trigger builds stripped Linux x86_64 and aarch64 binaries (native arm64 runner, no cross-compile), packages each as `grimoire-<version>-linux-<arch>.tar.gz` (with `README.md`, `SPEC.md`, `LICENSE` alongside) plus a `.sha256`, and publishes one GitHub release with auto-generated notes from the commit log since the previous tag.

## License

GPL-3.0-or-later.
