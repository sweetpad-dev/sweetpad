// The hand-rolled JSON reader below uses `while let Some(_) = chars.next()`
// in two places where the inner break/continue can't easily be a `for`.
#![allow(clippy::while_let_on_iterator)]

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use sweetpad::xcspec::load_catalog;

fn project_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn xcspec_root() -> PathBuf {
    project_root().join("xcspec-cache/xcode-26.5.0")
}

fn sdksettings_root() -> PathBuf {
    project_root().join("xcspec-cache/xcode-26.5.0/sdksettings")
}

/// Parse the buildSettings dict out of a captured `xcodebuild -showBuildSettings -json`
/// fixture. We avoid pulling in serde just for this — the format is shallow enough
/// to read by hand.
fn read_oracle_settings(path: &PathBuf) -> BTreeMap<String, String> {
    let s = fs::read_to_string(path).unwrap();
    // The oracle is a JSON array of objects; we want the first object's
    // `"buildSettings"` dict.
    let marker = "\"buildSettings\"";
    let start = s.find(marker).expect("oracle missing buildSettings");
    let after = &s[start + marker.len()..];
    let open = after.find('{').expect("missing buildSettings open brace");
    let body = &after[open + 1..];

    let mut out = BTreeMap::new();
    let mut chars = body.char_indices().peekable();
    while let Some(&(_, c)) = chars.peek() {
        if c.is_whitespace() || c == ',' {
            chars.next();
            continue;
        }
        if c == '}' {
            break;
        }
        if c != '"' {
            break;
        }
        chars.next();
        let mut key = String::new();
        while let Some((_, c)) = chars.next() {
            if c == '"' {
                break;
            }
            key.push(c);
        }
        // Skip whitespace and colon
        while let Some(&(_, c)) = chars.peek() {
            if c.is_whitespace() || c == ':' {
                chars.next();
            } else {
                break;
            }
        }
        if chars.next().map(|(_, c)| c) != Some('"') {
            break;
        }
        let mut val = String::new();
        while let Some((_, c)) = chars.next() {
            if c == '\\' {
                if let Some((_, esc)) = chars.next() {
                    val.push(match esc {
                        'n' => '\n',
                        't' => '\t',
                        '"' => '"',
                        '\\' => '\\',
                        '/' => '/',
                        other => other,
                    });
                }
                continue;
            }
            if c == '"' {
                break;
            }
            val.push(c);
        }
        out.insert(key, val);
    }
    out
}

#[test]
fn catalog_loads_with_substantial_breadth() {
    let catalog = load_catalog(&xcspec_root(), Some(&sdksettings_root())).unwrap();
    // Captured Xcode 26.5.0 holds 333 xcspec files; the resulting catalog
    // should be in the multi-thousand range across universal + per-type.
    assert!(
        catalog.assignment_count() > 2000,
        "expected catalog with thousands of assignments, got {}",
        catalog.assignment_count()
    );
    assert!(
        catalog.universal.len() > 500,
        "expected 500+ universal defaults, got {}",
        catalog.universal.len()
    );
    assert!(
        catalog.product_types.len() > 10,
        "expected 10+ product types, got {}",
        catalog.product_types.len()
    );
    assert_eq!(
        catalog.sdks.len(),
        10,
        "expected exactly 10 SDKs in the captured fixture"
    );
}

#[test]
fn macos_layer_contains_canonical_apple_defaults() {
    let catalog = load_catalog(&xcspec_root(), Some(&sdksettings_root())).unwrap();
    let layer = catalog.layer_for(Some("com.apple.product-type.tool"), Some("macosx26.5"));
    let key_set: std::collections::BTreeSet<&str> = layer.iter().map(|a| a.key.as_str()).collect();
    // A canonical sample drawn from the captured oracle — these come from
    // BuildSystem.Options/Properties and SDKSettings.DefaultProperties.
    for k in [
        "ALWAYS_SEARCH_USER_PATHS",
        "BUNDLE_FORMAT",
        "CLANG_ENABLE_EXPLICIT_MODULES",
        "DEAD_CODE_STRIPPING",
        "LLVM_TARGET_TRIPLE_VENDOR",
        "ONLY_ACTIVE_ARCH",
        "PLATFORM_NAME",
        "PROJECT_NAME",
        "SDKROOT",
        "TARGET_NAME",
    ] {
        assert!(
            key_set.contains(k),
            "expected `{k}` in macOS+tool defaults; keys: {} of them",
            key_set.len()
        );
    }
}

