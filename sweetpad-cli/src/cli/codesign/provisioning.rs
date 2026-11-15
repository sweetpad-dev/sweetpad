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
//! team and the profile's `get-task-allow`, as Xcode adds them. A macOS
//! target names its application and team only beside a profile, which a
//! restricted entitlement needs to launch, and is debuggable when Swift
//! Build asks for the base entitlements.

// What this prints is the tool's answer, which Swift Build reads, so it
// doesn't go through `Output`.
#![allow(clippy::print_stdout, clippy::print_stderr)]

use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use super::der::{self, Reader, oids};
use super::identity::{self, Certificate, Identity, IdentitySet, Kind};
use super::plist::{self, Plist};
use super::time::now_iso8601;

/// The name this binary runs under to answer Swift Build.
pub const TOOL: &str = "sweetpad-provisioning";
pub const PROFILES_VARIABLE: &str = "SWEETPAD_PROVISIONING_PROFILES";
/// Set to `ad-hoc` for an archive, whose export signs it again: a target the
/// identities can't sign is signed ad hoc, with its entitlements, in place of
/// failing the build.
pub const FALLBACK_VARIABLE: &str = "SWEETPAD_CODE_SIGN_FALLBACK";
/// The identifier of the device a build runs on, which a development or ad
/// hoc profile must list. [`main`] adds it to the request under
/// [`DEVICE_KEY`].
pub const DEVICE_VARIABLE: &str = "SWEETPAD_PROVISIONING_DEVICE";
const DEVICE_KEY: &str = "sweetpadDeviceIdentifier";

