//! `xcrun`, which run scripts start Xcode's tools through, like
//! `xcrun -sdk macosx swift script.swift`, and ask about an SDK with, like
//! `xcrun --sdk iphoneos --show-sdk-path`.
//!
//! A tool runs from the script's `PATH`, which has the toolchain's tools and
//! the build's stand-ins. The SDKs are the Darwin SDK's, in the folders
//! `XCODE_EXTRA_PLATFORM_FOLDERS` names, which Swift Build passes to a run
//! script. A script runs on the build host, which here isn't macOS: `--sdk
//! macosx` runs the tool without `SDKROOT`, the way a Mac script means it to
//! run for the host, since the toolchain builds nothing for Linux against a
//! Darwin SDK. Any other SDK becomes the tool's `SDKROOT`, as `xcrun` makes
//! it, and without `--sdk` the tool keeps the script's. `--toolchain`, the
//! cache flags and `--verbose` change nothing here.

use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use sweetpad_lib::derived_data::plist_strings;

/// `xcrun`'s exit status when it finds no tool of the name.
const NOT_FOUND: u8 = 72;

const USAGE: &str = "\
Usage: xcrun [options] <tool name> ... arguments ...

Find and execute the named command line tool.

Options:
  -h, --help                  show this help message and exit
  --version                   show the xcrun version
  -v, --verbose               show verbose logging output
  --sdk <sdk name>            find the tool for the given SDK name
  --toolchain <name>          find the tool for the given toolchain
  -l, --log                   show commands to be executed (with --run)
  -f, --find                  only find and print the tool path
  -r, --run                   find and execute the tool (the default behavior)
  -n, --no-cache              do not use the lookup cache
  -k, --kill-cache            invalidate all existing cache entries
  --show-sdk-path             show selected SDK install path
  --show-sdk-version          show selected SDK version
  --show-sdk-build-version    show selected SDK build version
  --show-sdk-platform-path    show selected SDK platform path
  --show-sdk-platform-version show selected SDK platform version
";

pub(super) fn main(args: &[String]) -> ExitCode {
    let request = match parse(args) {
        Ok(request) => request,
        Err(message) => {
            eprintln!("xcrun: error: {message}");
            eprint!("{USAGE}");
            return ExitCode::FAILURE;
        }
    };
    let folders = platform_folders();
    match run(&request, &folders) {
        Ok(code) => code,
        Err(Failure { message, code }) => {
            eprintln!("xcrun: error: {message}");
            ExitCode::from(code)
        }
    }
}

/// What the command line asks for.
#[derive(Debug, Default, PartialEq)]
struct Request {
    mode: Mode,
    sdk: Option<String>,
    log: bool,
    /// The `--show-sdk-*` queries, in the order given.
    queries: Vec<Query>,
    /// The tool and its arguments.
    command: Vec<String>,
}

#[derive(Debug, Default, Clone, Copy, PartialEq)]
enum Mode {
    /// Run the tool, the default.
    #[default]
    Run,
    /// Print the tool's path.
    Find,
    Help,
    Version,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Query {
    Path,
    Version,
    BuildVersion,
    PlatformPath,
    PlatformVersion,
}

/// Options come before the tool's name, which ends them. `xcrun` takes its
/// long options with one dash or two.
fn parse(args: &[String]) -> Result<Request, String> {
    let mut request = Request::default();
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        let option = if arg.starts_with("--") {
            &arg[1..]
        } else {
            arg.as_str()
        };
        match option {
            "-h" | "-help" => request.mode = Mode::Help,
            "-version" => request.mode = Mode::Version,
            "-sdk" => request.sdk = Some(super::value(arg, &mut it)?.clone()),
            "-toolchain" => {
                super::value(arg, &mut it)?;
            }
            "-f" | "-find" => request.mode = Mode::Find,
            "-l" | "-log" => request.log = true,
            "-v" | "-verbose" | "-r" | "-run" | "-n" | "-no-cache" | "-k" | "-kill-cache" => {}
            "-show-sdk-path" => request.queries.push(Query::Path),
            "-show-sdk-version" => request.queries.push(Query::Version),
            "-show-sdk-build-version" => request.queries.push(Query::BuildVersion),
            "-show-sdk-platform-path" => request.queries.push(Query::PlatformPath),
            "-show-sdk-platform-version" => request.queries.push(Query::PlatformVersion),
            flag if flag.starts_with('-') => return Err(format!("unrecognized option: {arg}")),
            _ => {
                request.command.push(arg.clone());
                request.command.extend(it.cloned());
                break;
            }
        }
    }
    Ok(request)
}

