//! Regression tests for xcconfig parse/resolve engine behaviors verified
//! against real xcodebuild: include-cycle handling, same-layer `$(inherited)`
//! chaining, chained/extended modifiers, comment stripping, `#include?`
//! error scoping, `$$` escaping, full glob conditions, and
//! NSString.boolValue condition coercion.

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use sweetpad::condition;
use sweetpad::resolver::{ResolveContext, expand_one, flatten_xcconfig, resolve};
use sweetpad::xcconfig::{Assignment, Entry, parse};

fn ctx_macos() -> ResolveContext {
    ResolveContext {
        sdk: "macosx".into(),
        arch: "arm64".into(),
        configuration: "Debug".into(),
        variant: String::new(),
    }
}

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
            .map(|(k, v)| sweetpad::xcconfig::Condition {
                key: (*k).into(),
                value: (*v).into(),
            })
            .collect(),
        value: value.into(),
        condition: None,
    }
}

/// Fresh per-test scratch dir under the system temp dir.
fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("sweetpad-xcc-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn lookup(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
        .collect()
}

// =====================================================================
// Fix 1: #include cycles must not crash (Xcode warns and skips)
// =====================================================================

#[test]
fn include_cycle_between_two_files_is_skipped() {
    let dir = scratch("cycle2");
    let a = dir.join("a.xcconfig");
    let b = dir.join("b.xcconfig");
    fs::write(&a, "#include \"b.xcconfig\"\nA_KEY = a_val\n").unwrap();
    fs::write(&b, "#include \"a.xcconfig\"\nB_KEY = b_val\n").unwrap();
    let ass = flatten_xcconfig(&a).unwrap();
    let a_count = ass.iter().filter(|x| x.key == "A_KEY").count();
    let b_count = ass.iter().filter(|x| x.key == "B_KEY").count();
    assert_eq!(a_count, 1, "A_KEY should appear exactly once");
    assert_eq!(b_count, 1, "B_KEY should appear exactly once");
}

#[test]
fn self_include_is_skipped() {
    let dir = scratch("cycle1");
    let a = dir.join("self.xcconfig");
    fs::write(&a, "#include \"self.xcconfig\"\nFOO = bar\n").unwrap();
    let ass = flatten_xcconfig(&a).unwrap();
    assert_eq!(ass.iter().filter(|x| x.key == "FOO").count(), 1);
}

// =====================================================================
// Fix 2: $(inherited) chains across assignments within ONE layer
// =====================================================================

#[test]
fn inherited_chains_within_single_layer() {
    // `#include "base.xcconfig"` + the includer flatten into a single layer,
    // so the includer's `$(inherited)` must see the include's value.
    let layer = vec![
        unconditional("OTHER_LDFLAGS", "-lz"),
        unconditional("OTHER_LDFLAGS", "$(inherited) -lssl"),
    ];
    let r = resolve(&[&layer], &ctx_macos());
    assert_eq!(
        r.get("OTHER_LDFLAGS").map(String::as_str),
        Some("-lz -lssl")
    );
}

#[test]
fn conditional_inherited_chains_within_single_layer() {
    let layer = vec![
        unconditional("FOO", "base"),
        conditional("FOO", &[("sdk", "macosx*")], "$(inherited) extra"),
    ];
    let r = resolve(&[&layer], &ctx_macos());
    assert_eq!(r.get("FOO").map(String::as_str), Some("base extra"));
}

#[test]
fn inherited_chains_within_layer_then_falls_through_to_lower_layer() {
    // Two in-layer $(inherited) appends compose, and the still-unsubstituted
    // leading $(inherited) picks up the lower layer at merge time.
    let lower = vec![unconditional("K", "a")];
    let upper = vec![
        unconditional("K", "$(inherited) b"),
        unconditional("K", "$(inherited) c"),
    ];
    let r = resolve(&[&lower, &upper], &ctx_macos());
    assert_eq!(r.get("K").map(String::as_str), Some("a b c"));
}

