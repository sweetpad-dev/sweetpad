//! Argv comparator for the compiler-args oracle.
//!
//! The build-settings comparator (in the parent module) scores a `KEY → value`
//! map: the key pins the pairing, and each value lands in the exact / canonical
//! / structural tier. A compiler command line has no such keys — it's an
//! ordered token vector — so this module first normalises an argv into a
//! multiset of [`Item`]s (standalone flags + `(flag, value)` pairs), classifies
//! pure build-geometry items out, then reconciles the oracle's items against
//! ours tier by tier:
//!
//! 1. **exact** — byte-equal item.
//! 2. **canonical** — equal after [`super::canonicalize_value`] strips the
//!    volatile `$HOME` / DerivedData-hash / Xcode-dir / SDK-version / project-root
//!    drift (the same normaliser the build-settings tiers use).
//! 3. **structural** — equal once every absolute path (bare, or embedded behind
//!    a `-I`/`-F`/`-L`/… prefix) collapses to `<ABS>`. Mirrors the build-settings
//!    "both sides are absolute paths anchored at different roots" tier.
//!
//! Whatever the oracle has that we never matched is **missing** (a recall
//! defect); whatever we emit that the oracle never had is **extra** (a precision
//! defect). Both feed a per-flag tally so a systematic gap is visible. Geometry
//! items (`-o`, `-output-file-map`, `-index-store-path`, header maps, the object
//! filelist, …) are counted but never scored — the generator has no reason to
//! reproduce a per-build output path.

// `o`/`m` (oracle/mine) and `e`/`c`/`s` (exact/canonical/structural) read
// clearly in the tight reconcile loop.
#![allow(clippy::many_single_char_names)]

use std::collections::BTreeMap;
use std::path::Path;

use super::{JsonValue, MismatchTally, canonicalize_value, parse_json};

// ----- oracle reader -------------------------------------------------------

/// One captured tool invocation. `swift`/`link` use `arguments` (+ `input_files`
/// for swift); `clang` is split into a shared `common_arguments` set plus a
/// per-file delta.
#[derive(Debug, Clone, Default)]
pub struct Invocation {
    pub tool: Option<String>,
    pub arguments: Vec<String>,
    pub input_files: Vec<String>,
}

#[derive(Debug, Clone, Default)]
pub struct ClangFile {
    pub file: String,
    pub extra_arguments: Vec<String>,
}

#[derive(Debug, Clone, Default)]
pub struct ClangTool {
    pub common_arguments: Vec<String>,
    pub files: Vec<ClangFile>,
}

#[derive(Debug, Clone, Default)]
pub struct TargetTools {
    pub target: String,
    pub swift: Option<Invocation>,
    pub clang: Option<ClangTool>,
    pub link: Option<Invocation>,
}

#[derive(Debug, Clone, Default)]
pub struct CompilerArgsOracle {
    pub slug: String,
    pub scheme: String,
    pub configuration: String,
    pub destination: String,
    pub sdk: String,
    pub arch: String,
    pub targets: Vec<TargetTools>,
}

fn json_str(v: &JsonValue) -> String {
    v.as_string().unwrap_or("").to_string()
}

fn json_str_array(v: Option<&JsonValue>) -> Vec<String> {
    v.and_then(JsonValue::as_array)
        .map(|a| a.iter().map(json_str).collect())
        .unwrap_or_default()
}

