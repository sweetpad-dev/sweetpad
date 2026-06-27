//! Shared scaffolding for the snapshot-oracle integration tests.
//!
//! `corpus_oracle.rs` and the four per-source oracle tests all do the same
//! thing: read a captured `xcodebuild -showBuildSettings` JSON, resolve the
//! matching (target, config, sdk, arch[, override]) with our resolver, and
//! score every shared key into three tiers — exact (byte-equal), canonical
//! (equal after [`canonicalize_value`] strips the volatile machine-specific
//! path segments), and structural (both sides absolute paths). This module
//! holds everything common to all five: the hand-rolled JSON reader, the
//! canonicalizer, the corpus directory walk + project lookup, the [`Stats`]
//! accumulator, and the [`compare`] core that classifies one resolved map
//! against one oracle map.

#![allow(
    clippy::while_let_on_iterator,
    clippy::too_many_lines,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_possible_wrap,
    clippy::needless_lifetimes,
    clippy::manual_let_else,
    clippy::if_same_then_else,
    clippy::collapsible_if,
    clippy::collapsible_else_if,
    clippy::branches_sharing_code,
    dead_code
)]

use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};

use sweetpad_lib::xcspec;

/// Argv comparator for the compiler-args oracle (see `DOCS.md` §7 (compiler arguments)).
/// Reuses this module's [`canonicalize_value`] and [`MismatchTally`].
pub mod argv;

// ----- tiny JSON reader ----------------------------------------------------

#[derive(Debug)]
pub enum JsonValue {
    String(String),
    Number(String),
    Bool(bool),
    Null,
    Array(Vec<JsonValue>),
    Object(BTreeMap<String, JsonValue>),
}

impl JsonValue {
    pub fn as_string(&self) -> Option<&str> {
        if let JsonValue::String(s) = self {
            Some(s)
        } else {
            None
        }
    }

    pub fn as_object(&self) -> Option<&BTreeMap<String, JsonValue>> {
        if let JsonValue::Object(o) = self {
            Some(o)
        } else {
            None
        }
    }

    pub fn as_array(&self) -> Option<&[JsonValue]> {
        if let JsonValue::Array(a) = self {
            Some(a)
        } else {
            None
        }
    }
}

struct JsonParser<'a> {
    s: &'a [u8],
    pos: usize,
}

pub fn parse_json(input: &str) -> Result<JsonValue, String> {
    let mut p = JsonParser {
        s: input.as_bytes(),
        pos: 0,
    };
    p.skip_ws();
    let v = p.value()?;
    p.skip_ws();
    if p.peek().is_some() {
        return Err("trailing data".into());
    }
    Ok(v)
}

impl JsonParser<'_> {
    fn peek(&self) -> Option<u8> {
        self.s.get(self.pos).copied()
    }

    fn advance(&mut self) -> Option<u8> {
        let b = self.peek();
        if b.is_some() {
            self.pos += 1;
        }
        b
    }

    fn skip_ws(&mut self) {
        while let Some(b) = self.peek() {
            if matches!(b, b' ' | b'\t' | b'\n' | b'\r') {
                self.pos += 1;
            } else {
                break;
            }
        }
    }

    fn value(&mut self) -> Result<JsonValue, String> {
        self.skip_ws();
        match self.peek() {
            Some(b'"') => self.string(),
            Some(b'[') => self.array(),
            Some(b'{') => self.object(),
            Some(b't') => self.literal("true", JsonValue::Bool(true)),
            Some(b'f') => self.literal("false", JsonValue::Bool(false)),
            Some(b'n') => self.literal("null", JsonValue::Null),
            Some(_) => self.number(),
            None => Err("unexpected EOF".into()),
        }
    }

    fn string(&mut self) -> Result<JsonValue, String> {
        if self.advance() != Some(b'"') {
            return Err("expected \"".into());
        }
        let mut s = String::new();
        loop {
            match self.advance() {
                Some(b'"') => return Ok(JsonValue::String(s)),
                Some(b'\\') => match self.advance() {
                    Some(b'"') => s.push('"'),
                    Some(b'\\') => s.push('\\'),
                    Some(b'/') => s.push('/'),
                    Some(b'n') => s.push('\n'),
                    Some(b't') => s.push('\t'),
                    Some(b'r') => s.push('\r'),
                    Some(b'b') => s.push('\x08'),
                    Some(b'f') => s.push('\x0c'),
                    Some(b'u') => {
                        let mut code: u32 = 0;
                        for _ in 0..4 {
                            let d = self
                                .advance()
                                .ok_or_else(|| "bad unicode escape".to_string())?;
                            let v: u32 = match d {
                                b'0'..=b'9' => u32::from(d - b'0'),
                                b'a'..=b'f' => u32::from(d - b'a' + 10),
                                b'A'..=b'F' => u32::from(d - b'A' + 10),
                                _ => return Err("bad hex digit".into()),
                            };
                            code = code * 16 + v;
                        }
                        if let Some(c) = char::from_u32(code) {
                            s.push(c);
                        }
                    }
                    Some(b) => s.push(b as char),
                    None => return Err("bad escape".into()),
                },
                Some(b) => {
                    if b < 0x80 {
                        s.push(b as char);
                    } else {
                        self.pos -= 1;
                        let len = utf8_len(b);
                        let bytes = &self.s[self.pos..self.pos + len];
                        s.push_str(
                            std::str::from_utf8(bytes).map_err(|_| "bad UTF-8".to_string())?,
                        );
                        self.pos += len;
                    }
                }
                None => return Err("unterminated string".into()),
            }
        }
    }

    fn array(&mut self) -> Result<JsonValue, String> {
        self.pos += 1; // '['
        let mut items = Vec::new();
        loop {
            self.skip_ws();
            if self.peek() == Some(b']') {
                self.pos += 1;
                return Ok(JsonValue::Array(items));
            }
            items.push(self.value()?);
            self.skip_ws();
            match self.peek() {
                Some(b',') => {
                    self.pos += 1;
                }
                Some(b']') => {
                    self.pos += 1;
                    return Ok(JsonValue::Array(items));
                }
                _ => return Err("expected , or ] in array".into()),
            }
        }
    }

    fn object(&mut self) -> Result<JsonValue, String> {
        self.pos += 1; // '{'
        let mut map = BTreeMap::new();
        loop {
            self.skip_ws();
            if self.peek() == Some(b'}') {
                self.pos += 1;
                return Ok(JsonValue::Object(map));
            }
            let key = match self.string()? {
                JsonValue::String(s) => s,
                _ => return Err("non-string key".into()),
            };
            self.skip_ws();
            if self.advance() != Some(b':') {
                return Err("expected :".into());
            }
            let value = self.value()?;
            map.insert(key, value);
            self.skip_ws();
            match self.peek() {
                Some(b',') => {
                    self.pos += 1;
                }
                Some(b'}') => {
                    self.pos += 1;
                    return Ok(JsonValue::Object(map));
                }
                _ => return Err("expected , or } in object".into()),
            }
        }
    }

    fn literal(&mut self, lit: &str, val: JsonValue) -> Result<JsonValue, String> {
        for &b in lit.as_bytes() {
            if self.advance() != Some(b) {
                return Err(format!("expected {lit}"));
            }
        }
        Ok(val)
    }

    fn number(&mut self) -> Result<JsonValue, String> {
        let start = self.pos;
        while let Some(b) = self.peek() {
            if matches!(b, b'0'..=b'9' | b'.' | b'-' | b'+' | b'e' | b'E') {
                self.pos += 1;
            } else {
                break;
            }
        }
        let s =
            std::str::from_utf8(&self.s[start..self.pos]).map_err(|_| "bad number".to_string())?;
        Ok(JsonValue::Number(s.to_string()))
    }
}