#[test]
fn plain_overwrite_within_layer_still_replaces() {
    let layer = vec![unconditional("K", "first"), unconditional("K", "second")];
    let r = resolve(&[&layer], &ctx_macos());
    assert_eq!(r.get("K").map(String::as_str), Some("second"));
}

// =====================================================================
// Fix 3: chained modifiers apply left to right
// =====================================================================

#[test]
fn chained_modifiers_apply_left_to_right() {
    let map = lookup(&[("PRODUCT_NAME", "My App")]);
    assert_eq!(
        expand_one("$(PRODUCT_NAME:lower:rfc1034identifier)", &map),
        "my-app"
    );
}

#[test]
fn default_modifier_composes_in_chain() {
    let map = lookup(&[]);
    // `default=` consumes only its own segment; the next operator applies to
    // the substituted value.
    assert_eq!(expand_one("$(UNSET:default=AbC:lower)", &map), "abc");
}

// =====================================================================
// Fix 4: :rfc1034identifier preserves '.'
// =====================================================================

#[test]
fn rfc1034identifier_preserves_dots() {
    let map = lookup(&[("NAME", "My.App Name")]);
    assert_eq!(expand_one("$(NAME:rfc1034identifier)", &map), "My.App-Name");
}

// =====================================================================
// Fix 5: /* */ block comments + comment-before-continuation ordering
// =====================================================================

#[test]
fn block_comment_whole_line_is_stripped() {
    let c = parse("/* a comment */\nFOO = bar\n").unwrap();
    assert_eq!(c.entries.len(), 1);
    let Entry::Assignment(a) = &c.entries[0] else {
        panic!("expected assignment");
    };
    assert_eq!(a.key, "FOO");
    assert_eq!(a.value, "bar");
}

#[test]
fn block_comment_inline_in_value_is_stripped() {
    let c = parse("FOO = bar /* note */\n").unwrap();
    let Entry::Assignment(a) = &c.entries[0] else {
        panic!("expected assignment");
    };
    assert_eq!(a.value, "bar");
}

#[test]
fn multiline_block_comment_preserves_line_numbers() {
    // The malformed line is line 4; the spanning comment must not shift it.
    let err = parse("/* line1\nline2 */\nGOOD = 1\nBAD BAD\n").unwrap_err();
    assert_eq!(err.line, 4, "line numbers must survive block comments");
}

#[test]
fn line_comment_with_trailing_backslash_does_not_continue() {
    // Comments are stripped BEFORE continuation joining: a backslash inside a
    // comment is comment text, not a line continuation.
    let c = parse("FOO = bar // see C:\\\nBAR = baz\n").unwrap();
    assert_eq!(c.entries.len(), 2);
    let Entry::Assignment(a) = &c.entries[0] else {
        panic!("expected assignment");
    };
    assert_eq!(a.value, "bar");
    let Entry::Assignment(b) = &c.entries[1] else {
        panic!("expected assignment");
    };
    assert_eq!(b.key, "BAR");
    assert_eq!(b.value, "baz");
}

#[test]
fn double_slash_in_url_starts_comment() {
    // Real Xcode treats `//` as a comment opener even mid-"URL".
    let c = parse("FOO = http://example.com\n").unwrap();
    let Entry::Assignment(a) = &c.entries[0] else {
        panic!("expected assignment");
    };
    assert_eq!(a.value, "http:");
}

#[test]
fn unterminated_block_comment_runs_to_eof() {
    let c = parse("FOO = bar\n/* unterminated\nBAZ = nope\n").unwrap();
    assert_eq!(c.entries.len(), 1);
}

// =====================================================================
// Fix 6: <DEVELOPER_DIR> substitution in #include paths
// =====================================================================