/// Parse a committed `compiler-args/*.json` oracle. Returns `None` if the file
/// is unreadable or isn't the expected `{ …, targets: [...] }` shape.
pub fn read_compiler_args(path: &Path) -> Option<CompilerArgsOracle> {
    let text = std::fs::read_to_string(path).ok()?;
    let root = parse_json(&text).ok()?;
    let obj = root.as_object()?;
    let get = |k: &str| obj.get(k).map(json_str).unwrap_or_default();

    let mut targets = Vec::new();
    for t in obj.get("targets")?.as_array()? {
        let to = t.as_object()?;
        let mut tt = TargetTools {
            target: to.get("target").map(json_str).unwrap_or_default(),
            ..Default::default()
        };
        if let Some(sw) = to.get("swift").and_then(JsonValue::as_object) {
            tt.swift = Some(Invocation {
                tool: None,
                arguments: json_str_array(sw.get("arguments")),
                input_files: json_str_array(sw.get("inputFiles")),
            });
        }
        if let Some(cl) = to.get("clang").and_then(JsonValue::as_object) {
            let mut files = Vec::new();
            for f in cl.get("files").and_then(JsonValue::as_array).unwrap_or(&[]) {
                if let Some(fo) = f.as_object() {
                    files.push(ClangFile {
                        file: fo.get("file").map(json_str).unwrap_or_default(),
                        extra_arguments: json_str_array(fo.get("extraArguments")),
                    });
                }
            }
            tt.clang = Some(ClangTool {
                common_arguments: json_str_array(cl.get("commonArguments")),
                files,
            });
        }
        if let Some(ln) = to.get("link").and_then(JsonValue::as_object) {
            tt.link = Some(Invocation {
                tool: ln.get("tool").map(json_str),
                arguments: json_str_array(ln.get("arguments")),
                input_files: Vec::new(),
            });
        }
        targets.push(tt);
    }

    Some(CompilerArgsOracle {
        slug: get("slug"),
        scheme: get("scheme"),
        configuration: get("configuration"),
        destination: get("destination"),
        sdk: get("sdk"),
        arch: get("arch"),
        targets,
    })
}

// ----- argv normalisation --------------------------------------------------

/// A normalised argv token: a standalone flag (or bare operand) when
/// `value` is `None`, or a `(flag, value)` pair otherwise. An attached
/// option like `-DDEBUG` or `-I/inc` is split into `("-D","DEBUG")` /
/// `("-I","/inc")` so the value lands in the right tier; the same split is
/// applied to both sides, so the comparison stays symmetric regardless of
/// whether the pairing is "really" how the tool parses it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    pub flag: String,
    pub value: Option<String>,
}

/// Flags that consume the **next** token as their value.
const VALUE_FLAGS: &[&str] = &[
    "-module-name",
    "-sdk",
    "-target",
    "-target-variant",
    "-swift-version",
    "-num-threads",
    "-I",
    "-F",
    "-L",
    "-isysroot",
    "-isystem",
    "-iquote",
    "-idirafter",
    "-resource-dir",
    "-Xcc",
    "-Xfrontend",
    "-Xclang",
    "-Xlinker",
    "-Xllvm",
    "-enable-upcoming-feature",
    "-enable-experimental-feature",
    "-framework",
    "-weak_framework",
    "-install_name",
    "-compatibility_version",
    "-current_version",
    "-o",
    "-MF",
    "-MT",
    "-MQ",
    "-module-cache-path",
    "-index-store-path",
    "-output-file-map",
    "-emit-module-path",
    "-emit-objc-header-path",
    "-emit-module-interface-path",
    "-emit-private-module-interface-path",
    "-emit-package-module-interface-path",
    "-clang-build-session-file",
    "-clang-scanner-module-cache-path",
    "-sdk-module-cache-path",
    "-const-gather-protocols-file",
    "-filelist",
    "-working-directory",
    "-pch-output-dir",
    "-stats-output-dir",
    "-plugin-path",
    "-load-plugin-executable",
    "-external-plugin-path",
    "-in-process-plugin-server-path",
    "-blocklist-file",
    "-coverage-prefix-map",
    "-debug-prefix-map",
    "-file-prefix-map",
];

/// Flags whose value is glued to the flag (`-DDEBUG`, `-I/inc`, `-lobjc`).
/// Order matters only in that none is a prefix of another (case keeps `-I`
/// uppercase distinct from `-isystem`).
const ATTACHED_PREFIXES: &[&str] = &["-I", "-F", "-L", "-D", "-U", "-l"];

/// Normalise an argv into [`Item`]s. Unknown flags become standalone items;
/// bare operands become `Item { flag: <token>, value: None }` (their absolute
/// paths still reach the structural tier).
#[must_use]
pub fn parse_argv(argv: &[String]) -> Vec<Item> {
    let mut out = Vec::with_capacity(argv.len());
    let mut i = 0;
    while i < argv.len() {
        let tok = &argv[i];
        if tok.starts_with('-') {
            if VALUE_FLAGS.contains(&tok.as_str()) {
                out.push(Item {
                    flag: tok.clone(),
                    value: argv.get(i + 1).cloned(),
                });
                i += 2;
                continue;
            }
            if let Some(p) = ATTACHED_PREFIXES
                .iter()
                .find(|p| tok.len() > p.len() && tok.starts_with(**p))
            {
                out.push(Item {
                    flag: (*p).to_string(),
                    value: Some(tok[p.len()..].to_string()),
                });
                i += 1;
                continue;
            }
            out.push(Item {
                flag: tok.clone(),
                value: None,
            });
            i += 1;
        } else {
            out.push(Item {
                flag: tok.clone(),
                value: None,
            });
            i += 1;
        }
    }
    out
}