#[must_use]
pub fn main(_args: &[String]) -> ExitCode {
    let mut input = Vec::new();
    if let Err(e) = std::io::stdin().read_to_end(&mut input) {
        eprintln!("error: {TOOL}: couldn't read the request: {e}");
        return ExitCode::FAILURE;
    }
    let fallback = std::env::var(FALLBACK_VARIABLE).is_ok_and(|v| v == "ad-hoc");
    let device = std::env::var(DEVICE_VARIABLE)
        .ok()
        .filter(|d| !d.is_empty());
    let inputs = plist::parse(&input)
        .map(|request| match (request, device) {
            (Plist::Dict(mut entries), Some(device)) => {
                entries.push((DEVICE_KEY.to_string(), Plist::String(device)));
                Plist::Dict(entries)
            }
            (request, _) => request,
        })
        .and_then(|request| {
            let answered = answer(&request, &profile_folders(), || {
                IdentitySet::from_environment(false)
            });
            match answered {
                Err(why) if fallback => Ok(archive_ad_hoc(&request, &why)),
                answered => answered,
            }
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
pub(crate) fn profile_folders() -> Vec<PathBuf> {
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
    load: impl FnOnce() -> Result<Option<IdentitySet>, String>,
) -> Result<Plist, String> {
    let text = |key: &str| plist::string(request, key).unwrap_or_default();
    let dict = |key: &str| match plist::get(request, key) {
        Some(Plist::Dict(entries)) => entries.clone(),
        _ => Vec::new(),
    };
    let name = text("signingCertificateIdentifier");
    let bundle = text("bundleIdentifier");
    let destination = text("entitlementsDestination");
    // A device build also resolves host macro targets. A command-line Apple
    // identity must not give their ELF executables an expanded signing identity:
    // Swift Build would then pass Darwin's -no_adhoc_codesign to the host linker.
    if name.is_empty() || text("sdkRoot").starts_with("linux") {
        return Ok(Plist::Dict(Vec::new()));
    }
    let mut entitlements = Plist::Dict(dict("productTypeEntitlements"));
    let mac = platform(text("sdkRoot")) == "OSX";
    if mac && plist::get(request, "wantsBaseEntitlementInjection") == Some(&Plist::Bool(true)) {
        // The macOS platform's base entitlements, which an archive's
        // packaging takes out again.
        plist::set(
            &mut entitlements,
            "com.apple.security.get-task-allow",
            Plist::Bool(true),
        );
    }
    if text("sdkRoot").contains("simulator") {
        merge(&mut entitlements, dict("projectEntitlements"));
        let team = plist::string(request, "teamID").filter(|t| !t.is_empty());
        return Ok(simulator(entitlements, bundle, destination, team));
    }
    if name == "-" {
        merge(&mut entitlements, dict("projectEntitlements"));
        return Ok(ad_hoc(entitlements, bundle, destination, text("sdkRoot")));
    }

    let set = load()?.ok_or_else(|| {
        format!(
            "CODE_SIGN_IDENTITY is '{name}', and there's no keychain to find it in here: \
             export the identity from Keychain Access as a .p12 file, point {} at it and \
             put its password in {}",
            identity::P12_VARIABLE,
            identity::PASSWORD_VARIABLE
        )
    })?;
    let (identity, team, profile) = choose(request, folders, &set, name, bundle)?;
    let certificate = &identity.certificate;
    let project = dict("projectEntitlements");
    if let Some(profile) = &profile {
        check_entitlements(profile, &project, &team)?;
    }
    merge(&mut entitlements, project);
    if let Some(profile) = &profile {
        for key in ["aps-environment", "com.apple.developer.aps-environment"] {
            if plist::get(&entitlements, key).is_some()
                && let Some(value) = plist::get(&profile.entitlements, key)
            {
                plist::set(&mut entitlements, key, value.clone());
            }
        }
    }
    if !mac || profile.is_some() {
        plist::set(
            &mut entitlements,
            application_key(mac),
            Plist::String(format!("{team}.{bundle}")),
        );
        plist::set(
            &mut entitlements,
            "com.apple.developer.team-identifier",
            Plist::String(team.clone()),
        );
    }
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

/// The identity, team and profile a target signs with: of the identities
/// `name` picks, the newest that has a profile, as Xcode picks the
/// certificate a profile holds.
fn choose<'a>(
    request: &Plist,
    folders: &[PathBuf],
    set: &'a IdentitySet,
    name: &str,
    bundle: &str,
) -> Result<(&'a Identity, String, Option<Profile>), String> {
    let wanted = plist::string(request, "teamID").filter(|t| !t.is_empty());
    let named = set.named(name, None);
    let Some(first) = named.first() else {
        return Err(format!(
            "CODE_SIGN_IDENTITY is '{name}', but {}",
            set.missing(&format!("matches '{name}'"))
        ));
    };
    let candidates = set.named(name, wanted);
    if candidates.is_empty() {
        return Err(format!(
            "DEVELOPMENT_TEAM is {}, but '{}' belongs to team {}",
            wanted.unwrap_or_default(),
            first.certificate.common_name,
            first.team().unwrap_or_default()
        ));
    }
    let supported = plist::string(request, "provisioningProfileSupport") != Some("unsupported");
    let mut chosen = None;
    let mut first_error = None;
    for identity in candidates {
        let found = team(request, identity, bundle).and_then(|team| {
            let profile = if supported {
                find_profile(request, folders, identity, &team)?
            } else {
                None
            };
            Ok((team, profile))
        });
        match found {
            Ok((team, profile)) => {
                let has_profile = profile.is_some();
                if chosen.is_none() || has_profile {
                    chosen = Some((identity, team, profile));
                }
                if has_profile {
                    break;
                }
            }
            Err(e) => {
                first_error.get_or_insert(e);
            }
        }
    }
    chosen.ok_or_else(|| first_error.unwrap_or_default())
}

/// The inputs for an archive's ad hoc signature when the identities can't
/// sign a target, `why` saying what was missing: the target's entitlements,
/// with the team's prefixes when `DEVELOPMENT_TEAM` names one, which an
/// export signs again with the identity and profile of its method.
pub(super) fn archive_ad_hoc(request: &Plist, why: &str) -> Plist {
    let text = |key: &str| plist::string(request, key).unwrap_or_default();
    let dict = |key: &str| match plist::get(request, key) {
        Some(Plist::Dict(entries)) => entries.clone(),
        _ => Vec::new(),
    };
    let bundle = text("bundleIdentifier");
    let team = Some(text("teamID")).filter(|t| !t.is_empty());
    let mut entitlements = Plist::Dict(dict("productTypeEntitlements"));
    merge(&mut entitlements, dict("projectEntitlements"));
    let application = team.map_or_else(|| bundle.to_string(), |t| format!("{t}.{bundle}"));
    plist::set(
        &mut entitlements,
        application_key(platform(text("sdkRoot")) == "OSX"),
        Plist::String(application),
    );
    let mut warning = format!("signing '{bundle}' ad hoc for its export to sign again: {why}");
    if let Some(team) = team {
        plist::set(
            &mut entitlements,
            "com.apple.developer.team-identifier",
            Plist::String(team.to_string()),
        );
    } else {
        warning.push_str("; set DEVELOPMENT_TEAM so its entitlements name the team");
    }
    let (signed, simulated) = split(entitlements, text("entitlementsDestination"));
    let mut inputs = vec![
        ("identityHash".to_string(), Plist::String("-".to_string())),
        ("identityName".to_string(), Plist::String("-".to_string())),
        ("signedEntitlements".to_string(), signed),
        ("simulatedEntitlements".to_string(), simulated),
        (
            "warnings".to_string(),
            Plist::Array(vec![Plist::String(warning)]),
        ),
    ];
    if let Some(team) = team {
        for key in ["appIdentifierPrefix", "teamIdentifierPrefix"] {
            inputs.push((key.to_string(), Plist::String(format!("{team}."))));
        }
    }
    Plist::Dict(inputs)
}

/// The key of the application identifier entitlement: macOS has its own.
fn application_key(mac: bool) -> &'static str {
    if mac {
        "com.apple.application-identifier"
    } else {
        "application-identifier"
    }
}

/// The inputs for a simulator build, which Xcode signs to run locally
/// whatever identity the target names. Its entitlements go in the
/// executable's `__entitlements` section, where the simulator reads App
/// Groups and keychain groups from, with the team's prefix on the
/// application identifier and, through the inputs, on the keychain groups.
/// The signature itself carries none.
fn simulator(
    mut entitlements: Plist,
    bundle: &str,
    destination: &str,
    team: Option<&str>,
) -> Plist {
    let prefix = team.map(|team| format!("{team}."));
    plist::set(
        &mut entitlements,
        "application-identifier",
        Plist::String(format!("{}{bundle}", prefix.as_deref().unwrap_or_default())),
    );
    let (signed, simulated) = split(entitlements, destination);
    let mut inputs = vec![
        ("identityHash".to_string(), Plist::String("-".to_string())),
        ("identityName".to_string(), Plist::String("-".to_string())),
        ("signedEntitlements".to_string(), signed),
        ("simulatedEntitlements".to_string(), simulated),
    ];
    if let Some(prefix) = prefix {
        for key in ["appIdentifierPrefix", "teamIdentifierPrefix"] {
            inputs.push((key.to_string(), Plist::String(prefix.clone())));
        }
    }
    Plist::Dict(inputs)
}

/// The inputs for an ad hoc signature, as Swift Build's own console gives
/// them: the bundle identifier as the application identifier. A macOS app
/// signed to run locally names no application, which needs a profile.
fn ad_hoc(mut entitlements: Plist, bundle: &str, destination: &str, sdk_root: &str) -> Plist {
    if platform(sdk_root) != "OSX" {
        plist::set(
            &mut entitlements,
            "application-identifier",
            Plist::String(bundle.to_string()),
        );
    }
    let (signed, simulated) = split(entitlements, destination);
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
    let device = Some(text(DEVICE_KEY)).filter(|d| !d.is_empty());
    let profiles = load_profiles(folders);
    let picked = if text("provisioningStyle") == "manual" {
        let named = [
            text("provisioningProfileUUID"),
            text("provisioningProfileSpecifier"),
        ]
        .into_iter()
        .find(|n| !n.is_empty());
        let picked = named.and_then(|name| find_named(profiles, name));
        if let Some(profile) = &picked {
            check_named(profile, identity, team, bundle)?;
            if let Some(device) = device {
                check_device(profile, device)?;
            }
        }
        picked
    } else {
        let platform = platform(text("sdkRoot"));
        let picked = pick(profiles, identity, team, bundle, platform, device);
        if picked.is_none()
            && let Some(device) = device
            && let Some(other) = pick(
                load_profiles(folders),
                identity,
                team,
                bundle,
                platform,
                None,
            )
        {
            check_device(&other, device)?;
        }
        picked
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

/// That `profile` allows `entitlements`, as Xcode checks before it signs:
/// each one a profile has to grant is in it, with a value it allows.
/// `$(AppIdentifierPrefix)` and `$(TeamIdentifierPrefix)` are `team`'s; a
/// value naming another setting isn't checked.
pub(super) fn check_entitlements(
    profile: &Profile,
    entitlements: &[(String, Plist)],
    team: &str,
) -> Result<(), String> {
    for (key, value) in entitlements {
        if !provisioned(key) {
            continue;
        }
        let Some(granted) = plist::get(&profile.entitlements, key) else {
            return Err(format!(
                "Provisioning profile \"{}\" doesn't include the {key} entitlement.",
                profile.name
            ));
        };
        if !is_environment(key) && !allows(granted, &expand(value, team)) {
            return Err(format!(
                "Provisioning profile \"{}\" doesn't match the entitlements file's value for \
                 the {key} entitlement.",
                profile.name
            ));
        }
    }
    Ok(())
}

/// Whether a profile has to grant entitlement `key`.
fn provisioned(key: &str) -> bool {
    (key.starts_with("com.apple.developer.") && key != "com.apple.developer.team-identifier")
        || [
            "aps-environment",
            "com.apple.security.application-groups",
            "keychain-access-groups",
        ]
        .contains(&key)
}

/// Whether `key` is the push environment, whose value the profile sets
/// rather than checks: an archive signs for production whatever the
/// entitlements file says.
pub(super) fn is_environment(key: &str) -> bool {
    key == "aps-environment" || key == "com.apple.developer.aps-environment"
}

/// `value` with the team prefixes Swift Build fills in later.
fn expand(value: &Plist, team: &str) -> Plist {
    match value {
        Plist::String(text) => Plist::String(
            text.replace("$(AppIdentifierPrefix)", &format!("{team}."))
                .replace("$(TeamIdentifierPrefix)", &format!("{team}.")),
        ),
        Plist::Array(items) => Plist::Array(items.iter().map(|i| expand(i, team)).collect()),
        other => other.clone(),
    }
}

/// Whether a profile's `granted` value allows `wanted`: equal, or matched
/// by a `*` wildcard; for a list, each item by some granted one.
fn allows(granted: &Plist, wanted: &Plist) -> bool {
    match (granted, wanted) {
        (_, Plist::String(w)) if w.contains("$(") => true,
        (Plist::String(g), Plist::String(w)) => {
            g == w
                || g.strip_suffix('*')
                    .is_some_and(|prefix| w.starts_with(prefix))
        }
        (Plist::Array(gs), Plist::String(_)) => gs.iter().any(|g| allows(g, wanted)),
        (_, Plist::Array(ws)) => ws.iter().all(|w| allows(granted, w)),
        (Plist::Bool(g), Plist::Bool(w)) => *g || !*w,
        _ => true,
    }
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
pub(super) fn platform(sdk_root: &str) -> &'static str {
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
    /// The devices it lists, and whether it runs on any device, as
    /// enterprise and Developer ID profiles do.
    pub devices: Vec<String>,
    pub all_devices: bool,
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
            devices: strings("ProvisionedDevices"),
            all_devices: plist::get(&value, "ProvisionsAllDevices") == Some(&Plist::Bool(true)),
            expiration: plist::string(&value, "ExpirationDate")
                .unwrap_or_default()
                .to_string(),
            creation: plist::string(&value, "CreationDate")
                .unwrap_or_default()
                .to_string(),
        })
    }
}

impl Profile {
    /// The application identifier it allows, with its team: exact, or a
    /// prefix ending in `*`. A macOS profile names it with Apple's prefix.
    pub fn application_identifier(&self) -> Option<&str> {
        plist::string(&self.entitlements, "application-identifier")
            .or_else(|| plist::string(&self.entitlements, "com.apple.application-identifier"))
    }

    /// How closely it fits `application`, a `TEAM.bundle`: an exact
    /// identifier over the longest wildcard, `None` when it doesn't allow it.
    pub fn fit(&self, application: &str) -> Option<usize> {
        let pattern = self.application_identifier()?;
        if pattern == application {
            return Some(usize::MAX);
        }
        let prefix = pattern.strip_suffix('*')?;
        application.starts_with(prefix).then_some(prefix.len())
    }

    /// What its certificates sign.
    pub fn certificate_kinds(&self) -> Vec<Kind> {
        let mut kinds: Vec<Kind> = self
            .certificates
            .iter()
            .filter_map(|der| Certificate::parse(der).ok())
            .flat_map(|c| c.kinds())
            .collect();
        kinds.sort();
        kinds.dedup();
        kinds
    }

    /// Whether it's for `platform`, as [`platform`] names one. A watch app's
    /// profile may name iOS.
    pub fn is_for(&self, platform: &str) -> bool {
        self.platforms
            .iter()
            .any(|p| p == platform || (platform == "watchOS" && p == "iOS"))
    }

    /// Whether it lets an app run on `device`: a development or ad hoc
    /// profile lists the device or runs on any; the other kinds aren't
    /// bound to devices.
    pub fn runs_on(&self, device: &str) -> bool {
        !matches!(
            self.kind(),
            ProfileKind::Development | ProfileKind::AdHoc | ProfileKind::MacDevelopment
        ) || self.all_devices
            || self.devices.iter().any(|d| d.eq_ignore_ascii_case(device))
    }

    pub fn is_expired(&self, now: &str) -> bool {
        !self.expiration.is_empty() && self.expiration.as_str() <= now
    }

    /// What it's for, as Xcode tells one kind from another: by its
    /// certificates, its devices and whether it lets a debugger attach.
    pub fn kind(&self) -> ProfileKind {
        let kinds = self.certificate_kinds();
        let only_development = !kinds.is_empty() && kinds.iter().all(|k| *k == Kind::Development);
        if self.platforms.iter().any(|p| p == "OSX") {
            if kinds.contains(&Kind::DeveloperIdApplication) {
                ProfileKind::DeveloperId
            } else if !self.devices.is_empty() || only_development {
                ProfileKind::MacDevelopment
            } else {
                ProfileKind::MacAppStore
            }
        } else if only_development
            || plist::get(&self.entitlements, "get-task-allow") == Some(&Plist::Bool(true))
        {
            ProfileKind::Development
        } else if self.all_devices {
            ProfileKind::Enterprise
        } else if !self.devices.is_empty() {
            ProfileKind::AdHoc
        } else {
            ProfileKind::AppStore
        }
    }
}

/// What a profile is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ProfileKind {
    Development,
    AdHoc,
    AppStore,
    Enterprise,
    MacDevelopment,
    MacAppStore,
    DeveloperId,
}

