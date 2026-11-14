//! Re-signing an archived app for distribution, as Xcode's export does:
//! each nested framework, library, extension and XPC service first, then
//! the app, with the identity the export method takes and, for each app and
//! extension, the profile of its kind embedded and the archived
//! entitlements moved to it. iOS and macOS apps alike (see [`walk`]).

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use super::blobs;
use super::bundle::{Flavor, Layout};
use super::identity::{Identity, IdentitySet, Kind};
use super::macho;
use super::plist::{self, Plist};
use super::provisioning::{self, Profile, ProfileKind};
use super::{Options, Signer};

/// How an export distributes the app: ExportOptions' `method`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Method {
    Debugging,
    AppStoreConnect,
    ReleaseTesting,
    Enterprise,
    DeveloperId,
}

impl Method {
    /// The certificate and profile kinds it signs with, on macOS or not.
    fn kinds(self, mac: bool) -> (Kind, ProfileKind) {
        match (self, mac) {
            (Self::Debugging, false) => (Kind::Development, ProfileKind::Development),
            (Self::Debugging, true) => (Kind::Development, ProfileKind::MacDevelopment),
            (Self::AppStoreConnect, false) => (Kind::IosDistribution, ProfileKind::AppStore),
            (Self::AppStoreConnect, true) => (Kind::MacAppDistribution, ProfileKind::MacAppStore),
            (Self::ReleaseTesting, _) => (Kind::IosDistribution, ProfileKind::AdHoc),
            (Self::Enterprise, _) => (Kind::IosDistribution, ProfileKind::Enterprise),
            (Self::DeveloperId, _) => (Kind::DeveloperIdApplication, ProfileKind::DeveloperId),
        }
    }
}

/// What to re-sign and how, from ExportOptions.
pub(crate) struct Request {
    /// The copy of the archived app to re-sign in place.
    pub app: PathBuf,
    pub method: Method,
    /// `teamID`, else the archived signature's team, else the identities'
    /// only one.
    pub team: Option<String>,
    /// `signingStyle` is manual: every app and extension must have its
    /// profile named.
    pub manual: bool,
    /// `signingCertificate`: a name or SHA-1 that narrows the identity.
    pub signing_certificate: Option<String>,
    /// `provisioningProfiles`: a profile name or UUID for each bundle
    /// identifier; any other bundle gets the newest profile of its kind.
    pub profiles: Vec<(String, String)>,
    /// `iCloudContainerEnvironment`, which is Production when left out of
    /// an export for distribution.
    pub icloud_environment: Option<String>,
}

/// One thing to sign.
pub(crate) struct Step {
    pub path: PathBuf,
    /// For an app or extension, the profile it embeds: none for a
    /// Developer ID app whose entitlements need none.
    pub(super) profile: Option<Profile>,
    /// For an app or extension, the entitlements it signs with.
    pub(super) entitlements: Option<Plist>,
    pub(super) bundle: Option<String>,
}

impl Step {
    /// The name of the profile it embeds.
    pub fn profile(&self) -> Option<&str> {
        self.profile.as_ref().map(|p| p.name.as_str())
    }

    /// The bundle identifier of an app or extension.
    pub fn bundle_id(&self) -> Option<&str> {
        self.bundle.as_deref()
    }
}

/// The identity and every step, innermost first.
pub(crate) struct Plan {
    pub identity: String,
    pub sha1: String,
    pub team: String,
    pub steps: Vec<Step>,
}

/// The plan for `request`, with the identities in `SWEETPAD_CODE_SIGN_P12`
/// and the profiles where the provisioning stand-in finds them.
pub(crate) fn plan(request: &Request) -> Result<Plan, String> {
    let set = load()?;
    plan_with(
        request,
        &set,
        &provisioning::load_profiles(&provisioning::profile_folders()),
    )
}

/// Re-sign as `plan` says.
pub(crate) fn apply(plan: &Plan) -> Result<(), String> {
    apply_with(plan, &load()?, SystemTime::now())
}

fn load() -> Result<IdentitySet, String> {
    IdentitySet::from_environment(true)?.ok_or_else(|| {
        format!(
            "exporting signs again, and there's no keychain here: point {} at the .p12 files \
             of your identities, their password in {}",
            super::identity::P12_VARIABLE,
            super::identity::PASSWORD_VARIABLE
        )
    })
}

