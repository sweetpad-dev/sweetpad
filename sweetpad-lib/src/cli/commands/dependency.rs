//! `sweetpad dependency …` (alias `dep`) — view and manage Swift Package
//! Manager dependencies.
//!
//! Works on all three container kinds. For an `.xcodeproj`/`.xcworkspace` there
//! is no Apple CLI to add/remove SPM packages, so we edit `project.pbxproj`
//! directly through [`crate::spm_pbxproj`]; for a `Package.swift` we drive the
//! Swift 6 `swift package add-dependency`/`add-target-dependency`/`resolve`
//! commands. `list` shows each declared package's requested requirement next to
//! its locked version from `Package.resolved`.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use clap::{Args, Subcommand};

use crate::cli::output::Output;
use crate::cli::resolve::{self, Container};
use crate::cli::{
    CliError, CliResult, CommandResult, Context, ErrorKind, Render, Rendered, buildlog, process,
    swiftpm, xcodebuild,
};
use crate::pbxproj::Value;
use crate::spm_pbxproj::{self, DeclaredPackage, RequirementSpec};

#[derive(Debug, Subcommand)]
pub enum Action {
    /// List declared dependencies and their resolved (locked) versions.
    List {
        /// Also list resolved pins that aren't directly declared (transitive).
        #[arg(long)]
        transitive: bool,
    },
    /// Add a package, resolve it, then link a product to a target.
    Add(AddArgs),
    /// Remove a whole package, or unlink one product from one target.
    Remove(RemoveArgs),
    /// Update resolved versions, or change a package's requirement (bump,
    /// pin, or downgrade) and re-resolve.
    Update(UpdateArgs),
    /// Resolve dependencies into `Package.resolved`.
    Resolve,
}

/// Flags for `dependency update`.
#[derive(Debug, Args)]
pub struct UpdateArgs {
    /// Package to update (identity/URL/path/name). Omitted → update everything.
    pub package: Option<String>,

    #[command(flatten)]
    pub requirement: RequirementArgs,

    /// When changing a requirement, edit it only; don't re-resolve.
    #[arg(long)]
    pub no_resolve: bool,
}

/// Flags for `dependency add`.
#[derive(Debug, Args)]
pub struct AddArgs {
    /// Remote git URL, or a local directory path containing `Package.swift`.
    pub url: String,

    #[command(flatten)]
    pub requirement: RequirementArgs,

    /// Product(s) to link (repeatable). Omitted → you are prompted after resolve.
    #[arg(long = "product")]
    pub products: Vec<String>,

    /// Target(s) to link the product(s) into (repeatable). Omitted → prompted.
    #[arg(long = "target")]
    pub targets: Vec<String>,

    /// Skip the resolve that updates `Package.resolved` after mutating.
    #[arg(long)]
    pub no_resolve: bool,
}

/// The version requirement for `add`, mirroring `swift package add-dependency`.
/// All flags are optional at parse time — a requirement is required only for a
/// *remote* package (a local path has no version), so the "exactly one of these"
/// rule is enforced contextually in [`requirement_spec`] rather than by a clap
/// group that would also force one on a local add.
#[derive(Debug, Args)]
pub struct RequirementArgs {
    /// `from: "x.y.z"` — up to the next major version.
    #[arg(long, value_name = "VERSION")]
    pub from: Option<String>,

    /// `exact: "x.y.z"`.
    #[arg(long, value_name = "VERSION")]
    pub exact: Option<String>,

    /// `.upToNextMinor(from: "x.y.z")`.
    #[arg(long = "up-to-next-minor-from", value_name = "VERSION")]
    pub up_to_next_minor_from: Option<String>,

    /// `branch: "name"`.
    #[arg(long, value_name = "BRANCH")]
    pub branch: Option<String>,

    /// `revision: "sha"`.
    #[arg(long, value_name = "SHA")]
    pub revision: Option<String>,

    /// Upper bound of a half-open `from ..< to` range. Requires `--from`.
    #[arg(long, value_name = "VERSION", requires = "from")]
    pub to: Option<String>,
}

impl RequirementArgs {
    /// Whether no requirement flag was given (so `update` just re-resolves).
    fn is_empty(&self) -> bool {
        self.from.is_none()
            && self.exact.is_none()
            && self.up_to_next_minor_from.is_none()
            && self.branch.is_none()
            && self.revision.is_none()
            && self.to.is_none()
    }
}

/// Flags for `dependency remove`.
#[derive(Debug, Args)]
pub struct RemoveArgs {
    /// Package to remove: identity, repository URL, local path, or its name.
    pub package: String,

    /// Narrow to unlinking this product only (keep the package reference).
    #[arg(long = "product")]
    pub product: Option<String>,

    /// Narrow to this target only.
    #[arg(long = "target")]
    pub target: Option<String>,
}

pub fn run(ctx: &mut Context, action: &Action) -> CommandResult {
    match action {
        Action::List { transitive } => list(ctx, *transitive),
        Action::Add(args) => add(ctx, args).map(|()| Rendered::Streamed),
        Action::Remove(args) => remove(ctx, args).map(|()| Rendered::Streamed),
        Action::Update(args) => update(ctx, args).map(|()| Rendered::Streamed),
        Action::Resolve => resolve_action(ctx).map(|()| Rendered::Streamed),
    }
}

// ---------------------------------------------------------------------------
// list
// ---------------------------------------------------------------------------