/// Flags that are pure build geometry — an output/intermediate location the
/// generator has no reason to reproduce. Their items are counted but never
/// scored.
const GEOMETRY_FLAGS: &[&str] = &[
    "-o",
    "-output-file-map",
    "-index-store-path",
    "-serialize-diagnostics",
    "-emit-module-path",
    "-emit-objc-header-path",
    "-emit-module-interface-path",
    "-emit-private-module-interface-path",
    "-emit-package-module-interface-path",
    "-emit-dependencies-path",
    "-emit-reference-dependencies-path",
    "-MF",
    "-MT",
    "-MQ",
    "-module-cache-path",
    "-clang-build-session-file",
    "-clang-scanner-module-cache-path",
    "-sdk-module-cache-path",
    "-const-gather-protocols-file",
    "-filelist",
    "-working-directory",
    "-pch-output-dir",
    "-stats-output-dir",
    "-dependency-file",
    // clang dependency / diagnostics / index output plumbing
    "-MMD",
    "-MD",
    "-MM",
    "--serialize-diagnostics",
    "-index-unit-output-path",
];

/// Substrings that mark an item's value as a per-build intermediate/cache/output
/// artifact — geometry regardless of which flag carries it. (`Build/Products/`
/// is deliberately absent: a search path into the products dir is a real flag
/// whose path the structural tier credits.)
const GEOMETRY_MARKERS: &[&str] = &[
    ".hmap",
    "Intermediates.noindex",
    "ModuleCache.noindex",
    "Index.noindex",
    "SDKStatCaches.noindex",
    "CompilationCache.noindex",
    "-OutputFileMap.json",
    ".SwiftFileList",
    ".LinkFileList",
    ".modulevalidation",
    "/DerivedSources",
    "_const_extract_protocols",
    "_dependency_info.dat",
    "prebuilt-modules",
    "ExplicitPrecompiledModules",
];

/// Driver sub-flags that always introduce a build-geometry path in the
/// following `-Xcc`/`-Xfrontend`/`-Xlinker` token, so the flag itself is
/// geometry too. The `-Xlinker` ones are the linker's debug-info / dependency /
/// LTO output plumbing.
const GEOMETRY_XARG_VALUES: &[&str] = &[
    "-iquote",
    "-ivfsstatcache",
    "-ivfsoverlay",
    "-const-gather-protocols-file",
    "-add_ast_path",
    "-dependency_info",
    "-object_path_lto",
    "-final_output",
    "-map",
];

fn is_geometry(it: &Item) -> bool {
    if GEOMETRY_FLAGS.contains(&it.flag.as_str()) {
        return true;
    }
    // `-j<N>` is the host core count, not a reproducible decision.
    if let Some(rest) = it.flag.strip_prefix("-j")
        && !rest.is_empty()
        && rest.bytes().all(|b| b.is_ascii_digit())
    {
        return true;
    }
    // `-Xcc -iquote` / `-Xlinker -add_ast_path` etc. — the next token (a header
    // map / vfs / AST / dependency / LTO path) is geometry, so is the flag.
    if matches!(it.flag.as_str(), "-Xcc" | "-Xclang" | "-Xfrontend" | "-Xlinker")
        && it
            .value
            .as_deref()
            .is_some_and(|v| GEOMETRY_XARG_VALUES.contains(&v))
    {
        return true;
    }
    let blob = it.value.as_deref().unwrap_or("");
    GEOMETRY_MARKERS
        .iter()
        .any(|m| blob.contains(m) || it.flag.contains(m))
}

// ----- tier keys -----------------------------------------------------------

const SEP: char = '\u{1}';

fn exact_key(it: &Item) -> String {
    match &it.value {
        Some(v) => format!("{}{SEP}{}", it.flag, v),
        None => it.flag.clone(),
    }
}

