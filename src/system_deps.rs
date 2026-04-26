//! System dependencies — binaries that spells expect to find in `PATH` but
//! that grimoire does not manage as full spells (compilers, fetchers, build
//! glue). Each entry maps to a package name per package-manager family; if
//! missing at cast time, grimoire installs it via the distro's package
//! manager rather than failing.

use crate::distro::Distro;
use crate::shell;
use anyhow::{Result, bail};

/// Distro package-manager families grimoire knows how to drive for installing
/// system dependencies.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PkgMgr {
    Dnf,
    Apt,
    Pacman,
}

impl PkgMgr {
    /// Resolve the package manager for the current distro using the same
    /// implicit-distro lists as `ChannelType`. Returns `None` for distros
    /// grimoire can't drive automatically.
    pub fn for_distro(distro: &Distro) -> Option<PkgMgr> {
        use crate::spell::ChannelType;
        if ChannelType::Dnf
            .implicit_distros()
            .iter()
            .any(|d| distro.matches(d))
        {
            Some(PkgMgr::Dnf)
        } else if ChannelType::Apt
            .implicit_distros()
            .iter()
            .any(|d| distro.matches(d))
        {
            Some(PkgMgr::Apt)
        } else if ChannelType::Pacman
            .implicit_distros()
            .iter()
            .any(|d| distro.matches(d))
        {
            Some(PkgMgr::Pacman)
        } else {
            None
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            PkgMgr::Dnf => "dnf",
            PkgMgr::Apt => "apt",
            PkgMgr::Pacman => "pacman",
        }
    }

    /// Build the install command for the given packages.
    pub fn install_command(self, packages: &[&str]) -> String {
        let pkgs = packages.join(" ");
        match self {
            PkgMgr::Dnf => format!("sudo dnf install -y {pkgs}"),
            PkgMgr::Apt => format!("sudo apt-get install -y {pkgs}"),
            PkgMgr::Pacman => format!("sudo pacman -S --needed --noconfirm {pkgs}"),
        }
    }
}

/// Look up the package that provides `binary` for the given package manager.
/// Returns `None` if grimoire has no hint for this binary on this manager —
/// the caller should treat that as a hard error.
pub fn package_for(binary: &str, mgr: PkgMgr) -> Option<&'static str> {
    match (binary, mgr) {
        ("cc", PkgMgr::Dnf) => Some("gcc"),
        ("cc", PkgMgr::Apt) => Some("build-essential"),
        ("cc", PkgMgr::Pacman) => Some("base-devel"),

        ("pkg-config", PkgMgr::Dnf) => Some("pkgconf-pkg-config"),
        ("pkg-config", PkgMgr::Apt) => Some("pkg-config"),
        ("pkg-config", PkgMgr::Pacman) => Some("pkgconf"),

        ("make", _) => Some("make"),
        ("curl", _) => Some("curl"),
        ("wget", _) => Some("wget"),
        ("git", _) => Some("git"),

        _ => None,
    }
}

/// Check each entry in `deps` with `command -v`. For any that are missing,
/// install the matching package via the current distro's package manager.
/// On `dry_run`, only print the plan.
///
/// Errors:
/// - The current distro has no known package manager.
/// - A binary has no install hint for the current package manager.
/// - The install command exits non-zero.
/// - A binary is still missing after the install attempt.
pub fn ensure(deps: &[String], dry_run: bool) -> Result<()> {
    if deps.is_empty() {
        return Ok(());
    }
    let missing: Vec<String> = deps.iter().filter(|d| !is_in_path(d)).cloned().collect();
    if missing.is_empty() {
        return Ok(());
    }

    let distro = crate::distro::current();
    let Some(mgr) = PkgMgr::for_distro(distro) else {
        bail!(
            "missing system dependencies {:?} but distro {:?} has no known package manager — install them manually",
            missing,
            distro.id,
        );
    };

    let mut packages: Vec<&str> = Vec::with_capacity(missing.len());
    for binary in &missing {
        let Some(pkg) = package_for(binary, mgr) else {
            bail!(
                "no install hint for `{binary}` on {} ({}); please install it manually",
                distro.id,
                mgr.label(),
            );
        };
        packages.push(pkg);
    }
    packages.sort();
    packages.dedup();

    let cmd = mgr.install_command(&packages);
    eprintln!("→ Installing system dependencies: {}", missing.join(", "));
    eprintln!("  {cmd}");

    if dry_run {
        return Ok(());
    }

    let exit = shell::run("system_deps", &cmd)?;
    if exit != 0 {
        bail!("system dependency install failed (exit {exit})");
    }

    let still_missing: Vec<String> = missing.iter().filter(|d| !is_in_path(d)).cloned().collect();
    if !still_missing.is_empty() {
        bail!(
            "system dependencies still missing after install: {}",
            still_missing.join(", "),
        );
    }
    Ok(())
}