pub(super) fn plan_with(
    request: &Request,
    set: &IdentitySet,
    profiles: &[Profile],
) -> Result<Plan, String> {
    let app = &request.app;
    let info = read_info(app)?;
    let mac = app.join("Contents").is_dir();
    let platform =
        provisioning::platform(plist::string(&info, "DTPlatformName").unwrap_or(if mac {
            "macosx"
        } else {
            "iphoneos"
        }));
    let archived = archived(app)?;
    let team = match (
        &request.team,
        team_of(archived.as_ref()),
        set.teams().as_slice(),
    ) {
        (Some(team), _, _) => team.clone(),
        (None, Some(team), _) => team,
        (None, None, [only]) => (*only).to_string(),
        (None, None, _) => {
            return Err(
                "the archive names no team and the identities belong to several: set teamID in \
                 ExportOptions.plist, or pass '--team'"
                    .to_string(),
            );
        }
    };
    let (kind, profile_kind) = request.method.kinds(mac);
    let identity = set.select(kind, Some(&team), request.signing_certificate.as_deref())?;
    let mut steps = Vec::new();
    for path in walk(app)? {
        if request.method == Method::DeveloperId {
            check_runtime(&path)?;
        }
        let mut bundle = None;
        let (mut profile, mut entitlements) = (None, None);
        if is_provisioned(&path) {
            let id = bundle_id(&path)?;
            let bundle = bundle.insert(id);
            let archived = archived_entitlements(&path)?;
            // A Developer ID app embeds a profile only for what Apple grants
            // through one, or when ExportOptions names one.
            let provisioned = request.method != Method::DeveloperId
                || restricted(&archived)
                || request.profiles.iter().any(|(id, _)| id == bundle);
            let mut signed = if provisioned {
                let chosen = choose_profile(
                    request,
                    profiles,
                    profile_kind,
                    identity,
                    &team,
                    bundle,
                    platform,
                )?;
                let signed = transform(archived, &chosen, &team, bundle, mac, request.method)?;
                profile = Some(chosen);
                signed
            } else {
                unprovisioned(archived, &team)
            };
            let icloud = "com.apple.developer.icloud-container-environment";
            if let Some(environment) = &request.icloud_environment
                && plist::get(&signed, icloud).is_some()
            {
                plist::set(&mut signed, icloud, Plist::String(environment.clone()));
            }
            entitlements = Some(signed);
        }
        steps.push(Step {
            path,
            profile,
            entitlements,
            bundle,
        });
    }
    if mac && request.method == Method::AppStoreConnect {
        let sandboxed = steps
            .last()
            .and_then(|step| step.entitlements.as_ref())
            .is_some_and(|entitlements| {
                matches!(
                    plist::get(entitlements, "com.apple.security.app-sandbox"),
                    Some(Plist::Bool(true))
                )
            });
        if !sandboxed {
            return Err(
                "the Mac App Store takes only sandboxed apps, and the app's entitlements don't \
                 turn on com.apple.security.app-sandbox: set ENABLE_APP_SANDBOX = YES and \
                 archive again"
                    .to_string(),
            );
        }
    }
    Ok(Plan {
        identity: identity.certificate.common_name.clone(),
        sha1: identity.certificate.sha1_hex(),
        team,
        steps,
    })
}

pub(super) fn apply_with(plan: &Plan, set: &IdentitySet, time: SystemTime) -> Result<(), String> {
    let identity: &Identity = set
        .by_sha1(&plan.sha1)
        .ok_or_else(|| format!("the identity '{}' is gone from the set", plan.identity))?;
    for step in &plan.steps {
        let mut options = Options {
            identity: plan.sha1.clone(),
            der_entitlements: true,
            path: Some(step.path.clone()),
            ..Options::default()
        };
        match &step.entitlements {
            Some(entitlements) => {
                let embedded = embedded_profile(&step.path);
                match &step.profile {
                    Some(profile) => {
                        std::fs::copy(&profile.path, &embedded)
                            .map_err(|e| format!("couldn't write {}: {e}", embedded.display()))?;
                    }
                    // A profile the archive embeds is the wrong kind.
                    None if embedded.exists() => std::fs::remove_file(&embedded)
                        .map_err(|e| format!("couldn't remove {}: {e}", embedded.display()))?,
                    None => {}
                }
                options.entitlements_data = Some(plist::to_xml(entitlements).into_bytes());
                options.preserve = vec!["flags".to_string()];
            }
            None => {
                options.preserve = ["identifier", "entitlements", "flags"]
                    .map(str::to_string)
                    .to_vec();
            }
        }
        let signer = Signer {
            options: &options,
            identity: Some(identity),
            time,
        };
        if step.path.is_dir() {
            signer.sign_bundle(&step.path)?;
        } else {
            signer.sign_file(&step.path, None)?;
        }
    }
    Ok(())
}