fn canonical_key(it: &Item) -> String {
    let f = normalize_value(&it.flag, false);
    match &it.value {
        Some(v) => format!("{f}{SEP}{}", normalize_value(v, false)),
        None => f,
    }
}

fn structural_key(it: &Item) -> String {
    let f = normalize_value(&it.flag, true);
    match &it.value {
        Some(v) => format!("{f}{SEP}{}", normalize_value(v, true)),
        None => f,
    }
}

/// Normalise one flag or value for tier comparison. An attached search-path
/// prefix (`-I/p`, `-isystem/p`, `-fmodule-map-file=/p`, …) is split off first
/// so [`canonicalize_value`] sees only the path: otherwise a path that collapses
/// to `<PROJECT_ROOT>` swallows the `-I`, diverging from a DerivedData-anchored
/// counterpart that keeps it. At the structural tier any resulting absolute path
/// (bare, or behind a prefix) becomes `<ABS>`, so a search path matches whatever
/// root it points at — the argv analogue of the build-settings "both absolute
/// paths" tier.
fn normalize_value(v: &str, structural: bool) -> String {
    let (prefix, rest) = split_attached_path(v);
    let canon = canonicalize_value(rest);
    let body = if structural && is_path_like(&canon) {
        "<ABS>".to_string()
    } else {
        canon
    };
    format!("{prefix}{body}")
}

/// Split a leading attached path-prefix off a raw value that glues a path to it,
/// returning `(prefix, path)`; `("", value)` when there is none.
fn split_attached_path(v: &str) -> (&str, &str) {
    for p in [
        "-I",
        "-F",
        "-L",
        "-isystem",
        "-iquote",
        "-idirafter",
        "-ivfsoverlay",
        "-fmodule-map-file=",
        "-fprebuilt-module-path=",
        "-fmodules-cache-path=",
    ] {
        if let Some(rest) = v.strip_prefix(p)
            && rest.starts_with('/')
        {
            return (&v[..p.len()], rest);
        }
    }
    ("", v)
}

/// Whether a (possibly canonicalised) value is an absolute path — a raw `/` or a
/// placeholder [`canonicalize_value`] leaves at a path's head.
fn is_path_like(s: &str) -> bool {
    const HEADS: &[&str] = &[
        "/",
        "<HOME>",
        "<PROJECT_ROOT>",
        "<XCODE_DEV>",
        "<DARWIN_CACHE>",
        "<TUIST_BUILD>",
        "<BUILD_DIR>",
    ];
    HEADS.iter().any(|h| s.starts_with(h))
}

// ----- reconciliation ------------------------------------------------------

/// Per-comparison argv stats. `structural + missing == oracle_items` and
/// `structural + extra == our_items` by construction, so `structural_pct` is
/// recall and `precision_pct` is precision, both over the **scored** items
/// (geometry excluded).
#[derive(Default, Clone, Copy)]
pub struct ArgvStats {
    pub oracle_items: u64,
    pub our_items: u64,
    pub exact: u64,
    pub canonical: u64,
    pub structural: u64,
    pub missing: u64,
    pub extra: u64,
    pub geometry_oracle: u64,
    pub geometry_our: u64,
}

impl ArgvStats {
    pub fn merge(&mut self, o: ArgvStats) {
        self.oracle_items += o.oracle_items;
        self.our_items += o.our_items;
        self.exact += o.exact;
        self.canonical += o.canonical;
        self.structural += o.structural;
        self.missing += o.missing;
        self.extra += o.extra;
        self.geometry_oracle += o.geometry_oracle;
        self.geometry_our += o.geometry_our;
    }

    pub fn exact_pct(&self) -> u64 {
        pct(self.exact, self.oracle_items)
    }
    pub fn canonical_pct(&self) -> u64 {
        pct(self.canonical, self.oracle_items)
    }
    pub fn structural_pct(&self) -> u64 {
        pct(self.structural, self.oracle_items)
    }
    pub fn precision_pct(&self) -> u64 {
        pct(self.structural, self.our_items)
    }
}

fn pct(n: u64, d: u64) -> u64 {
    if d > 0 { n * 100 / d } else { 100 }
}