/// Why `xcrun` failed, and the status it exits with.
#[derive(Debug)]
struct Failure {
    message: String,
    code: u8,
}

impl Failure {
    fn new(message: String) -> Self {
        Self { message, code: 1 }
    }
}

fn run(request: &Request, folders: &[PathBuf]) -> Result<ExitCode, Failure> {
    match request.mode {
        Mode::Help => {
            print!("{USAGE}");
            return Ok(ExitCode::SUCCESS);
        }
        Mode::Version => {
            println!("xcrun version {} (sweetpad)", env!("CARGO_PKG_VERSION"));
            return Ok(ExitCode::SUCCESS);
        }
        Mode::Run | Mode::Find => {}
    }
    if !request.queries.is_empty() {
        let sdk = selected_sdk(request.sdk.as_deref(), folders)?;
        for query in &request.queries {
            println!("{}", sdk.answer(*query)?);
        }
        return Ok(ExitCode::SUCCESS);
    }
    let Some((tool, args)) = request.command.split_first() else {
        return Err(Failure::new("no tool to run".to_string()));
    };
    let program = find_tool(tool).ok_or_else(|| Failure {
        message: format!("unable to find utility \"{tool}\", not a developer tool or in PATH"),
        code: NOT_FOUND,
    })?;
    if request.mode == Mode::Find {
        println!("{}", program.display());
        return Ok(ExitCode::SUCCESS);
    }
    let sdkroot = match &request.sdk {
        Some(name) => Some(tool_sdkroot(&locate(name, folders)?)),
        None => None,
    };
    if request.log {
        let words: Vec<String> = std::iter::once(program.display().to_string())
            .chain(args.iter().cloned())
            .collect();
        println!("xcrun: note: {}", words.join(" "));
    }
    let mut command = Command::new(&program);
    let clang = ["clang", "clang++", "cc", "c++"].contains(&tool.as_str());
    let cross_args = if clang && !cfg!(target_os = "macos") {
        darwin_clang_args(args).map_err(Failure::new)?
    } else {
        None
    };
    if let Some(args) = cross_args {
        command.args(&args);
        if !args
            .iter()
            .any(|a| ["-c", "-E", "-S", "-fsyntax-only"].contains(&a.as_str()))
            && let Some(xcrun) = super::find_program("xcrun")
            && let Some(folder) = xcrun.parent()
        {
            command.arg("-fuse-ld=lld");
            command.arg(format!("--ld-path={}", folder.join("ld64.lld").display()));
        }
    } else {
        command.args(args);
    }
    match sdkroot {
        Some(Some(path)) => {
            command.env("SDKROOT", path);
        }
        Some(None) => {
            command.env_remove("SDKROOT");
        }
        None => {}
    }
    Err(Failure::new(format!(
        "can't run {}: {}",
        program.display(),
        exec(&mut command)
    )))
}

/// Apple's Clang infers an Apple target from these deployment flags. A
/// Linux Clang needs an explicit triple as well as the SDK's sysroot.
fn darwin_clang_args(args: &[String]) -> Result<Option<Vec<String>>, String> {
    if args
        .iter()
        .any(|a| a == "-target" || a.starts_with("--target="))
    {
        return Ok(None);
    }
    let minimum = args.iter().find_map(|arg| {
        [
            ("-miphonesimulator-version-min=", "ios", "-simulator"),
            ("-mios-simulator-version-min=", "ios", "-simulator"),
            ("-miphoneos-version-min=", "ios", ""),
            ("-mmacosx-version-min=", "macosx", ""),
            ("-mtvos-simulator-version-min=", "tvos", "-simulator"),
            ("-mtvos-version-min=", "tvos", ""),
        ]
        .into_iter()
        .find_map(|(prefix, os, suffix)| {
            arg.strip_prefix(prefix)
                .map(|version| (arg, os, suffix, version))
        })
    });
    let Some((flag, os, suffix, version)) = minimum else {
        return Ok(None);
    };
    let arches: Vec<_> = args
        .windows(2)
        .filter(|a| a[0] == "-arch")
        .map(|a| &a[1])
        .collect();
    if arches.len() != 1 {
        return Err("cross-compiling with 'xcrun clang' needs exactly one '-arch'".into());
    }
    let mut out = vec![format!(
        "--target={}-apple-{os}{version}{suffix}",
        arches[0]
    )];
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        if arg == "-arch" {
            args.next();
        } else if arg != flag {
            out.push(arg.clone());
        }
    }
    Ok(Some(out))
}

