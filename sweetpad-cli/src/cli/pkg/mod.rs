//! Flat installer packages, as `productbuild --component <App>.app
//! /Applications --sign <identity>` writes them: a product archive whose
//! Distribution installs one component package, and that package's bill of
//! materials, payload and PackageInfo installing the app in /Applications.
//! The Mac App Store takes a Mac app as this.

mod bom;
mod payload;
mod xar;

use std::path::Path;
use std::time::SystemTime;

use crate::cli::codesign::installer::Installer;
use crate::cli::property_list as plist;
use crate::cli::xml_plist::escape;

/// What a package says of the app it installs, each part left out when
/// the app's Info.plist has none, as `productbuild` leaves it out.
struct App {
    /// The bundle's name, without `.app`.
    name: String,
    id: String,
    /// CFBundleDisplayName, else CFBundleName.
    title: Option<String>,
    short_version: Option<String>,
    version: Option<String>,
    minimum_system: Option<String>,
    architectures: Vec<&'static str>,
}

impl App {
    fn read(app: &Path) -> Result<Self, String> {
        let info_path = app.join("Contents/Info.plist");
        let data = std::fs::read(&info_path)
            .map_err(|e| format!("couldn't read {}: {e}", info_path.display()))?;
        let info = plist::read_info(&data)?;
        let optional = |key: &str| plist::string(&info, key).map(str::to_string);
        let required = |key: &str| {
            optional(key).ok_or_else(|| format!("{} has no {key}", info_path.display()))
        };
        let executable = app
            .join("Contents/MacOS")
            .join(required("CFBundleExecutable")?);
        let binary = std::fs::read(&executable)
            .map_err(|e| format!("couldn't read {}: {e}", executable.display()))?;
        let slices = payload::slices(&binary)
            .ok_or_else(|| format!("{} isn't a Mach-O executable", executable.display()))?;
        let mut architectures: Vec<&'static str> = slices
            .list
            .iter()
            .filter_map(|slice| match slice.cputype {
                0x0100_000c => Some("arm64"),
                0x0100_0007 => Some("x86_64"),
                _ => None,
            })
            .collect();
        architectures.sort_unstable();
        architectures.dedup();
        let name = app
            .file_stem()
            .and_then(|s| s.to_str())
            .ok_or_else(|| format!("{} has no UTF-8 name", app.display()))?
            .to_string();
        Ok(Self {
            name,
            id: required("CFBundleIdentifier")?,
            title: optional("CFBundleDisplayName").or_else(|| optional("CFBundleName")),
            short_version: optional("CFBundleShortVersionString"),
            version: optional("CFBundleVersion"),
            minimum_system: optional("LSMinimumSystemVersion"),
            architectures,
        })
    }

    /// The package's version: the app's in at least three parts, or 0.
    fn package_version(&self) -> String {
        let Some(short) = &self.short_version else {
            return "0".to_string();
        };
        let mut parts: Vec<&str> = short.split('.').collect();
        while parts.len() < 3 {
            parts.push("0");
        }
        parts.join(".")
    }
}

/// Write `app` as a product archive at `out`, signed by `signer` when there
/// is one.
pub(crate) fn product(app: &Path, signer: Option<&Installer>, out: &Path) -> Result<(), String> {
    let info = App::read(app)?;
    let staged = out.with_extension("payload");
    let result = (|| {
        let payload = payload::write(app, &staged)?;
        let bom = bom::bill(&payload.entries)?;
        let component = format!("{}.pkg", info.id);
        let kbytes = payload.install_bytes() / 1024;
        let package_info = package_info(&info, payload.entries.len() - 1, kbytes);
        let distribution = distribution(&info, &component, kbytes);
        xar::write(
            out,
            &xar::Product {
                component: &component,
                bom: &bom,
                payload: &staged,
                payload_sha1: payload.sha1,
                payload_length: payload.length,
                package_info: package_info.as_bytes(),
                distribution: distribution.as_bytes(),
            },
            signer,
            SystemTime::now(),
        )
    })();
    let _ = std::fs::remove_file(&staged);
    if result.is_err() {
        let _ = std::fs::remove_file(out);
    }
    result
}