/// Greedy multiset reconcile of `o` against `m` by `key`. Returns how many
/// oracle items found a distinct partner and the unmatched leftovers on each
/// side.
fn reconcile(o: &[Item], m: &[Item], key: impl Fn(&Item) -> String) -> (u64, Vec<Item>, Vec<Item>) {
    let mut buckets: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    for (i, it) in m.iter().enumerate() {
        buckets.entry(key(it)).or_default().push(i);
    }
    let mut used = vec![false; m.len()];
    let mut matched = 0u64;
    let mut left_o = Vec::new();
    for it in o {
        let k = key(it);
        let hit = buckets
            .get(&k)
            .and_then(|idxs| idxs.iter().copied().find(|&i| !used[i]));
        if let Some(i) = hit {
            used[i] = true;
            matched += 1;
        } else {
            left_o.push(it.clone());
        }
    }
    let left_m = m
        .iter()
        .enumerate()
        .filter(|(i, _)| !used[*i])
        .map(|(_, it)| it.clone())
        .collect();
    (matched, left_o, left_m)
}

/// Score our argv against the oracle's. `miss`/`extra` accumulate the per-flag
/// tallies (recall / precision defects). Set `DEBUG_ARGV_FLAG=<flag>` to print
/// every miss/extra for that flag.
pub fn compare_argv(
    oracle: &[String],
    ours: &[String],
    miss: &mut MismatchTally,
    extra: &mut MismatchTally,
) -> ArgvStats {
    let mut st = ArgvStats::default();

    let mut o = Vec::new();
    for it in parse_argv(oracle) {
        if is_geometry(&it) {
            st.geometry_oracle += 1;
        } else {
            o.push(it);
        }
    }
    let mut m = Vec::new();
    for it in parse_argv(ours) {
        if is_geometry(&it) {
            st.geometry_our += 1;
        } else {
            m.push(it);
        }
    }
    st.oracle_items = o.len() as u64;
    st.our_items = m.len() as u64;

    let (e, o1, m1) = reconcile(&o, &m, exact_key);
    let (c, o2, m2) = reconcile(&o1, &m1, canonical_key);
    let (s, o3, m3) = reconcile(&o2, &m2, structural_key);
    st.exact = e;
    st.canonical = e + c;
    st.structural = e + c + s;
    st.missing = o3.len() as u64;
    st.extra = m3.len() as u64;

    let debug_flag = std::env::var("DEBUG_ARGV_FLAG").ok();
    for it in &o3 {
        *miss.entry(it.flag.clone()).or_insert(0) += 1;
        if debug_flag.as_deref() == Some(it.flag.as_str()) {
            eprintln!("MISSING {}", exact_key(it));
        }
    }
    for it in &m3 {
        *extra.entry(it.flag.clone()).or_insert(0) += 1;
        if debug_flag.as_deref() == Some(it.flag.as_str()) {
            eprintln!("EXTRA   {}", exact_key(it));
        }
    }
    st
}

/// Print the headline + a split missing/extra tally for one argv comparison
/// group (a tool family). The audit trail per run, mirroring the build-settings
/// `print_summary`.
pub fn print_argv_summary(title: &str, st: &ArgvStats, miss: &MismatchTally, extra: &MismatchTally) {
    println!("=== {title} ===");
    println!(
        "scored items: oracle={} ours={} | exact={} ({}%) canon={} ({}%) struct={} ({}%) precision={}%",
        st.oracle_items,
        st.our_items,
        st.exact,
        st.exact_pct(),
        st.canonical,
        st.canonical_pct(),
        st.structural,
        st.structural_pct(),
        st.precision_pct(),
    );
    println!(
        "geometry (excluded): oracle={} ours={} | missing={} extra={}",
        st.geometry_oracle, st.geometry_our, st.missing, st.extra
    );
    print_tally("missing (oracle has, we don't)", miss);
    print_tally("extra (we emit, oracle doesn't)", extra);
}

fn print_tally(label: &str, tally: &MismatchTally) {
    if tally.is_empty() {
        return;
    }
    println!("--- {label} ---");
    let mut entries: Vec<(&String, &u64)> = tally.iter().collect();
    entries.sort_by(|a, b| b.1.cmp(a.1));
    for (k, n) in entries.iter().take(40) {
        println!("  {n:<5} {k}");
    }
}
