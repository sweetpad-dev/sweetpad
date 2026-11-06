use std::path::{Path, PathBuf};

pub(super) fn source(input: &Path) -> Result<PathBuf, String> {
    if !input.is_dir() {
        return Ok(input.to_path_buf());
    }
    for name in ["designtime.nib", "designable.storyboard"] {
        let path = input.join(name);
        if path.is_file() {
            return Ok(path);
        }
    }
    Err(format!(
        "{} has no editable Interface Builder source",
        input.display()
    ))
}

pub(super) fn write_xib(
    output: &Path,
    runtime: &[u8],
    source: &[u8],
    flatten: bool,
) -> Result<(), String> {
    if let Some(parent) = output.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    if output.is_dir() {
        std::fs::remove_dir_all(output).map_err(|e| e.to_string())?;
    }
    if flatten {
        std::fs::write(output, runtime).map_err(|e| e.to_string())
    } else {
        if output.is_file() {
            std::fs::remove_file(output).map_err(|e| e.to_string())?;
        }
        std::fs::create_dir_all(output).map_err(|e| e.to_string())?;
        std::fs::write(output.join("runtime.nib"), runtime).map_err(|e| e.to_string())?;
        std::fs::write(output.join("designtime.nib"), source).map_err(|e| e.to_string())?;
        Ok(())
    }
}

pub(super) fn strip(input: &Path, output: &Path) -> Result<(), String> {
    if input.is_dir() && input.join("runtime.nib").is_file() {
        return strip(&input.join("runtime.nib"), output);
    }
    if input.is_dir() && input.join("Info.plist").is_file() {
        if input.canonicalize().map_err(|e| e.to_string())?
            != output.canonicalize().unwrap_or_default()
        {
            super::copy_dir(input, output)?;
        }
        let design = output.join("designable.storyboard");
        if design.exists() {
            std::fs::remove_file(design).map_err(|e| e.to_string())?;
        }
        return Ok(());
    }
    let bytes = std::fs::read(input).map_err(|e| format!("{}: {e}", input.display()))?;
    if !bytes.starts_with(b"NIBArchive") && !bytes.starts_with(b"bplist00") {
        return Err("stripping requires a compiled NIB archive or editable NIB package".into());
    }
    if let Some(parent) = output.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    std::fs::write(output, bytes).map_err(|e| e.to_string())
}
