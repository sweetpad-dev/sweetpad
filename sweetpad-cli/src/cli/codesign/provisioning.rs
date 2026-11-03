//! A stand-in for the provisioning Xcode does before a signed build, which
//! Swift Build asks its client for once per target: the identity to sign
//! with, the provisioning profile to embed and the entitlements to sign
//! into the product. Swift Build's console asks the program
//! `SWEETPAD_PROVISIONING` names (a patch of sweetpad's, `linux-setup.sh`
//! applies it), sending the target's request on standard input and reading
//! the inputs back from standard output, each an XML property list.
//!
//! The identity is the one in `SWEETPAD_CODE_SIGN_P12` (see
//! [`super::identity`]). The profile is picked as Xcode picks one, from
//! `SWEETPAD_PROVISIONING_PROFILES`, else the folders Xcode keeps them in:
//! for automatic signing, the most specific unexpired one for the team,
//! the bundle identifier, the platform and the identity's certificate; for
//! manual signing, the one `PROVISIONING_PROFILE_SPECIFIER` names. The
//! entitlements are the project's, with the application identifier, the
//! team and the profile's `get-task-allow`, as Xcode adds them.

// What this prints is the tool's answer, which Swift Build reads, so it
// doesn't go through `Output`.
#![allow(clippy::print_stdout, clippy::print_stderr)]

use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use super::der::{self, Reader, oids};
use super::identity::{self, Identity};
use super::plist::{self, Plist};

/// The name this binary runs under to answer Swift Build.
pub const TOOL: &str = "sweetpad-provisioning";
pub const PROFILES_VARIABLE: &str = "SWEETPAD_PROVISIONING_PROFILES";

#[must_use]
pub fn main(_args: &[String]) -> ExitCode {
    let mut input = Vec::new();
    if let Err(e) = std::io::stdin().read_to_end(&mut input) {
        eprintln!("error: {TOOL}: couldn't read the request: {e}");
        return ExitCode::FAILURE;
    }
    let inputs = plist::parse(&input)
        .and_then(|request| {
            answer(&request, &profile_folders(), || {
                Identity::from_environment(false)
            })
        })
        .unwrap_or_else(|message| {
            Plist::Dict(vec![(
                "errors".to_string(),
                Plist::Array(vec![Plist::String(message)]),
            )])
        });
    print!("{}", plist::to_xml(&inputs));
    ExitCode::SUCCESS
}

/// The folders profiles are looked up in.
fn profile_folders() -> Vec<PathBuf> {
    if let Some(folder) = std::env::var_os(PROFILES_VARIABLE).filter(|f| !f.is_empty()) {
        return vec![PathBuf::from(folder)];
    }
    let home = PathBuf::from(std::env::var_os("HOME").unwrap_or_default());
    vec![
        home.join("Library/Developer/Xcode/UserData/Provisioning Profiles"),
        home.join("Library/MobileDevice/Provisioning Profiles"),
    ]
}