fn utf8_len(b: u8) -> usize {
    if b < 0x80 {
        1
    } else if b < 0xC0 {
        1
    } else if b < 0xE0 {
        2
    } else if b < 0xF0 {
        3
    } else {
        4
    }
}

/// Parse a captured `-showBuildSettings -json` file and return the
/// `buildSettings` maps of every entry, as plain `String->String` maps
/// (non-string values are dropped, matching the oracle comparison). Returns
/// `None` if the file is unreadable, isn't a non-empty array, or any entry
/// lacks a `buildSettings` object.
pub fn read_build_settings(path: &Path) -> Option<Vec<BTreeMap<String, String>>> {
    let json = parse_json(&fs::read_to_string(path).ok()?).ok()?;
    let arr = json.as_array()?;
    if arr.is_empty() {
        return None;
    }
    let mut out = Vec::new();
    for entry in arr {
        let bs = entry.as_object()?.get("buildSettings")?.as_object()?;
        let mut map = BTreeMap::new();
        for (k, v) in bs {
            if let JsonValue::String(s) = v {
                map.insert(k.clone(), s.clone());
            }
        }
        out.push(map);
    }
    Some(out)
}

// ----- corpus roots --------------------------------------------------------

/// Pin the resolver's host-derived outputs to the corpus capture host: every
/// capture was taken on one Apple Silicon Mac as `hyzyla_home` (verify with
/// `grep '"USER"' fixtures/**/build-settings/*.json`). The `NATIVE_ARCH`
/// family, the `USER` family, and every `$HOME`-anchored DerivedData path are
/// host-derived, so without pinning the oracle scores drop below their
/// calibrated floors on any other machine (x86_64, Linux CI, another user's
/// Mac). Call at the top of every corpus-scoring test; idempotent.
pub fn pin_capture_host() {
    sweetpad_lib::project::set_host_override(sweetpad_lib::project::HostOverride {
        arch: Some("arm64".into()),
        user: Some("hyzyla_home".into()),
        home: Some("/Users/hyzyla_home".into()),
        // The capture host's `confstr(_CS_DARWIN_USER_CACHE_DIR)` — anchors
        // CCHROOT / CACHE_ROOT (verify with
        // `grep -m1 '"CCHROOT"' fixtures/alamofire/**/*.json`).
        darwin_user_cache: Some("/var/folders/wq/kkdk740d68qcqvttr995x13w0000gq/C/".into()),
    });
}

pub fn fixtures_root() -> PathBuf {
    PathBuf::from(env!("SWEETPAD_LIB_DIR")).join("fixtures")
}

/// The xcspec catalog root for a given Xcode version (e.g. `"26.0.1"`):
/// `xcspec-cache/xcode-<version>/`. The corpus keeps one directory per Xcode
/// version under both `fixtures/<slug>/xcode-<ver>/` and `xcspec-cache/`, and a
/// capture must be scored against the catalog from the *same* Xcode — the
/// documented defaults and per-product-type rules drift between majors.
pub fn xcspec_root_for(version: &str) -> PathBuf {
    PathBuf::from(env!("SWEETPAD_LIB_DIR")).join(format!("xcspec-cache/xcode-{version}"))
}

/// The `sdksettings/` subdir of [`xcspec_root_for`] — the per-SDK defaults
/// captured alongside the xcspecs for that same Xcode version.
pub fn sdksettings_root_for(version: &str) -> PathBuf {
    xcspec_root_for(version).join("sdksettings")
}

/// Extract the Xcode version from the `xcode-<ver>` path component every capture
/// and fixture lives under (e.g. `fixtures/alamofire/xcode-26.0.1/metadata/...`
/// → `"26.0.1"`). Returns `None` for a path with no such component, which the
/// oracle loops treat as a skip.
pub fn capture_xcode_version(path: &Path) -> Option<String> {
    path.iter()
        .filter_map(OsStr::to_str)
        .find_map(|c| c.strip_prefix("xcode-"))
        .map(str::to_owned)
}

/// Optional single-version restriction for the oracle tests, read from the
/// `ORACLE_ONLY_VERSION` env var (e.g. `16.4.0`). When set, an oracle test
/// scores ONLY captures from that Xcode version and skips its coverage-floor
/// assertions — turning the run into a per-version diagnostic. Use with
/// `-- --nocapture` to see the systematic-mismatch tally for that one version,
/// isolated from the rest of the corpus.
pub fn only_version() -> Option<String> {
    std::env::var("ORACLE_ONLY_VERSION")
        .ok()
        .filter(|s| !s.is_empty())
}

/// Lazily-loaded, memoized [`xcspec::Catalog`] keyed by Xcode version.
///
/// Each corpus-iterating oracle test scores captures that may span several
/// Xcode versions, and every capture must use the catalog from its own Xcode.
/// Loading a catalog parses a directory of xcspecs, so we parse each version's
/// catalog once and reuse it across every capture from that version — for a
/// single-version corpus this is exactly one load, identical to the old
/// load-once-upfront behaviour.
#[derive(Default)]
pub struct CatalogCache {
    by_version: BTreeMap<String, xcspec::Catalog>,
}

impl CatalogCache {
    pub fn new() -> Self {
        Self::default()
    }

    /// Borrow the catalog for `version`, loading and caching it from
    /// `xcspec-cache/xcode-<version>/` on first use. Panics with a pointed
    /// message if that version's xcspec cache is missing or unreadable — a
    /// capture without its matching catalog is a corpus-setup error, not a
    /// resolver result we want to silently score against the wrong defaults.
    pub fn get(&mut self, version: &str) -> &xcspec::Catalog {
        self.by_version
            .entry(version.to_owned())
            .or_insert_with(|| {
                let root = xcspec_root_for(version);
                let sdks = sdksettings_root_for(version);
                xcspec::load_catalog(&root, Some(&sdks)).unwrap_or_else(|e| {
                    panic!(
                        "failed to load xcspec catalog for Xcode {version} from {}: {e}",
                        root.display()
                    )
                })
            })
    }
}

// ----- corpus walking + project lookup -------------------------------------

/// Walk `dir`, applying `f` to every regular file and to every directory
/// whose name ends in `.xcodeproj` (treated as a leaf — we never descend into
/// it). The volatile build-output dirs (`.derived`, `.cache`, `DerivedData`)
/// are skipped entirely.
pub fn walk<T>(dir: &Path, out: &mut T, f: &impl Fn(&Path, &mut T)) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for e in entries.flatten() {
        let p = e.path();
        let name = p.file_name().and_then(OsStr::to_str);
        if matches!(name, Some(".derived" | ".cache" | "DerivedData")) {
            continue;
        }
        if p.is_dir() && p.extension() == Some(OsStr::new("xcodeproj")) {
            f(&p, out);
            continue;
        }
        if p.is_dir() {
            walk(&p, out, f);
        } else {
            f(&p, out);
        }
    }
}

/// Collect every `*.json` file directly inside a `build-settings/` directory
/// anywhere under `fixtures/`. This is the corpus the scheme-aggregated
/// oracle (`corpus_oracle.rs`) and the synthetic-override oracle consume.
pub fn find_oracles() -> Vec<PathBuf> {
    let mut out = Vec::new();
    walk(&fixtures_root(), &mut out, &|p, out| {
        if p.is_file()
            && p.parent()
                .and_then(|d| d.file_name())
                .and_then(OsStr::to_str)
                == Some("build-settings")
            && p.extension() == Some(OsStr::new("json"))
        {
            out.push(p.to_path_buf());
        }
    });
    out.sort();
    out
}

