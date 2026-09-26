//! The `xcodebuild -resolvePackageDependencies` runs behind `dependency
//! resolve` and `dependency update`, against a stub `xcodebuild` that handles
//! result bundles the way Xcode 27.0 does for a failed resolve: it writes one
//! where `-resultBundlePath` says, into the temp directory when no path is
//! given, and refuses a path that already exists. The stub writes a bundle on
//! every run, so a path reused across two resolves fails the second.

mod common;

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use common::TempDir;

fn tmp(tag: &str) -> TempDir {
    let dir = TempDir::new(&format!("sweetpad-dep-resolve-{tag}"));
    // Stop walk-up discovery at this directory.
    std::fs::create_dir_all(dir.join(".git")).unwrap();
    dir
}

/// A project directory with a stub `xcodebuild` on `PATH` that logs its argv
/// and exits with `status`, and an empty directory for the run's `TMPDIR`.
struct Fixture {
    root: TempDir,
    temp: PathBuf,
}

impl Fixture {
    fn new(tag: &str, status: i32) -> Self {
        Self::saying(tag, status, "")
    }

    /// [`Fixture::new`], with a stub that writes `said` to stderr before it
    /// exits, the way xcodebuild reports why a resolve failed.
    fn saying(tag: &str, status: i32, said: &str) -> Self {
        use std::os::unix::fs::PermissionsExt;

        let root = tmp(tag);
        let bin = root.join("bin");
        let temp = root.join("tmp");
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::create_dir_all(&temp).unwrap();
        std::fs::create_dir_all(root.join("App.xcodeproj")).unwrap();
        std::fs::write(root.join("said.txt"), said).unwrap();
        std::fs::write(
            bin.join("xcodebuild"),
            format!(
                "#!/bin/sh\n\
                 echo \"$*\" >> '{}'\n\
                 bundle=\n\
                 previous=\n\
                 for arg; do\n\
                 [ \"$previous\" = -resultBundlePath ] && bundle=$arg\n\
                 previous=$arg\n\
                 done\n\
                 if [ -z \"$bundle\" ]; then\n\
                 bundle=\"${{TMPDIR%/}}/ResultBundle_2026-26-09_15-04-0042.xcresult\"\n\
                 elif [ -e \"$bundle\" ]; then\n\
                 echo \"xcodebuild: error: Existing file at -resultBundlePath \\\"$bundle\\\"\" >&2\n\
                 exit 64\n\
                 fi\n\
                 mkdir -p \"$bundle\"\n\
                 cat '{}' >&2\n\
                 exit {status}\n",
                root.join("xcodebuild.log").display(),
                root.join("said.txt").display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(
            bin.join("xcodebuild"),
            std::fs::Permissions::from_mode(0o755),
        )
        .unwrap();
        Self { root, temp }
    }

    fn sweetpad(&self, args: &[&str]) -> Output {
        self.command(args)
            .output()
            .expect("failed to run the sweetpad binary")
    }

    fn command(&self, args: &[&str]) -> Command {
        let path_env = format!(
            "{}:{}",
            self.root.join("bin").display(),
            std::env::var("PATH").unwrap_or_default()
        );
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_sweetpad"));
        cmd.args(args)
            .current_dir(&self.root)
            .env("HOME", &self.root)
            .env("XDG_STATE_HOME", &self.root)
            .env("XDG_CONFIG_HOME", &self.root)
            .env("XDG_CACHE_HOME", &self.root)
            .env("TMPDIR", &self.temp)
            .env("PATH", path_env)
            .env_remove("FORCE_COLOR")
            .env_remove("CLICOLOR_FORCE");
        cmd
    }

    /// Each resolve's argv, one per line.
    fn resolves(&self) -> Vec<String> {
        std::fs::read_to_string(self.root.join("xcodebuild.log"))
            .unwrap_or_default()
            .lines()
            .map(str::to_string)
            .collect()
    }

    fn left_in_temp(&self) -> Vec<String> {
        entries(&self.temp)
    }
}

fn entries(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

#[test]
fn a_failed_resolve_leaves_no_result_bundle_behind() {
    let fixture = Fixture::new("failed", 1);
    let out = fixture.sweetpad(&["dep", "resolve", "--json"]);
    assert!(!out.status.success(), "expected the stub's resolve to fail");
    let resolves = fixture.resolves();
    assert_eq!(resolves.len(), 1, "{resolves:?}");
    assert!(
        resolves[0].contains(" -resultBundlePath "),
        "{}",
        resolves[0]
    );
    assert_eq!(fixture.left_in_temp(), Vec::<String>::new());
}

/// `update` resolves twice, into an empty clone directory and then in place,
/// and xcodebuild refuses a result bundle path that already exists.
#[test]
fn each_resolve_of_an_update_gets_a_fresh_result_bundle() {
    let fixture = Fixture::new("update", 0);
    let out = fixture.sweetpad(&["dep", "update", "--json"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let resolves = fixture.resolves();
    assert_eq!(resolves.len(), 2, "{resolves:?}");
    assert!(
        resolves
            .iter()
            .all(|argv| argv.contains(" -resultBundlePath ")),
        "{resolves:?}"
    );
    assert_eq!(fixture.left_in_temp(), Vec::<String>::new());
}

/// What Xcode prints when no version satisfies a requirement.
const UNRESOLVABLE: &str = "\
xcodebuild: error: Could not resolve package dependencies:
  Dependencies could not be resolved because root depends on 'swift-collections' 99.0.0.
  'swift-collections' 99.0.0 cannot be used because no versions of 'swift-collections' match the requirement 99.0.0.
";

/// The reason a resolve failed streams to stdout with the rest of its log, so
/// with stderr apart from stdout (`2>err.log`) the error carries it too.
#[test]
fn a_failed_resolve_names_the_reason_in_its_error() {
    let fixture = Fixture::saying("reason", 1, UNRESOLVABLE);
    for verb in ["resolve", "update"] {
        let out = fixture.sweetpad(&["dep", verb]);
        assert!(
            !out.status.success(),
            "{verb}: expected the resolve to fail"
        );
        let stderr = String::from_utf8(out.stderr).unwrap();
        let expected = "\
error: resolving package dependencies
  xcodebuild -resolvePackageDependencies exited with a non-zero status:
  xcodebuild: error: Could not resolve package dependencies:
    Dependencies could not be resolved because root depends on 'swift-collections' 99.0.0.
    'swift-collections' 99.0.0 cannot be used because no versions of 'swift-collections' match the requirement 99.0.0.
";
        assert!(stderr.ends_with(expected), "{verb}:\n{stderr}");
    }
}

/// With stdout and stderr in one file, the reason already sits just above the
/// error, which does not repeat it.
#[test]
fn the_error_does_not_repeat_a_reason_just_above_it() {
    let fixture = Fixture::saying("once", 1, UNRESOLVABLE);
    let transcript = fixture.root.join("transcript.txt");
    let file = std::fs::File::create(&transcript).unwrap();
    let status = fixture
        .command(&["dep", "resolve"])
        .stdout(file.try_clone().unwrap())
        .stderr(file)
        .status()
        .unwrap();
    assert!(!status.success(), "expected the resolve to fail");
    let shown = std::fs::read_to_string(&transcript).unwrap();
    assert_eq!(
        shown.matches("Dependencies could not be resolved").count(),
        1,
        "{shown}"
    );
    assert!(
        shown.ends_with(
            "error: resolving package dependencies\n  \
             xcodebuild -resolvePackageDependencies exited with a non-zero status\n"
        ),
        "{shown}"
    );
}
