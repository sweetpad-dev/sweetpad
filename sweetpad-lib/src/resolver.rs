use std::collections::BTreeMap;
use std::fmt;
use std::io;
use std::path::{Path, PathBuf};

use crate::condition;
use crate::xcconfig::{self, Assignment, Condition, Entry};

#[derive(Debug, Clone, Default)]
pub struct ResolveContext {
    pub sdk: String,
    pub arch: String,
    pub configuration: String,
    /// Build variant (`normal`, `profile`, `debug`). xcodebuild's default
    /// variant is `normal`, so callers resolving a standard build should pass
    /// `"normal"` for `[variant=normal]` conditions to match.
    pub variant: String,
}

impl ResolveContext {
    /// Whether one `[key=pattern]` condition matches this context's bindings.
    /// xcodebuild matches `[sdk=…]` against the *versioned* canonical SDK
    /// name (`macosx26.0`) — [`crate::build_context::BuildContext`] binds
    /// that canonical name from the catalog, so the ubiquitous
    /// `[sdk=macosx*]` form matches while a bare `[sdk=macosx]` does not,
    /// exactly like xcodebuild. Its aggregated `-showBuildSettings` view
    /// likewise binds `arch=undefined_arch` (per-arch conditionals only fire
    /// for a concrete per-arch resolve, e.g. compiler args).
    #[must_use]
    pub fn matches(&self, cond: &Condition) -> bool {
        match cond.key.as_str() {
            "sdk" => glob_match(&cond.value, &self.sdk),
            "arch" => glob_match(&cond.value, &self.arch),
            "config" | "configuration" => glob_match(&cond.value, &self.configuration),
            "variant" => glob_match(&cond.value, &self.variant),
            _ => false,
        }
    }
}

/// Glob over `*` — any number of stars, at any position (`*64` matches
/// `arm64`). xcodebuild condition patterns use only `*`; there is no `?` or
/// character-class syntax. Iterative two-pointer matching with backtracking
/// to the most recent star; byte-wise comparison is exact for UTF-8 since
/// `*` is ASCII.
fn glob_match(pattern: &str, s: &str) -> bool {
    let p = pattern.as_bytes();
    let t = s.as_bytes();
    let (mut pi, mut ti) = (0usize, 0usize);
    // Index of the last `*` seen in the pattern, and the position in `t`
    // that star is currently matched up to.
    let mut star: Option<usize> = None;
    let mut mark = 0usize;
    while ti < t.len() {
        if pi < p.len() && p[pi] == b'*' {
            star = Some(pi);
            mark = ti;
            pi += 1;
        } else if pi < p.len() && p[pi] == t[ti] {
            pi += 1;
            ti += 1;
        } else if let Some(sp) = star {
            // Mismatch after a star: let the star consume one more byte.
            pi = sp + 1;
            mark += 1;
            ti = mark;
        } else {
            return false;
        }
    }
    // Trailing stars match the empty remainder.
    while pi < p.len() && p[pi] == b'*' {
        pi += 1;
    }
    pi == p.len()
}

#[derive(Debug)]
pub enum Error {
    Io {
        path: PathBuf,
        source: io::Error,
    },
    Parse {
        path: PathBuf,
        source: xcconfig::ParseError,
    },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Io { path, source } => write!(f, "I/O error on {}: {source}", path.display()),
            Error::Parse { path, source } => {
                write!(f, "parse error in {}: {source}", path.display())
            }
        }
    }
}

impl std::error::Error for Error {}

/// Load an xcconfig file and recursively inline any #include directives,
/// producing a flat list of assignments in source order.
pub fn flatten_xcconfig(path: &Path) -> Result<Vec<Assignment>, Error> {
    let mut out = Vec::new();
    flatten_into(path, &mut out, &mut Vec::new())?;
    Ok(out)
}

fn flatten_into(
    path: &Path,
    out: &mut Vec<Assignment>,
    chain: &mut Vec<PathBuf>,
) -> Result<(), Error> {
    // Cycle guard: Xcode warns ("Skipping the inclusion of … because it is
    // already included") and skips a file already on the include chain rather
    // than failing the build — or, as naive recursion would here, overflowing
    // the stack. Canonicalize so the same file reached through different
    // lexical spellings is still caught; fall back to the lexical path when
    // canonicalize fails (e.g. the file is missing — the read below reports
    // that properly).
    let id = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    if chain.contains(&id) {
        return Ok(());
    }
    chain.push(id);
    let result = flatten_entries(path, out, chain);
    chain.pop();
    result
}