/// Collect every `*.json` file directly inside a `compiler-args/` directory
/// anywhere under `fixtures/`. The corpus the compiler-args oracle
/// (`compiler_args_oracle.rs`) consumes.
pub fn find_compiler_args_oracles() -> Vec<PathBuf> {
    let mut out = Vec::new();
    walk(&fixtures_root(), &mut out, &|p, out| {
        if p.is_file()
            && p.parent()
                .and_then(|d| d.file_name())
                .and_then(OsStr::to_str)
                == Some("compiler-args")
            && p.extension() == Some(OsStr::new("json"))
        {
            out.push(p.to_path_buf());
        }
    });
    out.sort();
    out
}

/// Collect every JSON file (excluding `.meta.json`) directly inside a
/// directory named `name` anywhere under `fixtures/`. Used to gather the
/// `_per_target`, `_project_defaults`, and `_xcconfig_resolution` captures.
pub fn find_capture_files(parent_dir_name: &str) -> Vec<PathBuf> {
    let mut out = Vec::new();
    walk(&fixtures_root(), &mut out, &|p, out| {
        if p.is_file()
            && p.extension() == Some(OsStr::new("json"))
            && !p.to_string_lossy().ends_with(".meta.json")
            && enclosing_dir_named(p, parent_dir_name)
        {
            out.push(p.to_path_buf());
        }
    });
    out.sort();
    out
}

/// Whether any ancestor directory of `p` is named `name`. The `_per_target`
/// etc. captures live two levels below the named dir (`_per_target/<Proj>/<f>.json`),
/// so we check the whole ancestor chain rather than just the immediate parent.
fn enclosing_dir_named(p: &Path, name: &str) -> bool {
    p.iter().any(|c| c == OsStr::new(name))
}

/// Locate the .xcodeproj that the given scheme-oracle file corresponds to.
///
/// Oracle paths look like:
///   fixtures/<project>/xcode-<ver>/metadata/[<sub>/]schemes/<scheme>/build-settings/<f>.json
///
/// The corresponding `.xcodeproj` lives at the same position relative to a
/// `raw/` sibling of `metadata`. For non-tuist projects the path between
/// `metadata` and `schemes` is empty; for tuist-fixtures there's a sub-fixture
/// directory that we need to follow into `raw/` to disambiguate the dozen-plus
/// projects named `App.xcodeproj`.
pub fn find_xcodeproj_for_oracle(oracle: &Path, project_name: &str) -> Option<PathBuf> {
    find_xcodeproj_between(oracle, "schemes", project_name)
}

/// General form of [`find_xcodeproj_for_oracle`]: re-root the oracle path at
/// the `raw/` sibling of `metadata`, keeping the sub-fixture components that
/// sit between `metadata` and the `marker` directory (`schemes`, `_per_target`,
/// `_project_defaults`, `_xcconfig_resolution`, `_synthetic`, ...), then search
/// that sub-fixture tree for `<project_name>.xcodeproj`.
pub fn find_xcodeproj_between(oracle: &Path, marker: &str, project_name: &str) -> Option<PathBuf> {
    let comps: Vec<&OsStr> = oracle.iter().collect();
    let metadata_idx = comps.iter().rposition(|c| *c == OsStr::new("metadata"))?;
    let marker_idx = comps.iter().rposition(|c| *c == OsStr::new(marker))?;
    if marker_idx <= metadata_idx {
        return None;
    }
    let mut root = PathBuf::new();
    for (i, c) in comps.iter().enumerate() {
        if i < metadata_idx {
            root.push(c);
        } else if i == metadata_idx {
            root.push("raw");
        } else if i > metadata_idx && i < marker_idx {
            root.push(c);
        }
    }
    let target = format!("{project_name}.xcodeproj");
    find_dir_named(&root, &target)
}

pub fn find_file_named(dir: &Path, name: &str) -> Option<PathBuf> {
    if !dir.is_dir() {
        return None;
    }
    let entries = fs::read_dir(dir).ok()?;
    for e in entries.flatten() {
        let p = e.path();
        let basename = p.file_name().and_then(OsStr::to_str);
        if matches!(basename, Some(".derived" | ".cache" | "DerivedData")) {
            continue;
        }
        if p.is_file() && basename == Some(name) {
            return Some(p);
        }
        if p.is_dir() {
            if let Some(found) = find_file_named(&p, name) {
                return Some(found);
            }
        }
    }
    None
}

pub fn find_dir_named(dir: &Path, name: &str) -> Option<PathBuf> {
    if !dir.is_dir() {
        return None;
    }
    let entries = fs::read_dir(dir).ok()?;
    for e in entries.flatten() {
        let p = e.path();
        let basename = p.file_name().and_then(OsStr::to_str);
        if matches!(basename, Some(".derived" | ".cache" | "DerivedData")) {
            continue;
        }
        if basename == Some(name) && p.is_dir() {
            return Some(p);
        }
        if p.is_dir() && p.extension() != Some(OsStr::new("xcodeproj")) {
            if let Some(found) = find_dir_named(&p, name) {
                return Some(found);
            }
        }
    }
    None
}

// ----- Stats + comparison core ---------------------------------------------

#[derive(Default, Clone, Copy)]
pub struct Stats {
    pub files: u64,
    pub oracle_keys: u64,
    pub our_keys: u64,
    pub shared_keys: u64,
    /// Byte-for-byte identical values.
    pub exact_matches: u64,
    /// `exact_matches` plus pairs that become equal after canonicalizing the
    /// volatile machine-specific segments that show up in xcodebuild output:
    /// the user's `$HOME`, the macOS per-user `$DARWIN_USER_CACHE`
    /// (`/var/folders/<x>/<long-hash>/`), the 28-character DerivedData
    /// project hash, the Xcode build number embedded in `CCHROOT`, the Xcode
    /// developer dir, the SDK version, and the project root. See
    /// [`canonicalize_value`] for the placeholder substitutions.
    pub canonical_matches: u64,
    /// `canonical_matches` plus pairs that differ only because both values
    /// are absolute paths anchored at different project roots (we run
    /// against `fixtures/.../raw/` but the oracle was captured against the
    /// user's checkout). This is the loosest comparison and measures
    /// resolver correctness independent of geometry.
    pub structural_matches: u64,
}

impl Stats {
    pub fn merge(&mut self, other: Stats) {
        self.files += other.files;
        self.oracle_keys += other.oracle_keys;
        self.our_keys += other.our_keys;
        self.shared_keys += other.shared_keys;
        self.exact_matches += other.exact_matches;
        self.canonical_matches += other.canonical_matches;
        self.structural_matches += other.structural_matches;
    }

    pub fn exact_pct(&self) -> u64 {
        if self.shared_keys > 0 {
            self.exact_matches * 100 / self.shared_keys
        } else {
            0
        }
    }

    pub fn canonical_pct(&self) -> u64 {
        if self.shared_keys > 0 {
            self.canonical_matches * 100 / self.shared_keys
        } else {
            0
        }
    }

    pub fn structural_pct(&self) -> u64 {
        if self.shared_keys > 0 {
            self.structural_matches * 100 / self.shared_keys
        } else {
            0
        }
    }
}