/// The provisioning inputs for one target's `request`, with the profiles
/// in `folders` and the identity `load` reads, when the request needs one.
pub(super) fn answer(
    request: &Plist,
    folders: &[PathBuf],
    load: impl FnOnce() -> Result<Option<Identity>, String>,
) -> Result<Plist, String> {
    let text = |key: &str| plist::string(request, key).unwrap_or_default();
    let dict = |key: &str| match plist::get(request, key) {
        Some(Plist::Dict(entries)) => entries.clone(),
        _ => Vec::new(),
    };
    let name = text("signingCertificateIdentifier");
    let bundle = text("bundleIdentifier");
    let destination = text("entitlementsDestination");
    if name.is_empty() {
        return Ok(Plist::Dict(Vec::new()));
    }
    let mut entitlements = Plist::Dict(dict("productTypeEntitlements"));
    if name == "-" {
        merge(&mut entitlements, dict("projectEntitlements"));
        return Ok(ad_hoc(entitlements, bundle, destination, text("sdkRoot")));
    }

    let identity = load()?.ok_or_else(|| {
        format!(
            "CODE_SIGN_IDENTITY is '{name}', and there's no keychain to find it in here: \
             export the identity from Keychain Access as a .p12 file, point {} at it and \
             put its password in {}",
            identity::P12_VARIABLE,
            identity::PASSWORD_VARIABLE
        )
    })?;
    let certificate = &identity.certificate;
    if !identity.matches(name) {
        return Err(format!(
            "CODE_SIGN_IDENTITY is '{name}', but the identity in {} is '{}'",
            identity::P12_VARIABLE,
            certificate.common_name
        ));
    }
    let team = team(request, &identity, bundle)?;
    let profile = if text("provisioningProfileSupport") == "unsupported" {
        None
    } else {
        find_profile(request, folders, &identity, &team)?
    };
    let project = dict("projectEntitlements");
    if let Some(profile) = &profile {
        check_entitlements(profile, &project)?;
    }
    merge(&mut entitlements, project);
    plist::set(
        &mut entitlements,
        "application-identifier",
        Plist::String(format!("{team}.{bundle}")),
    );
    plist::set(
        &mut entitlements,
        "com.apple.developer.team-identifier",
        Plist::String(team.clone()),
    );
    if let Some(allow) = profile
        .as_ref()
        .and_then(|p| plist::get(&p.entitlements, "get-task-allow"))
    {
        plist::set(&mut entitlements, "get-task-allow", allow.clone());
    }
    let (signed, simulated) = split(entitlements, destination);
    let prefix = format!("{team}.");
    let mut inputs = vec![
        (
            "identityHash".to_string(),
            Plist::String(certificate.sha1_hex()),
        ),
        (
            "identityName".to_string(),
            Plist::String(certificate.common_name.clone()),
        ),
        ("signedEntitlements".to_string(), signed),
        ("simulatedEntitlements".to_string(), simulated),
        (
            "appIdentifierPrefix".to_string(),
            Plist::String(prefix.clone()),
        ),
        ("teamIdentifierPrefix".to_string(), Plist::String(prefix)),
    ];
    if let Some(profile) = profile {
        inputs.push(("profileName".to_string(), Plist::String(profile.name)));
        inputs.push(("profileUUID".to_string(), Plist::String(profile.uuid)));
        inputs.push((
            "profilePath".to_string(),
            Plist::String(profile.path.to_string_lossy().into_owned()),
        ));
    }
    Ok(Plist::Dict(inputs))
}

/// The inputs for an ad hoc signature, as Swift Build's own console gives
/// them: the bundle identifier as the application identifier, and a
/// simulator's build debuggable.
fn ad_hoc(mut entitlements: Plist, bundle: &str, destination: &str, sdk_root: &str) -> Plist {
    plist::set(
        &mut entitlements,
        "application-identifier",
        Plist::String(bundle.to_string()),
    );
    let (mut signed, simulated) = split(entitlements, destination);
    if sdk_root.contains("simulator") {
        plist::set(&mut signed, "get-task-allow", Plist::Bool(true));
    }
    Plist::Dict(vec![
        ("identityHash".to_string(), Plist::String("-".to_string())),
        ("identityName".to_string(), Plist::String("-".to_string())),
        ("signedEntitlements".to_string(), signed),
        ("simulatedEntitlements".to_string(), simulated),
    ])
}

/// The team a target signs for: `DEVELOPMENT_TEAM`, else the identity's.
fn team(request: &Plist, identity: &Identity, bundle: &str) -> Result<String, String> {
    let name = &identity.certificate.common_name;
    match (
        plist::string(request, "teamID").filter(|t| !t.is_empty()),
        identity.team(),
    ) {
        (Some(team), Some(own)) if team != own => Err(format!(
            "DEVELOPMENT_TEAM is {team}, but '{name}' belongs to team {own}"
        )),
        (Some(team), _) | (None, Some(team)) => Ok(team.to_string()),
        (None, None) => Err(format!(
            "signing '{bundle}' needs a team: set DEVELOPMENT_TEAM"
        )),
    }
}