/// Replace this process with `command`, returning only when that fails.
#[cfg(unix)]
fn exec(command: &mut Command) -> std::io::Error {
    std::os::unix::process::CommandExt::exec(command)
}

#[cfg(not(unix))]
fn exec(command: &mut Command) -> std::io::Error {
    match command.status() {
        Ok(status) => std::process::exit(status.code().unwrap_or(1)),
        Err(e) => e,
    }
}

/// The tool named `name`: a path as given, else the first on `PATH`.
fn find_tool(name: &str) -> Option<PathBuf> {
    if name.contains('/') {
        let path = PathBuf::from(name);
        return path.is_file().then_some(path);
    }
    super::find_program(name)
}

/// The `SDKROOT` a tool run for `sdk` gets: none for macOS, the host's
/// platform on a Mac, and the SDK's path for any other.
fn tool_sdkroot(sdk: &Sdk) -> Option<PathBuf> {
    (sdk.family() != "macosx").then(|| sdk.path.clone())
}

/// The folders holding the Darwin SDK's platforms: those
/// `XCODE_EXTRA_PLATFORM_FOLDERS` names, else the Darwin SDK bundle's.
fn platform_folders() -> Vec<PathBuf> {
    match std::env::var_os("XCODE_EXTRA_PLATFORM_FOLDERS") {
        Some(folders) if !folders.is_empty() => std::env::split_paths(&folders).collect(),
        _ => vec![crate::cli::swbuild::darwin_sdk_bundle().join("Developer/Platforms")],
    }
}

/// The SDK a query is about: `--sdk`, else `SDKROOT`, else macOS's.
fn selected_sdk(name: Option<&str>, folders: &[PathBuf]) -> Result<Sdk, Failure> {
    let from_env = std::env::var("SDKROOT").ok().filter(|s| !s.is_empty());
    locate(name.or(from_env.as_deref()).unwrap_or("macosx"), folders)
}

/// An SDK of a platform, and what its `SDKSettings.plist` says about it.
#[derive(Debug)]
struct Sdk {
    path: PathBuf,
    /// The `.platform` folder it's in, if it's in one.
    platform: Option<PathBuf>,
    /// Like `iphonesimulator27.0`.
    canonical_name: String,
}

impl Sdk {
    fn at(path: PathBuf) -> Option<Self> {
        let settings = plist_strings(&path.join("SDKSettings.plist"));
        let canonical_name = setting(&settings, "CanonicalName")?.to_ascii_lowercase();
        let platform = path
            .ancestors()
            .find(|a| a.extension().is_some_and(|e| e == "platform"))
            .map(Path::to_path_buf);
        Some(Self {
            path,
            platform,
            canonical_name,
        })
    }

    /// The canonical name without its version, like `iphonesimulator`.
    fn family(&self) -> &str {
        self.canonical_name
            .trim_end_matches(|c: char| c.is_ascii_digit() || c == '.')
    }

    fn answer(&self, query: Query) -> Result<String, Failure> {
        let missing = |item: &str| {
            Failure::new(format!(
                "unable to lookup item '{item}' in SDK '{}'",
                self.path.display()
            ))
        };
        let platform = || {
            self.platform
                .as_ref()
                .ok_or_else(|| missing("PlatformPath"))
        };
        match query {
            Query::Path => Ok(self.path.display().to_string()),
            Query::Version => setting(
                &plist_strings(&self.path.join("SDKSettings.plist")),
                "Version",
            )
            .ok_or_else(|| missing("Version")),
            Query::BuildVersion => setting(
                &plist_strings(
                    &self
                        .path
                        .join("System/Library/CoreServices/SystemVersion.plist"),
                ),
                "ProductBuildVersion",
            )
            .ok_or_else(|| missing("ProductBuildVersion")),
            Query::PlatformPath => Ok(platform()?.display().to_string()),
            Query::PlatformVersion => {
                setting(&plist_strings(&platform()?.join("Info.plist")), "Version")
                    .ok_or_else(|| missing("PlatformVersion"))
            }
        }
    }
}

