//! Scratch diagnostic: resolve a (project, target, config, sdk, arch) tuple
//! against a versioned xcspec cache and print selected keys (or all).
//! Usage: cargo run --example dump_settings -- <xcodeproj> <target> <config> <sdk> <arch> <xcspec-ver> [KEY ...]

use sweetpad_core::build_context::{BuildContext, ResolveQuery};
use sweetpad_lib::xcspec;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (proj, target, config, sdk, arch, ver) =
        (&args[0], &args[1], &args[2], &args[3], &args[4], &args[5]);
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let xcspec_root = root.join(format!("xcspec-cache/xcode-{ver}"));
    let sdks = xcspec_root.join("sdksettings");
    let catalog = xcspec::load_catalog(&xcspec_root, Some(&sdks)).unwrap();
    let ctx = BuildContext::open(std::path::Path::new(proj))
        .unwrap()
        .with_xcspec(catalog);
    let resolved = ctx
        .resolve(&ResolveQuery::new(target, config, sdk, arch))
        .unwrap();
    let keys: Vec<&String> = args.iter().skip(6).collect();
    for (k, v) in &resolved.settings {
        if keys.is_empty() || keys.contains(&k) {
            println!("{k} = {v:?}");
        }
    }
}