fn list(ctx: &mut Context, transitive: bool) -> CommandResult {
    let container = resolve::container(ctx)?;
    let pins = read_resolved(&resolved_path(&container));
    let direct = gather_direct(&container, &pins)?;

    let declared: HashSet<&str> = direct.iter().map(|e| e.identity.as_str()).collect();
    let mut transitive_entries: Vec<PinEntry> = Vec::new();
    if transitive {
        for (identity, pin) in &pins {
            if !declared.contains(identity.as_str()) {
                transitive_entries.push(PinEntry {
                    identity: identity.clone(),
                    location: pin.location.clone(),
                    locked: Some(pin.display()),
                });
            }
        }
        transitive_entries.sort_by(|a, b| a.identity.cmp(&b.identity));
    }

    Ok(Rendered::data(DependencyList {
        container_kind: kind_str(&container),
        direct,
        transitive: transitive_entries,
    }))
}

/// The direct (declared) dependencies of a container, correlated with their pins.
fn gather_direct(
    container: &Container,
    pins: &HashMap<String, Pin>,
) -> Result<Vec<PackageEntry>, CliError> {
    match container {
        Container::Project(p) => Ok(packages_to_entries(&read_project(p)?, pins)),
        Container::Workspace(p) => {
            let ws = crate::workspace::open(p).map_err(|e| {
                CliError::new(format!("failed to read workspace {}: {e}", p.display()))
            })?;
            let mut entries = Vec::new();
            for member in &ws.project_refs {
                entries.extend(packages_to_entries(&read_project(member)?, pins));
            }
            Ok(entries)
        }
        Container::SwiftPackage(_) => {
            let manifest = swiftpm::manifest(container)?;
            Ok(manifest
                .declared_dependencies()
                .iter()
                .map(|d| dep_to_entry(d, pins))
                .collect())
        }
    }
}

fn read_project(xcodeproj: &Path) -> Result<Vec<DeclaredPackage>, CliError> {
    let root = crate::project::parse_pbxproj(xcodeproj)
        .map_err(|e| CliError::new(format!("failed to read {}: {e}", xcodeproj.display())))?;
    Ok(spm_pbxproj::list_packages(&root))
}

fn packages_to_entries(pkgs: &[DeclaredPackage], pins: &HashMap<String, Pin>) -> Vec<PackageEntry> {
    pkgs.iter().map(|p| package_to_entry(p, pins)).collect()
}

fn package_to_entry(pkg: &DeclaredPackage, pins: &HashMap<String, Pin>) -> PackageEntry {
    let requirement = match &pkg.requirement {
        Some(r) => r.display(),
        None if pkg.kind.is_remote() => "—".to_string(),
        None => "local".to_string(),
    };
    PackageEntry {
        identity: pkg.identity.clone(),
        display: pkg.kind.display().to_string(),
        remote: pkg.kind.is_remote(),
        requirement,
        locked: pins.get(&pkg.identity).map(Pin::display),
        links: pkg
            .products
            .iter()
            .map(|l| (l.product.clone(), l.target.clone()))
            .collect(),
    }
}

fn dep_to_entry(dep: &swiftpm::DeclaredDep, pins: &HashMap<String, Pin>) -> PackageEntry {
    let identity = dep.identity.to_ascii_lowercase();
    PackageEntry {
        locked: pins.get(&identity).map(Pin::display),
        identity,
        display: dep.location.clone(),
        remote: dep.remote,
        requirement: dep.requirement.clone(),
        links: Vec::new(),
    }
}

/// One declared package in the list payload: its requested requirement, its
/// locked version, and the `(product, target)` links it participates in.
struct PackageEntry {
    identity: String,
    display: String,
    remote: bool,
    requirement: String,
    locked: Option<String>,
    links: Vec<(String, String)>,
}

/// A resolved-only pin (transitive dependency) in the list payload.
struct PinEntry {
    identity: String,
    location: Option<String>,
    locked: Option<String>,
}

struct DependencyList {
    container_kind: &'static str,
    direct: Vec<PackageEntry>,
    transitive: Vec<PinEntry>,
}

impl Render for DependencyList {
    fn human(&self, out: &Output) {
        if self.direct.is_empty() && self.transitive.is_empty() {
            out.note("no package dependencies");
            return;
        }
        for (i, p) in self.direct.iter().enumerate() {
            if i > 0 {
                out.line("");
            }
            out.line(&format!(
                "{} ({})",
                p.identity,
                if p.remote { "remote" } else { "local" }
            ));
            out.line(&format!("  {}", p.display));
            out.line(&format!("  requested: {}", p.requirement));
            out.line(&format!(
                "  locked:    {}",
                p.locked.as_deref().unwrap_or("—")
            ));
            for (product, target) in &p.links {
                out.line(&format!("  link:      {product} → {target}"));
            }
        }
        if !self.transitive.is_empty() {
            out.line("");
            out.line("transitive:");
            for t in &self.transitive {
                out.line(&format!(
                    "  {} {}",
                    t.identity,
                    t.locked.as_deref().unwrap_or("—")
                ));
            }
        }
    }

    fn json(&self) -> serde_json::Value {
        let direct: Vec<serde_json::Value> = self
            .direct
            .iter()
            .map(|p| {
                serde_json::json!({
                    "identity": p.identity,
                    "location": p.display,
                    "kind": if p.remote { "remote" } else { "local" },
                    "requirement": p.requirement,
                    "resolvedVersion": p.locked,
                    "links": p.links.iter()
                        .map(|(product, target)| serde_json::json!({ "product": product, "target": target }))
                        .collect::<Vec<_>>(),
                })
            })
            .collect();
        let transitive: Vec<serde_json::Value> = self
            .transitive
            .iter()
            .map(|t| {
                serde_json::json!({
                    "identity": t.identity,
                    "location": t.location,
                    "resolvedVersion": t.locked,
                })
            })
            .collect();
        serde_json::json!({
            "containerKind": self.container_kind,
            "direct": direct,
            "transitive": transitive,
        })
    }
}