/// ` name="value"`, when there is a value.
fn attribute(name: &str, value: Option<&str>) -> String {
    value.map_or_else(String::new, |value| {
        format!(" {name}=\"{}\"", escape_attribute(value))
    })
}

fn package_info(app: &App, files: usize, kbytes: u64) -> String {
    let id = escape_attribute(&app.id);
    let path = escape_attribute(&format!("./{}.app", app.name));
    let version = attribute("CFBundleVersion", app.version.as_deref());
    // The order `productbuild` writes these in.
    let bundle = match &app.short_version {
        Some(short) => format!(
            "<bundle path=\"{path}\" id=\"{id}\"{}{version}/>",
            attribute("CFBundleShortVersionString", Some(short))
        ),
        None => format!("<bundle id=\"{id}\"{version} path=\"{path}\"/>"),
    };
    format!(
        r#"<?xml version="1.0" encoding="utf-8"?>
<pkg-info overwrite-permissions="true" relocatable="false" identifier="{id}" postinstall-action="none" version="{package_version}" format-version="2" generator-version="sweetpad-{generator}" install-location="/Applications" auth="root" preserve-xattr="true">
    <payload numberOfFiles="{files}" installKBytes="{kbytes}"/>
    {bundle}
    <bundle-version>
        <bundle id="{id}"/>
    </bundle-version>
    <upgrade-bundle>
        <bundle id="{id}"/>
    </upgrade-bundle>
    <update-bundle/>
    <atomic-update-bundle/>
    <strict-identifier>
        <bundle id="{id}"/>
    </strict-identifier>
    <relocate/>
</pkg-info>"#,
        package_version = escape_attribute(&app.package_version()),
        generator = env!("CARGO_PKG_VERSION"),
    )
}

fn distribution(app: &App, component: &str, kbytes: u64) -> String {
    let id = escape_attribute(&app.id);
    let short = app.short_version.as_deref();
    let title = attribute("title", app.title.as_deref());
    let title_line = app
        .title
        .as_deref()
        .map(|title| format!("\n    <title>{}</title>", escape(title)))
        .unwrap_or_default();
    let volume_check = app
        .minimum_system
        .as_deref()
        .map(|minimum| {
            format!(
                "\n    <volume-check>\n        <allowed-os-versions>\n            \
                 <os-version min=\"{}\"/>\n        </allowed-os-versions>\n    </volume-check>",
                escape_attribute(minimum)
            )
        })
        .unwrap_or_default();
    format!(
        r#"<?xml version="1.0" encoding="utf-8"?>
<installer-gui-script minSpecVersion="2">
    <pkg-ref id="{id}">
        <bundle-version>
            <bundle{short_attribute}{build} id="{id}" path="{name}.app"/>
        </bundle-version>
    </pkg-ref>
    <product id="{id}"{product_version}/>{title_line}
    <options customize="never" require-scripts="false" hostArchitectures="{architectures}"/>{volume_check}
    <choices-outline>
        <line choice="default">
            <line choice="{id}"/>
        </line>
    </choices-outline>
    <choice id="default"{title}{versions}/>
    <choice id="{id}"{title} visible="false" customLocation="/Applications">
        <pkg-ref id="{id}"/>
    </choice>
    <pkg-ref id="{id}" version="{version}" onConclusion="none" installKBytes="{kbytes}" updateKBytes="0">#{component}</pkg-ref>
</installer-gui-script>"#,
        short_attribute = attribute("CFBundleShortVersionString", short),
        build = attribute("CFBundleVersion", app.version.as_deref()),
        name = escape_attribute(&app.name),
        product_version = attribute("version", short),
        architectures = app.architectures.join(","),
        versions = attribute("versStr", short),
        version = escape_attribute(&app.package_version()),
        component = escape(component),
    )
}

fn escape_attribute(text: &str) -> String {
    escape(text).replace('"', "&quot;")
}

#[cfg(test)]
mod tests;