impl ProfileKind {
    pub fn is_development(self) -> bool {
        matches!(self, Self::Development | Self::MacDevelopment)
    }

    /// The name to ask for one in an error.
    pub fn name(self) -> &'static str {
        match self {
            Self::Development => "development",
            Self::AdHoc => "ad hoc",
            Self::AppStore => "App Store",
            Self::Enterprise => "enterprise",
            Self::MacDevelopment => "Mac development",
            Self::MacAppStore => "Mac App Store",
            Self::DeveloperId => "Developer ID",
        }
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

pub(super) fn load_profiles(folders: &[PathBuf]) -> Vec<Profile> {
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

/// The best of `profiles` that `accept` takes: for `team`, `bundle` and
/// `platform`, holding `identity`'s certificate, unexpired; an exact
/// application identifier over a wildcard, then the newest.
fn rank(
    profiles: Vec<Profile>,
    identity: &Identity,
    team: &str,
    bundle: &str,
    platform: &str,
    accept: impl Fn(&Profile) -> bool,
) -> Option<Profile> {
    let now = now_iso8601();
    let application = format!("{team}.{bundle}");
    profiles
        .into_iter()
        .filter(|p| p.teams.iter().any(|t| t == team))
        .filter(|p| p.is_for(platform))
        .filter(|p| p.certificates.contains(&identity.certificate.der))
        .filter(|p| !p.is_expired(&now) && accept(p))
        .filter_map(|p| Some((p.fit(&application)?, p)))
        .max_by(|(a, p), (b, q)| a.cmp(b).then_with(|| p.creation.cmp(&q.creation)))
        .map(|(_, p)| p)
}

/// The profile automatic signing picks at build time: one of the
/// identity's kind, development or distribution, that runs on `device`
/// when the build has one.
fn pick(
    profiles: Vec<Profile>,
    identity: &Identity,
    team: &str,
    bundle: &str,
    platform: &str,
    device: Option<&str>,
) -> Option<Profile> {
    rank(profiles, identity, team, bundle, platform, |p| {
        p.kind().is_development() == identity.is_development()
            && device.is_none_or(|d| p.runs_on(d))
    })
}

/// That `profile` runs on `device`, as Xcode checks before a device build.
fn check_device(profile: &Profile, device: &str) -> Result<(), String> {
    if profile.runs_on(device) {
        return Ok(());
    }
    Err(format!(
        "Provisioning profile \"{}\" doesn't include the currently selected device \
         (identifier {device}).",
        profile.name
    ))
}

/// The profile of `kind` an export embeds for `bundle`.
pub(super) fn pick_for_export(
    profiles: Vec<Profile>,
    kind: ProfileKind,
    identity: &Identity,
    team: &str,
    bundle: &str,
    platform: &str,
) -> Option<Profile> {
    rank(profiles, identity, team, bundle, platform, |p| {
        p.kind() == kind
    })
}

/// The profile a name or UUID picks, as manual signing and ExportOptions'
/// `provisioningProfiles` name one: of several with one name, the newest
/// unexpired.
pub(super) fn find_named(profiles: Vec<Profile>, name: &str) -> Option<Profile> {
    let now = now_iso8601();
    profiles
        .into_iter()
        .filter(|p| p.uuid == name || p.name == name)
        .max_by(|p, q| {
            (!p.is_expired(&now))
                .cmp(&!q.is_expired(&now))
                .then_with(|| p.creation.cmp(&q.creation))
        })
}

/// That a profile named for a target fits it, as Xcode checks one: not
/// expired, allowing its application identifier and holding the signing
/// certificate.
pub(super) fn check_named(
    profile: &Profile,
    identity: &Identity,
    team: &str,
    bundle: &str,
) -> Result<(), String> {
    if profile.is_expired(&now_iso8601()) {
        return Err(format!(
            "Provisioning profile \"{}\" expired on {}.",
            profile.name, profile.expiration
        ));
    }
    if profile.fit(&format!("{team}.{bundle}")).is_none() {
        let app = profile.application_identifier().unwrap_or_default();
        let app = app.split_once('.').map_or(app, |(_, id)| id);
        return Err(format!(
            "Provisioning profile \"{}\" has app ID \"{app}\", which does not match the \
             bundle ID \"{bundle}\".",
            profile.name
        ));
    }
    if !profile.certificates.contains(&identity.certificate.der) {
        return Err(format!(
            "Provisioning profile \"{}\" doesn't include signing certificate \"{}\".",
            profile.name, identity.certificate.common_name
        ));
    }
    Ok(())
}