fn flatten_entries(
    path: &Path,
    out: &mut Vec<Assignment>,
    chain: &mut Vec<PathBuf>,
) -> Result<(), Error> {
    // Shared cache entry — iterate by reference and clone each assignment out
    // rather than consuming the (now `Arc`-owned) parse.
    let xcc = xcconfig::parse_file_cached(path).map_err(|e| match e {
        xcconfig::Error::Io(source) => Error::Io {
            path: path.to_path_buf(),
            source,
        },
        xcconfig::Error::Parse(source) => Error::Parse {
            path: path.to_path_buf(),
            source,
        },
    })?;
    let base_dir = path.parent().unwrap_or_else(|| Path::new("."));
    for entry in &xcc.entries {
        match entry {
            Entry::Assignment(a) => out.push(a.clone()),
            Entry::Include(inc) => {
                // A leading `<DEVELOPER_DIR>` is a literal placeholder
                // xcodebuild substitutes with the active Developer directory
                // (e.g. `#include "<DEVELOPER_DIR>/Library/Xcode/…"`).
                let inc_path = if let Some(rest) = inc.path.strip_prefix("<DEVELOPER_DIR>") {
                    crate::xcode::detect_developer_dir().join(rest.trim_start_matches('/'))
                } else {
                    base_dir.join(&inc.path)
                };
                match flatten_into(&inc_path, out, chain) {
                    Ok(()) => {}
                    // `#include?` forgives exactly one failure mode: the
                    // optional file *itself* being absent. A present-but-
                    // malformed optional include, or a non-optional missing
                    // include nested inside it, is still a hard error —
                    // matching xcodebuild.
                    Err(Error::Io { path, source })
                        if inc.optional
                            && path == inc_path
                            && source.kind() == io::ErrorKind::NotFound => {}
                    Err(e) => return Err(e),
                }
            }
        }
    }
    Ok(())
}

/// Resolve a stack of layers (each a slice of assignments) into a flat
/// settings map under the given context.
///
/// Layers apply in order. Within a layer, every assignment whose conditions
/// all match the context is evaluated; later matches override earlier ones for
/// the same key, with `$(inherited)` (or a self-reference) in the later value
/// folding in the earlier same-layer value first. Any `$(inherited)` /
/// `${inherited}` still present after that is substituted with the value
/// resolved by earlier layers (or "" if none).
///
/// xcspec assignments may also carry a free-form `Condition` expression (see
/// [`crate::condition`]). Those are evaluated in a second pass against the
/// result of pass 1 — that way conditions can reference `$(PRODUCT_TYPE)`,
/// `$(MACH_O_TYPE)`, and other settings that aren't known until the bulk of
/// the resolve has already happened.
///
/// After all layers merge, `$(VAR)`, `${VAR}`, and `${VAR:modifier}` references
/// are expanded against the merged map by fixed-point iteration.
#[must_use]
pub fn resolve(layers: &[&[Assignment]], ctx: &ResolveContext) -> BTreeMap<String, String> {
    // Pass 1: build a baseline ignoring `Assignment.condition` (all are
    // treated as truthy). This gives us a dict that the conditions in pass 2
    // can be evaluated against.
    let baseline = resolve_once(layers, ctx, None);

    // If no conditional assignments are present at all, skip pass 2.
    if !layers
        .iter()
        .any(|l| l.iter().any(|a| a.condition.is_some()))
    {
        return baseline;
    }

    // Inject context values (`arch`, `variant`, `configuration`, `sdk`) so
    // conditions that reference them work — xcspec sources actually write
    // `$(variant) == profile` (lowercase). The baseline already has
    // capitalised CONFIGURATION etc. from the project layer; we add the
    // lowercase aliases without overwriting.
    let eval_dict = with_context_aliases(&baseline, ctx);

    // Pass 2: re-resolve, this time consulting `Assignment.condition` and
    // dropping any whose expression evaluates false against the pass-1 dict.
    resolve_once(layers, ctx, Some(&eval_dict))
}

