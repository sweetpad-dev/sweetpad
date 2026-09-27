//! Throwaway directories for tool runs that leave files in `$TMPDIR`.
//!
//! The Swift driver makes a `TemporaryDirectory.*` in `$TMPDIR` for each run
//! and removes it only after a run whose jobs it runs itself, such as a
//! compile and a link. A run it hands to one `swift-frontend` that takes its
//! place leaves it behind: `--version`, `-print-target-info`, `-typecheck` and
//! a single-file `-c` do, on Swift 6.4 (Xcode 27.0). So do a `-###` dry run
//! and a driver that dies. SwiftPM adds a lock file there named for the
//! scratch path it builds in. A child whose `TMPDIR` is a [`ScratchDir`]
//! leaves all of that in a directory that goes when the run is done.
//!
//! A child that has to keep the user's `TMPDIR` gets [`TmpdirLeftovers`]
//! instead, which removes the driver's directories the run left once it has
//! exited.

use std::collections::HashSet;
use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

pub use sweetpad_lib::scratch::ScratchDir;

/// What the Swift driver names the directory it makes for a run.
const DRIVER_DIR_PREFIX: &str = "TemporaryDirectory.";

/// The file the Swift driver writes into that directory as soon as it makes
/// it. SwiftPM makes `TemporaryDirectory.*` directories of its own in-process,
/// without one.
const DRIVER_MARKER: &str = ".keep-directory";

/// The programs that leave the Swift driver's directories in the `TMPDIR` they
/// run in: `xcodebuild`, whose build service runs `swiftc --version` before a
/// build, `swift`, whose SwiftPM runs `swiftc -print-target-info`, and the
/// driver itself.
const DRIVER_TOOLS: &[&str] = &["xcodebuild", "swift", "swiftc"];

/// The Swift driver's directories a build tool leaves in the user's `TMPDIR`,
/// removed once the child has exited: by [`TmpdirLeftovers::remove`], or when
/// this drops.
///
/// `xcodebuild` and `swift` keep that `TMPDIR`, since SwiftPM's cross-process
/// locks live there, and a lock only excludes the other clients (Xcode, a
/// second build) because they all take it in the same directory. Every run
/// that keeps it goes through this, so the lock files stay and the driver's
/// directories go. A command for any other program, or one that sets the
/// child's `TMPDIR` itself, gets a guard that does nothing.
#[must_use]
pub struct TmpdirLeftovers(Option<DriverLeftovers>);

impl TmpdirLeftovers {
    /// Note the driver's directories already in `TMPDIR`, before `cmd` starts.
    pub fn before(cmd: &Command) -> Self {
        let tool = Path::new(cmd.get_program())
            .file_name()
            .and_then(OsStr::to_str);
        let sets_tmpdir = cmd.get_envs().any(|(key, _)| key == "TMPDIR");
        let leaves = tool.is_some_and(|tool| DRIVER_TOOLS.contains(&tool)) && !sets_tmpdir;
        Self(leaves.then(|| DriverLeftovers::before_run(&std::env::temp_dir())))
    }

    /// Remove what the run left, once the child has exited, and say how many
    /// directories went.
    #[must_use]
    pub fn remove(mut self) -> usize {
        self.0.take().map_or(0, |leftovers| leftovers.remove_new())
    }
}

impl Drop for TmpdirLeftovers {
    fn drop(&mut self) {
        if let Some(leftovers) = self.0.take() {
            let _ = leftovers.remove_new();
        }
    }
}

/// The `TemporaryDirectory.*` entries in a directory before a run, to tell
/// the ones it added from the rest.
struct DriverLeftovers {
    dir: PathBuf,
    before: HashSet<OsString>,
}

impl DriverLeftovers {
    /// Note the `TemporaryDirectory.*` entries in `dir`.
    fn before_run(dir: &Path) -> Self {
        Self {
            dir: dir.to_path_buf(),
            before: driver_dirs(dir).into_iter().collect(),
        }
    }

    /// Once the child has exited, remove each `TemporaryDirectory.*` that is
    /// new since [`DriverLeftovers::before_run`] and holds nothing but the
    /// driver's `.keep-directory`, the lock files and everything else staying.
    /// Returns how many went.
    ///
    /// A live driver's directory looks the same until its first job writes
    /// there, and a job it hands over can name paths inside it. So nothing is
    /// removed while a Swift driver runs with this `TMPDIR` (or with none,
    /// which means the same per-user directory), while a running process
    /// names one of the directories, or when the processes can't be listed.
    fn remove_new(&self) -> usize {
        self.remove_new_unless(|names| in_use(&self.dir, names, run_ps))
    }