#[test]
fn include_developer_dir_placeholder_is_substituted() {
    let dir = scratch("devdir");
    let dev = dir.join("Developer");
    fs::create_dir_all(&dev).unwrap();
    fs::write(dev.join("shared.xcconfig"), "FROM_DEV_DIR = yes\n").unwrap();
    let main = dir.join("main.xcconfig");
    fs::write(&main, "#include \"<DEVELOPER_DIR>/shared.xcconfig\"\n").unwrap();
    // detect_developer_dir() reads DEVELOPER_DIR live on every call (no memo
    // on the env path), so an override here is honoured. This is the only
    // env-mutating test in this binary; nothing else here reads it.
    unsafe { std::env::set_var("DEVELOPER_DIR", &dev) };
    let result = flatten_xcconfig(&main);
    unsafe { std::env::remove_var("DEVELOPER_DIR") };
    let ass = result.unwrap();
    assert_eq!(ass.len(), 1);
    assert_eq!(ass[0].key, "FROM_DEV_DIR");
}

// =====================================================================
// Fix 7: #include? forgives ONLY the optional file itself being missing
// =====================================================================

#[test]
fn optional_include_missing_file_is_skipped() {
    let dir = scratch("opt-missing");
    let main = dir.join("main.xcconfig");
    fs::write(&main, "#include? \"missing.xcconfig\"\nFOO = bar\n").unwrap();
    let ass = flatten_xcconfig(&main).unwrap();
    assert_eq!(ass.len(), 1);
    assert_eq!(ass[0].key, "FOO");
}

#[test]
fn optional_include_present_but_malformed_still_errors() {
    let dir = scratch("opt-malformed");
    let main = dir.join("main.xcconfig");
    fs::write(dir.join("broken.xcconfig"), "THIS IS NOT VALID\n").unwrap();
    fs::write(&main, "#include? \"broken.xcconfig\"\nFOO = bar\n").unwrap();
    assert!(
        flatten_xcconfig(&main).is_err(),
        "a malformed optional include must still be a hard error"
    );
}

#[test]
fn optional_include_with_nested_missing_required_include_still_errors() {
    let dir = scratch("opt-nested");
    let main = dir.join("main.xcconfig");
    fs::write(dir.join("present.xcconfig"), "#include \"gone.xcconfig\"\n").unwrap();
    fs::write(&main, "#include? \"present.xcconfig\"\n").unwrap();
    assert!(
        flatten_xcconfig(&main).is_err(),
        "#include? only forgives the optional file itself being absent"
    );
}

// =====================================================================
// Fix 8: $$ escapes a literal dollar
// =====================================================================

#[test]
fn double_dollar_collapses_to_literal_dollar() {
    let map = lookup(&[("B", "unused")]);
    assert_eq!(expand_one("a$$b", &map), "a$b");
    // The escaped dollar does NOT start a reference even when followed by `(`.
    assert_eq!(expand_one("a$$(B)c", &map), "a$(B)c");
}

// =====================================================================
// Fix 9: $(inherited:modifier) applies the modifier to the inherited value
// =====================================================================

#[test]
fn inherited_with_modifier_keeps_and_transforms_value() {
    let lower = vec![unconditional("K", "AbC")];
    let upper = vec![unconditional("K", "$(inherited:lower)")];
    let r = resolve(&[&lower, &upper], &ctx_macos());
    assert_eq!(r.get("K").map(String::as_str), Some("abc"));
}

#[test]
fn self_reference_with_modifier_keeps_and_transforms_value() {
    let lower = vec![unconditional("K", "AbC")];
    let upper = vec![unconditional("K", "${K:upper}")];
    let r = resolve(&[&lower, &upper], &ctx_macos());
    assert_eq!(r.get("K").map(String::as_str), Some("ABC"));
}

// =====================================================================
// Fix 10: condition globs support `*` at any position
// =====================================================================

#[test]
fn condition_glob_star_in_any_position() {
    let layer = vec![
        unconditional("FOO", "base"),
        conditional("FOO", &[("arch", "*64")], "matched"),
    ];
    let r = resolve(&[&layer], &ctx_macos());
    assert_eq!(r.get("FOO").map(String::as_str), Some("matched"));
}