/// Where a bundle keeps code to sign before it, relative to the folder its
/// signature seals: an iOS app's frameworks, extensions, watch app and App
/// Clip, and a macOS bundle's frameworks, plug-ins, XPC services, login
/// items, system extensions and helpers.
const CODE_FOLDERS: &[&str] = &[
    "Frameworks",
    "SharedFrameworks",
    "PlugIns",
    "Extensions",
    "XPCServices",
    "Library/LoginItems",
    "Library/SystemExtensions",
    "Library/QuickLook",
    "Library/Spotlight",
    "Helpers",
    "Watch",
    "AppClips",
];

/// The code in `bundle` to sign, innermost first, the bundle last: each
/// nested bundle with what it holds before it, each library, and each
/// executable beside a macOS bundle's own.
fn walk(bundle: &Path) -> Result<Vec<PathBuf>, String> {
    let layout = Layout::of(bundle)?;
    let mut steps = Vec::new();
    let mut folders: Vec<&str> = CODE_FOLDERS.to_vec();
    if layout.flavor == Flavor::Mac {
        folders.push("MacOS");
    }
    for folder in folders {
        let dir = layout.root.join(folder);
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        let mut paths: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
        paths.sort();
        for path in paths {
            let metadata = std::fs::symlink_metadata(&path)
                .map_err(|e| format!("couldn't read {}: {e}", path.display()))?;
            if metadata.is_symlink() {
                continue;
            }
            let is_main = layout
                .executable
                .as_ref()
                .is_some_and(|e| layout.root.join(e) == path);
            if metadata.is_dir() {
                // A folder that isn't a bundle holds no code to sign.
                if path.extension().is_some() && Layout::of(&path).is_ok() {
                    steps.extend(walk(&path)?);
                }
            } else if !is_main && is_mach_o(&path) {
                steps.push(path);
            }
        }
    }
    steps.push(bundle.to_path_buf());
    Ok(steps)
}

/// Whether the file at `path` is Mach-O code.
fn is_mach_o(path: &Path) -> bool {
    use std::io::Read as _;
    let mut magic = [0u8; 4];
    std::fs::File::open(path)
        .and_then(|mut f| f.read_exact(&mut magic))
        .is_ok()
        && matches!(
            magic,
            [0xcf | 0xce, 0xfa, 0xed, 0xfe] | [0xca, 0xfe, 0xba, 0xbe]
        )
}

/// Whether `path` is an app or extension, which embeds a profile and signs
/// with entitlements, rather than a framework, library or XPC service.
fn is_provisioned(path: &Path) -> bool {
    path.extension().is_some_and(|e| e == "app" || e == "appex")
}

fn read_info(bundle: &Path) -> Result<Plist, String> {
    Layout::of(bundle).map(|layout| layout.info)
}

fn bundle_id(bundle: &Path) -> Result<String, String> {
    let layout = Layout::of(bundle)?;
    plist::string(&layout.info, "CFBundleIdentifier")
        .map(str::to_string)
        .ok_or_else(|| format!("{} names no CFBundleIdentifier", layout.info_path.display()))
}

/// The bundle's main executable.
fn executable(bundle: &Path) -> Result<PathBuf, String> {
    let layout = Layout::of(bundle)?;
    let name = layout
        .executable
        .ok_or_else(|| format!("{} names no CFBundleExecutable", layout.info_path.display()))?;
    Ok(layout.root.join(name))
}

/// The archived signature of a bundle's executable, read from its first
/// slice.
fn archived(bundle: &Path) -> Result<Option<blobs::Existing>, String> {
    let path = executable(bundle)?;
    let data =
        std::fs::read(&path).map_err(|e| format!("couldn't read {}: {e}", path.display()))?;
    let slices = macho::split(&data).map_err(|e| format!("{}: {e}", path.display()))?;
    let Some(slice) = slices.first() else {
        return Ok(None);
    };
    let layout =
        macho::Layout::parse(&slice.data).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(layout.signature(&slice.data).and_then(blobs::read_existing))
}

