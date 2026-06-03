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
    pub variant: String,
}

impl ResolveContext {
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

fn glob_match(pattern: &str, s: &str) -> bool {
    if pattern == "*" {
        true
    } else if let Some(prefix) = pattern.strip_suffix('*') {
        s.starts_with(prefix)
    } else {
        pattern == s
    }
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
    flatten_into(path, &mut out)?;
    Ok(out)
}

fn flatten_into(path: &Path, out: &mut Vec<Assignment>) -> Result<(), Error> {
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
                let inc_path = base_dir.join(&inc.path);
                match flatten_into(&inc_path, out) {
                    Ok(()) => {}
                    Err(e) => {
                        if inc.optional {
                            // #include? silently skips missing/erroring files.
                            let _ = e;
                        } else {
                            return Err(e);
                        }
                    }
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
/// the same key. `$(inherited)` and `${inherited}` in a layer's value are
/// substituted with the value resolved by earlier layers (or "" if none).
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
        if !map.contains_key(&ass.key) {
            order.push(ass.key.clone());
        }
        map.insert(ass.key.clone(), ass.value.clone());
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
    let self_paren = format!("$({key})");
    let self_brace = format!("${{{key}}}");
    value
        .replace("$(inherited)", inherited)
        .replace("${inherited}", inherited)
        .replace(&self_paren, inherited)
        .replace(&self_brace, inherited)
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

fn expand_one_with_depth(value: &str, lookup: &BTreeMap<String, String>, depth: usize) -> String {
    if depth >= MAX_EXPAND_DEPTH {
        return value.to_string();
    }
    let mut out = String::new();
    let bytes = value.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if b == b'$' && i + 1 < bytes.len() && (bytes[i + 1] == b'(' || bytes[i + 1] == b'{') {
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
        Some(m) => apply_modifier(&resolved, m),
        None => resolved,
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
        "quote" => format!("'{raw}'"),
        "identifier" => raw
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '_' {
                    c
                } else {
                    '_'
                }
            })
            .collect(),
        "rfc1034identifier" => raw
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '-' {
                    c
                } else {
                    '-'
                }
            })
            .collect(),
        // `c99extidentifier` is Apple's C99-extended-identifier modifier:
        // alphanumerics + `_` pass through, every other character becomes
        // `_`. Used by `$(PRODUCT_NAME:c99extidentifier)` to turn a
        // human-readable product name into a valid Swift/Objective-C
        // module name.
        "c99extidentifier" => raw
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '_' {
                    c
                } else {
                    '_'
                }
            })
            .collect(),
        _ => raw.to_string(),
    }
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