fn resolve_once(
    layers: &[&[Assignment]],
    ctx: &ResolveContext,
    condition_against: Option<&BTreeMap<String, String>>,
) -> BTreeMap<String, String> {
    let mut current: BTreeMap<String, String> = BTreeMap::new();
    for layer in layers {
        let reduced = reduce_layer(layer, ctx, condition_against);
        for (key, value) in reduced {
            let inherited = current.get(&key).cloned().unwrap_or_default();
            let merged = substitute_inherited(&key, &value, &inherited);
            current.insert(key, merged);
        }
    }
    expand_variables(&mut current);
    current
}

fn with_context_aliases(
    baseline: &BTreeMap<String, String>,
    ctx: &ResolveContext,
) -> BTreeMap<String, String> {
    let mut d = baseline.clone();
    // Don't clobber if the layer already set a different value.
    for (k, v) in [
        ("variant", ctx.variant.as_str()),
        ("arch", ctx.arch.as_str()),
        ("configuration", ctx.configuration.as_str()),
        ("sdk", ctx.sdk.as_str()),
    ] {
        d.entry(k.to_string()).or_insert_with(|| v.to_string());
    }
    d
}

fn reduce_layer(
    assignments: &[Assignment],
    ctx: &ResolveContext,
    condition_against: Option<&BTreeMap<String, String>>,
) -> Vec<(String, String)> {
    let mut order: Vec<String> = Vec::new();
    let mut map: BTreeMap<String, String> = BTreeMap::new();
    for ass in assignments {
        if !ass.conditions.iter().all(|c| ctx.matches(c)) {
            continue;
        }
        if let (Some(against), Some(raw)) = (condition_against, ass.condition.as_deref())
            && let Some(expr) = condition::parse(raw)
            && !condition::evaluate(&expr, against)
        {
            continue;
        }
        // xcodebuild chains assignments for the same key within one table:
        // `$(inherited)` (or a self-reference) in a later assignment picks up
        // the value accumulated *earlier in this same layer*, so an
        // `#include`d default composes with the includer's `KEY = $(inherited)
        // extra` (the include and the includer flatten into one layer here).
        // An unreplaced `$(inherited)` — no earlier same-layer assignment —
        // survives to merge time, where it picks up the lower layer.
        let folded = if let Some(prev) = map.get(&ass.key) {
            substitute_inherited(&ass.key, &ass.value, prev)
        } else {
            order.push(ass.key.clone());
            ass.value.clone()
        };
        map.insert(ass.key.clone(), folded);
    }
    order
        .into_iter()
        .map(|k| {
            let v = map.remove(&k).unwrap_or_default();
            (k, v)
        })
        .collect()
}

fn substitute_inherited(key: &str, value: &str, inherited: &str) -> String {
    // `$(inherited)` / `${inherited}` mean "the value resolved by lower layers".
    // A setting referencing *itself* (`$(KEY)` / `${KEY}` inside KEY's own value)
    // means the same thing — xcodebuild resolves `DEVELOPMENT_TEAM =
    // $(DEVELOPMENT_TEAM)` to the inherited project/xcconfig value, not to empty,
    // and `FRAMEWORK_SEARCH_PATHS = $(FRAMEWORK_SEARCH_PATHS) extra` to
    // "<inherited> extra". We fold both into the inherited value here, at merge
    // time, so the lower-layer value isn't lost when an upper layer re-states the
    // key in terms of itself. (`strip_self_reference` in the variable-expansion
    // pass remains as a backstop for any self-reference that survives this.)
    //
    // Both forms also accept a modifier chain — `$(inherited:lower)`,
    // `${KEY:suffix}` — which xcodebuild applies to the inherited value, so we
    // substitute the transformed value rather than dropping it.
    let bytes = value.as_bytes();
    let mut out = String::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'$' && i + 1 < bytes.len() && (bytes[i + 1] == b'(' || bytes[i + 1] == b'{')
        {
            let open = bytes[i + 1];
            let close = if open == b'(' { b')' } else { b'}' };
            let body_start = i + 2;
            let mut j = body_start;
            let mut depth = 1i32;
            while j < bytes.len() {
                let c = bytes[j];
                if c == open {
                    depth += 1;
                } else if c == close {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
                j += 1;
            }
            if depth == 0 {
                let body = &value[body_start..j];
                let (name, mods) = match find_top_level_colon(body) {
                    Some(idx) => (&body[..idx], Some(&body[idx + 1..])),
                    None => (body, None),
                };
                if name == "inherited" || name == key {
                    match mods {
                        Some(m) => out.push_str(&apply_modifiers(inherited, m)),
                        None => out.push_str(inherited),
                    }
                    i = j + 1;
                    continue;
                }
            }
            // Not an inherited/self reference (or unterminated): emit the `$`
            // and rescan from the open bracket so nested references inside —
            // `$(FOO_$(inherited))` — are still found.
            out.push('$');
            i += 1;
        } else {
            let c = value[i..].chars().next().expect("valid UTF-8 boundary");
            out.push(c);
            i += c.len_utf8();
        }
    }
    out
}