// ---------------------------------------------------------------------------
// add
// ---------------------------------------------------------------------------

fn add(ctx: &mut Context, args: &AddArgs) -> CliResult {
    let container = resolve::container(ctx)?;
    match container {
        Container::SwiftPackage(_) => add_to_package(ctx, &container, args),
        Container::Project(_) | Container::Workspace(_) => add_to_xcode(ctx, &container, args),
    }
}

fn add_to_xcode(ctx: &mut Context, container: &Container, args: &AddArgs) -> CliResult {
    let xcodeproj = pick_xcodeproj(ctx, container, None)?;
    let remote = looks_remote(&args.url);

    // Validate the requirement (remote only) before anything else, so a bad
    // requirement is reported ahead of the prompt/mutation.
    let spec = if remote {
        Some(requirement_spec(&args.requirement)?)
    } else {
        None
    };

    // Fail before mutating anything if we can neither be told nor prompt for the
    // products/targets to link — otherwise we'd leave a dangling package ref.
    if (args.products.is_empty() || args.targets.is_empty()) && !ctx.out.is_interactive() {
        return Err(CliError::new(
            "non-interactive: pass --product and --target to add without prompting",
        ));
    }

    // 1. Add the package reference (only) and write it, so resolution can fetch.
    let mut root = parse_owned(&xcodeproj)?;
    let ref_guid = if let Some(spec) = &spec {
        spm_pbxproj::add_remote_dependency(&mut root, &args.url, spec).map_err(CliError::new)?
    } else {
        let rel = local_relative_path(&xcodeproj, &args.url)?;
        spm_pbxproj::add_local_dependency(&mut root, &rel).map_err(CliError::new)?
    };
    write_pbxproj(&xcodeproj, &root)?;
    ctx.out.note(&format!("added package {}", args.url));

    // 2. Settle the products and targets to link (resolve-then-prompt).
    let products = resolve_products(ctx, container, &args.url, remote, &args.products)?;
    let targets = settle_targets(ctx, &xcodeproj, &args.targets)?;

    // 3. Link each product into each target and write.
    for product in &products {
        for target in &targets {
            spm_pbxproj::link_product(&mut root, &ref_guid, product, target)
                .map_err(CliError::new)?;
        }
    }
    write_pbxproj(&xcodeproj, &root)?;

    // 4. Ensure Package.resolved is current. Discovering a remote package's
    //    products already resolved (and wrote the lockfile), so only resolve
    //    here when discovery didn't — and unless told to skip.
    let resolved_in_discovery = remote && args.products.is_empty();
    if !args.no_resolve && !resolved_in_discovery {
        resolve_packages(container, None, &ctx.out, false)?;
    }

    report_added(ctx, &args.url, &products, &targets);
    Ok(())
}

fn add_to_package(ctx: &mut Context, container: &Container, args: &AddArgs) -> CliResult {
    let remote = looks_remote(&args.url);

    // `swift package add-dependency` is a Swift 6 feature; fail clearly on older
    // toolchains instead of surfacing a raw "unknown subcommand" exit.
    if swiftpm::swift_major_version().is_some_and(|v| v < 6) {
        return Err(CliError::new(
            "adding a dependency to a Package.swift needs Swift 6+ (swift package add-dependency); edit Package.swift and run `dep resolve` instead",
        ));
    }

    // 1. Add the dependency to the manifest.
    let requirement = if remote {
        swift_flags_for(&requirement_spec(&args.requirement)?)
    } else {
        Vec::new()
    };
    swiftpm::add_dependency(container, &args.url, &requirement)?;
    ctx.out.note(&format!("added package {}", args.url));

    // 2. Resolve to fetch the package (needed to discover its products).
    let need_discovery = args.products.is_empty();
    if !args.no_resolve || need_discovery {
        ctx.out.step("Resolving package dependencies", || {
            swiftpm::resolve(container, ctx.out.is_json())
        })?;
    }

    // 3. Settle products + targets and link each pair.
    let (package_name, products) = if need_discovery {
        let (name, available) = package_products(container, &args.url)?;
        (name, choose("product", &available, &args.products, ctx)?)
    } else {
        (package_display_name(&args.url), args.products.clone())
    };
    let target_names = swiftpm::manifest(container)?
        .targets
        .iter()
        .map(|t| t.name.clone())
        .collect::<Vec<_>>();
    let targets = choose("target", &target_names, &args.targets, ctx)?;

    for product in &products {
        for target in &targets {
            swiftpm::add_target_dependency(container, product, target, &package_name)?;
        }
    }

    report_added(ctx, &args.url, &products, &targets);
    Ok(())
}

/// Settle the product list for an xcodeproj add: explicit `--product` flags, or
/// discover the package's real products (resolving first) and prompt.
fn resolve_products(
    ctx: &mut Context,
    container: &Container,
    url: &str,
    remote: bool,
    flags: &[String],
) -> Result<Vec<String>, CliError> {
    if !flags.is_empty() {
        return Ok(flags.to_vec());
    }
    if !ctx.out.is_interactive() {
        return Err(CliError::new(
            "non-interactive: pass --product (and --target) to add without prompting",
        ));
    }
    let available = discover_products(ctx, container, url, remote)?;
    if available.is_empty() {
        return Err(CliError::new("the package declares no products to link"));
    }
    choose("product", &available, &[], ctx)
}

