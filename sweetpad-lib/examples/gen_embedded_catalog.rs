//! Regenerate the catalog blob baked into the binary (`src/catalog_embedded.bin`).
//!
//! Run after refreshing the corpus to a newer Xcode (see
//! `DOCS.md` §10 (updating Xcode versions)). It parses the chosen `xcspec-cache/xcode-<ver>`
//! into a [`Catalog`] and serializes it with [`catalog_cache::serialize`].
//!
//! ```sh
//! cargo run --release --example gen_embedded_catalog            # defaults to latest below
//! cargo run --release --example gen_embedded_catalog -- 26.5.0  # explicit version
//! ```
//!
//! The fingerprint is written as `0`: the embedded blob isn't tied to an
//! on-disk source, so [`catalog_cache::embedded`] ignores it.

#![allow(clippy::cast_precision_loss)] // human-facing KB readout in a dev tool

use std::path::{Path, PathBuf};

use sweetpad_lib::catalog_cache;
use sweetpad_lib::xcspec;

/// The version baked in by default — keep pointed at the newest captured major.
const DEFAULT_VERSION: &str = "26.5.0";

fn main() {
    let version = std::env::args()
        .nth(1)
        .unwrap_or_else(|| DEFAULT_VERSION.to_string());
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let root = manifest.join(format!("xcspec-cache/xcode-{version}"));
    let sdksettings = root.join("sdksettings");
    let out = manifest.join("src/catalog_embedded.bin");

    assert!(
        root.is_dir(),
        "no xcspec cache for {version} at {}",
        root.display()
    );

    let catalog = xcspec::load_catalog(&root, Some(&sdksettings))
        .unwrap_or_else(|e| panic!("parse {}: {e}", root.display()));
    let bytes = catalog_cache::serialize(&catalog, 0);
    std::fs::write(&out, &bytes).unwrap_or_else(|e| panic!("write {}: {e}", out.display()));

    let assignments: usize = catalog.universal.len()
        + catalog
            .domain_specific
            .values()
            .map(Vec::len)
            .sum::<usize>()
        + catalog.sdks.values().map(Vec::len).sum::<usize>()
        + catalog
            .product_types
            .values()
            .map(|p| p.defaults.len())
            .sum::<usize>();
    println!(
        "wrote {} ({:.1} KB) from Xcode {version}: {} assignments, {} product types",
        rel(&out, &manifest),
        bytes.len() as f64 / 1024.0,
        assignments,
        catalog.product_types.len(),
    );
}

fn rel(path: &Path, base: &Path) -> String {
    path.strip_prefix(base)
        .unwrap_or(path)
        .to_string_lossy()
        .into_owned()
}
