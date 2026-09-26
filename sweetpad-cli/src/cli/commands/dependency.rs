//! `sweetpad dependency …` (alias `dep`) — view and manage Swift Package
//! Manager dependencies.
//!
//! Works on all three container kinds. For an `.xcodeproj`/`.xcworkspace` there
//! is no Apple CLI to add/remove SPM packages, so we edit the project document
//! directly — `project.pbxproj` through [`sweetpad_lib::spm_pbxproj`], or the
//! `project.xcproj` Xcode 27.2 writes in its place through
//! [`sweetpad_lib::spm_xcproj`]; for a `Package.swift` we drive the
//! Swift 6 `swift package add-dependency`/`add-target-dependency`/`resolve`
//! commands. `list` shows each declared package's requested requirement next to
//! its locked version from `Package.resolved`.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use clap::{Args, Subcommand};

use crate::cli::output::Output;
use crate::cli::pbxedit::{self, Editable};
use crate::cli::resolve::{self, Container};
use crate::cli::{
    CliError, CliResult, CommandResult, Context, ErrorKind, Render, Rendered, buildlog, process,
    swiftpm, xcodebuild,
};
use sweetpad_lib::spm::{DeclaredPackage, PackageKind, RequirementSpec, identity_from_url};
use sweetpad_lib::{spm_pbxproj, spm_xcproj};

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
    /// Update resolved versions, or change a package's requirement.
    ///
    /// With no requirement flags, re-resolves to the latest allowed versions;
    /// with one, rewrites the package's requirement (bump, pin, or downgrade)
    /// and then re-resolves.
    Update(UpdateArgs),
    /// Resolve dependencies into 'Package.resolved'.
    Resolve,
}

/// Flags for `dependency update`.
#[derive(Debug, Args)]
pub struct UpdateArgs {
    /// Package to update (identity/URL/path/name); when omitted, every
    /// package updates.
    pub package: Option<String>,

    #[command(flatten)]
    pub requirement: RequirementArgs,

    /// When changing a requirement, edit it only; don't re-resolve.
    #[arg(long)]
    pub no_resolve: bool,

    /// Edit a generated project (XcodeGen/Tuist) anyway — the change is
    /// deliberate and will be lost on the next regenerate.
    #[arg(long)]
    pub force: bool,
}

/// Flags for `dependency add`.
#[derive(Debug, Args)]
pub struct AddArgs {
    /// Remote git URL, or a local directory path containing 'Package.swift'.
    pub url: String,

    #[command(flatten)]
    pub requirement: RequirementArgs,

    /// Product(s) to link (repeatable); when omitted you're prompted after
    /// the resolve.
    #[arg(long = "product")]
    pub products: Vec<String>,

    /// Target(s) to link the product(s) into (repeatable); when omitted
    /// you're prompted.
    #[arg(long = "target")]
    pub targets: Vec<String>,

    /// Skip the resolve that updates 'Package.resolved' after mutating.
    #[arg(long)]
    pub no_resolve: bool,

    /// Edit a generated project (XcodeGen/Tuist) anyway — the change is
    /// deliberate and will be lost on the next regenerate.
    #[arg(long)]
    pub force: bool,
}

/// The version requirement for `add`, mirroring `swift package add-dependency`.
/// All flags are optional at parse time — a requirement is required only for a
/// *remote* package (a local path has no version), so the "exactly one of these"
/// rule is enforced contextually in [`requirement_spec`] rather than by a clap
/// group that would also force one on a local add.
#[derive(Debug, Args)]
pub struct RequirementArgs {
    /// This version up to the next major (SwiftPM 'from: "x.y.z"').
    #[arg(long, value_name = "VERSION")]
    pub from: Option<String>,

    /// Exactly this version (SwiftPM 'exact: "x.y.z"').
    #[arg(long, value_name = "VERSION")]
    pub exact: Option<String>,

    /// This version up to the next minor (SwiftPM '.upToNextMinor(from:)').
    #[arg(long = "up-to-next-minor-from", value_name = "VERSION")]
    pub up_to_next_minor_from: Option<String>,

    /// Follow a branch (SwiftPM 'branch: "name"').
    #[arg(long, value_name = "BRANCH")]
    pub branch: Option<String>,

    /// Pin to a commit (SwiftPM 'revision: "sha"').
    #[arg(long, value_name = "SHA")]
    pub revision: Option<String>,

    /// Upper bound of a half-open 'from ..< to' range; requires '--from'.
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

/// Flags for `dependency remove`. Unlike `add`'s repeatable link lists, the
/// narrowing flags here are deliberately single: `remove` unlinks one
/// product/target edge (or drops the whole package when neither is given).
#[derive(Debug, Args)]
pub struct RemoveArgs {
    /// Package to remove: identity, repository URL, local path, or its name.
    pub package: String,

    /// Narrow to unlinking this one product only (keep the package reference).
    #[arg(long = "product")]
    pub product: Option<String>,

    /// Narrow to this one target only.
    #[arg(long = "target")]
    pub target: Option<String>,