/// Settle the targets to link into: explicit `--target` flags, or prompt over
/// the project's targets.
fn settle_targets(
    ctx: &mut Context,
    xcodeproj: &Path,
    flags: &[String],
) -> Result<Vec<String>, CliError> {
    if !flags.is_empty() {
        return Ok(flags.to_vec());
    }
    if !ctx.out.is_interactive() {
        return Err(CliError::new(
            "non-interactive: pass --target to add without prompting",
        ));
    }
    let proj = crate::project::open(xcodeproj)
        .map_err(|e| CliError::new(format!("failed to read {}: {e}", xcodeproj.display())))?;
    let names: Vec<String> = proj.targets.iter().map(|t| t.name.clone()).collect();
    choose("target", &names, &[], ctx)
}

/// Resolve a remote package into a known clone dir and read its products from
/// the checkout; for a local package, read products straight from its directory.
fn discover_products(
    ctx: &mut Context,
    container: &Container,
    url: &str,
    remote: bool,
) -> Result<Vec<String>, CliError> {
    if !remote {
        let manifest = swiftpm::manifest_at(Path::new(url))?;
        return Ok(product_names(&manifest));
    }
    let clone = clone_dir();
    ctx.out.step("Resolving package dependencies", || {
        resolve_packages(container, Some(&clone), &ctx.out, true)
    })?;
    let checkout = resolve_checkout(&clone, url).ok_or_else(|| {
        CliError::new("could not locate the resolved package checkout to read its products")
    })?;
    Ok(product_names(&swiftpm::manifest_at(&checkout)?))
}

/// For a `Package.swift` add: resolve, then read the just-added package's name
/// and products from its `.build` checkout.
fn package_products(container: &Container, url: &str) -> Result<(String, Vec<String>), CliError> {
    let pkg_dir = swiftpm::package_dir(container).unwrap_or_else(|| PathBuf::from("."));
    let checkout = resolve_checkout(&pkg_dir.join(".build"), url).ok_or_else(|| {
        CliError::new("could not locate the resolved package checkout to read its products")
    })?;
    let manifest = swiftpm::manifest_at(&checkout)?;
    Ok((manifest.name.clone(), product_names(&manifest)))
}

// ---------------------------------------------------------------------------
// remove
// ---------------------------------------------------------------------------

fn remove(ctx: &mut Context, args: &RemoveArgs) -> CliResult {
    let container = resolve::container(ctx)?;
    match container {
        Container::SwiftPackage(_) => Err(CliError::new(
            "removing dependencies from a Package.swift isn't supported (edit the manifest directly)",
        )),
        Container::Project(_) | Container::Workspace(_) => remove_from_xcode(ctx, &container, args),
    }
}

fn remove_from_xcode(ctx: &mut Context, container: &Container, args: &RemoveArgs) -> CliResult {
    let xcodeproj = pick_xcodeproj(ctx, container, Some(&args.package))?;
    let mut root = parse_owned(&xcodeproj)?;
    let ref_guid = find_package_or_hint(&root, container, &args.package, &xcodeproj)?;

    if args.product.is_none() && args.target.is_none() {
        // For a local package, Xcode may omit the product->package back-ref, so
        // pass the local package's declared product names to clean those up too.
        let orphans = local_product_names(&root, &ref_guid, &xcodeproj);
        spm_pbxproj::remove_package(&mut root, &ref_guid, &orphans).map_err(CliError::new)?;
        write_pbxproj(&xcodeproj, &root)?;
        remove_pin(container, &args.package);
        report_removed(ctx, &args.package, None);
    } else {
        let unlinked = spm_pbxproj::unlink(
            &mut root,
            &ref_guid,
            args.product.as_deref(),
            args.target.as_deref(),
        )
        .map_err(CliError::new)?;
        if unlinked.is_empty() {
            return Err(CliError::new(
                "no matching product/target link found to unlink",
            ));
        }
        write_pbxproj(&xcodeproj, &root)?;
        report_removed(ctx, &args.package, Some(&unlinked));
    }
    Ok(())
}

/// Drop a package's pin from `Package.resolved` and re-serialize. Best-effort:
/// a missing or unreadable lockfile is fine (the next resolve regenerates it).
fn remove_pin(container: &Container, query: &str) {
    let path = resolved_path(container);
    let Ok(text) = std::fs::read_to_string(&path) else {
        return;
    };
    let Ok(mut json) = serde_json::from_str::<serde_json::Value>(&text) else {
        return;
    };
    let id = spm_pbxproj::identity_from_url(query);
    if let Some(pins) = json
        .get_mut("pins")
        .and_then(serde_json::Value::as_array_mut)
    {
        pins.retain(|p| {
            p.get("identity")
                .and_then(serde_json::Value::as_str)
                .map(str::to_ascii_lowercase)
                != Some(id.clone())
        });
    }
    let _ = std::fs::write(&path, crate::spm_resolved::serialize(&json));
}

/// The product names a local package declares — passed to `remove_package` so
/// products Xcode wrote without a `package` back-ref are cleaned up. Empty for a
/// remote package or when the local manifest can't be read.
fn local_product_names(root: &Value, ref_guid: &str, xcodeproj: &Path) -> Vec<String> {
    let Some(pkg) = spm_pbxproj::list_packages(root)
        .into_iter()
        .find(|p| p.guid == ref_guid)
    else {
        return Vec::new();
    };
    let spm_pbxproj::PackageKind::Local { relative_path } = pkg.kind else {
        return Vec::new();
    };
    let dir = xcodeproj
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(&relative_path);
    swiftpm::manifest_at(&dir)
        .ok()
        .map(|m| product_names(&m))
        .unwrap_or_default()
}

