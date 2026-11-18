//! Swift Build invokes libtool once per architecture. Flatten input archives
//! and index their Mach-O symbols with the Swift toolchain's Darwin archiver.
use std::path::{Path, PathBuf};

pub(in crate::cli::darwin_tools) fn main(args: &[String]) -> Result<(), String> {
    if matches!(args, [flag] if matches!(flag.as_str(), "-V" | "-version" | "--version")) {
        let status = std::process::Command::new("llvm-ar")
            .arg("--version")
            .status()
            .map_err(|e| e.to_string())?;
        return status
            .success()
            .then_some(())
            .ok_or_else(|| format!("llvm-ar exited with {status}"));
    }
    let args = super::response::expand(args)?;
    let mut output = None;
    let mut info = None;
    let mut arch = None;
    let mut inputs = Vec::new();
    let mut libraries = Vec::new();
    let mut search = Vec::new();
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        let mut value = || super::super::value(arg, &mut args);
        match arg.as_str() {
            "-static" | "-D" | "-no_warning_for_no_symbols" => {}
            "-o" => output = Some(PathBuf::from(value()?)),
            "-dependency_info" => info = Some(PathBuf::from(value()?)),
            "-arch_only" => arch = Some(value()?.as_str()),
            "-filelist" => inputs.extend(super::FileList::read(value()?)?.paths()),
            "-syslibroot" => {
                value()?;
            }
            "-L" => search.push(PathBuf::from(value()?)),
            _ if arg.starts_with("-L") => search.push(PathBuf::from(&arg[2..])),
            _ if arg.starts_with("-l") => {
                let library = if arg == "-l" {
                    value()?.as_str()
                } else {
                    &arg[2..]
                };
                let name = if Path::new(library).extension().is_some_and(|ext| ext == "o") {
                    library.to_owned()
                } else {
                    format!("lib{library}.a")
                };
                libraries.push((inputs.len(), name));
                inputs.push(PathBuf::new());
            }
            _ if arg.starts_with('-') => {
                return Err(format!("unsupported static archive option {arg}"));
            }
            _ => inputs.push(PathBuf::from(arg)),
        }
    }
    for (index, name) in libraries {
        inputs[index] = search
            .iter()
            .map(|dir| dir.join(&name))
            .find(|p| p.is_file())
            .ok_or_else(|| format!("couldn't find {name}"))?;
    }
    let output = output.ok_or("missing -o for static archive")?;
    let scratch = super::relocatable::scratch(&output)?;
    let mut members = Vec::with_capacity(inputs.len());
    for (i, input) in inputs.iter().enumerate() {
        if let Some(arch) = arch {
            let data = super::super::read(input)?;
            let Some(slice) = super::super::lipo::archive_slice(input, &data, arch)? else {
                continue;
            };
            if slice.len() < data.len() {
                let path = scratch
                    .join(i.to_string())
                    .join(input.file_name().ok_or("missing archive member name")?);
                super::super::write(&path, slice)?;
                members.push(path);
                continue;
            }
        }
        members.push(input.clone());
    }
    if members.is_empty() {
        return Err("no matching objects for static archive".into());
    }
    super::relocatable::write_archive(&scratch, &output, &members, "qcsDL")?;
    if let Some(info) = info {
        super::super::write(
            &info,
            &super::relocatable::dependency_info("sweetpad libtool", &inputs, &output),
        )?;
    }
    Ok(())
}