    /// Edit a generated project (XcodeGen/Tuist) anyway — the change is
    /// deliberate and will be lost on the next regenerate.
    #[arg(long)]
    pub force: bool,
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

/// `dependency list`: each declared package with its locked version, and the
/// transitive pins too when asked.
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
            let ws = sweetpad_lib::workspace::open(p).map_err(|e| {
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
    Ok(list_packages(&Editable::parse(xcodeproj)?))
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

/// `dependency add`: declare the package, then link its products into targets.
fn add(ctx: &mut Context, args: &AddArgs) -> CliResult {
    let container = resolve::container(ctx)?;
    match container {
        Container::SwiftPackage(_) => add_to_package(ctx, &container, args),
        Container::Project(_) | Container::Workspace(_) => add_to_xcode(ctx, &container, args),
    }
}

fn add_to_xcode(ctx: &mut Context, container: &Container, args: &AddArgs) -> CliResult {
    let xcodeproj = pick_xcodeproj(ctx, container, None)?;
    pbxedit::guard_generated(ctx.project_file(container), &xcodeproj, args.force)?;
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

    // The reference must be written *before* product discovery (resolution
    // needs it on disk), but everything after can still fail or be cancelled
    // at a picker — snapshot the pristine document so no path leaves a
    // dangling, unlinked package reference behind. The sibling backup covers
    // the paths a snapshot can't: a signal `_exit`ing mid-resolve.
    let document_path = pbxedit::document_path(&xcodeproj);
    heal_interrupted_mutation(&document_path, &ctx.out);
    let mut document = Editable::parse(&xcodeproj)?;
    let pristine = std::fs::read_to_string(&document_path)
        .map_err(|e| CliError::new(format!("failed to read {}: {e}", document_path.display())))?;
    // The discovery resolve rewrites Package.resolved with the new package's
    // pins — snapshot it too, so a cancel doesn't leave ghost pins that make
    // `dep list`/`dep remove` misreport the abandoned package.
    let pristine_lockfile = read_lockfile(container);
    let backup = MutationBackup::create(&document_path, &pristine)?;

    // 1. Add the package reference (only) and write it, so resolution can fetch.
    let added = match &spec {
        Some(spec) => add_remote(&mut document, &args.url, spec),
        None => local_relative_path(&xcodeproj, &args.url)
            .and_then(|rel| add_local(&mut document, &rel)),
    }
    .and_then(|id| document.write(&xcodeproj).map(|()| id));
    let package_id = match added {
        Ok(id) => id,
        Err(e) => {
            backup.commit();
            return Err(e);
        }
    };
    ctx.out.note(&format!("added package {}", args.url));

    let linked = (|| -> Result<(Vec<String>, Vec<String>), CliError> {
        // 2. Settle the products and targets to link (resolve-then-prompt).
        let products = resolve_products(ctx, container, &args.url, remote, &args.products)?;
        let targets = settle_targets(ctx, &xcodeproj, &args.targets)?;

        // 3. Link each product into each target and write.
        for product in &products {
            for target in &targets {
                link_product(&mut document, &package_id, product, target)?;
            }
        }
        document.write(&xcodeproj)?;

        // 4. Ensure Package.resolved is current. Discovering a remote package's
        //    products already resolved (and wrote the lockfile), so only resolve
        //    here when discovery didn't — and unless told to skip.
        let resolved_in_discovery = remote && args.products.is_empty();
        if !args.no_resolve && !resolved_in_discovery {
            resolve_packages(container, None, &ctx.out, false)?;
        }
        Ok((products, targets))
    })();

    let result = match linked {
        Ok((products, targets)) => {
            report_added(ctx, &args.url, &products, &targets);
            Ok(())
        }
        Err(e) => {
            restore_or_remove_lockfile(container, pristine_lockfile);
            // Report the rollback honestly — claiming success while the
            // dangling reference remains would hide exactly the state this
            // rollback exists to prevent.
            if pbxedit::write_atomic(&document_path, &pristine).is_ok() {
                ctx.out
                    .note("rolled the package reference back out of the project (nothing linked)");
            } else {
                ctx.out.warn(&format!(
                    "could not roll the package reference back out of {} — the project may \
                     reference the package without linking it",
                    document_path.display()
                ));
            }
            Err(e)
        }
    };
    backup.commit();
    result
}

fn add_to_package(ctx: &mut Context, container: &Container, args: &AddArgs) -> CliResult {
    let remote = looks_remote(&args.url);

    // `swift package add-dependency` is a Swift 6 feature; fail clearly on older
    // toolchains instead of surfacing a raw "unknown subcommand" exit.
    if swiftpm::swift_major_version().is_some_and(|v| v < 6) {
        return Err(CliError::new(
            "adding a dependency to a Package.swift needs Swift 6+ (swift package add-dependency); edit Package.swift and run 'dep resolve' instead",
        ));
    }

    // Validate the requirement (remote only) before anything else, so a bad
    // requirement is reported ahead of the prompt/mutation.
    let requirement = if remote {
        Some(swift_flags_for(&requirement_spec(&args.requirement)?))
    } else {
        None
    };

    // Fail before mutating anything if we can neither be told nor prompt for
    // the products/targets to link — the same guard as the xcodeproj path, so a
    // non-interactive add never leaves a dangling manifest edit behind a
    // misclassified prompt failure.
    if (args.products.is_empty() || args.targets.is_empty()) && !ctx.out.is_interactive() {
        return Err(CliError::new(
            "non-interactive: pass --product and --target to add without prompting",
        ));
    }

    // Package.swift is edited in step 1 but the pickers below can still be
    // cancelled — snapshot the pristine manifest so no path leaves a
    // dependency declared and linked nowhere. The sibling backup covers a
    // signal `_exit`ing mid-resolve, past the in-memory rollback.
    let manifest_path = container.path().to_path_buf();
    heal_interrupted_mutation(&manifest_path, &ctx.out);
    let pristine = std::fs::read_to_string(&manifest_path)
        .map_err(|e| CliError::new(format!("failed to read {}: {e}", manifest_path.display())))?;
    // The resolve in step 2 writes the new package's pins into
    // Package.resolved — snapshot it too, so a cancel doesn't leave ghost
    // pins that make `dep list`/`dep remove` misreport the abandoned package.
    let pristine_lockfile = read_lockfile(container);
    let backup = MutationBackup::create(&manifest_path, &pristine)?;

    // 1. Add the dependency to the manifest. SwiftPM resolves a local path
    //    against the package root, so it is written relative to that.
    let quiet = ctx.out.is_json() || ctx.out.is_ndjson();
    let added = match &requirement {
        Some(flags) => swiftpm::add_dependency(container, &args.url, Some(flags), quiet),
        None => local_relative_path(&manifest_path, &args.url)
            .and_then(|rel| swiftpm::add_dependency(container, &rel, None, quiet)),
    };
    if let Err(e) = added {
        backup.commit();
        return Err(e);
    }
    ctx.out.note(&format!("added package {}", args.url));

    let linked = (|| -> Result<(Vec<String>, Vec<String>), CliError> {
        // 2. Resolve to fetch the package (needed to discover a remote
        //    package's products).
        let need_discovery = args.products.is_empty();
        if !args.no_resolve || (need_discovery && remote) {
            ctx.out.step("Resolving package dependencies", || {
                swiftpm::resolve(container, ctx.out.is_json() || ctx.out.is_ndjson())
            })?;
        }

        // 3. Settle products + targets and link each pair. The `--package`
        // value must be the dependency's *identity* (the URL basename), not
        // the checkout manifest's declared name — for firebase-ios-sdk the
        // manifest says "Firebase", and `.product(package: "Firebase")`
        // fails the next resolve with "unknown package".
        let package_name = package_display_name(&args.url);
        let products = if need_discovery {
            let available = package_products(container, &args.url, remote)?;
            choose("product", &available, &args.products, ctx)?
        } else {
            args.products.clone()
        };
        let target_names = swiftpm::manifest(container)?
            .targets
            .iter()
            .map(|t| t.name.clone())
            .collect::<Vec<_>>();
        let targets = choose("target", &target_names, &args.targets, ctx)?;

        for product in &products {
            for target in &targets {
                swiftpm::add_target_dependency(
                    container,
                    product,
                    target,
                    &package_name,
                    ctx.out.is_json() || ctx.out.is_ndjson(),
                )?;
            }
        }
        Ok((products, targets))
    })();

    let result = match linked {
        Ok((products, targets)) => {
            report_added(ctx, &args.url, &products, &targets);
            Ok(())
        }
        Err(e) => {
            let _ = pbxedit::write_atomic(&manifest_path, &pristine);
            restore_or_remove_lockfile(container, pristine_lockfile);
            ctx.out
                .note("rolled the dependency back out of Package.swift (nothing linked)");
            Err(e)
        }
    };
    backup.commit();
    result
}

/// Roll `Package.resolved` back to a [`read_lockfile`] snapshot after a
/// cancelled add: restore the pristine text, or — when no lockfile existed
/// before — remove the one the discovery resolve created, so no ghost pins
/// for the abandoned package survive either way.
fn restore_or_remove_lockfile(container: &Container, pristine: Option<String>) {
    match pristine {
        Some(text) => {
            let _ = std::fs::write(resolved_path(container), text);
        }
        None => {
            let _ = std::fs::remove_file(resolved_path(container));
        }
    }
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
    let proj = sweetpad_lib::project::open(xcodeproj)
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
        return products_to_link(&swiftpm::manifest_at(Path::new(url))?);
    }
    let clone = CloneDir::new();
    ctx.out.step("Resolving package dependencies", || {
        resolve_packages(container, Some(&clone.path), &ctx.out, true)
    })?;
    let checkout = resolve_checkout(&clone.path, url).ok_or_else(|| {
        CliError::new("could not locate the resolved package checkout to read its products")
    })?;
    products_to_link(&swiftpm::manifest_at(&checkout)?)
}

/// For a `Package.swift` add: read the just-added package's products from its
/// `.build` checkout, which the resolve before this made. A local package has
/// no checkout, since SwiftPM reads a path dependency in place, so its
/// products come straight from its directory.
fn package_products(
    container: &Container,
    url: &str,
    remote: bool,
) -> Result<Vec<String>, CliError> {
    if !remote {
        return products_to_link(&swiftpm::manifest_at(Path::new(url))?);
    }
    let pkg_dir = swiftpm::package_dir(container).unwrap_or_else(|| PathBuf::from("."));
    let checkout = resolve_checkout(&pkg_dir.join(".build"), url).ok_or_else(|| {
        CliError::new("could not locate the resolved package checkout to read its products")
    })?;
    products_to_link(&swiftpm::manifest_at(&checkout)?)
}

/// The products an added package offers for the product picker, on either
/// kind of project. A package that declares none has nothing to link, which
/// is an error here rather than a picker with nothing in it.
fn products_to_link(manifest: &swiftpm::Manifest) -> Result<Vec<String>, CliError> {
    let names = product_names(manifest);
    if names.is_empty() {
        return Err(CliError::new("the package declares no products to link"));
    }
    Ok(names)
}

/// `dependency remove`: drop a package, or unlink one product from one target.
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
    pbxedit::guard_generated(ctx.project_file(container), &xcodeproj, args.force)?;
    heal_interrupted_mutation(&pbxedit::document_path(&xcodeproj), &ctx.out);
    let mut document = Editable::parse(&xcodeproj)?;
    let package_id = find_package_or_hint(&document, container, &args.package, &xcodeproj)?;

    if args.product.is_none() && args.target.is_none() {
        // For a local package, Xcode may omit the product->package back-ref, so
        // pass the local package's declared product names to clean those up too.
        let orphans = local_product_names(&document, &package_id, &xcodeproj);
        remove_package(&mut document, &package_id, &orphans)?;
        document.write(&xcodeproj)?;
        remove_pin(container, &args.package);
        report_removed(ctx, &args.package, None);
    } else {
        let unlinked = unlink(
            &mut document,
            &package_id,
            args.product.as_deref(),
            args.target.as_deref(),
        )?;
        if unlinked.is_empty() {
            return Err(CliError::new(
                "no matching product/target link found to unlink",
            ));
        }
        document.write(&xcodeproj)?;
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
    let id = identity_from_url(query);
    // v2/v3 store `pins` at the top level; v1 nested them under `object` (and
    // its pins carry no `identity` key — derive one, as `read_resolved` does).
    let has_top = json.get("pins").is_some_and(serde_json::Value::is_array);
    let pins = if has_top {
        json.get_mut("pins")
            .and_then(serde_json::Value::as_array_mut)
    } else {
        json.get_mut("object")
            .and_then(|o| o.get_mut("pins"))
            .and_then(serde_json::Value::as_array_mut)
    };
    if let Some(pins) = pins {
        pins.retain(|p| pin_identity(p).map(|i| i.to_ascii_lowercase()) != Some(id.clone()));
    }
    let _ = pbxedit::write_atomic(&path, &sweetpad_lib::spm_resolved::serialize(&json));
}

/// A pin's identity: the `identity` key (v2/v3), or derived from
/// `repositoryURL`/`package` for v1 pins that carry none.
fn pin_identity(pin: &serde_json::Value) -> Option<String> {
    pin.get("identity")
        .and_then(serde_json::Value::as_str)
        .map(str::to_string)
        .or_else(|| {
            pin.get("repositoryURL")
                .or_else(|| pin.get("package"))
                .and_then(serde_json::Value::as_str)
                .map(identity_from_url)
        })
}

/// The product names a local package declares — passed to `remove_package` so
/// products Xcode wrote without a `package` back-ref are cleaned up. Empty for a
/// remote package or when the local manifest can't be read.
fn local_product_names(document: &Editable, package_id: &str, xcodeproj: &Path) -> Vec<String> {
    let Some(pkg) = list_packages(document)
        .into_iter()
        .find(|p| p.id == package_id)
    else {
        return Vec::new();
    };
    let PackageKind::Local { relative_path } = pkg.kind else {
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
    document: &Editable,
    container: &Container,
    query: &str,
    xcodeproj: &Path,
) -> Result<String, CliError> {
    if let Some(id) = find_package(document, query) {
        return Ok(id);
    }
    Err(transitive_hint(container, query).unwrap_or_else(|| {
        CliError::new(format!(
            "no package matching '{query}' in {}",
            xcodeproj.display()
        ))
        .kind(ErrorKind::TargetResolution)
    }))
}

/// A friendlier error when `query` names a transitive dependency (present in
/// `Package.resolved` but not directly declared) — you can't manage it directly.
fn transitive_hint(container: &Container, query: &str) -> Option<CliError> {
    let pins = read_resolved(&resolved_path(container));
    let id = identity_from_url(query);
    pins.contains_key(&id).then(|| {
        CliError::new(format!(
            "'{query}' is a transitive dependency (resolved but not directly declared); it's pulled in by one of your direct packages — change that package's requirement or remove it instead"
        ))
        .kind(ErrorKind::TargetResolution)
    })
}

/// `dependency update`: re-pin, or rewrite one package's requirement and then
/// re-pin.
fn update(ctx: &mut Context, args: &UpdateArgs) -> CliResult {
    let container = resolve::container(ctx)?;
    if args.requirement.is_empty() {
        return update_resolve(ctx, &container, args.package.as_deref());
    }

    // Requirement change (bump / pin / downgrade) — needs a target package.
    let Some(package) = &args.package else {
        return Err(
            CliError::new("a package is required when changing the requirement")
                .kind(ErrorKind::Usage),
        );
    };
    let spec = requirement_spec(&args.requirement)?;
    if let Container::SwiftPackage(_) = container {
        return Err(CliError::new(
            "changing a Package.swift dependency's requirement via the CLI isn't supported; edit Package.swift, then run 'dep resolve'",
        ));
    }

    let xcodeproj = pick_xcodeproj(ctx, &container, Some(package))?;
    pbxedit::guard_generated(ctx.project_file(&container), &xcodeproj, args.force)?;
    let document_path = pbxedit::document_path(&xcodeproj);
    heal_interrupted_mutation(&document_path, &ctx.out);
    let pristine = std::fs::read_to_string(&document_path)
        .map_err(|e| CliError::new(format!("failed to read {}: {e}", document_path.display())))?;
    let mut document = Editable::parse(&xcodeproj)?;
    let package_id = find_package_or_hint(&document, &container, package, &xcodeproj)?;
    set_requirement(&mut document, &package_id, &spec)?;
    document.write(&xcodeproj)?;

    let mut changes = Vec::new();
    if !args.no_resolve {
        // A failed resolve rolls the whole update back, requirement edit and
        // pin both, so an offline resolve or a bad requirement leaves the
        // project as it was.
        match repin(ctx, &container, Some(package)) {
            Ok(moved) => changes = moved,
            Err(e) => {
                let _ = pbxedit::write_atomic(&document_path, &pristine);
                ctx.out
                    .note("rolled the requirement change back (the resolve failed)");
                return Err(e);
            }
        }
    }
    report_updated(ctx, Some(package), &changes, true);
    Ok(())
}

/// Snapshot the lockfile's text (if it exists) before a destructive prune, so
/// a failed resolve can put it back instead of losing every pinned version.
fn read_lockfile(container: &Container) -> Option<String> {
    std::fs::read_to_string(resolved_path(container)).ok()
}

/// Best-effort restore of a [`read_lockfile`] snapshot after a failed resolve.
fn restore_lockfile(ctx: &Context, container: &Container, snapshot: Option<String>) {
    let Some(text) = snapshot else { return };
    if std::fs::write(resolved_path(container), text).is_ok() {
        ctx.out
            .note("restored Package.resolved after the failed resolve");
    } else {
        ctx.out
            .warn("the resolve failed and Package.resolved could not be restored");
    }
}

/// Plain update (no requirement change): bump pins to the latest the current
/// requirements allow, for one package or everything.
fn update_resolve(ctx: &mut Context, container: &Container, package: Option<&str>) -> CliResult {
    let changes = if let Container::SwiftPackage(_) = container {
        let before = read_lockfile(container);
        swiftpm::update(container, package, ctx.out.is_json() || ctx.out.is_ndjson())?;
        let after = read_lockfile(container).unwrap_or_default();
        pin_changes(before.as_deref(), &after, package)
    } else {
        // Verify the named package is actually declared before pruning, so a
        // typo errors (with the did-you-mean hint) instead of reporting
        // "updated <typo>" after a no-op resolve.
        if let Some(p) = package {
            let xcodeproj = pick_xcodeproj(ctx, container, Some(p))?;
            let document = Editable::parse(&xcodeproj)?;
            find_package_or_hint(&document, container, p, &xcodeproj)?;
        }
        repin(ctx, container, package)?
    };
    report_updated(ctx, package, &changes, false);
    Ok(())
}

/// Delete `Package.resolved` so a fresh resolve re-pins everything (update all).
fn delete_lockfile(container: &Container) {
    let _ = std::fs::remove_file(resolved_path(container));
}

/// Re-pin `package`, or every package when `None`, to the newest version its
/// requirement allows, keeping the others on their pins while those still fit.
/// Returns the pins that moved.
///
/// xcodebuild has no update, and pruning the pin is not enough on Xcode 27: a
/// resolve keeps a checkout already in SourcePackages while it satisfies the
/// requirement, and then doesn't write the pruned pin back. A resolve into an
/// empty clone directory has no checkout to keep and still writes its pins to
/// `Package.resolved`, so this runs one of those, then a normal resolve that
/// checks the new pins out. The empty directory also gets past Xcode 27
/// refusing to move a package from a version that declares SwiftPM traits to
/// one that declares none ("Disabled default traits … on package … that
/// declares no traits"), which it does when both a lockfile and a checkout of
/// the old version are there. Measured on Xcode 27.2 with both project formats.
///
/// A failed resolve puts the lockfile back the way it was.
fn repin(
    ctx: &Context,
    container: &Container,
    package: Option<&str>,
) -> Result<Vec<PinChange>, CliError> {
    let snapshot = read_lockfile(container);
    match package {
        Some(p) => remove_pin(container, p),
        None => delete_lockfile(container),
    }
    let clone = CloneDir::new();
    let resolved = resolve_packages(container, Some(&clone.path), &ctx.out, false)
        .and_then(|()| resolve_packages(container, None, &ctx.out, false));
    drop(clone);
    if let Err(e) = resolved {
        if snapshot.is_some() {
            restore_lockfile(ctx, container, snapshot);
        } else {
            delete_lockfile(container);
        }
        return Err(e);
    }
    let after = read_lockfile(container).unwrap_or_default();
    Ok(pin_changes(snapshot.as_deref(), &after, package))
}

/// A locked version an update moved.
#[derive(Debug, PartialEq, Eq)]
struct PinChange {
    package: String,
    from: Option<String>,
    to: Option<String>,
}

impl PinChange {
    /// `identity old → new`, with `—` for a side that has no pin.
    fn display(&self) -> String {
        let side = |v: &Option<String>| v.clone().unwrap_or_else(|| "—".to_string());
        format!("{} {} → {}", self.package, side(&self.from), side(&self.to))
    }
}

/// The pins that differ between two `Package.resolved` texts, `first`'s ahead
/// of the rest, which follow in identity order.
fn pin_changes(before: Option<&str>, after: &str, first: Option<&str>) -> Vec<PinChange> {
    let before = before.map(parse_resolved).unwrap_or_default();
    let after = parse_resolved(after);
    let mut ids: Vec<&String> = before.keys().chain(after.keys()).collect();
    ids.sort();
    ids.dedup();
    let mut changes: Vec<PinChange> = ids
        .into_iter()
        .filter_map(|id| {
            let from = before.get(id).map(Pin::display);
            let to = after.get(id).map(Pin::display);
            (from != to).then(|| PinChange {
                package: id.clone(),
                from,
                to,
            })
        })
        .collect();
    if let Some(first) = first.map(identity_from_url) {
        changes.sort_by_key(|c| c.package != first);
    }
    changes
}

/// Report an update: the pins that moved, or that none did. `requirement` is
/// whether the package's requirement was rewritten, which is an update even
/// when its pin stays.
fn report_updated(ctx: &Context, package: Option<&str>, changes: &[PinChange], requirement: bool) {
    if ctx.out.is_json() || ctx.out.is_ndjson() {
        let changes: Vec<serde_json::Value> = changes
            .iter()
            .map(|c| serde_json::json!({ "package": c.package, "from": c.from, "to": c.to }))
            .collect();
        ctx.out.result_value(&serde_json::json!({
            "updated": package.unwrap_or("all packages"),
            "changes": changes,
        }));
    } else if !changes.is_empty() {
        let moved: Vec<String> = changes.iter().map(PinChange::display).collect();
        ctx.out.note(&format!("updated {}", moved.join(", ")));
    } else if let Some(package) = package.filter(|_| requirement) {
        ctx.out.note(&format!("updated {package}"));
    } else if let Some(package) = package {
        ctx.out.note(&format!(
            "{package} is already on the newest version its requirement allows"
        ));
    } else {
        ctx.out
            .note("every package is already on the newest version its requirement allows");
    }
}

/// `dependency resolve`: bring `Package.resolved` up to date.
fn resolve_action(ctx: &mut Context) -> CliResult {
    let container = resolve::container(ctx)?;
    resolve_packages(&container, None, &ctx.out, false)?;
    if ctx.out.is_json() || ctx.out.is_ndjson() {
        ctx.out
            .result_value(&serde_json::json!({ "resolved": true }));
    } else {
        ctx.out.note("resolved package dependencies");
    }
    Ok(())
}

/// Resolve a container's package dependencies. `clone_dir` relocates the
/// checkouts (for `add` discovery and [`repin`]); `quiet` discards stdout (for
/// `--json` and the discovery step).
fn resolve_packages(
    container: &Container,
    clone_dir: Option<&Path>,
    out: &Output,
    quiet: bool,
) -> CliResult {
    if let Container::SwiftPackage(_) = container {
        return swiftpm::resolve(container, quiet || out.is_json() || out.is_ndjson());
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
    let bundle = ResolveBundle::new();
    args.push("-resultBundlePath".to_string());
    args.push(bundle.path.to_string_lossy().into_owned());
    let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
    let cwd = xcodebuild::working_dir(container);
    // Beautify like `build`: quiet/JSON captures both streams (so nothing
    // interleaves with the envelope; the tail rides back in the error), `-v`
    // passes raw output through, otherwise the buildlog renderer shows a clean
    // "Resolving" spinner and the errors that explain a failure. Those go to
    // stdout, so the error on stderr repeats them unless the two streams are
    // one file and they already sit just above it.
    let mut failure_detail = String::new();
    let ok = if quiet || out.is_json() || out.is_ndjson() {
        let run = process::run_captured("xcodebuild", &arg_refs, cwd.as_deref())?;
        if !run.success {
            failure_detail = format!(":\n{}", run.tail);
        }
        run.success
    } else if out.is_verbose() {
        process::run("xcodebuild", &arg_refs, cwd.as_deref(), false)?
    } else {
        let (ok, errors) = buildlog::run_keeping_errors(
            "xcodebuild",
            &arg_refs,
            cwd.as_deref(),
            out,
            "Resolving",
        )?;
        if !ok && !errors.is_empty() && !Output::streams_share_a_file() {
            failure_detail = errors.iter().fold(":".to_string(), |mut detail, line| {
                detail.push_str("\n  ");
                detail.push_str(line);
                detail
            });
        }
        ok
    };
    if ok {
        Ok(())
    } else {
        Err(CliError::new(format!(
            "xcodebuild -resolvePackageDependencies exited with a non-zero status{failure_detail}"
        ))
        .context("resolving package dependencies"))
    }
}

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
        )
        .kind(ErrorKind::Usage));
    }
    if primaries > 1 {
        return Err(
            CliError::new("only one version requirement may be given").kind(ErrorKind::Usage)
        );
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
        return Err(CliError::new("--to requires --from").kind(ErrorKind::Usage));
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
    multi_select(kind, available, ctx.out.use_color_stderr())
}

fn multi_select(kind: &str, items: &[String], color: bool) -> Result<Vec<String>, CliError> {
    // dialoguer answers an empty list with a bare IO error, so a project with
    // no targets gets a message instead.
    if items.is_empty() {
        return Err(CliError::new(format!(
            "there are no {kind}s to choose from"
        )));
    }
    let theme: Box<dyn dialoguer::theme::Theme> = if color {
        Box::new(dialoguer::theme::ColorfulTheme::default())
    } else {
        Box::new(dialoguer::theme::SimpleTheme)
    };
    let chosen = dialoguer::MultiSelect::with_theme(theme.as_ref())
        .with_prompt(format!("Select {kind}(s)"))
        .items(items)
        .interact()
        .map_err(|e| CliError::new(format!("prompt failed: {e}")).kind(ErrorKind::UserCancel))?;
    if chosen.is_empty() {
        return Err(CliError::new(format!("no {kind} chosen")).kind(ErrorKind::UserCancel));
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
            if let Ok(ws) = sweetpad_lib::workspace::open(p) {
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
        .find_map(|d| sweetpad_lib::scheme::find_scheme_file(d, name))
    {
        None => true,
        Some(file) => match sweetpad_lib::scheme::parse_file(&file) {
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
    let ws = sweetpad_lib::workspace::open(p)
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
    Editable::parse(xcodeproj)
        .ok()
        .is_some_and(|document| find_package(&document, query).is_some())
}

// The package edits, in whichever format the bundle holds. A package id is the
// reference object's GUID in a pbxproj and the package's name in a
// `project.xcproj` — [`DeclaredPackage::id`] carries whichever the document
// uses, and each backend takes back only its own.

fn list_packages(document: &Editable) -> Vec<DeclaredPackage> {
    match document {
        Editable::Pbxproj(root) => spm_pbxproj::list_packages(root),
        Editable::Xcproj(root) => spm_xcproj::list_packages(root),
    }
}

fn find_package(document: &Editable, query: &str) -> Option<String> {
    match document {
        Editable::Pbxproj(root) => spm_pbxproj::find_package(root, query),
        Editable::Xcproj(root) => spm_xcproj::find_package(root, query),
    }
}

fn add_remote(
    document: &mut Editable,
    url: &str,
    spec: &RequirementSpec,
) -> Result<String, CliError> {
    match document {
        Editable::Pbxproj(root) => spm_pbxproj::add_remote_dependency(root, url, spec),
        Editable::Xcproj(root) => spm_xcproj::add_remote_dependency(root, url, spec),
    }
    .map_err(CliError::new)
}

fn add_local(document: &mut Editable, relative_path: &str) -> Result<String, CliError> {
    match document {
        Editable::Pbxproj(root) => spm_pbxproj::add_local_dependency(root, relative_path),
        Editable::Xcproj(root) => spm_xcproj::add_local_dependency(root, relative_path),
    }
    .map_err(CliError::new)
}

fn link_product(
    document: &mut Editable,
    package_id: &str,
    product: &str,
    target: &str,
) -> CliResult {
    match document {
        Editable::Pbxproj(root) => spm_pbxproj::link_product(root, package_id, product, target),
        Editable::Xcproj(root) => spm_xcproj::link_product(root, package_id, product, target),
    }
    .map_err(CliError::new)
}

fn set_requirement(document: &mut Editable, package_id: &str, spec: &RequirementSpec) -> CliResult {
    match document {
        Editable::Pbxproj(root) => spm_pbxproj::set_requirement(root, package_id, spec),
        Editable::Xcproj(root) => spm_xcproj::set_requirement(root, package_id, spec),
    }
    .map_err(CliError::new)
}

fn remove_package(document: &mut Editable, package_id: &str, orphans: &[String]) -> CliResult {
    match document {
        Editable::Pbxproj(root) => spm_pbxproj::remove_package(root, package_id, orphans),
        Editable::Xcproj(root) => spm_xcproj::remove_package(root, package_id, orphans),
    }
    .map_err(CliError::new)
}

fn unlink(
    document: &mut Editable,
    package_id: &str,
    product: Option<&str>,
    target: Option<&str>,
) -> Result<Vec<(String, String)>, CliError> {
    match document {
        Editable::Pbxproj(root) => spm_pbxproj::unlink(root, package_id, product, target),
        Editable::Xcproj(root) => spm_xcproj::unlink(root, package_id, product, target),
    }
    .map_err(CliError::new)
}

/// A crash-safe pristine copy of a file about to be mutated in multiple steps.
/// The in-memory rollback covers every *returned* error, but a signal between
/// the first write and the rollback `_exit`s past it — the sibling backup
/// survives, and [`heal_interrupted_mutation`] restores it at the start of the
/// next dependency command. Success (or a completed rollback) removes it.
struct MutationBackup {
    backup: PathBuf,
}

impl MutationBackup {
    fn create(target: &Path, pristine: &str) -> Result<Self, CliError> {
        let backup = mutation_backup_path(target);
        std::fs::write(&backup, pristine)
            .map_err(|e| CliError::new(format!("failed to write {}: {e}", backup.display())))?;
        Ok(Self { backup })
    }

    /// The mutation landed (or was rolled back in memory): drop the backup.
    fn commit(self) {
        let _ = std::fs::remove_file(&self.backup);
    }
}

fn mutation_backup_path(target: &Path) -> PathBuf {
    let mut name = target.file_name().unwrap_or_default().to_os_string();
    name.push(".sweetpad-rollback");
    target.with_file_name(name)
}

/// Restore a backup a signal-interrupted earlier run left behind (see
/// [`MutationBackup`]), so a half-applied dependency edit heals instead of
/// festering as a dangling package reference.
fn heal_interrupted_mutation(target: &Path, out: &Output) {
    let backup = mutation_backup_path(target);
    if !backup.exists() {
        return;
    }
    match std::fs::read_to_string(&backup) {
        Ok(text) if pbxedit::write_atomic(target, &text).is_ok() => {
            let _ = std::fs::remove_file(&backup);
            out.warn(&format!(
                "restored {} (an earlier dependency edit was interrupted mid-run)",
                target.display()
            ));
        }
        _ => out.warn(&format!(
            "{} exists but could not be restored — inspect it manually",
            backup.display()
        )),
    }
}

/// Path to the local package directory, relative to the directory holding
/// `document` (an `.xcodeproj` or a `Package.swift`) — how both project
/// document formats and SwiftPM record a local package.
fn local_relative_path(document: &Path, url: &str) -> Result<String, CliError> {
    let target = PathBuf::from(url);
    if !target.exists() {
        return Err(CliError::new(format!(
            "local package path '{url}' does not exist"
        )));
    }
    // A bare `--project App.xcodeproj` has parent `Some("")`, not `None` —
    // and canonicalizing "" fails, which would write an absolute (machine-
    // specific) relativePath into the project.
    let proj_dir = match document.parent() {
        Some(p) if !p.as_os_str().is_empty() => p,
        _ => Path::new("."),
    };
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

/// The pid-keyed `-clonedSourcePackagesDirPath` that `add`'s product discovery
/// and [`repin`] resolve into. Dropping it removes the checkouts (a full source
/// clone of the package graph, often hundreds of MB) and the empty lock files
/// SwiftPM creates beside it in the temp directory for each path it locks
/// inside, named after that path both as given and with symlinks resolved
/// (`_var_folders_…_T_sweetpad-spm-<pid>_Package.resolved.lock`,
/// `_private_var_folders_…`), which nothing else clears.
struct CloneDir {
    path: PathBuf,
}

impl CloneDir {
    fn new() -> Self {
        Self {
            path: std::env::temp_dir().join(format!("sweetpad-spm-{}", std::process::id())),
        }
    }
}

impl Drop for CloneDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
        let (Some(temp), Some(name)) = (self.path.parent(), self.path.file_name()) else {
            return;
        };
        let locked_inside = format!("_{}_", name.to_string_lossy());
        let Ok(entries) = std::fs::read_dir(temp) else {
            return;
        };
        for entry in entries.flatten() {
            let file = entry.file_name();
            let file = file.to_string_lossy();
            if file.ends_with(".lock") && file.contains(&locked_inside) {
                let _ = std::fs::remove_file(entry.path());
            }
        }
    }
}

/// The pid-keyed `-resultBundlePath` every xcodebuild resolve passes. A failed
/// resolve writes a result bundle, and given no path xcodebuild writes it to
/// `ResultBundle_<date>.xcresult` in the temp directory, where nothing clears
/// it. Dropping this removes the bundle. xcodebuild refuses a path that
/// already exists (Xcode 27.0), so one an interrupted run left behind is
/// removed first.
struct ResolveBundle {
    path: PathBuf,
}

impl ResolveBundle {
    fn new() -> Self {
        let path =
            std::env::temp_dir().join(format!("sweetpad-resolve-{}.xcresult", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        Self { path }
    }
}

impl Drop for ResolveBundle {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

/// Locate a resolved package's checkout under `base` — a cloned-source-packages
/// dir or a `.build` dir holding `workspace-state.json` + `checkouts/`. Prefers
/// the precise identity→subpath map in `workspace-state.json` (robust to
/// monorepo sub-paths and case differences), falling back to a basename guess.
fn resolve_checkout(base: &Path, url: &str) -> Option<PathBuf> {
    let identity = identity_from_url(url);
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
    let id = identity_from_url(url);
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
    if ctx.out.is_json() || ctx.out.is_ndjson() {
        ctx.out.result_value(&serde_json::json!({
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
    if ctx.out.is_json() || ctx.out.is_ndjson() {
        let payload = match unlinked {
            None => serde_json::json!({ "removed": package }),
            Some(links) => serde_json::json!({
                "unlinked": links.iter()
                    .map(|(product, target)| serde_json::json!({ "product": product, "target": target }))
                    .collect::<Vec<_>>(),
            }),
        };
        ctx.out.result_value(&payload);
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
    std::fs::read_to_string(path)
        .map(|text| parse_resolved(&text))
        .unwrap_or_default()
}

/// [`read_resolved`] over text already in hand.
fn parse_resolved(text: &str) -> HashMap<String, Pin> {
    let mut map = HashMap::new();
    let Ok(json) = serde_json::from_str::<serde_json::Value>(text) else {
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
        // v2/v3 pins carry `identity`; v1 pins have no such key — derive it
        // from `repositoryURL` (or the `package` display name) instead of
        // silently skipping every v1 pin the branch above went to the
        // trouble of finding.
        let identity = pin
            .get("identity")
            .and_then(serde_json::Value::as_str)
            .map(str::to_string)
            .or_else(|| {
                pin.get("repositoryURL")
                    .or_else(|| pin.get("package"))
                    .and_then(serde_json::Value::as_str)
                    .map(identity_from_url)
            });
        let Some(identity) = identity else {
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
                    .or_else(|| pin.get("repositoryURL"))
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_string),
            },
        );
    }
    map
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lockfile(pins: &[(&str, &str)]) -> String {
        let pins: Vec<serde_json::Value> = pins
            .iter()
            .map(|(identity, version)| {
                serde_json::json!({
                    "identity": identity,
                    "kind": "remoteSourceControl",
                    "location": format!("https://github.com/apple/{identity}.git"),
                    "state": { "revision": format!("rev-{version}"), "version": version },
                })
            })
            .collect();
        sweetpad_lib::spm_resolved::serialize(&serde_json::json!({
            "originHash": "fresh",
            "pins": pins,
            "version": 3,
        }))
    }

    #[test]
    fn the_updated_package_leads_the_pins_that_moved() {
        let before = lockfile(&[
            ("swift-algorithms", "1.0.0"),
            ("swift-collections", "1.0.0"),
            ("swift-numerics", "1.0.2"),
        ]);
        let after = lockfile(&[
            ("swift-algorithms", "1.2.1"),
            ("swift-collections", "1.0.0"),
            ("swift-numerics", "1.1.1"),
        ]);
        let changes = pin_changes(
            Some(&before),
            &after,
            Some("https://github.com/apple/swift-numerics.git"),
        );
        let shown: Vec<String> = changes.iter().map(PinChange::display).collect();
        assert_eq!(
            shown,
            [
                "swift-numerics 1.0.2 → 1.1.1",
                "swift-algorithms 1.0.0 → 1.2.1"
            ]
        );
    }

    #[test]
    fn a_pin_that_appears_or_goes_is_a_change() {
        let before = lockfile(&[("swift-numerics", "1.0.2")]);
        let after = lockfile(&[("swift-algorithms", "1.2.1")]);
        let shown: Vec<String> = pin_changes(Some(&before), &after, None)
            .iter()
            .map(PinChange::display)
            .collect();
        assert_eq!(
            shown,
            ["swift-algorithms — → 1.2.1", "swift-numerics 1.0.2 → —"]
        );
        assert_eq!(pin_changes(None, &after, None).len(), 1);
    }

    /// Both kinds of project read an added package's products through this,
    /// so neither opens the product picker with nothing in it.
    #[test]
    fn a_package_with_no_products_has_nothing_to_link() {
        let manifest: swiftpm::Manifest =
            serde_json::from_str(r#"{"name":"Empty","products":[],"targets":[]}"#).unwrap();
        let err = products_to_link(&manifest).unwrap_err();
        assert_eq!(err.to_string(), "the package declares no products to link");
    }

    #[test]
    fn a_picker_with_nothing_to_offer_is_an_error() {
        let err = multi_select("target", &[], false).unwrap_err();
        assert_eq!(err.to_string(), "there are no targets to choose from");
        assert_eq!(err.error_kind(), ErrorKind::Generic);
    }

    #[test]
    fn identical_lockfiles_have_no_changes() {
        let pins = lockfile(&[("swift-collections", "1.1.4")]);
        assert_eq!(pin_changes(Some(&pins), &pins, None), []);
    }

    /// The lock names a discovery `dep add` left behind on Xcode 27.0, with
    /// the temp path shortened.
    #[test]
    fn dropping_the_clone_dir_removes_its_lock_files() {
        let temp = std::env::temp_dir().join(format!("sweetpad-clonedir-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&temp);
        let clone = CloneDir {
            path: temp.join("sweetpad-spm-7"),
        };
        std::fs::create_dir_all(clone.path.join("checkouts/swift-collections")).unwrap();
        let locks = [
            "_private_var_folders_T_sweetpad-spm-7_Package.resolved.lock",
            "_private_var_folders_T_sweetpad-spm-7_workspace-state.json.lock",
            "_private_var_folders_T_sweetpad-spm-7_checkouts_swift-collections_.build.lock",
            "_var_folders_T_sweetpad-spm-7_Package.resolved.lock",
        ];
        let unrelated = [
            "_private_var_folders_T_sweetpad-spm-71_Package.resolved.lock",
            "_private_var_folders_T_sweetpad-spm-7_notes.txt",
        ];
        for name in locks.iter().chain(&unrelated) {
            std::fs::write(temp.join(name), "").unwrap();
        }
        drop(clone);
        let mut left: Vec<String> = std::fs::read_dir(&temp)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        left.sort();
        let _ = std::fs::remove_dir_all(&temp);
        assert_eq!(left, unrelated);
    }

    /// xcodebuild refuses a `-resultBundlePath` that exists, so a bundle an
    /// interrupted run left at this process's path goes before the resolve.
    #[test]
    fn a_resolve_bundle_path_starts_absent_and_goes_on_drop() {
        let path =
            std::env::temp_dir().join(format!("sweetpad-resolve-{}.xcresult", std::process::id()));
        std::fs::create_dir_all(path.join("Data")).unwrap();
        let bundle = ResolveBundle::new();
        assert_eq!(bundle.path, path);
        assert!(!path.exists(), "the leftover bundle is still there");
        std::fs::create_dir_all(path.join("Data")).unwrap();
        drop(bundle);
        assert!(!path.exists(), "the resolve's bundle is still there");
    }
}