/// Assert per-version coverage floors instead of one blended floor across all
/// captured Xcode versions. Each version's exact/canonical ceiling is fixed by
/// the geometry of its own captured corpus (we resolve against `raw/` while the
/// oracle was captured at the original checkout, so path-anchored values —
/// `PROJECT_DIR`, `SRCROOT`, every absolute `*_SEARCH_PATHS`, `DEVELOPER_DIR`, …
/// — can never byte- or canonical-match), so a single blended floor drifts as
/// versions are added and masks a per-version regression. `structural` is the
/// geometry-independent correctness signal and stays ~99% on every version.
///
/// `floor(version)` returns the codified `(exact, canonical, structural)` percent
/// floor for that version (set from the first clean run minus a small margin), or
/// `None` for a freshly captured version with no floor yet — those get only the
/// `structural >= 98` safety guard, and the observed numbers are always printed
/// so a floor can be codified. The printed line is the audit trail for every run.
pub fn assert_version_floors(
    label: &str,
    per_version: &BTreeMap<String, Stats>,
    floor: impl Fn(&str) -> Option<(u64, u64, u64)>,
) {
    println!("\n--- per Xcode version (floor check: {label}) ---");
    // Collect every violation and panic once at the end, so a failing version
    // never hides the observed numbers of the versions after it — the printed
    // block is the audit trail for recalibration.
    let mut violations: Vec<String> = Vec::new();
    for (ver, s) in per_version {
        let (e, c, st) = (s.exact_pct(), s.canonical_pct(), s.structural_pct());
        if let Some((fe, fc, fst)) = floor(ver) {
            println!(
                "  {ver:<10} exact={e}% (≥{fe}) canon={c}% (≥{fc}) struct={st}% (≥{fst}) shared={}",
                s.shared_keys
            );
            if e < fe {
                violations.push(format!(
                    "[{label} {ver}] exact {e}% < floor {fe}% ({}/{} keys) — value regression?",
                    s.exact_matches, s.shared_keys
                ));
            }
            if c < fc {
                violations.push(format!(
                    "[{label} {ver}] canonical {c}% < floor {fc}% ({}/{})",
                    s.canonical_matches, s.shared_keys
                ));
            }
            if st < fst {
                violations.push(format!(
                    "[{label} {ver}] structural {st}% < floor {fst}% ({}/{})",
                    s.structural_matches, s.shared_keys
                ));
            }
        } else {
            println!(
                "  {ver:<10} exact={e}% canon={c}% struct={st}% shared={} \
                 [NO CODIFIED FLOOR — add to version_floor()]",
                s.shared_keys
            );
            if st < 98 {
                violations.push(format!(
                    "[{label} {ver}] structural {st}% < 98% safety floor (no codified \
                     floor yet; observed exact={e}% canon={c}%)"
                ));
            }
        }
    }
    assert!(violations.is_empty(), "{}", violations.join("\n"));
}

pub type MismatchTally = BTreeMap<String, u64>;

/// Whether a value is an absolute path (or a space-joined list of them) for
/// the structural tier. Search-path values frequently carry xcodebuild's
/// leading-space artifact (` /path/a /path/b` from an empty `$(inherited)`),
/// so the check skips leading whitespace — otherwise two path lists differing
/// only in their roots would read as a genuine value mismatch instead of
/// path-geometry drift.
pub fn is_absolute_path(v: &str) -> bool {
    v.trim_start().starts_with('/')
}

/// Classify every key shared between `resolved` (our output) and `oracle`
/// (the captured `buildSettings`) into the exact / canonical / structural
/// tiers, returning the per-comparison [`Stats`] (one "file"). The two
/// tallies record which keys drive the misses:
///
/// - `mismatch_tally`: keys that miss even the loosest (structural) tier —
///   the values genuinely disagree.
/// - `canon_only_tally`: keys that match structurally but NOT canonically —
///   both sides are absolute paths anchored at different roots. These are the
///   path-geometry differences, separated out so callers can see the
///   canonical-vs-structural gap without it polluting the real-miss list.
///
/// Callers that want a global tally and a per-project one accumulate into the
/// per-project tally here, then [`merge_tally`] the per-project tallies into
/// the global at print time — so one comparison pass feeds both. A test that
/// only wants the headline numbers can pass two throwaway tallies.
///
/// Set `DEBUG_DIFF_KEY=<KEY>` in the environment to print every real miss for
/// that key (with both values and the source file).
pub fn compare(
    resolved: &BTreeMap<String, String>,
    oracle: &BTreeMap<String, String>,
    source: &Path,
    mismatch_tally: &mut MismatchTally,
    canon_only_tally: &mut MismatchTally,
) -> Stats {
    let mut stats = Stats::default();
    let mut shared = 0u64;
    let mut matches = 0u64;
    let mut canonical = 0u64;
    let mut structural = 0u64;
    for (k, our_val) in resolved {
        if let Some(oracle_val) = oracle.get(k) {
            shared += 1;
            if our_val == oracle_val {
                matches += 1;
                canonical += 1;
                structural += 1;
            } else if canonicalize_value(our_val) == canonicalize_value(oracle_val) {
                // The values differ only in one of the volatile path
                // components ($HOME / $DARWIN_CACHE / DerivedData hash / Xcode
                // build / SDK version / project root). Counts as canonical.
                canonical += 1;
                structural += 1;
            } else if is_absolute_path(our_val) && is_absolute_path(oracle_val) {
                // Both sides are absolute paths anchored at different project
                // roots — semantically equivalent for resolver correctness.
                structural += 1;
                *canon_only_tally.entry(k.clone()).or_insert(0) += 1;
                if std::env::var("DEBUG_DIFF_KEY").ok().as_deref() == Some(k.as_str()) {
                    eprintln!(
                        "CANON-ONLY {} ::\n  ours   = {:?}\n  oracle = {:?}\n  canon(ours)   = {:?}\n  canon(oracle) = {:?}\n  :: {}",
                        k,
                        our_val,
                        oracle_val,
                        canonicalize_value(our_val),
                        canonicalize_value(oracle_val),
                        source.display()
                    );
                }
            } else {
                *mismatch_tally.entry(k.clone()).or_insert(0) += 1;
                if std::env::var("DEBUG_DIFF_KEY").ok().as_deref() == Some(k.as_str()) {
                    eprintln!(
                        "MISMATCH {} :: ours={:?} oracle={:?} :: {}",
                        k,
                        our_val,
                        oracle_val,
                        source.display()
                    );
                }
            }
        }
    }
    stats.files = 1;
    stats.oracle_keys = oracle.len() as u64;
    stats.our_keys = resolved.len() as u64;
    stats.shared_keys = shared;
    stats.exact_matches = matches;
    stats.canonical_matches = canonical;
    stats.structural_matches = structural;
    stats
}

/// Fold `src` into `dst`, summing counts per key. Used to build a global
/// mismatch tally from the per-group tallies that [`compare`] fills.
pub fn merge_tally(dst: &mut MismatchTally, src: &MismatchTally) {
    for (k, n) in src {
        *dst.entry(k.clone()).or_insert(0) += n;
    }
}

/// Print the standard headline + per-key mismatch diagnostics shared by every
/// oracle test, so each one is a useful diagnostic and not just pass/fail.
pub fn print_summary(title: &str, total: &Stats, mismatch_tally: &MismatchTally) {
    println!("=== {title} ===");
    println!("files: {} processed", total.files);
    println!(
        "oracle keys total: {}, our keys total: {}",
        total.oracle_keys, total.our_keys
    );
    if total.shared_keys > 0 {
        println!(
            "shared: {}, exact: {} ({}%), canonical: {} ({}%), structural: {} ({}%)",
            total.shared_keys,
            total.exact_matches,
            total.exact_pct(),
            total.canonical_matches,
            total.canonical_pct(),
            total.structural_matches,
            total.structural_pct(),
        );
    }
    println!("\n--- top 30 systematic mismatches (key, # of oracles it fails in) ---");
    let mut entries: Vec<(&String, &u64)> = mismatch_tally.iter().collect();
    entries.sort_by(|a, b| b.1.cmp(a.1));
    for (k, n) in entries.iter().take(30) {
        println!("  {n:<5} {k}");
    }
}

// ----- canonicalization ----------------------------------------------------