/// Locate a package by query, or fail with a transitive-dependency hint when the
/// name is a resolved-but-not-declared pin.
fn find_package_or_hint(
    root: &Value,
    container: &Container,
    query: &str,
    xcodeproj: &Path,
) -> Result<String, CliError> {
    if let Some(guid) = spm_pbxproj::find_package(root, query) {
        return Ok(guid);
    }
    Err(transitive_hint(container, query).unwrap_or_else(|| {
        CliError::new(format!(
            "no package matching `{query}` in {}",
            xcodeproj.display()
        ))
        .kind(ErrorKind::TargetResolution)
    }))
}

/// A friendlier error when `query` names a transitive dependency (present in
/// `Package.resolved` but not directly declared) — you can't manage it directly.
fn transitive_hint(container: &Container, query: &str) -> Option<CliError> {
    let pins = read_resolved(&resolved_path(container));
    let id = spm_pbxproj::identity_from_url(query);
    pins.contains_key(&id).then(|| {
        CliError::new(format!(
            "`{query}` is a transitive dependency (resolved but not directly declared); it's pulled in by one of your direct packages — change that package's requirement or remove it instead"
        ))
        .kind(ErrorKind::TargetResolution)
    })
}

// ---------------------------------------------------------------------------
// update
// ---------------------------------------------------------------------------

fn update(ctx: &mut Context, args: &UpdateArgs) -> CliResult {
    let container = resolve::container(ctx)?;
    if args.requirement.is_empty() {
        return update_resolve(ctx, &container, args.package.as_deref());
    }

    // Requirement change (bump / pin / downgrade) — needs a target package.
    let Some(package) = &args.package else {
        return Err(CliError::new(
            "a package is required when changing the requirement",
        ));
    };
    let spec = requirement_spec(&args.requirement)?;
    if let Container::SwiftPackage(_) = container {
        return Err(CliError::new(
            "changing a Package.swift dependency's requirement via the CLI isn't supported; edit Package.swift, then run `dep resolve`",
        ));
    }

    let xcodeproj = pick_xcodeproj(ctx, &container, Some(package))?;
    let mut root = parse_owned(&xcodeproj)?;
    let ref_guid = find_package_or_hint(&root, &container, package, &xcodeproj)?;
    spm_pbxproj::set_requirement(&mut root, &ref_guid, &spec).map_err(CliError::new)?;
    write_pbxproj(&xcodeproj, &root)?;

    if !args.no_resolve {
        // Drop the stale pin so resolution re-pins to the new requirement
        // (needed when downgrading), then resolve.
        remove_pin(&container, package);
        resolve_packages(&container, None, &ctx.out, false)?;
    }
    report_updated(ctx, package);
    Ok(())
}

/// Plain update (no requirement change): bump pins to the latest the current
/// requirements allow, for one package or everything.
fn update_resolve(ctx: &mut Context, container: &Container, package: Option<&str>) -> CliResult {
    if let Container::SwiftPackage(_) = container {
        swiftpm::update(container, package, ctx.out.is_json())?;
    } else {
        // xcodebuild has no "update"; drop the pin(s) so the resolve re-pins to
        // the latest allowed — one package, or the whole lockfile.
        match package {
            Some(p) => remove_pin(container, p),
            None => delete_lockfile(container),
        }
        resolve_packages(container, None, &ctx.out, false)?;
    }
    report_updated(ctx, package.unwrap_or("all packages"));
    Ok(())
}

/// Delete `Package.resolved` so a fresh resolve re-pins everything (update all).
fn delete_lockfile(container: &Container) {
    let _ = std::fs::remove_file(resolved_path(container));
}

fn report_updated(ctx: &Context, what: &str) {
    if ctx.out.is_json() {
        ctx.out.json_value(&serde_json::json!({ "updated": what }));
    } else {
        ctx.out.note(&format!("updated {what}"));
    }
}

// ---------------------------------------------------------------------------
// resolve
// ---------------------------------------------------------------------------

fn resolve_action(ctx: &mut Context) -> CliResult {
    let container = resolve::container(ctx)?;
    resolve_packages(&container, None, &ctx.out, false)?;
    if ctx.out.is_json() {
        ctx.out.json_value(&serde_json::json!({ "resolved": true }));
    } else {
        ctx.out.note("resolved package dependencies");
    }
    Ok(())
}

