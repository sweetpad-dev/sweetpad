//! LLD binds /usr/lib/swift reexports through the first umbrella it opens.
//! Open a directly autolinked runtime first, while leaving SDK deployment
//! redirects and explicit loading policies to the linker. No output is patched.
use std::collections::BTreeSet;
use std::path::PathBuf;

pub(super) fn prefer_direct_runtime(
    args: &mut Vec<String>,
    libraries: &BTreeSet<String>,
    markers: &BTreeSet<String>,
) {
    if libraries.is_empty()
        || markers.is_empty()
        || args
            .iter()
            .any(|a| a == "-no_implicit_dylibs" || a == "-flat_namespace")
    {
        return;
    }
    let Some(platform) = args.windows(4).find(|a| a[0] == "-platform_version") else {
        return;
    };
    let platform_id = match platform[1].as_str() {
        "macos" => 1,
        "ios" => 2,
        "tvos" => 3,
        "watchos" => 4,
        "bridgeos" => 5,
        "mac-catalyst" => 6,
        "ios-simulator" => 7,
        "tvos-simulator" => 8,
        "watchos-simulator" => 9,
        "xros" => 11,
        "xros-simulator" => 12,
        _ => return,
    };
    let minimum = version(&platform[2]);
    let search = search_paths(args);
    let stub = |name: &str| {
        search
            .iter()
            .map(|p| p.join(format!("lib{name}.tbd")))
            .find(|p| p.is_file())
    };
    // An umbrella can be named on the command line or autolinked, as
    // libswiftDarwin is: LLD 23 opens autolinked libraries in ld64's order,
    // which puts the umbrella ahead of the runtime it reexports.
    let parents: Vec<(String, String)> = args
        .iter()
        .filter_map(|arg| {
            [
                "-weak-l",
                "-needed-l",
                "-lazy-l",
                "-reexport-l",
                "-upward-l",
                "-l",
            ]
            .iter()
            .find_map(|prefix| arg.strip_prefix(prefix))
        })
        .chain(libraries.iter().map(String::as_str))
        .filter_map(|name| {
            let text = std::fs::read_to_string(stub(name)?).ok()?;
            Some((name.to_string(), text))
        })
        .collect();
    let mut leading = Vec::new();
    for library in libraries.intersection(markers) {
        // Do not replace an explicit policy, including weak or reexport links.
        if args
            .iter()
            .any(|a| a == &format!("-l{library}") || a.ends_with(&format!("-l{library}")))
        {
            continue;
        }
        let Some(path) = stub(library) else {
            continue;
        };
        let Ok(child) = std::fs::read_to_string(path) else {
            continue;
        };
        let install = format!("/usr/lib/swift/lib{library}.dylib");
        if !child
            .lines()
            .any(|line| line.starts_with("install-name:") && line.contains(&format!("'{install}'")))
            || active_redirect(&child, platform_id, minimum)
        {
            continue;
        }
        if parents
            .iter()
            .any(|(name, text)| name != library && reexports(text, &install))
        {
            // Plain -l retains LLD's inference from weak/strong symbol references.
            leading.push(format!("-l{library}"));
        }
    }
    args.splice(0..0, leading);
}

/// The folders the linker looks for `-l` libraries in, in its order.
fn search_paths(args: &[String]) -> Vec<PathBuf> {
    let root = args
        .windows(2)
        .find(|a| a[0] == "-syslibroot")
        .map(|a| PathBuf::from(&a[1]));
    let mut search = Vec::new();
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        let path = if arg == "-L" {
            iter.next().map(String::as_str)
        } else {
            arg.strip_prefix("-L")
        };
        let Some(path) = path.map(PathBuf::from) else {
            continue;
        };
        // As in ld64, an absolute search path is looked up under the SDK
        // first, which is where -L/usr/lib/swift finds the Swift runtime.
        if let (Some(root), Ok(inner)) = (&root, path.strip_prefix("/")) {
            search.push(root.join(inner));
        }
        search.push(path);
    }
    if let Some(root) = &root {
        search.push(root.join("usr/lib"));
    }
    search
}

