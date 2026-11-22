//! The pinned Swift release: found on `PATH`, in swiftly's toolchains or
//! where setup installs it, or downloaded from swift.org, checked against
//! swift.org's signing keys and unpacked as published.

use std::fs;
use std::io::{Read, Write};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use super::host::Host;

/// A Swift toolchain of the pinned version.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Swift {
    /// Its `usr/bin`.
    pub(crate) bin: PathBuf,
    /// The first line of `swift --version`.
    pub(crate) version: String,
}

fn home() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_default()
}

/// Where setup installs Swift `version`.
pub(crate) fn install_dir(version: &str) -> PathBuf {
    home().join(format!("swift/swift-{version}-RELEASE"))
}

/// The `usr/bin` folders that may hold Swift `version`, in the order they
/// are tried: the `PATH`'s, swiftly's, then setup's own.
fn candidates(version: &str) -> Vec<PathBuf> {
    let mut found: Vec<PathBuf> = super::on_path("swift")
        .and_then(|swift| fs::canonicalize(swift).ok())
        .and_then(|swift| swift.parent().map(Path::to_path_buf))
        .into_iter()
        .collect();
    let swiftly = std::env::var_os("SWIFTLY_HOME_DIR").map_or_else(
        || {
            std::env::var_os("XDG_DATA_HOME")
                .map(PathBuf::from)
                .filter(|p| p.is_absolute())
                .unwrap_or_else(|| home().join(".local/share"))
                .join("swiftly")
        },
        PathBuf::from,
    );
    found.push(swiftly.join("toolchains").join(version).join("usr/bin"));
    found.push(install_dir(version).join("usr/bin"));
    found
}

/// The first line of `bin/swift --version` when it reports `version`.
fn reports(bin: &Path, version: &str) -> Option<String> {
    let swift = bin.join("swift");
    if !swift.is_file() {
        return None;
    }
    // `swift --version` leaves a `TemporaryDirectory.*` behind in `TMPDIR`.
    let scratch = sweetpad_core::scratch::ScratchDir::new("sweetpad-swift-version").ok()?;
    let output = Command::new(&swift)
        .arg("--version")
        .env("TMPDIR", scratch.as_os_str())
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&output.stdout);
    let line = text
        .lines()
        .find(|l| l.contains("Swift version"))?
        .trim()
        .to_string();
    let after = line.split("Swift version ").nth(1)?;
    let reported = after.split([' ', '(']).next()?;
    (output.status.success() && reported == version).then_some(line)
}

/// Swift `version`, wherever it is installed.
pub(crate) fn find(version: &str) -> Option<Swift> {
    candidates(version)
        .into_iter()
        .find_map(|bin| reports(&bin, version).map(|version| Swift { bin, version }))
}

/// The pinned Swift, looked up once per process.
pub(crate) fn pinned() -> Option<&'static Swift> {
    static PINNED: std::sync::OnceLock<Option<Swift>> = std::sync::OnceLock::new();
    PINNED
        .get_or_init(|| find(&crate::cli::commands::sdk::toolchain().swift))
        .as_ref()
}

/// `name` from the pinned Swift's `usr/bin` when it is installed, else the
/// bare name for `PATH` to resolve.
pub(crate) fn tool(name: &str) -> PathBuf {
    pinned().map_or_else(|| PathBuf::from(name), |swift| swift.bin.join(name))
}

/// The swift.org URL of the release for `platform`, such as `ubuntu24.04`
/// or `debian12-aarch64`.
pub(crate) fn release_url(version: &str, platform: &str) -> String {
    let tag = format!("swift-{version}-RELEASE");
    let folder = platform.replace('.', "");
    format!(
        "https://download.swift.org/swift-{version}-release/{folder}/{tag}/{tag}-{platform}.tar.gz"
    )
}

fn download_dir(cache: &Path) -> std::io::Result<PathBuf> {
    sweetpad_lib::cache::prepare(cache, Path::new("swift"))
}