/// Strip the volatile components from a build-setting value so the
/// comparison survives running on a different machine, user, Xcode
/// version, or fixture layout than the corpus was captured against:
///
/// | Placeholder       | Pattern matched                                              |
/// |-------------------|--------------------------------------------------------------|
/// | `<HOME>`          | `/Users/<name>/`                                             |
/// | `<DARWIN_CACHE>`  | `/var/folders/<2-char>/<long-hash>/`                         |
/// | `<HASH>`          | `DerivedData/<Name>-<28-lowercase-letters>/`                 |
/// | `<XCODE_BUILD>`   | `DeveloperTools/<x.y.z-BUILD>/`                              |
/// | `<XCODE_DEV>`     | any path through `/Applications/Xcode-*.app/Contents/Developer` |
/// | (`<Name>.sdk`)    | `<Name><digits>(.<digits>)*.sdk` → `<Name>.sdk`              |
///
/// Non-path values pass through unchanged. The replacements are
/// idempotent so running this on already-canonical text is a no-op.
pub fn canonicalize_value(v: &str) -> String {
    let mut s = canon_users_home(v);
    s = canon_darwin_user_cache(&s);
    s = canon_derived_data_hash(&s);
    s = canon_xcode_build(&s);
    s = canon_xcode_developer_dir(&s);
    s = canon_sdk_version(&s);
    s = canon_tuist_build(&s);
    s = canon_project_root(&s);
    s = canon_build_dir_plugin(&s);
    s
}

/// Collapse the two project-root layouts to a single `<PROJECT_ROOT>` so
/// values that differ *only* because the resolver ran against the fixture
/// checkout become canonical matches. The captured oracles were rooted at
/// the original checkout `<HOME>/.../corpus/<slug>/...`, while our resolver
/// runs against the re-laid-out fixture `<HOME>/.../fixtures/<slug>/xcode-<ver>/raw/...`.
/// Keys affected: `PROJECT_DIR`, `SRCROOT`, `SOURCE_ROOT`, `PROJECT_FILE_PATH`,
/// `LOCROOT`, `LOCSYMROOT`, and the project-relative entries embedded in the
/// absolute `*_SEARCH_PATHS`.
///
/// Both anchors collapse exactly one segment of slug, so the project-relative
/// SUFFIX (`/Example`, `/Modules/A`, `<proj>.xcodeproj`, ...) survives intact
/// and lines up. For the flat, single-level fixtures (alamofire, ice-cubes,
/// kingfisher, netnewswire) this is a clean equality.
///
/// TUIST CAVEAT: the tuist sub-fixture directory was flattened on import —
/// ours is `<PROJECT_ROOT>/examples_xcode_generated_foo` (underscores) while
/// the oracle keeps `<PROJECT_ROOT>/examples/xcode/generated_foo` (slashes).
/// The flattening rule is exactly "join the `examples/xcode/<dir>` path
/// segments with underscores" (every `fixtures/tuist-fixtures/.../raw/` entry
/// is `examples_xcode_*`), so after the root collapse we rewrite the oracle's
/// slash spelling to the flattened one. This is pure capture-host geometry:
/// the original `corpus/tuist-fixtures/examples/xcode/<dir>` checkout layout
/// doesn't exist in the imported fixture tree, so no resolver output running
/// against `raw/` could ever reproduce it byte-for-byte.
///
/// Idempotent: a value already containing `<PROJECT_ROOT>` matches neither
/// root anchor and passes through unchanged, and the flattened spelling
/// contains no `/examples/xcode/` left to rewrite. Operates on embedded
/// tokens (handles space-joined search paths and paths containing spaces) by
/// consuming each anchor occurrence in place.
pub fn canon_project_root(s: &str) -> String {
    let s = canon_our_raw_root(s);
    let s = canon_oracle_corpus_root(&s);
    // Tuist sub-fixture flattening (oracle spelling -> fixture spelling).
    // Anchored on the canonical `<PROJECT_ROOT>` placeholder, so only values
    // whose project root already collapsed are rewritten.
    s.replace(
        "<PROJECT_ROOT>/examples/xcode/",
        "<PROJECT_ROOT>/examples_xcode_",
    )
}

/// Our layout: replace the path token up through `/fixtures/<slug>/xcode-<ver>/raw`
/// with `<PROJECT_ROOT>`. Anchored on the `/xcode-<ver>/raw` marker, validated
/// against a preceding `/fixtures/` within the same (whitespace-delimited) token.
fn canon_our_raw_root(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < s.len() {
        let Some(rel) = s[i..].find("/xcode-") else {
            out.push_str(&s[i..]);
            break;
        };
        let marker = i + rel;
        // After `/xcode-` expect a version run (digits/dots) then `/raw`.
        let after = marker + "/xcode-".len();
        let ver_end = after
            + s[after..]
                .find(|c: char| !c.is_ascii_digit() && c != '.')
                .unwrap_or(s.len() - after);
        let raw_tail = "/raw";
        let matches_raw = s[ver_end..].starts_with(raw_tail)
            && s[ver_end + raw_tail.len()..]
                .chars()
                .next()
                .is_none_or(|c| c == '/' || c.is_whitespace());
        // The token before the marker must contain `/fixtures/`.
        let tok_start = path_token_start(s, marker, i);
        let has_fixtures = s[tok_start..marker].contains("/fixtures/");
        if matches_raw && has_fixtures {
            out.push_str(&s[i..tok_start]);
            out.push_str("<PROJECT_ROOT>");
            i = ver_end + raw_tail.len();
        } else {
            // Not our raw root — keep up through the marker and continue.
            out.push_str(&s[i..after]);
            i = after;
        }
    }
    out
}

/// Oracle layout: replace the path token up through `/corpus/<slug>` (exactly
/// one slug segment) with `<PROJECT_ROOT>`.
fn canon_oracle_corpus_root(s: &str) -> String {
    let anchor = "/corpus/";
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < s.len() {
        let Some(rel) = s[i..].find(anchor) else {
            out.push_str(&s[i..]);
            break;
        };
        let abs = i + rel;
        let slug_start = abs + anchor.len();
        // Consume the single slug segment (up to next '/' or token boundary).
        let slug_end = slug_start
            + s[slug_start..]
                .find(|c: char| c == '/' || c.is_whitespace())
                .unwrap_or(s.len() - slug_start);
        if slug_end > slug_start {
            let tok_start = path_token_start(s, abs, i);
            out.push_str(&s[i..tok_start]);
            out.push_str("<PROJECT_ROOT>");
            i = slug_end;
        } else {
            out.push_str(&s[i..slug_start]);
            i = slug_start;
        }
    }
    out
}

/// Strip the project-root prefix that precedes a `/Tuist/.build/`
/// segment. Our resolver outputs paths anchored at the fixture root
/// (`<HOME>/.../fixtures/.../raw/<sub>/Tuist/.build/...`) while the
/// captured oracles point at the user's original checkout
/// (`<HOME>/.../corpus/.../<other-sub>/Tuist/.build/...`). The suffix
/// inside `.build/` (checkouts, tuist-derived, etc.) is identical in
/// both, so canonicalising the prefix lines the values up.
fn canon_tuist_build(s: &str) -> String {
    canon_anchor_substring(s, "/Tuist/.build/", "<TUIST_BUILD>/")
}