fn expand_variables(map: &mut BTreeMap<String, String>) {
    for _ in 0..16 {
        let snapshot = map.clone();
        let mut changed = false;
        for value in map.values_mut() {
            let expanded = expand_one(value, &snapshot);
            if *value != expanded {
                *value = expanded;
                changed = true;
            }
        }
        if !changed {
            return;
        }
    }
}

/// Expand `$(VAR)` / `${VAR}` references — including nested forms like
/// `$(FOO_$(BAR))` and modifier syntax (`:lower`, `:default=X`) — against the
/// supplied lookup map. Bounded by [`MAX_EXPAND_DEPTH`] to prevent runaway
/// recursion on cyclic chains.
#[must_use]
pub fn expand_one(value: &str, lookup: &BTreeMap<String, String>) -> String {
    expand_one_with_depth(value, lookup, 0)
}

const MAX_EXPAND_DEPTH: usize = 32;

/// Per-value expansion output budget. The depth cap alone doesn't bound the
/// *work*: a doubling chain (`A = $(B) $(B)`, `B = $(C) $(C)`, …) is ~2^32
/// bytes before [`MAX_EXPAND_DEPTH`] binds. Past the budget the rest of the
/// value is carried through unexpanded; real settings are nowhere near it.
const MAX_EXPAND_BYTES: usize = 1 << 20;

fn expand_one_with_depth(value: &str, lookup: &BTreeMap<String, String>, depth: usize) -> String {
    if depth >= MAX_EXPAND_DEPTH {
        return value.to_string();
    }
    let mut out = String::new();
    let bytes = value.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if out.len() >= MAX_EXPAND_BYTES {
            out.push_str(&value[i..]);
            break;
        }
        let b = bytes[i];
        if b == b'$' && i + 1 < bytes.len() && bytes[i + 1] == b'$' {
            // `$$` is an escaped dollar — swift-build collapses it to a
            // literal `$`, which therefore cannot start a reference even when
            // a `(` follows.
            out.push('$');
            i += 2;
        } else if b == b'$' && i + 1 < bytes.len() && (bytes[i + 1] == b'(' || bytes[i + 1] == b'{')
        {
            let open = bytes[i + 1];
            let close = if open == b'(' { b')' } else { b'}' };
            let body_start = i + 2;
            // Track bracket depth so that `$(FOO_$(BAR))` matches the OUTER
            // close paren, not the inner one. Apple's xcspec defaults use
            // this nested form heavily.
            let mut j = body_start;
            let mut paren_depth = 1i32;
            while j < bytes.len() {
                let c = bytes[j];
                if c == open {
                    paren_depth += 1;
                } else if c == close {
                    paren_depth -= 1;
                    if paren_depth == 0 {
                        break;
                    }
                }
                j += 1;
            }
            if paren_depth != 0 {
                // Unterminated — pass through the `$`.
                out.push('$');
                i += 1;
                continue;
            }
            let spec = std::str::from_utf8(&bytes[body_start..j]).unwrap_or("");
            // Resolve any nested references inside the spec itself before
            // looking up the result. This is what lets `$(FOO_$(BAR))` work:
            // we first expand `FOO_$(BAR)` to e.g. `FOO_baz`, then look that
            // up.
            let resolved_spec = if spec.contains('$') {
                expand_one_with_depth(spec, lookup, depth + 1)
            } else {
                spec.to_string()
            };
            out.push_str(&resolve_var_with_depth(&resolved_spec, lookup, depth + 1));
            i = j + 1;
        } else if b == b'$' && i + 1 < bytes.len() && is_bare_var_start(bytes[i + 1]) {
            // Bare `$NAME` form (no parens/braces). Tuist-generated pbxprojs
            // use this for things like `$BUILD_DIR/Debug$EFFECTIVE_PLATFORM_NAME`.
            // Only expand when NAME resolves to a defined value — if it
            // doesn't, leave the `$` and the name intact so we don't
            // accidentally eat literal text that just happens to start with
            // `$<letter>` (script snippets, code samples in user comments
            // pulled into a setting, etc).
            let name_start = i + 1;
            let mut j = name_start;
            while j < bytes.len() && is_bare_var_continue(bytes[j]) {
                j += 1;
            }
            let name = std::str::from_utf8(&bytes[name_start..j]).unwrap_or("");
            if !name.is_empty() && lookup.contains_key(name) {
                out.push_str(&resolve_var_with_depth(name, lookup, depth + 1));
                i = j;
            } else {
                out.push('$');
                i += 1;
            }
        } else {
            let c = value[i..].chars().next().expect("valid UTF-8 boundary");
            out.push(c);
            i += c.len_utf8();
        }
    }
    out
}

