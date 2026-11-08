//! LLD binds /usr/lib/swift reexports through the first umbrella it opens.
//! Open a directly autolinked runtime first, while leaving SDK deployment
//! redirects and explicit loading policies to the linker. No output is patched.
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

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
    let mut search = Vec::new();
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        if arg == "-L" {
            if let Some(path) = iter.next() {
                search.push(PathBuf::from(path));
            }
        } else if let Some(path) = arg.strip_prefix("-L") {
            search.push(PathBuf::from(path));
        }
    }
    if let Some([_, root]) = args.windows(2).find(|a| a[0] == "-syslibroot") {
        search.push(Path::new(root).join("usr/lib"));
    }
    let stub = |name: &str| {
        search
            .iter()
            .map(|p| p.join(format!("lib{name}.tbd")))
            .find(|p| p.is_file())
    };
    let parents: Vec<_> = args
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
        .filter_map(&stub)
        .filter_map(|p| std::fs::read_to_string(p).ok())
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
        if parents.iter().any(|p| reexports(p, &install)) {
            // Plain -l retains LLD's inference from weak/strong symbol references.
            leading.push(format!("-l{library}"));
        }
    }
    args.splice(0..0, leading);
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
    fn only_actual_reexports_count() {
        let stub = "install-name: '/usr/lib/swift/Child.dylib'\nreexported-libraries:\n  - targets: [arm64-ios]\n    libraries: ['/usr/lib/swift/Child.dylib']\nexports:\n symbols: ['/usr/lib/swift/Other.dylib']";
        assert!(reexports(stub, "/usr/lib/swift/Child.dylib"));
        assert!(!reexports(stub, "/usr/lib/swift/Other.dylib"));
    }
}