/// Collapse the `$(BUILD_DIR)` prefix of a `-load-plugin-executable` argument
/// to `<BUILD_DIR>`. Tuist bakes
/// `-load-plugin-executable $(BUILD_DIR)/$(CONFIGURATION)$(EFFECTIVE_PLATFORM_NAME)/<macro>#<entry>`
/// into a target's `OTHER_SWIFT_FLAGS`. `$(BUILD_DIR)` expands to the DerivedData
/// build-products dir for our resolver (`.../Build/Products`) but to the
/// project-relative `SYMROOT` default (`.../build`) in the no-destination
/// captures — the very path-root drift the standalone `BUILD_DIR` /
/// `BUILT_PRODUCTS_DIR` keys already absorb in the structural tier. Because
/// `OTHER_SWIFT_FLAGS` is not a bare path it can never reach that tier, so
/// without this the drift reads as a genuine value mismatch in the per-key tally.
///
/// The baked argument is always exactly `<build-dir>/<config><platform>/<macro>#<entry>`
/// (verified across the whole corpus), so we keep its last two path segments and
/// replace everything before them with `<BUILD_DIR>`. The prefix content is never
/// inspected, so both the DerivedData and project-relative geometries collapse to
/// the same token; the `<config><platform>` segment is preserved, so a real
/// config/platform disagreement still surfaces. Guarded by the `#` plugin-entry
/// marker so only genuine `-load-plugin-executable` arguments are rewritten, and
/// idempotent — an already-`<BUILD_DIR>`-prefixed argument re-collapses to itself.
fn canon_build_dir_plugin(s: &str) -> String {
    let flag = "-load-plugin-executable ";
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < s.len() {
        let Some(rel) = s[i..].find(flag) else {
            out.push_str(&s[i..]);
            break;
        };
        let arg_start = i + rel + flag.len();
        let arg_end = arg_start
            + s[arg_start..]
                .find(char::is_whitespace)
                .unwrap_or(s.len() - arg_start);
        let arg = &s[arg_start..arg_end];
        out.push_str(&s[i..arg_start]);
        out.push_str(&collapse_build_dir_arg(arg).unwrap_or_else(|| arg.to_string()));
        i = arg_end;
    }
    out
}

/// Replace all but the last two `/`-segments of a `-load-plugin-executable`
/// argument with `<BUILD_DIR>`. Returns `None` (leave untouched) unless the
/// argument carries the `#` plugin-entry marker and has at least two segments.
fn collapse_build_dir_arg(arg: &str) -> Option<String> {
    if !arg.contains('#') {
        return None;
    }
    let last = arg.rfind('/')?; // slash before `<macro>#<entry>`
    let prev = arg[..last].rfind('/')?; // slash before `<config><platform>`
    Some(format!("<BUILD_DIR>{}", &arg[prev..]))
}

/// Find every occurrence of `anchor` in `s` and replace the leading
/// path token (walking backwards over non-whitespace, non-`=`, non-quote
/// characters from the anchor start) with `placeholder`. Used to
/// canonicalise paths that are EMBEDDED in flag tokens like
/// `-fmodule-map-file=/path/to/module.modulemap` — the simpler
/// per-token canonicalizer in [`canon_path_token`] only matches when
/// the entire token IS a path.
fn canon_anchor_substring(s: &str, anchor: &str, placeholder: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < s.len() {
        let Some(rel) = s[i..].find(anchor) else {
            out.push_str(&s[i..]);
            break;
        };
        let abs = i + rel;
        let prefix_start = path_token_start(s, abs, i);
        out.push_str(&s[i..prefix_start]);
        out.push_str(placeholder);
        i = abs + anchor.len();
    }
    out
}

fn path_token_start(s: &str, end: usize, floor: usize) -> usize {
    let bytes = s.as_bytes();
    let mut pos = end;
    while pos > floor {
        let b = bytes[pos - 1];
        if matches!(b, b' ' | b'\t' | b'\n' | b'=' | b'"' | b'\'') {
            break;
        }
        pos -= 1;
    }
    pos
}

fn canon_users_home(s: &str) -> String {
    // `/Users/<name>/...` — replace `/Users/<name>` with `<HOME>`.
    let mut out = String::with_capacity(s.len());
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i..].starts_with(b"/Users/") {
            // Find the next '/' after the username.
            let name_start = i + b"/Users/".len();
            let mut j = name_start;
            while j < bytes.len() && bytes[j] != b'/' {
                j += 1;
            }
            if j > name_start && j < bytes.len() {
                out.push_str("<HOME>");
                i = j;
                continue;
            }
        }
        out.push(bytes[i] as char);
        i += 1;
    }
    out
}

fn canon_darwin_user_cache(s: &str) -> String {
    // `/var/folders/<2-char>/<long-id>/...` — replace through the second
    // path segment after `/var/folders/`.
    let mut out = String::with_capacity(s.len());
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i..].starts_with(b"/var/folders/") {
            let mut j = i + b"/var/folders/".len();
            // Skip two `/`-terminated segments.
            let mut segments = 0;
            while j < bytes.len() && segments < 2 {
                if bytes[j] == b'/' {
                    segments += 1;
                }
                j += 1;
            }
            if segments == 2 {
                // j now points one past the second '/' — i.e. at the start
                // of the segment AFTER the long ID. Back up to keep the
                // trailing '/'.
                out.push_str("<DARWIN_CACHE>/");
                i = j;
                continue;
            }
        }
        out.push(bytes[i] as char);
        i += 1;
    }
    out
}

fn canon_derived_data_hash(s: &str) -> String {
    // `DerivedData/<Name>-<28-lowercase-letters>/` — replace just the
    // 28-char hash with `<HASH>`, keeping the project-name prefix intact.
    let mut out = String::with_capacity(s.len());
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i..].starts_with(b"DerivedData/") {
            // Copy `DerivedData/`.
            out.push_str("DerivedData/");
            let mut j = i + b"DerivedData/".len();
            // Copy the project-name segment up to and including the last '-'
            // before the hash.
            let seg_start = j;
            while j < bytes.len() && bytes[j] != b'/' {
                j += 1;
            }
            let seg = &s[seg_start..j];
            // Find the LAST '-' followed by exactly 28 lowercase letters.
            if let Some(dash) = seg.rfind('-') {
                let hash = &seg[dash + 1..];
                if hash.len() == 28 && hash.bytes().all(|b| b.is_ascii_lowercase()) {
                    out.push_str(&seg[..=dash]);
                    out.push_str("<HASH>");
                    i = j;
                    continue;
                }
            }
            // Not a DerivedData hash segment — copy it verbatim.
            out.push_str(seg);
            i = j;
            continue;
        }
        out.push(bytes[i] as char);
        i += 1;
    }
    out
}

fn canon_xcode_developer_dir(s: &str) -> String {
    // Apple's `xcodebuild` emits absolute paths through the Xcode
    // installation's Developer dir, e.g.
    //   /Applications/Xcode-26.0.1.app/Contents/Developer/Platforms/iPhoneSimulator.platform/...
    // Our resolver running against the fixture corpus produces paths
    // rooted at `<checkout>/xcspec-cache/xcode-26.0.1/sdksettings/...`
    // instead. Both contain the structural anchor `Platforms/<X>.platform`;
    // we strip everything before that anchor on whitespace-delimited
    // tokens that look like absolute paths and replace it with
    // `<XCODE_DEV>`.
    let tokens: Vec<String> = s.split_whitespace().map(canon_path_token).collect();
    if s.starts_with(char::is_whitespace) {
        format!(" {}", tokens.join(" "))
    } else {
        tokens.join(" ")
    }
}