    fn remove_new_unless(&self, in_use: impl FnOnce(&[OsString]) -> bool) -> usize {
        let left: Vec<OsString> = driver_dirs(&self.dir)
            .into_iter()
            .filter(|name| !self.before.contains(name))
            .filter(|name| holds_only_marker(&self.dir.join(name)))
            .collect();
        if left.is_empty() || in_use(&left) {
            return 0;
        }
        left.iter()
            .filter(|name| remove_leftover(&self.dir.join(name)))
            .count()
    }
}

/// The `TemporaryDirectory.*` entries in `dir`.
fn driver_dirs(dir: &Path) -> Vec<OsString> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .flatten()
        .map(|entry| entry.file_name())
        .filter(|name| name.to_string_lossy().starts_with(DRIVER_DIR_PREFIX))
        .collect()
}

/// Whether `path` is a directory holding the driver's marker and nothing else.
fn holds_only_marker(path: &Path) -> bool {
    let Ok(entries) = std::fs::read_dir(path) else {
        return false;
    };
    let names: Vec<OsString> = entries.flatten().map(|entry| entry.file_name()).collect();
    names == [DRIVER_MARKER]
}

/// Remove the marker, then the directory. The second step fails, and keeps
/// the directory, if anything else has appeared in it since.
fn remove_leftover(path: &Path) -> bool {
    std::fs::remove_file(path.join(DRIVER_MARKER)).is_ok() && std::fs::remove_dir(path).is_ok()
}