#[test]
fn condition_glob_multiple_stars() {
    let layer = vec![
        unconditional("FOO", "base"),
        conditional("FOO", &[("sdk", "*aco*")], "matched"),
        conditional("BAR", &[("sdk", "*xyz*")], "not_matched"),
    ];
    let r = resolve(&[&layer], &ctx_macos());
    assert_eq!(r.get("FOO").map(String::as_str), Some("matched"));
    assert_eq!(r.get("BAR"), None);
}

// =====================================================================
// Fix 11: [variant=normal] matches the default variant
// =====================================================================

#[test]
fn variant_normal_condition_matches_default_variant() {
    let mut ctx = ctx_macos();
    ctx.variant = "normal".into();
    let layer = vec![
        unconditional("FOO", "base"),
        conditional("FOO", &[("variant", "normal")], "normal_val"),
    ];
    let r = resolve(&[&layer], &ctx);
    assert_eq!(r.get("FOO").map(String::as_str), Some("normal_val"));
}

// =====================================================================
// Fix 12: path / identifier / quote modifiers
// =====================================================================

#[test]
fn modifier_dir_file_base_suffix() {
    let map = lookup(&[("P", "/tmp/a/b.txt"), ("BARE", "b.txt")]);
    assert_eq!(expand_one("$(P:dir)", &map), "/tmp/a");
    assert_eq!(expand_one("$(P:file)", &map), "b.txt");
    assert_eq!(expand_one("$(P:base)", &map), "b");
    assert_eq!(expand_one("$(P:suffix)", &map), ".txt");
    assert_eq!(expand_one("$(BARE:dir)", &map), ".");
    assert_eq!(expand_one("$(BARE:base)", &map), "b");
}

#[test]
fn modifier_suffix_empty_when_no_extension() {
    let map = lookup(&[("P", "/tmp/a/noext")]);
    assert_eq!(expand_one("$(P:suffix)", &map), "");
}

#[test]
fn modifier_standardizepath() {
    let map = lookup(&[("A", "/tmp/./a/../b//c"), ("B", "a/./b/../c"), ("C", "/..")]);
    assert_eq!(expand_one("$(A:standardizepath)", &map), "/tmp/b/c");
    assert_eq!(expand_one("$(B:standardizepath)", &map), "a/c");
    assert_eq!(expand_one("$(C:standardizepath)", &map), "/");
}

#[test]
fn modifier_identifier_prepends_underscore_on_leading_digit() {
    let map = lookup(&[("N", "1Password App")]);
    assert_eq!(expand_one("$(N:identifier)", &map), "_1Password_App");
    assert_eq!(expand_one("$(N:c99extidentifier)", &map), "_1Password_App");
}

#[test]
fn modifier_quote_only_when_needed() {
    let map = lookup(&[
        ("PLAIN", "simple-value_1.0"),
        ("SPACED", "two words"),
        ("QUOTED", "it's"),
    ]);
    assert_eq!(expand_one("$(PLAIN:quote)", &map), "simple-value_1.0");
    assert_eq!(expand_one("$(SPACED:quote)", &map), "'two words'");
    assert_eq!(expand_one("$(QUOTED:quote)", &map), "'it'\\''s'");
}

// =====================================================================
// Fix 13: condition boolean coercion follows NSString.boolValue
// =====================================================================

#[test]
fn condition_bool_value_follows_nsstring_boolvalue() {
    let truthy = [
        "YES", "yes", "Y", "y", "TRUE", "true", "t", "1", "9", "+1", "01", " -3",
    ];
    let falsy = ["NO", "no", "0", "", "no1", "foobar", "+", "-", "0x1"];
    for v in truthy {
        let s = lookup(&[("X", v)]);
        assert!(
            condition::evaluate(&condition::parse("$(X)").unwrap(), &s),
            "{v:?} should be truthy"
        );
    }
    for v in falsy {
        let s = lookup(&[("X", v)]);
        assert!(
            !condition::evaluate(&condition::parse("$(X)").unwrap(), &s),
            "{v:?} should be falsy"
        );
    }
}
