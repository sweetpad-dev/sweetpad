//! Which Linux this is, the packages a build needs on it, and installing
//! them with the distribution's package manager.

use std::collections::BTreeMap;
use std::process::{Command, Stdio};

use serde::Deserialize;

/// The package lists in `linux-toolchain.json`.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct HostPackages {
    pub(crate) apt: Vec<String>,
    pub(crate) pacman: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Manager {
    Apt,
    Pacman,
}

impl Manager {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Apt => "APT",
            Self::Pacman => "pacman",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Host {
    /// `PRETTY_NAME`, for messages.
    pub(crate) name: String,
    /// `None` on a distribution setup has no package list for.
    pub(crate) manager: Option<Manager>,
    pub(crate) packages: Vec<String>,
    /// The swift.org platform whose Swift release runs here, such as
    /// `ubuntu24.04` or `debian12-aarch64`.
    pub(crate) swift_platform: String,
    /// Whether the Swift release needs its ncurses libraries renamed, as on
    /// Arch.
    pub(crate) ncursesw: bool,
    pub(crate) warnings: Vec<String>,
}

/// The values of an `os-release` file.
pub(crate) fn os_release(text: &str) -> BTreeMap<String, String> {
    text.lines()
        .filter_map(|line| line.trim().split_once('='))
        .filter(|(key, _)| !key.starts_with('#'))
        .map(|(key, value)| {
            let value = value.trim();
            let unquoted = value
                .strip_prefix('"')
                .and_then(|v| v.strip_suffix('"'))
                .or_else(|| value.strip_prefix('\'').and_then(|v| v.strip_suffix('\'')))
                .unwrap_or(value);
            (key.trim().to_string(), unquoted.replace("\\\"", "\""))
        })
        .collect()
}

/// The host described by `os_release` on `arch` (`uname -m`).
pub(crate) fn detect(
    os_release_text: &str,
    arch: &str,
    pins: &HostPackages,
) -> Result<Host, String> {
    if !matches!(arch, "aarch64" | "x86_64") {
        return Err("sweetpad setup needs an ARM64 or x86_64 Linux machine.".into());
    }
    let values = os_release(os_release_text);
    let id = values.get("ID").map_or("", String::as_str);
    let version = values.get("VERSION_ID").map_or("", String::as_str);
    let name = values
        .get("PRETTY_NAME")
        .or_else(|| values.get("ID"))
        .cloned()
        .unwrap_or_else(|| "unknown Linux".into());
    let mut host = Host {
        name,
        manager: None,
        packages: Vec::new(),
        swift_platform: "ubuntu24.04".into(),
        ncursesw: false,
        warnings: Vec::new(),
    };
    match (id, version) {
        ("ubuntu", "24.04") => {
            host.manager = Some(Manager::Apt);
            host.packages.clone_from(&pins.apt);
        }
        ("debian", "12" | "13") => {
            host.manager = Some(Manager::Apt);
            host.packages.clone_from(&pins.apt);
            host.swift_platform = format!("debian{version}");
        }
        ("arch", _) => {
            // swift.org publishes no Arch release; the Ubuntu one runs with
            // Arch's compatibility libraries, which the probe build checks.
            host.ncursesw = true;
            if arch == "x86_64" {
                host.manager = Some(Manager::Pacman);
                host.packages.clone_from(&pins.pacman);
            } else {
                host.warnings.push(
                    "Arch Linux ARM is untested; setup uses the packages already installed.".into(),
                );
            }
        }
        ("alpine", _) => {
            return Err(
                "sweetpad runs on Alpine, but setting up the Apple toolchain needs \
                        glibc. Run setup in an Ubuntu or Debian container."
                    .into(),
            );
        }
        _ => host.warnings.push(format!(
            "{} is untested. Setup uses the packages already installed: install a C/C++ \
             toolchain, Bash and Swift's runtime libraries with your package \
             manager. It tries the Ubuntu 24.04 Swift release.",
            host.name
        )),
    }
    if arch == "aarch64" {
        host.swift_platform.push_str("-aarch64");
    }
    Ok(host)
}

/// This machine.
pub(crate) fn current(pins: &HostPackages) -> Result<Host, String> {
    let text = std::fs::read_to_string("/etc/os-release")
        .or_else(|_| std::fs::read_to_string("/usr/lib/os-release"))
        .unwrap_or_default();
    detect(&text, std::env::consts::ARCH, pins)
}

/// The host's packages that are not installed.
pub(crate) fn missing_packages(host: &Host) -> Result<Vec<String>, String> {
    let Some(manager) = host.manager else {
        return Ok(Vec::new());
    };
    let mut command = match manager {
        Manager::Apt => {
            let mut command = Command::new("dpkg-query");
            command.args(["-W", "-f", "${Package} ${db:Status-Abbrev}\n"]);
            command
        }
        Manager::Pacman => {
            let mut command = Command::new("pacman");
            command.arg("-T");
            command
        }
    };
    let output = command
        .args(&host.packages)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .map_err(|error| format!("Couldn't list installed packages: {error}."))?;
    let missing_status = match manager {
        Manager::Apt => 1,
        Manager::Pacman => 127,
    };
    if !output.status.success() && output.status.code() != Some(missing_status) {
        return Err(format!(
            "{} package query failed ({})",
            manager.name(),
            output.status
        ));
    }
    let text = String::from_utf8_lossy(&output.stdout);
    Ok(match manager {
        // Installed packages show as `ii`; packages dpkg has never heard of
        // print nothing.
        Manager::Apt => {
            let installed: Vec<&str> = text
                .lines()
                .filter_map(|line| line.split_once(' '))
                .filter(|(_, status)| status.as_bytes().get(1) == Some(&b'i'))
                .map(|(name, _)| name.split(':').next().unwrap_or(name))
                .collect();
            host.packages
                .iter()
                .filter(|p| !installed.contains(&p.as_str()))
                .cloned()
                .collect()
        }
        // `pacman -T` prints the dependencies that are not satisfied.
        Manager::Pacman => text.lines().map(str::to_string).collect(),
    })
}

/// A command the user can paste to install the packages the probe found.
pub(crate) fn missing_detail(manager: Manager, packages: &[String]) -> String {
    // SAFETY: geteuid has no preconditions.
    let prefix = if unsafe { libc::geteuid() } == 0 {
        ""
    } else {
        "sudo "
    };
    let command = match manager {
        Manager::Apt => format!("{prefix}apt-get update && {prefix}apt-get install"),
        Manager::Pacman => format!("{prefix}pacman -S --needed"),
    };
    format!(
        "Missing host packages. Run '{command} {}'.",
        packages.join(" ")
    )
}

/// Install `packages` with the host's package manager, through `sudo`
/// unless running as root. `interactive` lets `sudo` ask for a password.
pub(crate) fn install(host: &Host, packages: &[String], interactive: bool) -> Result<(), String> {
    let Some(manager) = host.manager else {
        return Ok(());
    };
    // SAFETY: geteuid has no preconditions.
    let root = unsafe { libc::geteuid() } == 0;
    let privileged = |program: &str| {
        let mut command = if root {
            Command::new(program)
        } else {
            let mut command = Command::new("sudo");
            if !interactive {
                command.arg("--non-interactive");
            }
            command.arg(program);
            command
        };
        // Package managers print progress; keep stdout for the report.
        command.stdout(Stdio::from(std::io::stderr()));
        command
    };
    if !root && super::on_path("sudo").is_none() {
        return Err(format!(
            "Installing {} packages needs root, and sudo isn't installed. Install sudo, or \
             pass --skip-system-packages after installing them yourself.",
            manager.name()
        ));
    }
    let run = |mut command: Command, what: &str| -> Result<(), String> {
        let status = command
            .status()
            .map_err(|error| format!("Couldn't start {what}: {error}."))?;
        if status.success() {
            Ok(())
        } else {
            Err(format!("{what} failed ({status})."))
        }
    };
    match manager {
        Manager::Apt => {
            let mut update = privileged("env");
            update.args(["DEBIAN_FRONTEND=noninteractive", "apt-get", "update", "-qq"]);
            run(update, "apt-get update")?;
            let mut install = privileged("env");
            install
                .args([
                    "DEBIAN_FRONTEND=noninteractive",
                    "apt-get",
                    "install",
                    "-yqq",
                ])
                .args(packages);
            run(install, "apt-get install")
        }
        Manager::Pacman => {
            // Refreshing Arch's database without a full upgrade leaves a
            // partial upgrade, so setup only installs, and the user upgrades
            // with 'pacman -Syu' first.
            let mut install = privileged("pacman");
            install
                .args(["-S", "--needed", "--noconfirm"])
                .args(packages);
            run(install, "pacman").map_err(|error| {
                format!("{error} Update Arch with 'pacman -Syu', then run setup again.")
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pins() -> HostPackages {
        crate::cli::commands::sdk::toolchain().host_packages.clone()
    }

    #[test]
    fn supported_distributions_get_their_packages_and_swift_release() {
        let host = detect(
            "ID=ubuntu\nVERSION_ID=\"24.04\"\nPRETTY_NAME=\"Ubuntu 24.04.3 LTS\"\n",
            "x86_64",
            &pins(),
        )
        .unwrap();
        assert_eq!(host.manager, Some(Manager::Apt));
        assert_eq!(host.swift_platform, "ubuntu24.04");
        assert!(!host.packages.contains(&"llvm-18".to_string()));
        assert!(host.packages.contains(&"bash".to_string()));
        assert_eq!(host.name, "Ubuntu 24.04.3 LTS");

        let host = detect("ID=debian\nVERSION_ID=\"13\"\n", "aarch64", &pins()).unwrap();
        assert_eq!(host.swift_platform, "debian13-aarch64");
        assert!(!host.packages.contains(&"llvm".to_string()));

        let host = detect("ID=arch\n", "x86_64", &pins()).unwrap();
        assert_eq!(host.manager, Some(Manager::Pacman));
        assert!(host.ncursesw && host.packages.contains(&"patchelf".to_string()));
        assert_eq!(host.swift_platform, "ubuntu24.04");

        let host = detect("ID=arch\n", "aarch64", &pins()).unwrap();
        assert_eq!((host.manager, host.warnings.len()), (None, 1));

        let host = detect("ID=fedora\nVERSION_ID=42\n", "x86_64", &pins()).unwrap();
        assert!(host.manager.is_none() && host.warnings[0].contains("untested"));
    }

    #[test]
    fn alpine_and_other_architectures_are_refused() {
        assert!(
            detect("ID=alpine\n", "x86_64", &pins())
                .unwrap_err()
                .contains("glibc")
        );
        assert!(detect("ID=ubuntu\nVERSION_ID=24.04\n", "riscv64", &pins()).is_err());
    }
}