/// Keep the lock file in place so a new arrival always locks the same inode.
fn install_lock(path: &Path, note: &mut dyn FnMut(&str)) -> Result<fs::File, String> {
    let file = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
        .map_err(|e| format!("Couldn't open {}: {e}.", path.display()))?;
    match file.try_lock() {
        Ok(()) => {}
        Err(fs::TryLockError::WouldBlock) => {
            note(&format!(
                "Waiting for another Swift setup ({})",
                path.display()
            ));
            file.lock()
                .map_err(|e| format!("Couldn't lock {}: {e}.", path.display()))?;
        }
        Err(fs::TryLockError::Error(e)) => {
            return Err(format!("Couldn't lock {}: {e}.", path.display()));
        }
    }
    Ok(file)
}

/// Return the verified toolchain and whether this call installed it.
pub(crate) fn install(
    host: &Host,
    version: &str,
    note: &mut dyn FnMut(&str),
) -> Result<(Swift, bool), String> {
    install_at(
        &install_dir(version),
        &sweetpad_lib::cache::root(),
        host,
        version,
        note,
    )
}

fn install_at(
    target: &Path,
    cache: &Path,
    host: &Host,
    version: &str,
    note: &mut dyn FnMut(&str),
) -> Result<(Swift, bool), String> {
    let parent = target.parent().unwrap_or(Path::new("/"));
    fs::create_dir_all(parent)
        .map_err(|e| format!("Couldn't create {}: {e}.", parent.display()))?;
    let _install = install_lock(&parent.join(format!(".swift-{version}-RELEASE.lock")), note)?;
    // Another setup may have finished while we waited, even with a different cache.
    if target.join("usr/bin/swift").is_file() {
        // Older Arch installs may still need the ncurses adjustment.
        if host.ncursesw && reports(&target.join("usr/bin"), version).is_none() {
            use_ncursesw(&target.join("usr"))?;
        }
        return check_toolchain(target, version, host).map(|swift| (swift, false));
    }
    if fs::symlink_metadata(target).is_ok() {
        return Err(format!(
            "{} holds an incomplete Swift. Move it aside and run setup again.",
            target.display()
        ));
    }
    let gpg = super::on_path("gpg").ok_or_else(|| {
        "Verifying the Swift download needs gpg. Install gnupg, or install Swift \
         yourself (for example with swiftly) and run setup again."
            .to_string()
    })?;
    let downloads = download_dir(cache).map_err(|e| e.to_string())?;
    let needed = 5 * 1024 * 1024 * 1024;
    let free = crate::cli::apple_archive::free_space(parent)?;
    if free < needed {
        return Err(format!(
            "Installing Swift needs 5 GiB free in {}, and {} GiB is.",
            parent.display(),
            free / (1 << 20) / 1024
        ));
    }
    let url = release_url(version, &host.swift_platform);
    let name = url.rsplit('/').next().unwrap_or_default();
    let archive = downloads.join(name);
    // Different installation destinations can share one XDG download cache.
    let _download = install_lock(&downloads.join(format!(".{name}.lock")), note)?;
    let work = sweetpad_core::scratch::ScratchDir::new_in(&downloads, ".verify")
        .map_err(|e| format!("Couldn't create a folder in {}: {e}.", downloads.display()))?;
    let signature = work.join("release.sig");
    let keys = work.join("swift-keys.asc");
    if !archive.is_file() {
        note(&format!("Downloading {url}"));
        fetch(&url, &archive)?;
    }
    fetch(&format!("{url}.sig"), &signature)?;
    fetch("https://www.swift.org/keys/all-keys.asc", &keys)?;
    note("Checking the download against swift.org's signing keys");
    verify(&gpg, &keys, &signature, &archive, &work)?;
    note(&format!("Unpacking Swift {version}"));
    let swift = unpack(&archive, target, version, host)?;
    let _ = fs::remove_file(&archive);
    Ok((swift, true))
}

fn check_toolchain(root: &Path, version: &str, host: &Host) -> Result<Swift, String> {
    let bin = root.join("usr/bin");
    let swift = Swift {
        version: reports(&bin, version).ok_or_else(|| {
            format!(
                "The Swift in {} doesn't start, or isn't version {version}.",
                root.display()
            )
        })?,
        bin,
    };
    probe(&swift, host)?;
    Ok(swift)
}