fn setting(settings: &[(String, String)], key: &str) -> Option<String> {
    settings
        .iter()
        .find_map(|(k, v)| (k == key).then(|| v.clone()))
}

/// The SDK `name` names: a path to one, or a canonical name with or without
/// its version, like `iphonesimulator27.0` or `iphonesimulator`, among the
/// platforms in `folders`. Of the names a platform keeps one SDK under,
/// `xcrun` gives the one with the version, like `iPhoneSimulator27.0.sdk`.
fn locate(name: &str, folders: &[PathBuf]) -> Result<Sdk, Failure> {
    let not_found = || Failure::new(format!("SDK \"{name}\" cannot be located"));
    if name.contains('/') {
        return Sdk::at(PathBuf::from(name)).ok_or_else(not_found);
    }
    let wanted = name.to_ascii_lowercase();
    let mut found: Option<Sdk> = None;
    for folder in folders {
        for platform in sorted_entries(folder, "platform") {
            for path in sorted_entries(&platform.join("Developer/SDKs"), "sdk") {
                let Some(sdk) = Sdk::at(path) else { continue };
                if sdk.canonical_name != wanted && sdk.family() != wanted {
                    continue;
                }
                let versioned = |sdk: &Sdk| {
                    let version = sdk.canonical_name[sdk.family().len()..].to_string();
                    !version.is_empty()
                        && sdk
                            .path
                            .file_stem()
                            .is_some_and(|s| s.to_string_lossy().ends_with(&version))
                };
                if found
                    .as_ref()
                    .is_none_or(|f| !versioned(f) && versioned(&sdk))
                {
                    found = Some(sdk);
                }
            }
        }
    }
    found.ok_or_else(not_found)
}

