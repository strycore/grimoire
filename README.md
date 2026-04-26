# grimoire

Declarative state for personal Linux desktops — share, version, and cast YAML spells.

> Status: alpha. Foundation in place (parsing, validation, listing). Execution is stubbed.

## What it is

You declare the states a machine should reach to do a task — "Rust dev", "Blender", "Android dev" — as single-file YAML *spells*. Each spell has a `verify` (how to check if the state is reached) and one or more `cast.channels` (how to reach it: flatpak, dnf, upstream installer, etc.). A canonical, contributable collection ships embedded in the binary; you can drop personal spells into `~/.config/grimoire/spells/` to extend or override.

A *manifest* — usually committed to a project repo as `.grimoire.toml` — declares which spells (with optional version constraints) the project needs:

```toml
spells = [
  "rust-dev >= 1.95",
  "bun-dev >= 1.0",
  "java-sdk >= 18, < 20",
  "android-dev",
]
```

Then `grimoire cast` (eventually) ensures the machine reaches that state.

See [SPEC.md](./SPEC.md) for the full design.

## Build

```sh
cargo build --release
./target/release/grimoire --help
```

## What works today

### Single-spell

```sh
grimoire ls                              # list available spells
grimoire ls --category development
grimoire show rust-dev                   # render a spell

grimoire scry rust-dev                   # status of one spell + version
grimoire cast rust-dev                   # cast (idempotent — no-op if already cast)
grimoire cast rust-dev --via dnf         # pick a non-default channel
grimoire cast rust-dev --recast          # force re-run even if verify passes
grimoire cast rust-dev --recast --dry-run  # show what would run, don't execute
```

### Manifest-driven (multi-spell)

Drop a `.grimoire.toml` at the root of a project:

```toml
spells = [
  "rust-dev >= 1.95",
  "bun-dev >= 1.0",
  "java-sdk >= 18, < 20",
  "android-dev",
]

[overrides.rust-dev]
channel = "rustup"

[profiles.work]
spells = ["rust-dev", "slack"]
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

### Dependencies (`requires:`)

Spells can declare other spells they need:

```yaml
name: android-dev
requires: ["java-sdk >= 17"]
```

`grimoire cast android-dev` (or `scry`) walks the graph: java-sdk gets
checked/cast first, in topological order. Constraints from multiple
dependents are AND-merged. Cycles are detected and refused.

### State machine

Each spell on this machine is in one of: **cast**, **outdated** (verify passes
but version constraint fails), **missing** (never cast), **drifted** (was cast
but verify now fails — surfaced for review, never auto-recast), or **invalid**.

Cast events are logged to `~/.local/state/grimoire/cast.db` (SQLite) so drift
detection survives across sessions.

`scribe`, update probes, and `requires:` resolution are next.

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

Either trigger builds a stripped Linux x86_64 binary, packages it as `grimoire-<version>-linux-x86_64.tar.gz` (with `README.md`, `SPEC.md`, `LICENSE` alongside) plus a `.sha256`, and publishes a GitHub release with auto-generated notes from the commit log since the previous tag.

## License

GPL-3.0-or-later.