fn is_in_path(binary: &str) -> bool {
    let snippet = format!("command -v {}", shell_quote(binary));
    matches!(shell::check("system_deps_check", &snippet), Ok(out) if out.ok())
}

/// Quote `s` for safe inclusion inside a single-argument bash word. We never
/// expect anything exotic in `system_requires` entries, but a hostile or
/// typoed value shouldn't break out of the snippet.
fn shell_quote(s: &str) -> String {
    let escaped = s.replace('\'', "'\\''");
    format!("'{escaped}'")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn distro(id: &str, id_like: &[&str]) -> Distro {
        Distro {
            id: id.to_string(),
            id_like: id_like.iter().map(|s| s.to_string()).collect(),
        }
    }

    #[test]
    fn pkgmgr_for_known_distros() {
        assert_eq!(
            PkgMgr::for_distro(&distro("fedora", &[])),
            Some(PkgMgr::Dnf)
        );
        assert_eq!(
            PkgMgr::for_distro(&distro("ubuntu", &[])),
            Some(PkgMgr::Apt)
        );
        assert_eq!(
            PkgMgr::for_distro(&distro("arch", &[])),
            Some(PkgMgr::Pacman)
        );
    }

    #[test]
    fn pkgmgr_uses_id_like_fallback() {
        // Mint reports ID=linuxmint, ID_LIKE=ubuntu debian
        assert_eq!(
            PkgMgr::for_distro(&distro("linuxmint", &["ubuntu", "debian"])),
            Some(PkgMgr::Apt),
        );
    }

    #[test]
    fn pkgmgr_unknown_distro() {
        assert_eq!(PkgMgr::for_distro(&distro("voidlinux", &[])), None);
    }

    #[test]
    fn cc_maps_per_distro() {
        assert_eq!(package_for("cc", PkgMgr::Dnf), Some("gcc"));
        assert_eq!(package_for("cc", PkgMgr::Apt), Some("build-essential"));
        assert_eq!(package_for("cc", PkgMgr::Pacman), Some("base-devel"));
    }

    #[test]
    fn common_binaries_share_name_across_managers() {
        for mgr in [PkgMgr::Dnf, PkgMgr::Apt, PkgMgr::Pacman] {
            assert_eq!(package_for("make", mgr), Some("make"));
            assert_eq!(package_for("curl", mgr), Some("curl"));
            assert_eq!(package_for("wget", mgr), Some("wget"));
            assert_eq!(package_for("git", mgr), Some("git"));
        }
    }

    #[test]
    fn unknown_binary_returns_none() {
        assert_eq!(package_for("nonsense-tool", PkgMgr::Dnf), None);
    }

    #[test]
    fn install_commands_have_noninteractive_flags() {
        let cmd = PkgMgr::Dnf.install_command(&["gcc"]);
        assert!(cmd.contains("-y"));
        let cmd = PkgMgr::Apt.install_command(&["build-essential"]);
        assert!(cmd.contains("-y"));
        let cmd = PkgMgr::Pacman.install_command(&["base-devel"]);
        assert!(cmd.contains("--noconfirm"));
    }

    #[test]
    fn ensure_no_op_on_empty_deps() {
        ensure(&[], false).unwrap();
    }

    #[test]
    fn shell_quote_escapes_single_quotes() {
        assert_eq!(shell_quote("cc"), "'cc'");
        assert_eq!(shell_quote("a'b"), "'a'\\''b'");
    }
}