/// Resolve a container's package dependencies. `clone_dir` relocates the
/// checkouts (used during `add` discovery); `quiet` discards stdout (for `--json`
/// and the discovery step).
fn resolve_packages(
    container: &Container,
    clone_dir: Option<&Path>,
    out: &Output,
    quiet: bool,
) -> CliResult {
    if let Container::SwiftPackage(_) = container {
        return swiftpm::resolve(container, quiet || out.is_json());
    }
    let mut args = vec!["-resolvePackageDependencies".to_string()];
    args.extend(xcodebuild::container_args(container));
    // xcodebuild *requires* a scheme to resolve a workspace (and accepts one for
    // a project); any scheme resolves the whole package graph, so use the first.
    if let Some(scheme) = first_scheme(container) {
        args.push("-scheme".to_string());
        args.push(scheme);
    }
    if let Some(dir) = clone_dir {
        args.push("-clonedSourcePackagesDirPath".to_string());
        args.push(dir.to_string_lossy().into_owned());
    }
    let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
    let cwd = xcodebuild::working_dir(container);
    // Beautify like `build`: quiet/JSON stays silent, `-v` passes raw output
    // through, otherwise the buildlog renderer shows a clean "Resolving" spinner.
    let ok = if quiet || out.is_json() {
        process::run("xcodebuild", &arg_refs, cwd.as_deref(), true)?
    } else if out.is_verbose() {
        process::run("xcodebuild", &arg_refs, cwd.as_deref(), false)?
    } else {
        buildlog::run("xcodebuild", &arg_refs, cwd.as_deref(), out, "Resolving")?
    };
    if ok {
        Ok(())
    } else {
        Err(
            CliError::new("xcodebuild -resolvePackageDependencies exited with a non-zero status")
                .context("resolving package dependencies"),
        )
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Resolve the version requirement for a remote add: exactly one primary flag,
/// with `--to` only alongside `--from`. Validated here (not via a clap group) so
/// the same flags can be optional for a local add.
fn requirement_spec(args: &RequirementArgs) -> Result<RequirementSpec, CliError> {
    let primaries = [
        args.from.is_some(),
        args.exact.is_some(),
        args.up_to_next_minor_from.is_some(),
        args.branch.is_some(),
        args.revision.is_some(),
    ]
    .into_iter()
    .filter(|set| *set)
    .count();
    if primaries == 0 {
        return Err(CliError::new(
            "a remote package needs a version requirement (--from/--exact/--up-to-next-minor-from/--branch/--revision)",
        ));
    }
    if primaries > 1 {
        return Err(CliError::new("only one version requirement may be given"));
    }
    if let Some(v) = &args.from {
        return Ok(match &args.to {
            Some(to) => RequirementSpec::Range {
                from: v.clone(),
                to: to.clone(),
            },
            None => RequirementSpec::UpToNextMajor(v.clone()),
        });
    }
    if args.to.is_some() {
        return Err(CliError::new("--to requires --from"));
    }
    if let Some(v) = &args.up_to_next_minor_from {
        return Ok(RequirementSpec::UpToNextMinor(v.clone()));
    }
    if let Some(v) = &args.exact {
        return Ok(RequirementSpec::Exact(v.clone()));
    }
    if let Some(b) = &args.branch {
        return Ok(RequirementSpec::Branch(b.clone()));
    }
    let revision = args.revision.clone().unwrap_or_default();
    Ok(RequirementSpec::Revision(revision))
}

/// Re-emit a validated requirement as `swift package add-dependency`'s own flags.
fn swift_flags_for(spec: &RequirementSpec) -> Vec<String> {
    match spec {
        RequirementSpec::UpToNextMajor(v) => vec!["--from".to_string(), v.clone()],
        RequirementSpec::Range { from, to } => {
            vec![
                "--from".to_string(),
                from.clone(),
                "--to".to_string(),
                to.clone(),
            ]
        }
        RequirementSpec::UpToNextMinor(v) => vec!["--up-to-next-minor-from".to_string(), v.clone()],
        RequirementSpec::Exact(v) => vec!["--exact".to_string(), v.clone()],
        RequirementSpec::Branch(b) => vec!["--branch".to_string(), b.clone()],
        RequirementSpec::Revision(r) => vec!["--revision".to_string(), r.clone()],
    }
}

/// Whether the argument is a remote package URL rather than a local path. An
/// existing directory on disk is always treated as local (so a path that happens
/// to contain `://` isn't misread); otherwise a scheme or `scp`-style git
/// address marks it remote.
fn looks_remote(url: &str) -> bool {
    if Path::new(url).is_dir() {
        return false;
    }
    url.contains("://") || url.starts_with("git@")
}

/// Choose `kind` items: pass through explicit `flags`, else prompt with a
/// multi-select (caller has already ensured an interactive terminal).
fn choose(
    kind: &str,
    available: &[String],
    flags: &[String],
    ctx: &Context,
) -> Result<Vec<String>, CliError> {
    if !flags.is_empty() {
        return Ok(flags.to_vec());
    }
    multi_select(&format!("Select {kind}(s)"), available, ctx.out.use_color())
}

fn multi_select(prompt: &str, items: &[String], color: bool) -> Result<Vec<String>, CliError> {
    let theme: Box<dyn dialoguer::theme::Theme> = if color {
        Box::new(dialoguer::theme::ColorfulTheme::default())
    } else {
        Box::new(dialoguer::theme::SimpleTheme)
    };
    let chosen = dialoguer::MultiSelect::with_theme(theme.as_ref())
        .with_prompt(prompt)
        .items(items)
        .interact()
        .map_err(|e| CliError::new(format!("prompt failed: {e}")).kind(ErrorKind::UserCancel))?;
    if chosen.is_empty() {
        return Err(CliError::new(format!("no {prompt} chosen")).kind(ErrorKind::UserCancel));
    }
    Ok(chosen.into_iter().map(|i| items[i].clone()).collect())
}

/// A *buildable* scheme of a container, to satisfy `xcodebuild
/// -resolvePackageDependencies` (which requires a scheme for a workspace and
/// rejects one whose Build action is empty — e.g. a Tuist "Generate Project"
/// helper scheme). Any buildable scheme resolves the whole package graph, so the
/// first one with build entries (or an autocreated scheme, which has none on
/// disk but always builds) is used; falls back to the first scheme.
fn first_scheme(container: &Container) -> Option<String> {
    let names = resolve::schemes(container).ok()?;
    // Where a scheme's `.xcscheme` may live: the container itself, plus every
    // member project for a workspace.
    let dirs: Vec<PathBuf> = match container {
        Container::Project(p) => vec![p.clone()],
        Container::Workspace(p) => {
            let mut dirs = vec![p.clone()];
            if let Ok(ws) = crate::workspace::open(p) {
                dirs.extend(ws.project_refs);
            }
            dirs
        }
        Container::SwiftPackage(_) => return None,
    };
    for name in &names {
        if scheme_builds(&dirs, name) {
            return Some(name.clone());
        }
    }
    names.into_iter().next()
}

/// Whether a scheme builds something: it has no materialized file (an
/// autocreated scheme for a buildable target) or its `BuildAction` has entries.
/// An unparseable file is assumed buildable rather than skipped.
fn scheme_builds(dirs: &[PathBuf], name: &str) -> bool {
    match dirs
        .iter()
        .find_map(|d| crate::scheme::find_scheme_file(d, name))
    {
        None => true,
        Some(file) => match crate::scheme::parse_file(&file) {
            Ok(scheme) => !scheme.build_entries.is_empty(),
            // Can't parse it — don't skip a possibly-good scheme on our account.
            Err(_) => true,
        },
    }
}

/// The `.xcodeproj` to mutate. For a workspace: an explicit `--project`, else the
/// member that already declares `owner` (when given, for remove/update), else the
/// sole member, else an interactive pick (strict error off a TTY).
fn pick_xcodeproj(
    ctx: &Context,
    container: &Container,
    owner: Option<&str>,
) -> Result<PathBuf, CliError> {
    let p = match container {
        Container::Project(p) => return Ok(p.clone()),
        Container::Workspace(p) => p,
        Container::SwiftPackage(_) => {
            return Err(CliError::new(
                "internal error: no .xcodeproj for a Swift package",
            ));
        }
    };
    if let Some(proj) = &ctx.targeting.project {
        return Ok(proj.clone());
    }
    let ws = crate::workspace::open(p)
        .map_err(|e| CliError::new(format!("failed to read workspace {}: {e}", p.display())))?;
    let members = ws.project_refs;
    if members.is_empty() {
        return Err(CliError::new(
            "the workspace references no projects to modify",
        ));
    }
    if members.len() == 1 {
        return Ok(members[0].clone());
    }
    // Prefer the single member that already declares the package being acted on.
    if let Some(query) = owner {
        let owners: Vec<&PathBuf> = members
            .iter()
            .filter(|m| member_declares(m, query))
            .collect();
        if owners.len() == 1 {
            return Ok(owners[0].clone());
        }
    }
    // Otherwise pick interactively (auto-picks a lone candidate, strict-errors
    // off a TTY) via the shared resolver picker.
    let labels: Vec<String> = members.iter().map(|m| m.display().to_string()).collect();
    let chosen = resolve::choose(ctx, "project", None, &labels)?;
    members
        .into_iter()
        .find(|m| m.display().to_string() == chosen)
        .ok_or_else(|| CliError::new("selected project not found in the workspace"))
}

/// Whether a member project declares a package matching `query`.
fn member_declares(xcodeproj: &Path, query: &str) -> bool {
    crate::project::parse_pbxproj(xcodeproj)
        .ok()
        .is_some_and(|root| spm_pbxproj::find_package(&root, query).is_some())
}

fn parse_owned(xcodeproj: &Path) -> Result<Value, CliError> {
    let path = xcodeproj.join("project.pbxproj");
    crate::pbxproj::parse_file(&path)
        .map_err(|e| CliError::new(format!("failed to parse {}: {e}", path.display())))
}

fn write_pbxproj(xcodeproj: &Path, root: &Value) -> CliResult {
    let name = xcodeproj
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("Project");
    let text = crate::pbxproj_writer::serialize(root, name);
    let path = xcodeproj.join("project.pbxproj");
    std::fs::write(&path, text)
        .map_err(|e| CliError::new(format!("failed to write {}: {e}", path.display())))
}

/// Path to the local package directory, relative to the project directory, for
/// an `XCLocalSwiftPackageReference.relativePath`.
fn local_relative_path(xcodeproj: &Path, url: &str) -> Result<String, CliError> {
    let target = PathBuf::from(url);
    if !target.exists() {
        return Err(CliError::new(format!(
            "local package path `{url}` does not exist"
        )));
    }
    let proj_dir = xcodeproj.parent().unwrap_or_else(|| Path::new("."));
    Ok(relative_path(proj_dir, &target))
}

/// `to` expressed relative to `from` (both canonicalized when possible), e.g.
/// `../Packages/Dep`. Falls back to the absolute path if there's no common root.
fn relative_path(from: &Path, to: &Path) -> String {
    let from = std::fs::canonicalize(from).unwrap_or_else(|_| from.to_path_buf());
    let to = std::fs::canonicalize(to).unwrap_or_else(|_| to.to_path_buf());
    let from_comps: Vec<_> = from.components().collect();
    let to_comps: Vec<_> = to.components().collect();
    let common = from_comps
        .iter()
        .zip(&to_comps)
        .take_while(|(a, b)| a == b)
        .count();
    if common == 0 {
        return to.to_string_lossy().into_owned();
    }
    let mut rel = PathBuf::new();
    for _ in 0..(from_comps.len() - common) {
        rel.push("..");
    }
    for c in &to_comps[common..] {
        rel.push(c.as_os_str());
    }
    let rel = rel.to_string_lossy().into_owned();
    if rel.is_empty() { ".".to_string() } else { rel }
}

fn product_names(manifest: &swiftpm::Manifest) -> Vec<String> {
    manifest.products.iter().map(|p| p.name.clone()).collect()
}

fn clone_dir() -> PathBuf {
    std::env::temp_dir().join(format!("sweetpad-spm-{}", std::process::id()))
}

/// Locate a resolved package's checkout under `base` — a cloned-source-packages
/// dir or a `.build` dir holding `workspace-state.json` + `checkouts/`. Prefers
/// the precise identity→subpath map in `workspace-state.json` (robust to
/// monorepo sub-paths and case differences), falling back to a basename guess.
fn resolve_checkout(base: &Path, url: &str) -> Option<PathBuf> {
    let identity = spm_pbxproj::identity_from_url(url);
    checkout_from_state(base, &identity).or_else(|| checkout_by_name(&base.join("checkouts"), url))
}

/// Map a package identity to its exact checkout dir via `workspace-state.json`.
fn checkout_from_state(base: &Path, identity: &str) -> Option<PathBuf> {
    let text = std::fs::read_to_string(base.join("workspace-state.json")).ok()?;
    let json: serde_json::Value = serde_json::from_str(&text).ok()?;
    let deps = json.get("object")?.get("dependencies")?.as_array()?;
    for dep in deps {
        let id = dep
            .get("packageRef")
            .and_then(|r| r.get("identity"))
            .and_then(serde_json::Value::as_str)
            .map(str::to_ascii_lowercase);
        if id.as_deref() == Some(identity) {
            let subpath = dep.get("subpath").and_then(serde_json::Value::as_str)?;
            return Some(base.join("checkouts").join(subpath));
        }
    }
    None
}

/// Fallback: guess the checkout dir by the URL's last component, then a
/// case-insensitive identity match against the directory names.
fn checkout_by_name(checkouts: &Path, url: &str) -> Option<PathBuf> {
    let basename = url
        .trim_end_matches('/')
        .rsplit('/')
        .next()?
        .trim_end_matches(".git");
    let direct = checkouts.join(basename);
    if direct.is_dir() {
        return Some(direct);
    }
    let id = spm_pbxproj::identity_from_url(url);
    std::fs::read_dir(checkouts)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .find(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .map(str::to_ascii_lowercase)
                == Some(id.clone())
        })
}

/// A display name for a package URL/path — its last path component (no `.git`).
fn package_display_name(url: &str) -> String {
    url.trim_end_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or(url)
        .trim_end_matches(".git")
        .to_string()
}

fn report_added(ctx: &Context, url: &str, products: &[String], targets: &[String]) {
    if ctx.out.is_json() {
        ctx.out.json_value(&serde_json::json!({
            "added": url,
            "products": products,
            "targets": targets,
        }));
    } else if products.is_empty() {
        ctx.out.note(&format!("added {url}"));
    } else {
        ctx.out.note(&format!(
            "linked {} into {}",
            products.join(", "),
            targets.join(", ")
        ));
    }
}

fn report_removed(ctx: &Context, package: &str, unlinked: Option<&[(String, String)]>) {
    if ctx.out.is_json() {
        let payload = match unlinked {
            None => serde_json::json!({ "removed": package }),
            Some(links) => serde_json::json!({
                "unlinked": links.iter()
                    .map(|(product, target)| serde_json::json!({ "product": product, "target": target }))
                    .collect::<Vec<_>>(),
            }),
        };
        ctx.out.json_value(&payload);
    } else {
        match unlinked {
            None => ctx.out.note(&format!("removed package {package}")),
            Some(links) => {
                for (product, target) in links {
                    ctx.out.note(&format!("unlinked {product} from {target}"));
                }
            }
        }
    }
}

fn kind_str(container: &Container) -> &'static str {
    match container {
        Container::Workspace(_) => "workspace",
        Container::Project(_) => "project",
        Container::SwiftPackage(_) => "package",
    }
}