fn version(value: &str) -> [u32; 3] {
    let mut result = [0; 3];
    for (entry, value) in result.iter_mut().zip(value.split('.')) {
        *entry = value.parse().unwrap_or(0);
    }
    result
}
fn active_redirect(stub: &str, platform: u32, minimum: [u32; 3]) -> bool {
    stub.split("$ld$previous$").skip(1).any(|rule| {
        let parts: Vec<_> = rule.split('$').take(7).collect();
        parts.len() >= 6
            && parts[2].parse::<u32>().ok() == Some(platform)
            && minimum >= version(parts[3])
            && minimum < version(parts[4])
            && parts[5].is_empty()
    })
}
fn reexports(stub: &str, install: &str) -> bool {
    // TBD v4 top-level reexport section. Other formats pass through unchanged.
    let mut section = false;
    for line in stub.lines() {
        if !line.starts_with(char::is_whitespace) {
            section = line.starts_with("reexported-libraries:");
        }
        if section && line.contains(&format!("'{install}'")) {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sdk_redirects_determine_boundaries_and_platform() {
        let stub = "'$ld$previous$/usr/lib/swift/Parent.dylib$$2$12.2$18.0$$'";
        assert!(active_redirect(stub, 2, version("17.6")));
        assert!(!active_redirect(stub, 2, version("18.0")));
        assert!(!active_redirect(stub, 7, version("17.6")));
        assert!(!active_redirect(
            "'$ld$previous$/Old$$2$12.2$18.0$_one$'",
            2,
            version("17.0")
        ));
    }
    #[test]
    fn an_autolinked_umbrella_puts_its_reexported_runtime_first() {
        // The stubs sit in the SDK, found through -syslibroot and an
        // absolute -L, as Swift Build passes them.
        let sdk = std::env::temp_dir().join(format!("sweetpad-reexports-{}", std::process::id()));
        let dir = sdk.join("usr/lib/swift");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("libswiftDarwin.tbd"),
            "install-name: '/usr/lib/swift/libswiftDarwin.dylib'\nreexported-libraries:\n  - targets: [ arm64e-macos ]\n    libraries: [ '/usr/lib/swift/libswift_Builtin_float.dylib' ]\n",
        )
        .unwrap();
        std::fs::write(
            dir.join("libswift_Builtin_float.tbd"),
            "install-name: '/usr/lib/swift/libswift_Builtin_float.dylib'\n",
        )
        .unwrap();
        let base: Vec<String> = ["-platform_version", "macos", "14.0", "27.2", "-syslibroot"]
            .iter()
            .map(ToString::to_string)
            .chain([sdk.display().to_string(), "-L/usr/lib/swift".to_string()])
            .collect();
        let set = |names: &[&str]| {
            names
                .iter()
                .map(ToString::to_string)
                .collect::<BTreeSet<_>>()
        };
        let mut args = base.clone();
        prefer_direct_runtime(
            &mut args,
            &set(&["swiftDarwin", "swift_Builtin_float"]),
            &set(&["swift_Builtin_float"]),
        );
        assert_eq!(args[0], "-lswift_Builtin_float");
        // A runtime is not its own umbrella.
        let mut alone = base.clone();
        prefer_direct_runtime(
            &mut alone,
            &set(&["swift_Builtin_float"]),
            &set(&["swift_Builtin_float"]),
        );
        assert_eq!(alone, base);
        std::fs::remove_dir_all(&sdk).unwrap();
    }

    #[test]
    fn only_actual_reexports_count() {
        let stub = "install-name: '/usr/lib/swift/Child.dylib'\nreexported-libraries:\n  - targets: [arm64-ios]\n    libraries: ['/usr/lib/swift/Child.dylib']\nexports:\n symbols: ['/usr/lib/swift/Other.dylib']";
        assert!(reexports(stub, "/usr/lib/swift/Child.dylib"));
        assert!(!reexports(stub, "/usr/lib/swift/Other.dylib"));
    }
}
