use std::{env, fs, path::Path, process::Command};

fn main() {
    embed_injection_client();
    emit_lib_dir();
    emit_version();
}

/// Stamp the version the binary reports.
///
/// `CARGO_PKG_VERSION` alone cannot tell a build off `main` from the release
/// that shares its number: the crate version changes only when a release bumps
/// it, so every commit after `cli-v0.1.2` reports `0.1.2` too. That turns "is
/// this already fixed in the release?" into a confident wrong answer — you read
/// the released version string off a local build and record a claim about code
/// nobody else can run.
///
/// So only a build made at the matching `cli-v<version>` tag reports the bare
/// version; anything else carries the commit it came from, which is also what a
/// bug report wants quoted. Sources unpacked without a repository fall back to
/// the bare version, having nothing more honest to say.
///
/// Uncommitted edits are not reflected: the rerun triggers below cover the
/// operations that move HEAD or the tags, not an unstaged working-tree change.
fn emit_version() {
    let version = env::var("CARGO_PKG_VERSION").unwrap_or_default();
    // This build script narrows `rerun-if-changed`, so the git state it reads
    // has to be declared or the stamp is computed once and never refreshed.
    if let Some(git_dir) = git(&["rev-parse", "--absolute-git-dir"]) {
        for path in ["HEAD", "refs", "packed-refs", "index"] {
            println!("cargo:rerun-if-changed={}", Path::new(&git_dir).join(path).display());
        }
    }
    println!("cargo:rustc-env=SWEETPAD_VERSION={}", stamp(&version));
}

/// Run a git command, yielding its trimmed stdout when it succeeds with output.
fn git(args: &[&str]) -> Option<String> {
    let out = Command::new("git").args(args).output().ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8(out.stdout).ok()?.trim().to_string();
    (!text.is_empty()).then_some(text)
}

fn stamp(version: &str) -> String {
    let Some(sha) = git(&["rev-parse", "--short", "HEAD"]) else {
        return version.to_string();
    };
    let tag = format!("cli-v{version}");
    if git(&["describe", "--tags", "--exact-match", "--match", &tag, "HEAD"]).is_some() {
        return version.to_string();
    }
    format!("{version}-dev+{sha}")
}

/// Stage the bundled hot-reload injection clients (CLI_DESIGN §9d) for the
/// `include_bytes!` in `cli::inject::client` — one per supported SDK (iOS
/// simulator + macOS). The dylibs are produced by
/// `vendor/injection-client/build.sh` (macOS + Xcode) and are intentionally not
/// committed, so copy each into `OUT_DIR` when present and otherwise stage an
/// empty placeholder — every build then compiles, and the CLI falls back at
/// runtime when a client wasn't bundled. CI and release builds run `build.sh`
/// first.
fn embed_injection_client() {
    stage_client("SweetpadInjectionClient.dylib", "injection-client.dylib");
    stage_client(
        "SweetpadInjectionClientMac.dylib",
        "injection-client-mac.dylib",
    );
}

fn stage_client(prebuilt: &str, staged: &str) {
    let src = Path::new(&env::var("CARGO_MANIFEST_DIR").unwrap())
        .join("vendor/injection-client/prebuilt")
        .join(prebuilt);
    let dest = Path::new(&env::var("OUT_DIR").unwrap()).join(staged);
    println!("cargo:rerun-if-changed={}", src.display());
    if src.exists() {
        fs::copy(&src, &dest).expect("stage bundled injection client into OUT_DIR");
    } else {
        fs::write(&dest, []).expect("stage empty injection-client placeholder into OUT_DIR");
    }
}

/// The CLI's SPM oracle test reads fixtures from the sibling `sweetpad-lib`
/// crate; expose its canonical path the same way `sweetpad-core` does.
fn emit_lib_dir() {
    let manifest = env::var("CARGO_MANIFEST_DIR").unwrap();
    let lib = Path::new(&manifest).parent().unwrap().join("sweetpad-lib");
    let lib = fs::canonicalize(&lib).unwrap_or(lib);
    println!("cargo:rustc-env=SWEETPAD_LIB_DIR={}", lib.display());
}