fn archived_entitlements(bundle: &Path) -> Result<Plist, String> {
    match archived(bundle)?.and_then(|e| e.entitlements) {
        Some(xml) => plist::parse(&xml),
        None => Ok(Plist::Dict(Vec::new())),
    }
}

/// The team an archived signature's entitlements name.
fn team_of(existing: Option<&blobs::Existing>) -> Option<String> {
    let xml = existing?.entitlements.as_ref()?;
    let entitlements = plist::parse(xml).ok()?;
    archived_team(&entitlements)
}

fn archived_team(entitlements: &Plist) -> Option<String> {
    if let Some(team) = plist::string(entitlements, "com.apple.developer.team-identifier") {
        return Some(team.to_string());
    }
    ["application-identifier", "com.apple.application-identifier"]
        .iter()
        .find_map(|key| plist::string(entitlements, key))
        .and_then(|id| id.split_once('.'))
        .map(|(team, _)| team.to_string())
}

fn choose_profile(
    request: &Request,
    profiles: &[Profile],
    kind: ProfileKind,
    identity: &Identity,
    team: &str,
    bundle: &str,
    platform: &str,
) -> Result<Profile, String> {
    if let Some((_, name)) = request.profiles.iter().find(|(id, _)| id == bundle) {
        let profile = provisioning::find_named(profiles.to_vec(), name).ok_or_else(|| {
            format!("No profile named \"{name}\" was found for '{bundle}' (provisioningProfiles)")
        })?;
        provisioning::check_named(&profile, identity, team, bundle)?;
        if profile.kind() != kind {
            return Err(format!(
                "Provisioning profile \"{}\" is {} {} profile, and this export needs {} {} one",
                profile.name,
                article(profile.kind().name()),
                profile.kind().name(),
                article(kind.name()),
                kind.name()
            ));
        }
        return Ok(profile);
    }
    if request.manual {
        let name = request
            .app
            .file_name()
            .map(|n| n.to_string_lossy())
            .unwrap_or_default();
        return Err(format!(
            "'{bundle}' in \"{name}\" requires a provisioning profile: manual signing names \
             one for each app and extension in provisioningProfiles"
        ));
    }
    provisioning::pick_for_export(profiles.to_vec(), kind, identity, team, bundle, platform)
        .ok_or_else(|| {
            format!(
                "No {} profile for '{bundle}' was found: none allows it for team {team} and \
                 '{}' (point {} at the folder that has yours, or name one in \
                 provisioningProfiles)",
                kind.name(),
                identity.certificate.common_name,
                provisioning::PROFILES_VARIABLE
            )
        })
}

fn article(word: &str) -> &'static str {
    if word.starts_with(['a', 'e', 'i', 'o', 'u', 'A', 'E', 'I', 'O', 'U']) {
        "an"
    } else {
        "a"
    }
}

/// Where a bundle embeds its profile.
fn embedded_profile(bundle: &Path) -> PathBuf {
    if bundle.join("Contents").is_dir() {
        bundle.join("Contents/embedded.provisionprofile")
    } else {
        bundle.join("embedded.mobileprovision")
    }
}

/// The entitlements an export signs `bundle` with: the archived ones, moved
/// to `team` and set from `profile` as Xcode's export sets them, then
/// checked against it.
pub(super) fn transform(
    archived: Plist,
    profile: &Profile,
    team: &str,
    bundle: &str,
    mac: bool,
    method: Method,
) -> Result<Plist, String> {
    let mut entitlements = match archived {
        Plist::Dict(_) => archived,
        _ => Plist::Dict(Vec::new()),
    };
    if let Some(old) = archived_team(&entitlements).filter(|old| old != team) {
        entitlements = move_team(entitlements, &format!("{old}."), &format!("{team}."));
    }
    let application = if mac {
        "com.apple.application-identifier"
    } else {
        "application-identifier"
    };
    plist::set(
        &mut entitlements,
        application,
        Plist::String(format!("{team}.{bundle}")),
    );
    plist::set(
        &mut entitlements,
        "com.apple.developer.team-identifier",
        Plist::String(team.to_string()),
    );
    let granted = |key: &str| plist::get(&profile.entitlements, key).cloned();
    if mac {
        let key = "com.apple.security.get-task-allow";
        match granted(key).filter(|_| method == Method::Debugging) {
            Some(value) => plist::set(&mut entitlements, key, value),
            None => plist::remove(&mut entitlements, key),
        }
    } else {
        let allow = granted("get-task-allow").unwrap_or(Plist::Bool(false));
        plist::set(&mut entitlements, "get-task-allow", allow);
    }
    for key in ["aps-environment", "com.apple.developer.aps-environment"] {
        if plist::get(&entitlements, key).is_some()
            && let Some(value) = granted(key)
        {
            plist::set(&mut entitlements, key, value);
        }
    }
    if let Some(value) = granted("beta-reports-active") {
        plist::set(&mut entitlements, "beta-reports-active", value);
    }
    let icloud = "com.apple.developer.icloud-container-environment";
    if method != Method::Debugging && plist::get(&entitlements, icloud).is_some() {
        plist::set(
            &mut entitlements,
            icloud,
            Plist::String("Production".to_string()),
        );
    }
    if let Plist::Dict(entries) = &entitlements {
        provisioning::check_entitlements(profile, entries, team)?;
    }
    Ok(entitlements)
}

