use std::path::{Path, PathBuf};

use crate::cli::keyed_archive::{self, Plist};

pub(super) fn validate_locale(locale: &str) -> Result<(), String> {
    if locale.is_empty() || locale == "." || locale == ".." || locale.contains(['/', '\\']) {
        return Err(format!("invalid companion strings locale {locale:?}"));
    }
    Ok(())
}

fn localization_root(output: &Path) -> Result<&Path, String> {
    output
        .parent()
        .filter(|parent| parent.file_name().is_some_and(|name| name == "Base.lproj"))
        .and_then(Path::parent)
        .ok_or("companion strings require an output inside Base.lproj".into())
}

pub(super) fn write_storyboard_companions(
    output: &Path,
    companions: &[(String, PathBuf)],
) -> Result<(), String> {
    if companions.is_empty() {
        return Ok(());
    }
    localization_root(output)?;
    let entries = companions
        .iter()
        .map(|(locale, path)| {
            (
                locale.clone(),
                Plist::String(path.to_string_lossy().into_owned()),
            )
        })
        .collect();
    std::fs::write(
        output.join("StringsFiles.plist"),
        keyed_archive::write(&Plist::Dict(entries)),
    )
    .map_err(|error| error.to_string())
}

pub(super) fn copy_xib_companions(
    output: &Path,
    companions: &[(String, PathBuf)],
) -> Result<(), String> {
    if companions.is_empty() {
        return Ok(());
    }
    let root = localization_root(output)?;
    let name = output
        .with_extension("strings")
        .file_name()
        .ok_or("missing XIB name")?
        .to_os_string();
    for (locale, source) in companions {
        let directory = root.join(format!("{locale}.lproj"));
        std::fs::create_dir_all(&directory).map_err(|error| error.to_string())?;
        std::fs::copy(source, directory.join(&name))
            .map_err(|error| format!("{}: {error}", source.display()))?;
    }
    Ok(())
}

pub(super) fn link_companions(
    input: &Path,
    target: &Path,
    destination: &Path,
) -> Result<(), String> {
    let mapping = input.join("StringsFiles.plist");
    if !mapping.exists() {
        return Ok(());
    }
    let root = sweetpad_lib::bplist::parse_file(&mapping).map_err(|error| error.to_string())?;
    let dictionary = root
        .as_dict()
        .ok_or("StringsFiles.plist must contain a dictionary")?;
    for (locale, value) in dictionary.iter() {
        validate_locale(locale)?;
        let source = Path::new(
            value
                .as_str()
                .ok_or("a companion strings path must be a string")?,
        );
        let name = source
            .file_name()
            .ok_or("a companion strings path has no file name")?;
        let directory = destination.join(format!("{locale}.lproj"));
        std::fs::create_dir_all(&directory).map_err(|error| error.to_string())?;
        std::fs::copy(source, directory.join(name))
            .map_err(|error| format!("{}: {error}", source.display()))?;
    }
    std::fs::remove_file(target.join("StringsFiles.plist")).map_err(|error| error.to_string())
}