/// `/bin/ps` with `args`, or `None` when it can't run or fails.
fn run_ps(args: &[&str]) -> Option<String> {
    let out = Command::new("/bin/ps")
        .args(args)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Whether a running process could still be using one of `names` in `dir`:
/// a Swift driver runs with `dir` as its `TMPDIR`, or with none, or a process
/// names one of them on its command line. True when `ps` can't say.
fn in_use(dir: &Path, names: &[OsString], ps: impl Fn(&[&str]) -> Option<String>) -> bool {
    let Some(commands) = ps(&["-axww", "-o", "pid=,comm="]) else {
        return true;
    };
    let drivers: Vec<&str> = commands
        .lines()
        .filter_map(|line| line.trim_start().split_once(' '))
        .filter(|(_, comm)| comm.trim_end().rsplit('/').next() == Some("swift-driver"))
        .map(|(pid, _)| pid)
        .collect();
    if !drivers.is_empty() {
        let pids = drivers.join(",");
        let Some(environments) = ps(&["-ww", "-E", "-o", "args=", "-p", &pids]) else {
            return true;
        };
        if environments.lines().any(|line| runs_in(line, dir)) {
            return true;
        }
    }
    let Some(arguments) = ps(&["-axww", "-o", "args="]) else {
        return true;
    };
    names.iter().any(|name| {
        let name = name.to_string_lossy();
        arguments.lines().any(|line| line.contains(name.as_ref()))
    })
}

/// Whether the process a `ps -E` line lists could have `dir` as its temp
/// directory. The line lists the environment after the arguments, so the last
/// `TMPDIR=` word is the one it runs with. With none it takes the per-user
/// default, which may well be `dir`; with one that doesn't resolve, there is
/// no telling.
fn runs_in(line: &str, dir: &Path) -> bool {
    let Some(tmpdir) = line
        .split(' ')
        .filter_map(|word| word.strip_prefix("TMPDIR="))
        .next_back()
    else {
        return true;
    };
    match (std::fs::canonicalize(tmpdir), std::fs::canonicalize(dir)) {
        (Ok(theirs), Ok(ours)) => theirs == ours,
        _ => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn driver_dir(tmp: &Path, name: &str, files: &[&str]) {
        let dir = tmp.join(name);
        std::fs::create_dir_all(&dir).unwrap();
        for file in files {
            std::fs::write(dir.join(file), "").unwrap();
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

    /// Only a build tool that keeps this process's `TMPDIR` is watched, named
    /// bare or by path.
    #[test]
    fn only_a_build_tool_in_this_tmpdir_is_watched() {
        let watched = |cmd: &Command| TmpdirLeftovers::before(cmd).0.is_some();
        assert!(watched(&Command::new("xcodebuild")));
        assert!(watched(&Command::new(
            "/Applications/Xcode.app/Contents/Developer/usr/bin/xcodebuild"
        )));
        assert!(watched(&Command::new("swift")));
        assert!(watched(&Command::new("swiftc")));
        assert!(!watched(&Command::new("xcrun")));
        assert!(!watched(&Command::new("/bin/ps")));
        let mut own = Command::new("xcodebuild");
        own.env("TMPDIR", "/tmp/elsewhere");
        assert!(!watched(&own));
        let mut none = Command::new("swift");
        none.env_remove("TMPDIR");
        assert!(!watched(&none));
    }

    #[test]
    fn only_the_drivers_new_empty_directories_go() {
        let tmp = ScratchDir::new("sweetpad-leftovers-test").unwrap();
        driver_dir(&tmp, "TemporaryDirectory.before", &[DRIVER_MARKER]);
        let leftovers = DriverLeftovers::before_run(&tmp);
        driver_dir(&tmp, "TemporaryDirectory.probe", &[DRIVER_MARKER]);
        // A live driver's, once its first job has written there.
        driver_dir(&tmp, "TemporaryDirectory.busy", &[DRIVER_MARKER, "a-1.o"]);
        // SwiftPM's own, which have no marker.
        driver_dir(&tmp, "TemporaryDirectory.swiftpm", &[]);
        std::fs::write(tmp.join("_Users_me_.swiftpm.lock"), "").unwrap();

        assert_eq!(leftovers.remove_new_unless(|_| false), 1);
        assert_eq!(
            entries(&tmp),
            [
                "TemporaryDirectory.before",
                "TemporaryDirectory.busy",
                "TemporaryDirectory.swiftpm",
                "_Users_me_.swiftpm.lock",
            ]
        );
    }

    #[test]
    fn nothing_goes_while_something_could_be_using_it() {
        let tmp = ScratchDir::new("sweetpad-leftovers-test").unwrap();
        let leftovers = DriverLeftovers::before_run(&tmp);
        driver_dir(&tmp, "TemporaryDirectory.probe", &[DRIVER_MARKER]);
        let mut asked = Vec::new();
        let removed = leftovers.remove_new_unless(|names| {
            asked = names.to_vec();
            true
        });
        assert_eq!(removed, 0);
        assert_eq!(asked, ["TemporaryDirectory.probe"]);
        assert_eq!(entries(&tmp), ["TemporaryDirectory.probe"]);
    }

    /// Whether [`in_use`] finds `TemporaryDirectory.probe` in `tmp` in use,
    /// with a `ps` that answers from canned listings: `pid=,comm=`, the
    /// drivers' environments, and everyone's arguments.
    fn probe_in_use(tmp: &Path, comms: &str, environments: &str, arguments: &str) -> bool {
        let names = [OsString::from("TemporaryDirectory.probe")];
        in_use(tmp, &names, |args: &[&str]| {
            let listing = if args.contains(&"-E") {
                environments
            } else if args.contains(&"pid=,comm=") {
                comms
            } else {
                arguments
            };
            Some(listing.to_string())
        })
    }

    const LAUNCHD: &str = "  1 /sbin/launchd\n";

    #[test]
    fn a_swift_driver_with_this_tmpdir_or_none_counts_as_using_it() {
        let tmp = ScratchDir::new("sweetpad-leftovers-test").unwrap();
        let other = ScratchDir::new("sweetpad-leftovers-test").unwrap();
        let driver = format!("{LAUNCHD} 42 /Xcode Beta.app/usr/bin/swift-driver\n");
        let ours = format!("swiftc a.swift HOME=/Users/me TMPDIR={}/\n", tmp.display());
        let none = "swiftc a.swift HOME=/Users/me\n";
        let theirs = format!("swiftc a.swift TMPDIR={}\n", other.display());

        assert!(probe_in_use(&tmp, &driver, &ours, ""));
        assert!(probe_in_use(&tmp, &driver, none, ""));
        assert!(!probe_in_use(&tmp, &driver, &theirs, ""));
        assert!(!probe_in_use(&tmp, LAUNCHD, "", ""));
    }

    #[test]
    fn a_process_naming_the_directory_counts_as_using_it() {
        let tmp = ScratchDir::new("sweetpad-leftovers-test").unwrap();
        let naming = format!(
            "/sbin/launchd\nswift-frontend -frontend -c -o {}/TemporaryDirectory.probe/a-1.o\n",
            tmp.display()
        );
        assert!(probe_in_use(&tmp, LAUNCHD, "", &naming));
        assert!(!probe_in_use(&tmp, LAUNCHD, "", "/sbin/launchd\n"));
    }

    #[test]
    fn no_process_listing_counts_as_in_use() {
        let tmp = ScratchDir::new("sweetpad-leftovers-test").unwrap();
        let names = [OsString::from("TemporaryDirectory.probe")];
        assert!(in_use(&tmp, &names, |_: &[&str]| None));
    }
}