/// End-to-end coverage check: the full resolver pipeline (defaults + Scratch
/// project settings) should reproduce the *user-authored* portion of
/// `xcodebuild -showBuildSettings` for the Scratch fixture, plus give us a
/// substantial share of the keys it normally fills in from xcspec defaults.
#[test]
#[allow(clippy::too_many_lines)]
fn scratch_resolves_against_captured_oracle_with_decent_coverage() {
    use sweetpad::xcconfig::Assignment;
    use sweetpad::{project, resolver, xcspec};

    let xcodeproj =
        project_root().join("_synthetic-xcconfigs/xcode-26.5.0/project/Scratch.xcodeproj");
    let xcodeproj_path = if xcodeproj.exists() {
        xcodeproj
    } else {
        project_root().join("fixtures/_synthetic-xcconfigs/xcode-26.5.0/project/Scratch.xcodeproj")
    };
    let bundle = project::build_settings(&xcodeproj_path, "Scratch", "Debug").unwrap();
    let catalog = xcspec::load_catalog(&xcspec_root(), Some(&sdksettings_root())).unwrap();
    let defaults = catalog.layer_for(bundle.product_type.as_deref(), Some("macosx26.5"));
    let built_in = project::built_in_settings(
        &xcodeproj_path,
        "Scratch",
        "Debug",
        bundle.product_type.as_deref(),
        "macosx26.5",
        None,
        false,
        false,
        None,
        None,
        &std::collections::BTreeMap::new(),
        None,
        catalog.xcode_version.as_deref(),
        catalog.developer_dir.as_deref(),
        None,
        false,
    );

    let mut layers: Vec<Vec<Assignment>> = vec![defaults, built_in];
    layers.extend(bundle.layers);
    let layer_refs: Vec<&[Assignment]> = layers.iter().map(Vec::as_slice).collect();
    let ctx = resolver::ResolveContext {
        sdk: "macosx".into(),
        arch: "arm64".into(),
        configuration: "Debug".into(),
        variant: String::new(),
    };
    let resolved = resolver::resolve(&layer_refs, &ctx);

    let oracle =
        read_oracle_settings(&project_root().join(
            "fixtures/_synthetic-xcconfigs/xcode-26.5.0/captures/conditional-sdk/without.json",
        ));
    let shared: std::collections::BTreeSet<&String> = resolved
        .keys()
        .filter(|k| oracle.contains_key(*k))
        .collect();

    // At least 70% of oracle keys should be present in our resolved output.
    // (Empirically this hits ~75% with the full xcspec catalog + built-ins;
    // the slack absorbs environmental drift across Xcode versions.)
    let overlap_pct = shared.len() * 100 / oracle.len();
    assert!(
        overlap_pct >= 70,
        "expected at least 70% of oracle keys to be present; got {overlap_pct}% \
         (shared={}, oracle={})",
        shared.len(),
        oracle.len()
    );

    // Of the keys we *do* produce that the oracle also has, at least 80% of
    // the values should match exactly (raw byte-equal — this test deliberately
    // does NOT canonicalize). The Scratch oracle was captured with a
    // project-local `build/` dir (legacy `-derivedDataPath` flag); our resolver
    // emits Xcode's default DerivedData layout, so the whole path-anchored
    // setting family (BUILD_DIR/OBJROOT/*_SEARCH_PATHS) can't byte-match on this
    // single synthetic fixture. The canonicalizing corpus oracles absorb that
    // drift (structural ~99%); here it caps the raw-exact rate at ~82%. Floor is
    // data-driven (observed 82%, was 80% before the XCODE_VERSION_* fix) minus a
    // small margin, so it guards regressions without over-fitting.
    let exact_value_matches = shared
        .iter()
        .filter(|k| resolved.get(**k) == oracle.get(**k))
        .count();
    let value_match_pct = exact_value_matches * 100 / shared.len();
    assert!(
        value_match_pct >= 80,
        "expected at least 80% exact value match on shared keys; got \
         {value_match_pct}% ({exact_value_matches}/{})",
        shared.len()
    );

    // Spot-check PROJECT_DIR (unchanged: still derived from the project's
    // location on disk) and that BUILD_DIR + BUILT_PRODUCTS_DIR are now
    // emitted in DerivedData layout (`~/Library/Developer/Xcode/DerivedData/
    // <Container>-<HASH>/Build/Products[/Config]`).
    let canonical = std::fs::canonicalize(&xcodeproj_path).unwrap();
    let canonical_project_dir = canonical.parent().unwrap().display().to_string();
    assert_eq!(
        resolved.get("PROJECT_DIR").map(String::as_str),
        Some(canonical_project_dir.as_str())
    );
    let build_dir = resolved.get("BUILD_DIR").map(String::as_str).unwrap();
    assert!(
        build_dir.contains("/Library/Developer/Xcode/DerivedData/Scratch-")
            && build_dir.ends_with("/Build/Products"),
        "BUILD_DIR should be a DerivedData path; got {build_dir}"
    );
    let built_products = resolved
        .get("BUILT_PRODUCTS_DIR")
        .map(String::as_str)
        .unwrap();
    assert!(
        built_products.contains("/Library/Developer/Xcode/DerivedData/Scratch-")
            && built_products.ends_with("/Build/Products/Debug"),
        "BUILT_PRODUCTS_DIR should be `<DerivedData>/Build/Products/Debug` \
         on macOS; got {built_products}"
    );
    // PACKAGE_TYPE is pulled from the leaf product-type's xcspec.
    assert_eq!(
        resolved.get("PACKAGE_TYPE").map(String::as_str),
        Some("com.apple.package-type.mach-o-executable")
    );

    // The user-authored settings from Scratch's pbxproj must survive at
    // their original values regardless of how many defaults we layered
    // underneath.
    for (key, want) in [
        ("ALWAYS_SEARCH_USER_PATHS", "NO"),
        ("MACOSX_DEPLOYMENT_TARGET", "12.0"),
        ("PRODUCT_NAME", "Scratch"),
        ("SDKROOT", "macosx"),
        ("SWIFT_VERSION", "5.0"),
    ] {
        assert_eq!(
            resolved.get(key).map(String::as_str),
            Some(want),
            "user-authored `{key}` should not be overridden by defaults"
        );
    }

    // Spot-check that nested indirect lookups resolve. BUNDLE_CONTENTS_FOLDER_PATH
    // is `$(BUNDLE_CONTENTS_FOLDER_PATH_$(BUNDLE_FORMAT))` and BUNDLE_FORMAT
    // resolves through the macOS xcspec override to "deep", so the final
    // value should be `Contents/`.
    assert_eq!(
        resolved
            .get("BUNDLE_CONTENTS_FOLDER_PATH")
            .map(String::as_str),
        Some("Contents/"),
        "nested indirect lookup BUNDLE_CONTENTS_FOLDER_PATH_$(BUNDLE_FORMAT) \
         should resolve to Contents/ for a macOS target"
    );
}