/// The entries of `dir` with the extension `extension`, by name.
fn sorted_entries(dir: &Path, extension: &str) -> Vec<PathBuf> {
    let mut entries: Vec<PathBuf> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|e| e == extension))
        .collect();
    entries.sort();
    entries
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::testdir::TempDir;

    fn strings(list: &[&str]) -> Vec<String> {
        list.iter().map(ToString::to_string).collect()
    }

    fn plist(entries: &[(&str, &str)]) -> String {
        let body = entries.iter().fold(String::new(), |mut body, (k, v)| {
            use std::fmt::Write;
            let _ = write!(body, "<key>{k}</key><string>{v}</string>");
            body
        });
        format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<plist version=\"1.0\"><dict>{body}</dict></plist>\n"
        )
    }

    /// A platforms folder with the SDKs as xtool lays them out: the SDK under
    /// its plain name, and a link to it under each versioned one.
    fn platforms(dir: &TempDir) -> PathBuf {
        let folder = dir.join("Platforms");
        for (platform, family, links) in [
            (
                "iPhoneSimulator",
                "iphonesimulator",
                &["iPhoneSimulator27.0"][..],
            ),
            ("MacOSX", "macosx", &["MacOSX27.0", "MacOSX27"][..]),
        ] {
            let platform_dir = folder.join(format!("{platform}.platform"));
            std::fs::create_dir_all(&platform_dir).unwrap();
            std::fs::write(
                platform_dir.join("Info.plist"),
                plist(&[("Version", "27.0")]),
            )
            .unwrap();
            let sdks = platform_dir.join("Developer/SDKs");
            let sdk = sdks.join(format!("{platform}.sdk"));
            std::fs::create_dir_all(sdk.join("System/Library/CoreServices")).unwrap();
            std::fs::write(
                sdk.join("SDKSettings.plist"),
                plist(&[
                    ("CanonicalName", &format!("{family}27.0")),
                    ("Version", "27.0"),
                ]),
            )
            .unwrap();
            std::fs::write(
                sdk.join("System/Library/CoreServices/SystemVersion.plist"),
                plist(&[("ProductBuildVersion", "24A430")]),
            )
            .unwrap();
            for link in links {
                std::os::unix::fs::symlink(
                    format!("{platform}.sdk"),
                    sdks.join(format!("{link}.sdk")),
                )
                .unwrap();
            }
        }
        folder
    }

    #[test]
    fn apple_deployment_flags_select_the_darwin_clang_target() {
        let args = strings(&[
            "-dynamiclib",
            "-arch",
            "arm64",
            "-miphonesimulator-version-min=15.0",
            "-isysroot",
            "/SDK with spaces",
            "source.c",
            "-o",
            "output.dylib",
        ]);
        assert_eq!(
            darwin_clang_args(&args).unwrap().unwrap(),
            strings(&[
                "--target=arm64-apple-ios15.0-simulator",
                "-dynamiclib",
                "-isysroot",
                "/SDK with spaces",
                "source.c",
                "-o",
                "output.dylib",
            ])
        );
        assert!(
            darwin_clang_args(&strings(&[
                "-arch",
                "arm64",
                "-arch",
                "x86_64",
                "-miphoneos-version-min=15.0",
            ]))
            .is_err()
        );
        assert!(
            darwin_clang_args(&strings(&[
                "--target=arm64-apple-ios15.0",
                "-miphoneos-version-min=15.0",
            ]))
            .unwrap()
            .is_none()
        );
    }

    #[test]
    fn options_end_at_the_tools_name() {
        let request = parse(&strings(&[
            "-sdk",
            "macosx",
            "-v",
            "swift",
            "script.swift",
            "--verbose",
        ]))
        .unwrap();
        assert_eq!(request.sdk.as_deref(), Some("macosx"));
        assert_eq!(
            request.command,
            strings(&["swift", "script.swift", "--verbose"])
        );

        let request = parse(&strings(&["--sdk", "iphoneos", "--find", "clang"])).unwrap();
        assert_eq!(request.mode, Mode::Find);
        assert_eq!(request.command, strings(&["clang"]));

        assert!(parse(&strings(&["--sdk"])).is_err());
        assert!(parse(&strings(&["--frobnicate", "swift"])).is_err());
    }

    #[test]
    fn an_sdk_is_found_by_name_with_or_without_its_version() {
        let dir = TempDir::new("sweetpad-xcrun-locate");
        let folders = [platforms(&dir)];
        for name in ["iphonesimulator", "iphonesimulator27.0", "iPhoneSimulator"] {
            let sdk = locate(name, &folders).unwrap();
            assert!(
                sdk.path
                    .ends_with("iPhoneSimulator.platform/Developer/SDKs/iPhoneSimulator27.0.sdk"),
                "{name}: {}",
                sdk.path.display()
            );
        }
        let mac = locate("macosx", &folders).unwrap();
        assert!(
            mac.path.ends_with("MacOSX27.0.sdk"),
            "{}",
            mac.path.display()
        );
        assert!(locate("iphonesimulator18.0", &folders).is_err());
        assert!(locate("watchos", &folders).is_err());
    }

    #[test]
    fn queries_answer_from_the_sdk_and_its_platform() {
        let dir = TempDir::new("sweetpad-xcrun-queries");
        let folders = [platforms(&dir)];
        let sdk = locate("iphonesimulator", &folders).unwrap();
        assert_eq!(sdk.answer(Query::Version).unwrap(), "27.0");
        assert_eq!(sdk.answer(Query::BuildVersion).unwrap(), "24A430");
        assert_eq!(sdk.answer(Query::PlatformVersion).unwrap(), "27.0");
        assert_eq!(
            PathBuf::from(sdk.answer(Query::PlatformPath).unwrap()),
            folders[0].join("iPhoneSimulator.platform")
        );
        let by_path = locate(&sdk.answer(Query::Path).unwrap(), &folders).unwrap();
        assert_eq!(by_path.canonical_name, "iphonesimulator27.0");
    }

    #[test]
    fn a_tool_run_for_macos_runs_for_the_host() {
        let dir = TempDir::new("sweetpad-xcrun-sdkroot");
        let folders = [platforms(&dir)];
        assert_eq!(tool_sdkroot(&locate("macosx", &folders).unwrap()), None);
        let simulator = locate("iphonesimulator", &folders).unwrap();
        assert_eq!(tool_sdkroot(&simulator), Some(simulator.path.clone()));
    }

    #[test]
    fn a_missing_tool_fails_as_xcrun_does() {
        let request = Request {
            command: strings(&["sweetpad-no-such-tool"]),
            ..Request::default()
        };
        let failure = run(&request, &[]).unwrap_err();
        assert_eq!(failure.code, NOT_FOUND);
        assert!(failure.message.contains("unable to find utility"));
    }
}
