//! Expand GNU response files before inspecting linker inputs. Swift Build uses
//! them for package object lists, including archives named with an `.o` suffix.

use std::collections::HashSet;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

pub(super) fn expand(args: &[String]) -> Result<Vec<String>, String> {
    fn append(
        args: &[String],
        active: &mut HashSet<PathBuf>,
        out: &mut Vec<String>,
    ) -> Result<(), String> {
        for arg in args {
            let Some(path) = arg.strip_prefix('@').map(Path::new).filter(|p| p.is_file()) else {
                // @rpath/@loader_path are Mach-O paths, not response files.
                // Leave missing files to the underlying linker's diagnostics.
                out.push(arg.clone());
                continue;
            };
            let path = path
                .canonicalize()
                .map_err(|e| format!("{}: {e}", path.display()))?;
            if active.len() >= 64 || !active.insert(path.clone()) {
                return Err(format!(
                    "recursive linker response file: {}",
                    path.display()
                ));
            }
            let text =
                std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
            // GNU response-file quoting: no shell expansion or execution.
            let words = sweetpad_lib::scheme::split_launch_argument(&text);
            append(&words, active, out)?;
            active.remove(&path);
        }
        Ok(())
    }
    let mut out = Vec::new();
    append(args, &mut HashSet::new(), &mut out)?;
    Ok(out)
}

/// Keep transformed response-file links below exec's argument-size limit.
/// Persist the input for dependency scans, changing its mtime only when its
/// contents change. Relative arguments retain the link command's working dir.
pub(super) fn forward(
    original: &[String],
    expanded: &[String],
    transformed: Vec<String>,
) -> Result<Vec<String>, String> {
    if transformed == expanded {
        return Ok(original.to_vec());
    }
    if original == expanded {
        return Ok(transformed);
    }
    let cache = sweetpad_core::paths::sweetpad_state_dir()
        .ok_or("linker response files require HOME or XDG_STATE_HOME")?
        .join("linker-inputs");
    rewrite(original, &transformed, &cache).map(|path| vec![format!("@{}", path.display())])
}

fn rewrite(original: &[String], transformed: &[String], cache: &Path) -> Result<PathBuf, String> {
    let mut key = std::hash::DefaultHasher::new();
    original.hash(&mut key);
    // Relative linker arguments are interpreted against the process cwd.
    std::env::current_dir()
        .map_err(|e| e.to_string())?
        .hash(&mut key);
    let path = cache.join(format!("link-{:x}.rsp", key.finish()));
    let text = transformed
        .iter()
        .map(|a| format!("\"{}\"", a.replace('\\', "\\\\").replace('"', "\\\"")))
        .collect::<Vec<_>>()
        .join("\n")
        + "\n";
    if std::fs::read_to_string(&path).is_ok_and(|old| old == text) {
        return Ok(path);
    }
    std::fs::create_dir_all(cache).map_err(|e| format!("{}: {e}", cache.display()))?;
    let temporary = path.with_extension(format!("{}.tmp", std::process::id()));
    std::fs::write(&temporary, text)
        .and_then(|()| std::fs::rename(&temporary, &path))
        .map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::testdir::TempDir;

    #[test]
    fn nested_response_files_preserve_quoted_paths_and_runtime_tokens() {
        let root = TempDir::new("link-response");
        let child = root.join("child ü.rsp");
        std::fs::write(&child, r#""path with spaces/Kit.o" escaped\ name.o"#).unwrap();
        let parent = root.join("parent.rsp");
        std::fs::write(
            &parent,
            format!("-dylib @\"{}\" -install_name @rpath/Kit", child.display()),
        )
        .unwrap();
        let args = vec![format!("@{}", parent.display())];
        assert_eq!(
            expand(&args).unwrap(),
            [
                "-dylib",
                "path with spaces/Kit.o",
                "escaped name.o",
                "-install_name",
                "@rpath/Kit"
            ]
        );
    }

    #[test]
    fn response_cycles_fail_without_recursing_forever() {
        let root = TempDir::new("link-response-cycle");
        let path = root.join("cycle.rsp");
        std::fs::write(&path, format!("@{}", path.display())).unwrap();
        assert!(
            expand(&[format!("@{}", path.display())])
                .unwrap_err()
                .contains("recursive")
        );
    }

    #[test]
    fn rewritten_response_round_trips_and_stays_unchanged_for_no_op_links() {
        let root = TempDir::new("link-response-write");
        let args = vec![
            "-force_load".into(),
            "directory ü/with \"quote\"\\name.o".into(),
            "@rpath/Kit".into(),
        ];
        let original = vec!["@original.rsp".into()];
        let path = rewrite(&original, &args, &root).unwrap();
        let modified = std::fs::metadata(&path).unwrap().modified().unwrap();
        assert_eq!(expand(&[format!("@{}", path.display())]).unwrap(), args);
        rewrite(&original, &args, &root).unwrap();
        assert_eq!(
            std::fs::metadata(&path).unwrap().modified().unwrap(),
            modified
        );
        assert_eq!(forward(&original, &args, args.clone()).unwrap(), original);
    }
}