fn canon_path_token(tok: &str) -> String {
    // A path can be wrapped in quotes when it's embedded in a flag value — e.g.
    // `--scan-executable "<xcode-dev>/…/libXCTestSwiftSupport.dylib"` inside
    // PRODUCT_TYPE_SWIFT_STDLIB_TOOL_FLAGS. The whole quoted blob is one
    // whitespace token, so unwrap the quotes, canonicalize the inner path, and
    // restore them — otherwise the Xcode-app-dir root (which differs between the
    // host Xcode and the fixture's Xcode) never collapses to `<XCODE_DEV>`.
    if let Some(inner) = tok.strip_prefix('"').and_then(|t| t.strip_suffix('"')) {
        return format!("\"{}\"", canon_path_token(inner));
    }
    // Only rewrite tokens that look like absolute paths (or already-
    // canonicalised `<HOME>/...`) and reach a known anchor segment.
    if !tok.starts_with('/') && !tok.starts_with("<HOME>") && !tok.starts_with("<DARWIN_CACHE>") {
        return tok.to_string();
    }
    for anchor in [
        "/Platforms/",
        "/Toolchains/",
        "/usr/bin",
        "/usr/lib",
        "/Library/Frameworks",
    ] {
        if let Some(idx) = tok.find(anchor) {
            // Leave the anchor itself in place; replace everything
            // before it with `<XCODE_DEV>`.
            return format!("<XCODE_DEV>{}", &tok[idx..]);
        }
    }
    tok.to_string()
}

fn canon_sdk_version(s: &str) -> String {
    // `<Name><digits>(.<digits>)*.sdk` → `<Name>.sdk`. We need to spot
    // the trailing version segment immediately before `.sdk` and trim
    // back to the last non-digit/dot run. Examples:
    //   iPhoneSimulator26.0.sdk → iPhoneSimulator.sdk
    //   MacOSX26.0.sdk          → MacOSX.sdk
    //   WatchSimulator26.0.sdk  → WatchSimulator.sdk
    let mut out = String::with_capacity(s.len());
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if let Some(sdk_at) = s[i..].find(".sdk")
            && sdk_at > 0
        {
            // Walk backward from `sdk_at` collecting trailing
            // digit/dot characters that form the version.
            let abs = i + sdk_at;
            let chunk = &s[i..abs];
            // Find the last non-version character.
            let trim_to = chunk
                .rfind(|c: char| !c.is_ascii_digit() && c != '.')
                .map_or(0, |p| p + 1);
            if trim_to < chunk.len() {
                // Emit the chunk up to and including the name, then
                // `.sdk`, and advance past `.sdk`.
                out.push_str(&chunk[..trim_to]);
                out.push_str(".sdk");
                i = abs + ".sdk".len();
                continue;
            }
        }
        // Walk forward by one char. We can't simply push `bytes[i]`
        // because we're walking by char boundaries.
        let c = s[i..].chars().next().expect("valid UTF-8 boundary");
        out.push(c);
        i += c.len_utf8();
    }
    out
}

fn canon_xcode_build(s: &str) -> String {
    // `DeveloperTools/<x.y.z-BUILD>/` — replace the version-build segment
    // with `<XCODE_BUILD>`.
    let mut out = String::with_capacity(s.len());
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i..].starts_with(b"DeveloperTools/") {
            out.push_str("DeveloperTools/");
            let mut j = i + b"DeveloperTools/".len();
            // Look ahead one segment; if it's a `<version>-<build>` shape,
            // replace it.
            let seg_start = j;
            while j < bytes.len() && bytes[j] != b'/' {
                j += 1;
            }
            let seg = &s[seg_start..j];
            // Shape: at least one '.' (version), one '-', and an
            // alphanumeric build identifier.
            if seg.contains('.') && seg.contains('-') && !seg.is_empty() {
                out.push_str("<XCODE_BUILD>");
                i = j;
                continue;
            }
            out.push_str(seg);
            i = j;
            continue;
        }
        out.push(bytes[i] as char);
        i += 1;
    }
    out
}

// ----- unit tests for the canonicalizer ------------------------------------

#[cfg(test)]
mod canon_tests {
    use super::*;

    #[test]
    fn canonicalize_users_home() {
        assert_eq!(
            canonicalize_value("/Users/hyzyla_home/Library/Developer/Xcode"),
            "<HOME>/Library/Developer/Xcode"
        );
        // Different username canonicalises to the same placeholder.
        assert_eq!(
            canonicalize_value("/Users/alice/Library/Developer/Xcode"),
            "<HOME>/Library/Developer/Xcode"
        );
    }

    #[test]
    fn canonicalize_darwin_user_cache() {
        let raw = "/var/folders/wq/kkdk740d68qcqvttr995x13w0000gq/C/com.apple.DeveloperTools";
        assert_eq!(
            canonicalize_value(raw),
            "<DARWIN_CACHE>/C/com.apple.DeveloperTools"
        );
        // Different per-user hash canonicalises identically.
        let raw2 = "/var/folders/aa/bbbbcccc/C/com.apple.DeveloperTools";
        assert_eq!(
            canonicalize_value(raw2),
            "<DARWIN_CACHE>/C/com.apple.DeveloperTools"
        );
    }

    #[test]
    fn canonicalize_derived_data_hash() {
        // 28-char lowercase hash — the canonical form Xcode emits.
        assert_eq!(
            canonicalize_value("DerivedData/Kingfisher-bsyqrgpmdpgiztchzytjjxsrwpeh/Build"),
            "DerivedData/Kingfisher-<HASH>/Build"
        );
        // Non-hash trailing segment is preserved.
        assert_eq!(
            canonicalize_value("DerivedData/ModuleCache.noindex/Session.modulevalidation"),
            "DerivedData/ModuleCache.noindex/Session.modulevalidation"
        );
    }

    #[test]
    fn canonicalize_xcode_build_number() {
        assert_eq!(
            canonicalize_value("com.apple.DeveloperTools/26.0.1-17A400/Xcode"),
            "com.apple.DeveloperTools/<XCODE_BUILD>/Xcode"
        );
    }

    #[test]
    fn canonicalize_full_cchroot_value() {
        let raw = "/var/folders/wq/kkdk740d68qcqvttr995x13w0000gq/C/com.apple.DeveloperTools/26.0.1-17A400/Xcode";
        let other = "/var/folders/aa/bbcc/C/com.apple.DeveloperTools/27.0-18B100/Xcode";
        assert_eq!(canonicalize_value(raw), canonicalize_value(other));
    }

    #[test]
    fn canonicalize_full_build_dir_value() {
        let raw = "/Users/hyzyla_home/Library/Developer/Xcode/DerivedData/Kingfisher-bsyqrgpmdpgiztchzytjjxsrwpeh/Build/Products";
        let other = "/Users/alice/Library/Developer/Xcode/DerivedData/Kingfisher-aaaaaaaaaaaaaaaaaaaaaaaaaaaa/Build/Products";
        assert_eq!(canonicalize_value(raw), canonicalize_value(other));
    }

    #[test]
    fn canonicalize_non_path_values_are_unchanged() {
        assert_eq!(canonicalize_value("YES"), "YES");
        assert_eq!(canonicalize_value("arm64 x86_64"), "arm64 x86_64");
        assert_eq!(canonicalize_value(""), "");
    }

    #[test]
    fn canonicalize_xcode_developer_dir() {
        assert_eq!(
            canonicalize_value(
                "/Applications/Xcode-26.0.1.app/Contents/Developer/Platforms/iPhoneSimulator.platform"
            ),
            "<XCODE_DEV>/Platforms/iPhoneSimulator.platform"
        );
        // Beta / unversioned Xcode app names also collapse.
        assert_eq!(
            canonicalize_value(
                "/Applications/Xcode-beta.app/Contents/Developer/Toolchains/XcodeDefault.xctoolchain"
            ),
            "<XCODE_DEV>/Toolchains/XcodeDefault.xctoolchain"
        );
        // The fixture-resident xcspec-cache path also normalises so the
        // corpus test's resolver output lines up with oracle paths.
        assert_eq!(
            canonicalize_value(
                "/Users/me/proj/xcspec-cache/xcode-26.0.1/sdksettings/Platforms/iPhoneSimulator.platform"
            ),
            "<XCODE_DEV>/Platforms/iPhoneSimulator.platform"
        );
    }