fn is_bare_var_start(b: u8) -> bool {
    b.is_ascii_alphabetic() || b == b'_'
}

fn is_bare_var_continue(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

fn resolve_var_with_depth(spec: &str, lookup: &BTreeMap<String, String>, depth: usize) -> String {
    // Only split on a `:` that isn't inside a nested `$()` or `${}` — if a
    // looked-up value still contains an unresolved `$(VAR:modifier)` ref, we
    // mustn't treat its inner colon as the modifier separator for the spec.
    let (name, modifier) = match find_top_level_colon(spec) {
        Some(idx) => (&spec[..idx], Some(&spec[idx + 1..])),
        None => (spec, None),
    };
    let raw = lookup.get(name).cloned().unwrap_or_default();
    // Apple's xcspecs define settings like `FRAMEWORK_SEARCH_PATHS =
    // $(FRAMEWORK_SEARCH_PATHS) $(SDKROOT)/...` — a literal self-reference
    // that, naively iterated, doubles the value on every fixed-point pass and
    // blows up to megabytes. Apple's resolver treats self-references inside a
    // setting's own value as empty; do the same.
    let raw = strip_self_reference(&raw, name);
    // Always fully resolve the looked-up value before returning. Otherwise
    // unresolved `$(...)` refs in the value leak into whatever string the
    // caller is building and break downstream parsing (e.g. an outer spec
    // like `FOO_$(BAR)` where BAR resolves to `$(BAZ:default=YES)` would
    // otherwise yield `FOO_$(BAZ:default=YES)` and confuse the next
    // resolve_var into mis-splitting on the inner colon). The depth cap
    // prevents runaway recursion on cyclic chains.
    let resolved = if raw.contains('$') && depth < MAX_EXPAND_DEPTH {
        expand_one_with_depth(&raw, lookup, depth + 1)
    } else {
        raw
    };
    match modifier {
        Some(m) => apply_modifiers(&resolved, m),
        None => resolved,
    }
}

/// Apply a (possibly chained) modifier spec — `lower:rfc1034identifier` —
/// left to right: `$(PRODUCT_NAME:lower:rfc1034identifier)` lowercases first,
/// then mangles. Segments split only on top-level colons (matching
/// [`find_top_level_colon`]) so a nested reference inside a `default=`
/// payload keeps its own colon; each segment — `default=…` included —
/// consumes exactly itself.
fn apply_modifiers(raw: &str, spec: &str) -> String {
    let mut value = raw.to_string();
    let mut rest = spec;
    loop {
        match find_top_level_colon(rest) {
            Some(idx) => {
                value = apply_modifier(&value, &rest[..idx]);
                rest = &rest[idx + 1..];
            }
            None => return apply_modifier(&value, rest),
        }
    }
}

fn find_top_level_colon(s: &str) -> Option<usize> {
    let bytes = s.as_bytes();
    let mut depth = 0i32;
    for (i, &b) in bytes.iter().enumerate() {
        match b {
            b'(' | b'{' => depth += 1,
            b')' | b'}' => depth -= 1,
            b':' if depth == 0 => return Some(i),
            _ => {}
        }
    }
    None
}

fn strip_self_reference(value: &str, var_name: &str) -> String {
    let paren = format!("$({var_name})");
    let brace = format!("${{{var_name}}}");
    if !value.contains(&paren) && !value.contains(&brace) {
        return value.to_string();
    }
    value.replace(&paren, "").replace(&brace, "")
}

fn apply_modifier(raw: &str, modifier: &str) -> String {
    if let Some(default_val) = modifier.strip_prefix("default=") {
        return if raw.is_empty() {
            default_val.to_string()
        } else {
            raw.to_string()
        };
    }
    match modifier {
        "lower" => raw.to_lowercase(),
        "upper" => raw.to_uppercase(),
        // Quote only when something actually needs quoting (whitespace or a
        // shell-special character); plain values pass through untouched.
        "quote" => quote_for_shell(raw),
        // `identifier` / `c99extidentifier` turn a human-readable string
        // (e.g. `$(PRODUCT_NAME:c99extidentifier)`) into a valid C/Swift/
        // Objective-C identifier: alphanumerics + `_` pass through, every
        // other character becomes `_`, and a leading digit gains a `_`
        // prefix so the result never starts with a digit.
        "identifier" | "c99extidentifier" => {
            let mut s: String = raw
                .chars()
                .map(|c| {
                    if c.is_ascii_alphanumeric() || c == '_' {
                        c
                    } else {
                        '_'
                    }
                })
                .collect();
            if s.chars().next().is_some_and(|c| c.is_ascii_digit()) {
                s.insert(0, '_');
            }
            s
        }
        // RFC 1034 DNS-label mangling for bundle identifiers: letters,
        // digits, `-`, and `.` (the component separator) are preserved;
        // everything else becomes `-`. "My.App Name" → "My.App-Name".
        "rfc1034identifier" => raw
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '-' || c == '.' {
                    c
                } else {
                    '-'
                }
            })
            .collect(),
        "dir" => path_dir(raw),
        "file" => path_file(raw).to_string(),
        "base" => split_ext(path_file(raw)).0.to_string(),
        "suffix" => split_ext(path_file(raw)).1.to_string(),
        "standardizepath" => standardize_path(raw),
        _ => raw.to_string(),
    }
}

