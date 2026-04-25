# Contributing to grimoire

Thanks for considering a contribution. Most contributions are new spells.

## Authoring a spell

A spell is a single YAML file in `spells/<name>.yaml`. Read [`SPEC.md`](./SPEC.md) for the full schema; the canonical machine-readable definition lives at [`schema/spell.schema.json`](./schema/spell.schema.json).

Quick template:

```yaml
name: my-thing
summary: One sentence describing what this brings to the machine.
verify: command -v my-thing
version_check: my-thing --version 2>/dev/null | awk '{print $NF}'
cast:
  default: upstream
  channels:
    upstream:
      type: shell
      summary: Official installer (recommended).
      run: |
        curl -fsSL https://example.com/install.sh | sh
      version_hint: latest
category: development
homepage: https://example.com
provides:
  binaries: [my-thing]
```

### Conventions worth following

- **One install per channel.** A channel encapsulates one way to reach the state. Don't bundle "install via flatpak, also do dnf cleanup" in a single channel.
- **Manual installs land in `${SOFTWARE_DIR:-$HOME/Software}/<AppName>/`.** Symlink the launcher into `~/.local/bin/<binname>`. Don't drop loose binaries or AppImages directly in `~/Software`.
- **Configurable paths use env vars with shell defaults.** `${SOFTWARE_DIR:-$HOME/Software}` lets users override in `~/.config/grimoire/config.toml [env]` while keeping out-of-the-box behavior sane.
- **Pin versions only when there's no programmatic "latest"**, and expose them as `${MYTHING_VERSION:-x.y.z}` so users can bump without editing the spell.
- **Don't ship a spell you can't validate works** on at least one machine. Better to merge fewer good spells than many flaky ones.

## Validating before you submit

```sh
cargo fmt --all --check
cargo clippy --all-targets -- -D warnings
cargo test
```

The pre-commit hook runs the first two automatically — wire it up once per clone:

```sh
git config core.hooksPath .githooks
```

`cargo test --test schema` runs the JSON Schema validator against every spell in `spells/`. A new spell that breaks the schema fails CI.

## Channels and `type:`

Pick the most specific `type:` you can. Today the tool only branches on `requires_sudo` and (eventually) update probes, but the type drives GUI badges and channel-aware behavior over time:

| `type:` | When to use |
|---|---|
| `shell` | Anything that runs an arbitrary script (curl-pipe-bash, git clone, custom tarball flow). |
| `dnf` | Native Fedora/RHEL package install. |
| `flatpak` | Flathub or other flatpak remote. |
| `snap` | Canonical Snap install. |
| `cargo` | `cargo install` from crates.io. |
| `pip` | `pip install` (system or user). |
| `npm` | `npm install -g`. |

If you need a new type, add it to `ChannelType` in `src/spell.rs` and the enum in `schema/spell.schema.json`.

## Manifest schema

The manifest schema lives at [`schema/manifest.schema.json`](./schema/manifest.schema.json). IDEs that understand JSON Schema (yaml-language-server, Even Better TOML in VS Code) will autocomplete and lint your `.grimoire.toml` files if you point them at it.

## Things grimoire is *not*

Out of scope, by design:

- **Tidying / reorganizing existing files on disk.** That's [fili](https://github.com/strycore/fili)'s job.
- **System configuration** beyond the personal `~`. No `/etc` rewriting, no package-manager mirror config, no service install.
- **Multi-host fleet management.** One machine, one user, one `~`.
- **Rollback semantics.** The audit log in `~/.local/state/grimoire/cast.db` is forensic, not transactional.

If your contribution leans on one of these, it probably belongs somewhere else.