/// The profile a target embeds: the one manual signing names, or the one
/// automatic signing picks, and an error when the target requires one and
/// none fits.
fn find_profile(
    request: &Plist,
    folders: &[PathBuf],
    identity: &Identity,
    team: &str,
) -> Result<Option<Profile>, String> {
    let text = |key: &str| plist::string(request, key).unwrap_or_default();
    let bundle = text("bundleIdentifier");
    let profiles = load_profiles(folders);
    let picked = if text("provisioningStyle") == "manual" {
        let specifier = text("provisioningProfileSpecifier");
        let uuid = text("provisioningProfileUUID");
        profiles.into_iter().find(|p| {
            (!uuid.is_empty() && p.uuid == uuid)
                || (!specifier.is_empty() && (p.name == specifier || p.uuid == specifier))
        })
    } else {
        pick(profiles, identity, team, bundle, platform(text("sdkRoot")))
    };
    if picked.is_none() && text("provisioningProfileSupport") == "required" {
        let looked = folders
            .iter()
            .map(|f| f.display().to_string())
            .collect::<Vec<_>>()
            .join(", ");
        return Err(format!(
            "No profiles for '{bundle}' were found: none in {looked} allows it for team \
             {team} and '{}' (point {PROFILES_VARIABLE} at the folder that has yours)",
            identity.certificate.common_name
        ));
    }
    Ok(picked)
}

/// That `profile` allows each of the project's entitlements that a profile
/// has to grant, as Xcode checks before it signs.
fn check_entitlements(profile: &Profile, project: &[(String, Plist)]) -> Result<(), String> {
    for (key, _) in project {
        let provisioned = (key.starts_with("com.apple.developer.")
            && key != "com.apple.developer.team-identifier")
            || [
                "aps-environment",
                "com.apple.security.application-groups",
                "keychain-access-groups",
            ]
            .contains(&key.as_str());
        if provisioned && plist::get(&profile.entitlements, key).is_none() {
            return Err(format!(
                "Provisioning profile \"{}\" doesn't include the {key} entitlement.",
                profile.name
            ));
        }
    }
    Ok(())
}

/// `entitlements` where the target's destination puts them: signed into the
/// product, or into the simulator's `__entitlements` section.
fn split(entitlements: Plist, destination: &str) -> (Plist, Plist) {
    let empty = Plist::Dict(Vec::new());
    if destination == "__entitlements" {
        (empty, entitlements)
    } else if destination == "Signature" {
        (entitlements, empty)
    } else {
        (empty.clone(), empty)
    }
}

fn merge(into: &mut Plist, entries: Vec<(String, Plist)>) {
    for (key, value) in entries {
        plist::set(into, &key, value);
    }
}

/// The platform a profile names for the SDK `sdk_root`, like
/// `iphoneos27.0`.
fn platform(sdk_root: &str) -> &'static str {
    let name = sdk_root.trim_end_matches(|c: char| c.is_ascii_digit() || c == '.');
    match name {
        "appletvos" => "tvOS",
        "watchos" => "watchOS",
        "xros" => "xrOS",
        "macosx" => "OSX",
        _ => "iOS",
    }
}

/// A provisioning profile, as much of it as picking one takes.
#[derive(Debug, Clone)]
pub(super) struct Profile {
    pub path: PathBuf,
    pub name: String,
    pub uuid: String,
    pub teams: Vec<String>,
    pub platforms: Vec<String>,
    pub entitlements: Plist,
    pub certificates: Vec<Vec<u8>>,
    /// When it expires and was made, as their ISO 8601 text, which sorts.
    pub expiration: String,
    pub creation: String,
}