/// `:dir` — the directory part: "/tmp/a/b.txt" → "/tmp/a", a bare filename
/// → ".", a first-level path → "/". Trailing slashes are ignored
/// ("/tmp/a/" names the same entry as "/tmp/a") — Apple leaves this corner
/// undocumented, so we take the dirname-like reading.
fn path_dir(raw: &str) -> String {
    let trimmed = raw.trim_end_matches('/');
    if trimmed.is_empty() {
        // "" → "."; "/" (all slashes) → "/".
        return if raw.is_empty() {
            ".".into()
        } else {
            "/".into()
        };
    }
    match trimmed.rfind('/') {
        Some(0) => "/".to_string(),
        Some(i) => trimmed[..i].to_string(),
        None => ".".to_string(),
    }
}

/// `:file` — the last path component: "/tmp/a/b.txt" → "b.txt". Trailing
/// slashes are ignored, same reading as [`path_dir`].
fn path_file(raw: &str) -> &str {
    let trimmed = raw.trim_end_matches('/');
    if trimmed.is_empty() {
        return if raw.is_empty() { "" } else { "/" };
    }
    match trimmed.rfind('/') {
        Some(i) => &trimmed[i + 1..],
        None => trimmed,
    }
}

/// Split a filename at its extension dot: "b.txt" → ("b", ".txt"); no dot →
/// empty suffix. A *leading* dot (".bashrc") is part of the name, not an
/// extension separator — matching NSString's pathExtension behavior.
fn split_ext(file: &str) -> (&str, &str) {
    match file.rfind('.') {
        Some(i) if i > 0 => (&file[..i], &file[i..]),
        _ => (file, ""),
    }
}