/// Whether a Developer ID app's entitlements need a profile: a capability
/// Apple grants through one, as against the sandbox and hardened runtime
/// exceptions any app may claim. An app that claims one without a profile
/// doesn't launch.
pub(super) fn restricted(entitlements: &Plist) -> bool {
    let Plist::Dict(entries) = entitlements else {
        return false;
    };
    entries.iter().any(|(key, value)| match key.as_str() {
        "com.apple.application-identifier"
        | "com.apple.developer.team-identifier"
        | "com.apple.security.get-task-allow" => false,
        "keychain-access-groups" => true,
        // Team-prefixed groups need none; `group.` ones do.
        "com.apple.security.application-groups" => match value {
            Plist::Array(groups) => groups
                .iter()
                .any(|g| matches!(g, Plist::String(g) if g.starts_with("group."))),
            _ => false,
        },
        key => key.starts_with("com.apple.developer."),
    })
}

/// The entitlements a Developer ID app without a profile signs with: the
/// archived ones moved to `team`, without the debugger's, and without the
/// application identifier only a profile can grant.
fn unprovisioned(archived: Plist, team: &str) -> Plist {
    let mut entitlements = match archived {
        Plist::Dict(_) => archived,
        _ => Plist::Dict(Vec::new()),
    };
    if let Some(old) = archived_team(&entitlements).filter(|old| old != team) {
        entitlements = move_team(entitlements, &format!("{old}."), &format!("{team}."));
    }
    for key in [
        "com.apple.security.get-task-allow",
        "com.apple.application-identifier",
        "com.apple.developer.team-identifier",
    ] {
        plist::remove(&mut entitlements, key);
    }
    entitlements
}

/// Fail when the program at or in `path` lacks the hardened runtime, which
/// notarization needs of every program.
fn check_runtime(path: &Path) -> Result<(), String> {
    let program = if path.is_dir() {
        executable(path)?
    } else {
        path.to_path_buf()
    };
    let data =
        std::fs::read(&program).map_err(|e| format!("couldn't read {}: {e}", program.display()))?;
    let slices = macho::split(&data).map_err(|e| format!("{}: {e}", program.display()))?;
    for slice in &slices {
        let layout =
            macho::Layout::parse(&slice.data).map_err(|e| format!("{}: {e}", program.display()))?;
        if layout.filetype != macho::MH_EXECUTE {
            return Ok(());
        }
        let flags = layout
            .signature(&slice.data)
            .and_then(blobs::read_existing)
            .map_or(0, |existing| existing.flags);
        if flags & blobs::FLAG_RUNTIME == 0 {
            return Err(format!(
                "{} isn't signed with the hardened runtime, which notarization needs for \
                 Developer ID: set ENABLE_HARDENED_RUNTIME = YES and archive again",
                program.display()
            ));
        }
    }
    Ok(())
}

/// `value` with each string that starts with `old`, a team prefix, moved
/// to `new`.
fn move_team(value: Plist, old: &str, new: &str) -> Plist {
    match value {
        Plist::String(text) => match text.strip_prefix(old) {
            Some(rest) => Plist::String(format!("{new}{rest}")),
            None => Plist::String(text),
        },
        Plist::Array(items) => {
            Plist::Array(items.into_iter().map(|i| move_team(i, old, new)).collect())
        }
        Plist::Dict(entries) => Plist::Dict(
            entries
                .into_iter()
                .map(|(k, v)| (k, move_team(v, old, new)))
                .collect(),
        ),
        other => other,
    }
}