impl Profile {
    pub fn read(path: &Path) -> Result<Self, String> {
        let data =
            std::fs::read(path).map_err(|e| format!("couldn't read {}: {e}", path.display()))?;
        let content = signed_content(&data).map_err(|e| format!("{}: {e}", path.display()))?;
        let value = plist::parse(&content).map_err(|e| format!("{}: {e}", path.display()))?;
        let strings = |key: &str| match plist::get(&value, key) {
            Some(Plist::Array(items)) => items
                .iter()
                .filter_map(|i| match i {
                    Plist::String(s) => Some(s.clone()),
                    _ => None,
                })
                .collect(),
            _ => Vec::new(),
        };
        let certificates = match plist::get(&value, "DeveloperCertificates") {
            Some(Plist::Array(items)) => items
                .iter()
                .filter_map(|i| match i {
                    Plist::Data(d) => Some(d.clone()),
                    _ => None,
                })
                .collect(),
            _ => Vec::new(),
        };
        Ok(Self {
            path: path.to_path_buf(),
            name: plist::string(&value, "Name")
                .unwrap_or_default()
                .to_string(),
            uuid: plist::string(&value, "UUID")
                .unwrap_or_default()
                .to_string(),
            teams: strings("TeamIdentifier"),
            platforms: strings("Platform"),
            entitlements: plist::get(&value, "Entitlements")
                .cloned()
                .unwrap_or(Plist::Dict(Vec::new())),
            certificates,
            expiration: plist::string(&value, "ExpirationDate")
                .unwrap_or_default()
                .to_string(),
            creation: plist::string(&value, "CreationDate")
                .unwrap_or_default()
                .to_string(),
        })
    }
}

/// The content a profile's CMS signature wraps: its property list.
fn signed_content(data: &[u8]) -> Result<Vec<u8>, String> {
    let info = Reader::new(data).read()?;
    let mut fields = info.children();
    if fields.read()?.content != oids::SIGNED_DATA {
        return Err("not a signed provisioning profile".to_string());
    }
    let signed = fields.read()?.expect(der::context(0))?.children().read()?;
    let mut parts = signed.children();
    parts.read()?;
    parts.read()?;
    let encapsulated = parts.read()?;
    let mut content = encapsulated.children();
    content.read()?;
    content
        .read()?
        .expect(der::context(0))?
        .children()
        .read()?
        .octets()
}

fn load_profiles(folders: &[PathBuf]) -> Vec<Profile> {
    let mut profiles = Vec::new();
    for folder in folders {
        let Ok(entries) = std::fs::read_dir(folder) else {
            continue;
        };
        let mut paths: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
        paths.sort();
        for path in paths {
            let extension = path
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or_default();
            if !["mobileprovision", "provisionprofile"].contains(&extension) {
                continue;
            }
            match Profile::read(&path) {
                Ok(profile) => profiles.push(profile),
                Err(message) => eprintln!("warning: {TOOL}: skipping {message}"),
            }
        }
    }
    profiles
}

/// The profile automatic signing picks: for `team`, `bundle` and
/// `platform`, with `identity`'s certificate, of its kind (development or
/// distribution), unexpired; an exact application identifier over a
/// wildcard, then the newest.
fn pick(
    profiles: Vec<Profile>,
    identity: &Identity,
    team: &str,
    bundle: &str,
    platform: &str,
) -> Option<Profile> {
    let now = now_iso8601();
    let application = format!("{team}.{bundle}");
    profiles
        .into_iter()
        .filter(|p| p.teams.iter().any(|t| t == team))
        .filter(|p| p.platforms.iter().any(|t| t == platform))
        .filter(|p| p.certificates.contains(&identity.certificate.der))
        .filter(|p| p.expiration.is_empty() || p.expiration > now)
        .filter(|p| {
            plist::get(&p.entitlements, "get-task-allow").is_some_and(|v| *v == Plist::Bool(true))
                == identity.is_development()
        })
        .filter_map(|p| {
            let pattern = plist::string(&p.entitlements, "application-identifier")?;
            let specificity = if pattern == application {
                usize::MAX
            } else {
                let prefix = pattern.strip_suffix('*')?;
                application.starts_with(prefix).then_some(prefix.len())?
            };
            Some((specificity, p))
        })
        .max_by(|(a, p), (b, q)| a.cmp(b).then_with(|| p.creation.cmp(&q.creation)))
        .map(|(_, p)| p)
}

/// The current time, as a property list writes a date.
fn now_iso8601() -> String {
    let seconds = std::time::SystemTime::now()
        .duration_since(std::time::SystemTime::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let days = i64::try_from(seconds / 86_400).unwrap_or(0);
    let rest = seconds % 86_400;
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        rest / 3600,
        rest % 3600 / 60,
        rest % 60
    )
}