/// Download `url` to `path`, through a `.part` file.
fn fetch(url: &str, path: &Path) -> Result<(), String> {
    let session = crate::cli::net::Session::from_env();
    let request = crate::cli::net::Request::new(minreq::Method::Get, url).timeout(6 * 3600);
    let response = session.stream(&request).map_err(|e| e.to_string())?;
    if !(200..300).contains(&response.status) {
        return Err(format!(
            "Downloading {url} failed (HTTP {}).",
            response.status
        ));
    }
    let mut body = crate::cli::net::decoded(&response.headers, response.body);
    let partial = path.with_extension("part");
    let failed = |e: std::io::Error| format!("Couldn't write {}: {e}.", partial.display());
    let mut output = fs::File::create(&partial).map_err(failed)?;
    let mut buffer = vec![0u8; 1024 * 1024];
    loop {
        let read = match body.read(&mut buffer) {
            Ok(0) => break,
            Ok(read) => read,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(format!("Downloading {url} was interrupted: {e}.")),
        };
        output.write_all(&buffer[..read]).map_err(failed)?;
    }
    output.sync_all().map_err(failed)?;
    fs::rename(&partial, path).map_err(|e| format!("Couldn't write {}: {e}.", path.display()))
}

/// Check `signature` over `archive` with swift.org's `keys`, in a keyring
/// of its own so the user's stays as it is.
fn verify(
    gpg: &Path,
    keys: &Path,
    signature: &Path,
    archive: &Path,
    work: &Path,
) -> Result<(), String> {
    let keyring = work.join(".gnupg");
    fs::DirBuilder::new()
        .mode(0o700)
        .create(&keyring)
        .map_err(|e| format!("Couldn't create {}: {e}.", keyring.display()))?;
    let run = |args: &[&std::ffi::OsStr]| {
        Command::new(gpg)
            .arg("--batch")
            .arg("--homedir")
            .arg(&keyring)
            .args(args)
            .stdin(Stdio::null())
            .output()
    };
    let imported = run(&["--quiet".as_ref(), "--import".as_ref(), keys.as_os_str()]);
    let verified = imported.and_then(|_| {
        run(&[
            "--verify".as_ref(),
            signature.as_os_str(),
            archive.as_os_str(),
        ])
    });
    match verified {
        Ok(output) if output.status.success() => Ok(()),
        Ok(output) => {
            let _ = fs::remove_file(archive);
            Err(format!(
                "The Swift download doesn't match swift.org's signature, so it was deleted: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            ))
        }
        Err(e) => Err(format!("Couldn't start gpg: {e}.")),
    }
}

/// Unpack the release `archive` into `target`, through a folder beside it,
/// so a failure never leaves a toolchain that looks usable.
fn unpack(archive: &Path, target: &Path, version: &str, host: &Host) -> Result<Swift, String> {
    let parent = target.parent().unwrap_or(Path::new("/"));
    let stage = sweetpad_core::scratch::ScratchDir::new_in(parent, ".sweetpad-toolchain")
        .map_err(|e| format!("Couldn't create a folder in {}: {e}.", parent.display()))?;
    let file = fs::File::open(archive)
        .map_err(|e| format!("Couldn't read {}: {e}.", archive.display()))?;
    let mut tar = tar::Archive::new(flate2::read::GzDecoder::new(std::io::BufReader::new(file)));
    tar.set_preserve_permissions(true);
    tar.set_preserve_mtime(true);
    tar.unpack(&stage)
        .map_err(|e| format!("Couldn't unpack {}: {}.", archive.display(), chain(&e)))?;
    let mut top = fs::read_dir(&stage)
        .map_err(|e| e.to_string())?
        .filter_map(Result::ok)
        .map(|entry| entry.path());
    let (Some(root), None) = (top.next(), top.next()) else {
        return Err(format!(
            "{} doesn't hold one toolchain folder.",
            archive.display()
        ));
    };
    if !root.join("usr/bin/swift").is_file() {
        return Err(format!("{} holds no usr/bin/swift.", archive.display()));
    }
    if host.ncursesw {
        use_ncursesw(&root.join("usr"))?;
    }
    let checked = check_toolchain(&root, version, host)?;
    if fs::symlink_metadata(target).is_ok() {
        return Err(format!(
            "{} appeared while Swift was unpacking. Move it aside and run setup again.",
            target.display()
        ));
    }
    fs::rename(&root, target)
        .map_err(|e| format!("Couldn't move Swift to {}: {e}.", target.display()))?;
    Ok(Swift {
        bin: target.join("usr/bin"),
        version: checked.version,
    })
}

/// `error` with the errors that caused it, which tar's own message leaves
/// out (`failed to unpack ...` without `No space left on device`).
fn chain(error: &dyn std::error::Error) -> String {
    let mut text = error.to_string();
    let mut source = error.source();
    while let Some(cause) = source {
        text.push_str(": ");
        text.push_str(&cause.to_string());
        source = cause.source();
    }
    text
}

/// Point the toolchain's binaries at `libncursesw`, as Arch's `swift-bin`
/// package does (aur.archlinux.org/packages/swift-bin). Only this private
/// toolchain changes, never the libraries under `/usr/lib`.
fn use_ncursesw(usr: &Path) -> Result<(), String> {
    let patchelf = super::on_path("patchelf")
        .ok_or("Swift on Arch needs patchelf. Install it with 'pacman -S patchelf'.")?;
    let mut pending = vec![usr.to_path_buf()];
    while let Some(dir) = pending.pop() {
        let entries =
            fs::read_dir(&dir).map_err(|e| format!("Couldn't read {}: {e}.", dir.display()))?;
        for entry in entries.filter_map(Result::ok) {
            let path = entry.path();
            let Ok(info) = fs::symlink_metadata(&path) else {
                continue;
            };
            if info.is_dir() {
                pending.push(path);
                continue;
            }
            let name = entry.file_name().to_string_lossy().into_owned();
            if !info.is_file() || !(info.permissions().mode() & 0o111 != 0 || name.contains(".so"))
            {
                continue;
            }
            if !dynamic_elf(&path) {
                continue;
            }
            let needed = Command::new(&patchelf)
                .arg("--print-needed")
                .arg(&path)
                .stderr(Stdio::null())
                .output()
                .map_err(|e| format!("Couldn't start patchelf: {e}."))?;
            let needed = String::from_utf8_lossy(&needed.stdout);
            for library in ["ncurses", "panel", "form"] {
                let old = format!("lib{library}.so.6");
                if needed.lines().any(|line| line.trim() == old) {
                    let status = Command::new(&patchelf)
                        .args(["--replace-needed", &old, &format!("lib{library}w.so.6")])
                        .arg(&path)
                        .status()
                        .map_err(|e| format!("Couldn't start patchelf: {e}."))?;
                    if !status.success() {
                        return Err(format!("patchelf couldn't update {}.", path.display()));
                    }
                }
            }
        }
    }
    Ok(())
}

/// Whether `path` is a 64-bit little-endian executable or shared library;
/// relocatable objects have no dependencies to change.
fn dynamic_elf(path: &Path) -> bool {
    let mut header = [0u8; 18];
    fs::File::open(path)
        .and_then(|mut f| f.read_exact(&mut header))
        .is_ok_and(|()| {
            header[..6] == *b"\x7fELF\x02\x01"
                && matches!(u16::from_le_bytes([header[16], header[17]]), 2 | 3)
        })
}

/// Compile and run a Foundation program: `swift --version` alone misses a
/// missing linker or runtime library.
pub(crate) fn probe(swift: &Swift, host: &Host) -> Result<(), String> {
    let scratch = sweetpad_core::scratch::ScratchDir::new("sweetpad-swift-probe")
        .map_err(|e| format!("Couldn't create a folder for the Swift check: {e}."))?;
    let source = scratch.join("main.swift");
    fs::write(
        &source,
        "import Foundation\nprint(\"Swift runs on \" + ProcessInfo.processInfo.operatingSystemVersionString)\n",
    )
    .map_err(|e| format!("Couldn't write {}: {e}.", source.display()))?;
    let binary = scratch.join("check");
    let compiled = Command::new(swift.bin.join("swiftc"))
        .arg(&source)
        .arg("-o")
        .arg(&binary)
        .env("TMPDIR", scratch.as_os_str())
        .stdin(Stdio::null())
        .output();
    let ran = compiled.as_ref().is_ok_and(|o| o.status.success())
        && Command::new(&binary)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .status()
            .is_ok_and(|s| s.success());
    if ran {
        return Ok(());
    }
    let detail = compiled.map_or_else(
        |e| e.to_string(),
        |o| String::from_utf8_lossy(&o.stderr).trim().to_string(),
    );
    Err(format!(
        "Swift can't compile and run a program on {}. Check the host packages, or use an \
         Ubuntu or Debian container.{}",
        host.name,
        if detail.is_empty() {
            String::new()
        } else {
            format!(" swiftc said: {detail}")
        }
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_host() -> Host {
        use crate::cli::linux_setup::host::{HostPackages, detect};
        detect(
            "ID=ubuntu\nVERSION_ID=24.04",
            "x86_64",
            &HostPackages::default(),
        )
        .unwrap()
    }

    fn release_fixture(path: &Path, compiler: &[u8]) {
        let gzip = flate2::write::GzEncoder::new(
            fs::File::create(path).unwrap(),
            flate2::Compression::fast(),
        );
        let mut tar = tar::Builder::new(gzip);
        for (name, bytes) in [
            (
                "release/usr/bin/swift",
                b"#!/bin/sh\necho 'Swift version 6.4.2'\n".as_slice(),
            ),
            ("release/usr/bin/swiftc", compiler),
        ] {
            let mut header = tar::Header::new_gnu();
            header.set_mode(0o755);
            header.set_size(bytes.len() as u64);
            tar.append_data(&mut header, name, bytes).unwrap();
        }
        tar.into_inner().unwrap().finish().unwrap();
    }

    #[test]
    fn a_waiting_install_reuses_the_verified_toolchain() {
        let dir = crate::cli::testdir::TempDir::new("swift-install-race");
        let target = dir.join("swift-6.4.2-RELEASE");
        let archive = dir.join("release.tar.gz");
        let lock = install_lock(
            &dir.join(".swift-6.4.2-RELEASE.lock"),
            &mut |_| unreachable!(),
        )
        .unwrap();
        let (waiting, started_wait) = std::sync::mpsc::channel();
        // Reusing the installation must work without accessing any download cache.
        let unavailable_cache = dir.join("unavailable-cache");
        fs::write(&unavailable_cache, b"").unwrap();
        let other_target = target.clone();
        let waiter = std::thread::spawn(move || {
            install_at(
                &other_target,
                &unavailable_cache,
                &test_host(),
                "6.4.2",
                &mut |msg| {
                    assert!(msg.starts_with("Waiting for another Swift setup"), "{msg}");
                    waiting.send(()).unwrap();
                },
            )
        });
        started_wait
            .recv_timeout(std::time::Duration::from_secs(10))
            .unwrap();

        // A failed compiler probe leaves the destination free for a retry.
        release_fixture(&archive, b"#!/bin/sh\nexit 1\n");
        assert!(unpack(&archive, &target, "6.4.2", &test_host()).is_err());
        assert!(!target.exists());
        release_fixture(
            &archive,
            b"#!/bin/sh\nprintf '#!/bin/sh\\nexit 0\\n' > \"$3\" && chmod +x \"$3\"\n",
        );
        let winner = unpack(&archive, &target, "6.4.2", &test_host()).unwrap();
        drop(lock);
        let (reused, installed) = waiter.join().unwrap().unwrap();
        assert!(!installed);
        assert_eq!(winner, reused);
        assert_eq!(winner.bin, target.join("usr/bin"));
    }

    #[test]
    fn catalog_then_setup_with_group_write_umask() {
        const CHILD: &str = "SWEETPAD_TEST_CATALOG_THEN_SETUP";
        if std::env::var_os(CHILD).is_none() {
            let home = crate::cli::testdir::TempDir::new("catalog-setup");
            let outcome = Command::new("sh")
                .args(["-c", "umask 0002; exec \"$@\"", "sh"])
                .arg(std::env::current_exe().unwrap())
                .arg("--exact")
                .arg(std::thread::current().name().unwrap())
                .env(CHILD, "1")
                .env("HOME", &*home)
                .env_remove("XDG_CACHE_HOME")
                .env_remove("SWEETPAD_CACHE_DIR")
                .output()
                .unwrap();
            assert!(outcome.status.success(), "{outcome:?}");
            assert!(home.join(".cache/sweetpad/swift").is_dir());
            return;
        }
        let root = sweetpad_lib::cache::root();
        assert!(!root.exists());
        let specs = root.parent().unwrap().parent().unwrap().join("specs");
        fs::create_dir(&specs).unwrap();
        sweetpad_lib::catalog_cache::load_cached_or_build(&specs, None, None).unwrap();
        assert!(
            fs::read_dir(&root)
                .unwrap()
                .flatten()
                .any(|entry| entry.file_name().to_string_lossy().starts_with("catalog-v"))
        );
        crate::cli::embedded::root().unwrap();
        let swift = download_dir(&root).unwrap();
        for directory in [root.join("distribution"), root, swift] {
            assert_eq!(
                directory.metadata().unwrap().permissions().mode() & 0o777,
                0o700
            );
        }
    }

    #[test]
    fn first_swift_download_creates_private_cache_directories() {
        let dir = crate::cli::testdir::TempDir::new("swift-cache");
        let cache = dir.join(".cache/sweetpad");
        let swift = download_dir(&cache).unwrap();
        for path in [&cache, &swift] {
            assert_eq!(path.metadata().unwrap().permissions().mode() & 0o777, 0o700);
        }
        fs::set_permissions(&cache, fs::Permissions::from_mode(0o775)).unwrap();
        assert!(download_dir(&cache).is_err());
    }

    #[test]
    fn release_urls_name_the_platform_folder_without_dots() {
        assert_eq!(
            release_url("6.4.2", "ubuntu24.04-aarch64"),
            "https://download.swift.org/swift-6.4.2-release/ubuntu2404-aarch64/swift-6.4.2-RELEASE/swift-6.4.2-RELEASE-ubuntu24.04-aarch64.tar.gz"
        );
        assert_eq!(
            release_url("6.4.2", "debian12"),
            "https://download.swift.org/swift-6.4.2-release/debian12/swift-6.4.2-RELEASE/swift-6.4.2-RELEASE-debian12.tar.gz"
        );
    }

    #[test]
    fn a_toolchain_counts_only_when_it_reports_the_pinned_version() {
        let dir = crate::cli::testdir::TempDir::new("swift-version");
        let bin = dir.join("usr/bin");
        fs::create_dir_all(&bin).unwrap();
        let swift = bin.join("swift");
        fs::write(
            &swift,
            "#!/bin/sh\necho 'Swift version 6.4.20 (swift-6.4.20-RELEASE)'\n",
        )
        .unwrap();
        fs::set_permissions(&swift, fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(reports(&bin, "6.4.2"), None);
        fs::write(&swift, "#!/bin/sh\necho 'Swift version 6.4.2 (swift-6.4.2-RELEASE)'\necho 'Target: x86_64-unknown-linux-gnu'\n").unwrap();
        assert_eq!(
            reports(&bin, "6.4.2").as_deref(),
            Some("Swift version 6.4.2 (swift-6.4.2-RELEASE)")
        );
    }

    #[test]
    fn only_dynamic_elf_files_are_patched() {
        let dir = crate::cli::testdir::TempDir::new("swift-elf");
        let mut header = b"\x7fELF\x02\x01".to_vec();
        header.resize(16, 0);
        for (kind, expected) in [(1u16, false), (2, true), (3, true)] {
            let mut data = header.clone();
            data.extend(kind.to_le_bytes());
            let path = dir.join(format!("elf{kind}"));
            fs::write(&path, data).unwrap();
            assert_eq!(dynamic_elf(&path), expected, "{kind}");
        }
        fs::write(dir.join("text"), "#!/bin/sh\n").unwrap();
        assert!(!dynamic_elf(&dir.join("text")));
    }
}