/// `:standardizepath` — lexically resolve `.` and `..` segments and collapse
/// `//`. The lexical subset of NSString's `standardizingPath` (no symlink
/// resolution; `..` at an absolute root drops out, leading `..` on a
/// relative path is kept).
fn standardize_path(raw: &str) -> String {
    let absolute = raw.starts_with('/');
    let mut parts: Vec<&str> = Vec::new();
    for seg in raw.split('/') {
        match seg {
            "" | "." => {}
            ".." => match parts.last() {
                Some(&"..") | None if !absolute => parts.push(".."),
                Some(_) => {
                    parts.pop();
                }
                _ => {}
            },
            other => parts.push(other),
        }
    }
    let joined = parts.join("/");
    if absolute {
        format!("/{joined}")
    } else if joined.is_empty() {
        ".".to_string()
    } else {
        joined
    }
}

/// `:quote` — shell-quote only when needed. A value made purely of
/// characters safe in an unquoted shell word passes through unchanged;
/// anything containing whitespace or a shell-special character is wrapped
/// in single quotes, with embedded single quotes escaped as `'\''`.
fn quote_for_shell(raw: &str) -> String {
    fn safe(c: char) -> bool {
        c.is_ascii_alphanumeric()
            || matches!(c, '_' | '-' | '+' | '=' | '/' | '.' | ',' | ':' | '@' | '%')
    }
    if !raw.is_empty() && raw.chars().all(safe) {
        return raw.to_string();
    }
    format!("'{}'", raw.replace('\'', "'\\''"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::xcconfig::Condition;

    fn unconditional(key: &str, value: &str) -> Assignment {
        Assignment {
            key: key.into(),
            conditions: Vec::new(),
            value: value.into(),
            condition: None,
        }
    }

    fn conditional(key: &str, conds: &[(&str, &str)], value: &str) -> Assignment {
        Assignment {
            key: key.into(),
            conditions: conds
                .iter()
                .map(|(k, v)| Condition {
                    key: (*k).into(),
                    value: (*v).into(),
                })
                .collect(),
            value: value.into(),
            condition: None,
        }
    }

    fn ctx_macos() -> ResolveContext {
        ResolveContext {
            sdk: "macosx".into(),
            arch: "arm64".into(),
            configuration: "Debug".into(),
            variant: String::new(),
        }
    }

    #[test]
    fn resolves_unconditional() {
        let layer = vec![unconditional("FOO", "bar")];
        let r = resolve(&[&layer], &ctx_macos());
        assert_eq!(r.get("FOO").map(String::as_str), Some("bar"));
    }

    #[test]
    fn conditional_matching_sdk_overrides_base() {
        let layer = vec![
            unconditional("FOO", "base"),
            conditional("FOO", &[("sdk", "macosx*")], "macos"),
        ];
        let r = resolve(&[&layer], &ctx_macos());
        assert_eq!(r.get("FOO").map(String::as_str), Some("macos"));
    }

    #[test]
    fn non_matching_condition_falls_back_to_base() {
        let layer = vec![
            unconditional("FOO", "base"),
            conditional("FOO", &[("sdk", "iphoneos*")], "ios"),
        ];
        let r = resolve(&[&layer], &ctx_macos());
        assert_eq!(r.get("FOO").map(String::as_str), Some("base"));
    }

    #[test]
    fn self_reference_inherits_lower_layer() {
        // `KEY = $(KEY)` in an upper layer means "the value from below" —
        // exactly like `$(inherited)`. This is how `DEVELOPMENT_TEAM =
        // $(DEVELOPMENT_TEAM)` resolves to the project-level team id.
        let lower = vec![unconditional("DEVELOPMENT_TEAM", "TEAM123")];
        let upper = vec![unconditional("DEVELOPMENT_TEAM", "$(DEVELOPMENT_TEAM)")];
        let r = resolve(&[&lower, &upper], &ctx_macos());
        assert_eq!(
            r.get("DEVELOPMENT_TEAM").map(String::as_str),
            Some("TEAM123")
        );
    }

    #[test]
    fn self_reference_appends_to_inherited() {
        // The Apple-xcspec idiom `KEY = $(KEY) extra` keeps the inherited value
        // and appends to it (rather than being stripped to just "extra").
        let lower = vec![unconditional("OTHER_LDFLAGS", "-lz")];
        let upper = vec![unconditional("OTHER_LDFLAGS", "$(OTHER_LDFLAGS) -lssl")];
        let r = resolve(&[&lower, &upper], &ctx_macos());
        assert_eq!(
            r.get("OTHER_LDFLAGS").map(String::as_str),
            Some("-lz -lssl")
        );
    }

    #[test]
    fn self_reference_with_no_lower_layer_is_empty() {
        let layer = vec![unconditional("DEVELOPMENT_TEAM", "$(DEVELOPMENT_TEAM)")];
        let r = resolve(&[&layer], &ctx_macos());
        assert_eq!(r.get("DEVELOPMENT_TEAM").map(String::as_str), Some(""));
    }

    #[test]
    fn inherited_picks_up_lower_layer() {
        let lower = vec![unconditional("OTHER_LDFLAGS", "-lm")];
        let upper = vec![unconditional(
            "OTHER_LDFLAGS",
            "$(inherited) -framework Foundation",
        )];
        let r = resolve(&[&lower, &upper], &ctx_macos());
        assert_eq!(
            r.get("OTHER_LDFLAGS").map(String::as_str),
            Some("-lm -framework Foundation")
        );
    }

    #[test]
    fn inherited_with_no_lower_layer_is_empty() {
        let layer = vec![unconditional(
            "OTHER_LDFLAGS",
            "$(inherited) -framework Foundation",
        )];
        let r = resolve(&[&layer], &ctx_macos());
        assert_eq!(
            r.get("OTHER_LDFLAGS").map(String::as_str),
            Some(" -framework Foundation")
        );
    }

    #[test]
    fn expands_simple_variable() {
        let layer = vec![
            unconditional("NAME", "world"),
            unconditional("GREETING", "hello $(NAME)"),
        ];
        let r = resolve(&[&layer], &ctx_macos());
        assert_eq!(r.get("GREETING").map(String::as_str), Some("hello world"));
    }

    #[test]
    fn expands_brace_variable() {
        let layer = vec![
            unconditional("NAME", "world"),
            unconditional("GREETING", "hello ${NAME}"),
        ];
        let r = resolve(&[&layer], &ctx_macos());
        assert_eq!(r.get("GREETING").map(String::as_str), Some("hello world"));
    }

    #[test]
    fn modifier_lower_and_upper() {
        let layer = vec![
            unconditional("BASE", "HelloWorld"),
            unconditional("LOW", "${BASE:lower}"),
            unconditional("UP", "${BASE:upper}"),
        ];
        let r = resolve(&[&layer], &ctx_macos());
        assert_eq!(r.get("LOW").map(String::as_str), Some("helloworld"));
        assert_eq!(r.get("UP").map(String::as_str), Some("HELLOWORLD"));
    }

    #[test]
    fn modifier_default_uses_fallback_when_unset() {
        let layer = vec![unconditional("OUT", "${UNSET:default=fallback}")];
        let r = resolve(&[&layer], &ctx_macos());
        assert_eq!(r.get("OUT").map(String::as_str), Some("fallback"));
    }

    #[test]
    fn modifier_default_keeps_value_when_set() {
        let layer = vec![
            unconditional("X", "hi"),
            unconditional("OUT", "${X:default=fallback}"),
        ];
        let r = resolve(&[&layer], &ctx_macos());
        assert_eq!(r.get("OUT").map(String::as_str), Some("hi"));
    }

    #[test]
    fn arch_condition_matches() {
        let layer = vec![
            unconditional("FOO", "base"),
            conditional("FOO", &[("arch", "arm64")], "arm64_val"),
        ];
        let r = resolve(&[&layer], &ctx_macos());
        assert_eq!(r.get("FOO").map(String::as_str), Some("arm64_val"));
    }

    #[test]
    fn config_condition_matches() {
        let layer = vec![
            unconditional("BAZ", "base"),
            conditional("BAZ", &[("config", "Debug")], "debug_val"),
            conditional("BAZ", &[("config", "Release")], "release_val"),
        ];
        let r = resolve(&[&layer], &ctx_macos());
        assert_eq!(r.get("BAZ").map(String::as_str), Some("debug_val"));
    }

    #[test]
    fn combined_conditions_require_all_to_match() {
        let layer = vec![
            unconditional("FOO", "base"),
            conditional("FOO", &[("sdk", "macosx*"), ("arch", "arm64")], "matched"),
            conditional(
                "FOO",
                &[("sdk", "macosx*"), ("arch", "x86_64")],
                "not_matched",
            ),
        ];
        let r = resolve(&[&layer], &ctx_macos());
        assert_eq!(r.get("FOO").map(String::as_str), Some("matched"));
    }
}
