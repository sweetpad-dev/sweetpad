//! The Xcode a build-gated oracle builds with, shared by the oracles that run
//! `xcodebuild`, `sourcekit-lsp` or the toolchain.
//!
//! `BSP_ORACLE_XCODE` names it: the `.app` or its `Developer` directory.
//! Unset, it is the selected one: `DEVELOPER_DIR`, then `xcode-select -p`,
//! which is what CI's matrix selects.

#![allow(dead_code)] // each oracle uses its own share of this

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::Command;

use sweetpad_lib::xcode::{self, XcodeLayout};

/// The Xcode an oracle runs against.
pub struct OracleXcode {
    /// The path as it was named, for a `--xcode` argument.
    pub path: PathBuf,
    /// Its roots and version, spelled through the canonical path.
    pub layout: XcodeLayout,
}

impl OracleXcode {
    /// The `DEVELOPER_DIR` for this Xcode's tools.
    pub fn developer_dir(&self) -> &Path {
        &self.layout.developer_dir
    }

    /// A tool from this Xcode's default toolchain, such as `swiftc`.
    pub fn tool(&self, name: &str) -> PathBuf {
        self.developer_dir()
            .join("Toolchains/XcodeDefault.xctoolchain/usr/bin")
            .join(name)
    }

    /// `program` with `DEVELOPER_DIR` set to this Xcode and `TMPDIR` to
    /// `tmp`, so what the Swift driver and SwiftPM leave in their `TMPDIR`
    /// goes with the oracle's scratch directory instead of staying in the
    /// user's.
    pub fn command(&self, program: impl AsRef<OsStr>, tmp: &Path) -> Command {
        let mut cmd = Command::new(program);
        cmd.env("DEVELOPER_DIR", self.developer_dir())
            .env("TMPDIR", tmp);
        cmd
    }
}

/// The Xcode `BSP_ORACLE_XCODE` names, if it is set.
fn pinned() -> Option<PathBuf> {
    std::env::var_os("BSP_ORACLE_XCODE")
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
}

/// The Xcode to run against, or `None` when there is none, with the reason
/// printed. An Xcode `BSP_ORACLE_XCODE` names has to exist.
pub fn find() -> Option<OracleXcode> {
    let pinned = pinned();
    let path = pinned.clone().unwrap_or_else(xcode::detect_developer_dir);
    let layout = match xcode::locate(&path) {
        Ok(layout) => layout,
        Err(e) => {
            assert!(pinned.is_none(), "BSP_ORACLE_XCODE: {e}");
            eprintln!("skipping: no Xcode to build with: {e}");
            return None;
        }
    };
    eprintln!(
        "running against Xcode {} at {}",
        layout.short_version,
        path.display()
    );
    Some(OracleXcode { path, layout })
}