    #[test]
    fn canonicalize_sdk_version_strips_trailing_digits() {
        assert_eq!(
            canonicalize_value(
                "Platforms/iPhoneSimulator.platform/Developer/SDKs/iPhoneSimulator26.0.sdk"
            ),
            "Platforms/iPhoneSimulator.platform/Developer/SDKs/iPhoneSimulator.sdk"
        );
        assert_eq!(
            canonicalize_value("Developer/SDKs/MacOSX26.0.sdk/usr/lib"),
            "Developer/SDKs/MacOSX.sdk/usr/lib"
        );
        // Unversioned `.sdk` directory passes through.
        assert_eq!(
            canonicalize_value("Developer/SDKs/MacOSX.sdk/usr/lib"),
            "Developer/SDKs/MacOSX.sdk/usr/lib"
        );
    }

    #[test]
    fn canonicalize_xcode_and_sdk_together() {
        let oracle = "/Applications/Xcode-26.0.1.app/Contents/Developer/Platforms/iPhoneSimulator.platform/Developer/SDKs/iPhoneSimulator26.0.sdk/Developer/Library/Frameworks";
        let ours = "/Users/me/proj/xcspec-cache/xcode-26.0.1/sdksettings/Platforms/iPhoneSimulator.platform/Developer/SDKs/iPhoneSimulator.sdk/Developer/Library/Frameworks";
        assert_eq!(canonicalize_value(oracle), canonicalize_value(ours));
    }

    #[test]
    fn canonicalize_project_root_flat_fixture() {
        // PROJECT_DIR for a single-level fixture: our raw layout and the oracle
        // checkout collapse to the same `<PROJECT_ROOT>`.
        let ours = "/Users/me/dev/sweetpad-lib/fixtures/alamofire/xcode-26.0.1/raw";
        let oracle = "/Users/me/dev/sweetpad-lib/corpus/alamofire";
        assert_eq!(canonicalize_value(ours), "<PROJECT_ROOT>");
        assert_eq!(canonicalize_value(ours), canonicalize_value(oracle));
    }

    #[test]
    fn canonicalize_project_root_keeps_relative_suffix() {
        // PROJECT_FILE_PATH: the project-relative suffix (subdir + .xcodeproj,
        // including a space in the dir name) survives the collapse and lines up.
        let ours = "/Users/me/dev/sweetpad-lib/fixtures/alamofire/xcode-26.0.1/raw/watchOS Example/watchOS Example.xcodeproj";
        let oracle =
            "/Users/me/dev/sweetpad-lib/corpus/alamofire/watchOS Example/watchOS Example.xcodeproj";
        assert_eq!(
            canonicalize_value(ours),
            "<PROJECT_ROOT>/watchOS Example/watchOS Example.xcodeproj"
        );
        assert_eq!(canonicalize_value(ours), canonicalize_value(oracle));
    }

    #[test]
    fn canonicalize_project_root_embedded_in_search_path() {
        // A space-joined search path with an embedded project-root token: only
        // the project-root prefix is rewritten, the rest of the token is intact.
        let ours = "<HOME>/dev/sweetpad-lib/fixtures/tuist-fixtures/xcode-26.0.1/raw/examples_xcode_generated_ios_app_with_static_libraries/Modules/A/../C/prebuilt/C";
        assert_eq!(
            canon_project_root(ours),
            "<PROJECT_ROOT>/examples_xcode_generated_ios_app_with_static_libraries/Modules/A/../C/prebuilt/C"
        );
        // The oracle's original-checkout spelling (slash-separated sub-fixture
        // path) flattens to the fixture's underscore spelling, so the two
        // geometries land on the same canonical value.
        let oracle = "<HOME>/dev/sweetpad-lib/corpus/tuist-fixtures/examples/xcode/generated_ios_app_with_static_libraries/Modules/A/../C/prebuilt/C";
        assert_eq!(
            canon_project_root(oracle),
            "<PROJECT_ROOT>/examples_xcode_generated_ios_app_with_static_libraries/Modules/A/../C/prebuilt/C"
        );
        assert_eq!(canon_project_root(oracle), canon_project_root(ours));
    }

    #[test]
    fn canonicalize_load_plugin_executable_build_dir() {
        // Our DerivedData geometry and the no-destination capture's
        // project-relative `build` geometry collapse to the same `<BUILD_DIR>`
        // token, so the embedded `-load-plugin-executable` arg lines up.
        let ours = " -Xcc -fmodule-map-file=/x/y.modulemap -load-plugin-executable /Users/me/Library/Developer/Xcode/DerivedData/App-bsyqrgpmdpgiztchzytjjxsrwpeh/Build/Products/Debug-iphoneos/CasePathsMacros#CasePathsMacros -plugin-path /Applications/Xcode.app/Contents/Developer/Toolchains/XcodeDefault.xctoolchain/usr/lib/swift/host/plugins/testing";
        let oracle = " -Xcc -fmodule-map-file=/x/y.modulemap -load-plugin-executable /Users/me/dev/proj/build/Debug-iphoneos/CasePathsMacros#CasePathsMacros -plugin-path /Applications/Xcode.app/Contents/Developer/Toolchains/XcodeDefault.xctoolchain/usr/lib/swift/host/plugins/testing";
        assert_eq!(canon_build_dir_plugin(ours), canon_build_dir_plugin(oracle));
        assert!(canon_build_dir_plugin(oracle).contains(
            "-load-plugin-executable <BUILD_DIR>/Debug-iphoneos/CasePathsMacros#CasePathsMacros "
        ));
        // The `<config><platform>` segment is preserved, so a real
        // platform disagreement still diverges after canonicalization.
        let sim = " -load-plugin-executable /a/b/build/Debug-iphonesimulator/CasePathsMacros#CasePathsMacros";
        assert_ne!(canon_build_dir_plugin(oracle), canon_build_dir_plugin(sim));
        // Idempotent: re-collapsing an already-canonical arg is a no-op.
        let collapsed = canon_build_dir_plugin(oracle);
        assert_eq!(canon_build_dir_plugin(&collapsed), collapsed);
        // No `-load-plugin-executable` flag: untouched.
        assert_eq!(canon_build_dir_plugin(" -DDEBUG -DFOO"), " -DDEBUG -DFOO");
    }

    #[test]
    fn canonicalize_project_root_is_idempotent() {
        // Running the collapse on already-canonical text is a no-op.
        let collapsed = "<PROJECT_ROOT>/Example/iOS Example.xcodeproj";
        assert_eq!(canon_project_root(collapsed), collapsed);
        // And a non-project path is untouched.
        let other = "<HOME>/Library/Developer/Xcode/DerivedData/App-<HASH>/Build";
        assert_eq!(canon_project_root(other), other);
    }
}

#[cfg(test)]
mod version_tests {
    use super::*;

    #[test]
    fn capture_xcode_version_reads_the_xcode_component() {
        // The `xcode-<ver>` dir between the slug and `metadata/` carries the version.
        assert_eq!(
            capture_xcode_version(Path::new(
                "fixtures/alamofire/xcode-26.0.1/metadata/schemes/A/build-settings/Debug__macOS.json"
            )),
            Some("26.0.1".to_owned())
        );
        // A future non-major version is read verbatim.
        assert_eq!(
            capture_xcode_version(Path::new("fixtures/kingfisher/xcode-16.4.0/raw")),
            Some("16.4.0".to_owned())
        );
        // No `xcode-<ver>` component → None (the oracle loop treats it as a skip).
        assert_eq!(
            capture_xcode_version(Path::new("fixtures/whatever/raw")),
            None
        );
    }
}