// ---------------------------------------------------------------------------
// Package.resolved reading
// ---------------------------------------------------------------------------

/// A locked pin from `Package.resolved`.
struct Pin {
    version: Option<String>,
    branch: Option<String>,
    revision: Option<String>,
    location: Option<String>,
}

impl Pin {
    /// The locked version, or `branch @ <short-sha>`, or a short revision.
    fn display(&self) -> String {
        if let Some(v) = &self.version {
            return v.clone();
        }
        if let Some(b) = &self.branch {
            return match &self.revision {
                Some(r) => format!("{b} @ {}", short_rev(r)),
                None => b.clone(),
            };
        }
        self.revision
            .as_deref()
            .map_or_else(|| "?".to_string(), short_rev)
    }
}

fn short_rev(rev: &str) -> String {
    rev.chars().take(7).collect()
}

fn resolved_path(container: &Container) -> PathBuf {
    match container {
        Container::Project(p) => p
            .join("project.xcworkspace")
            .join("xcshareddata")
            .join("swiftpm")
            .join("Package.resolved"),
        Container::Workspace(p) => p
            .join("xcshareddata")
            .join("swiftpm")
            .join("Package.resolved"),
        Container::SwiftPackage(p) => p.parent().map_or_else(
            || PathBuf::from("Package.resolved"),
            |d| d.join("Package.resolved"),
        ),
    }
}

/// Parse `Package.resolved` into `identity -> Pin`. A missing/unreadable file is
/// an empty map (the locked column just shows `—`).
fn read_resolved(path: &Path) -> HashMap<String, Pin> {
    let mut map = HashMap::new();
    let Ok(text) = std::fs::read_to_string(path) else {
        return map;
    };
    let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) else {
        return map;
    };
    // v2/v3 store `pins` at the top level; v1 nested them under `object`.
    let pins = json
        .get("pins")
        .and_then(serde_json::Value::as_array)
        .or_else(|| {
            json.get("object")
                .and_then(|o| o.get("pins"))
                .and_then(serde_json::Value::as_array)
        });
    let Some(pins) = pins else {
        return map;
    };
    for pin in pins {
        let Some(identity) = pin.get("identity").and_then(serde_json::Value::as_str) else {
            continue;
        };
        let state = pin.get("state");
        let field = |key: &str| {
            state
                .and_then(|s| s.get(key))
                .and_then(serde_json::Value::as_str)
                .map(str::to_string)
        };
        map.insert(
            identity.to_ascii_lowercase(),
            Pin {
                version: field("version"),
                branch: field("branch"),
                revision: field("revision"),
                location: pin
                    .get("location")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_string),
            },
        );
    }
    map
}
