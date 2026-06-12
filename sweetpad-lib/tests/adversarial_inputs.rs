//! Pathological-input tests: every parser/serializer must reject (or bound)
//! adversarial input with a clean error instead of panicking, overflowing the
//! stack, or expanding without limit. Each case here reproduces a concrete
//! failure mode that existed before the corresponding guard was added.

use std::time::Instant;

use sweetpad::{bplist, pbxproj, pbxproj_writer, xcscheme};

// ---------------------------------------------------------------------------
// pbxproj: unbounded recursion in parse_value/parse_array/parse_dict
// ---------------------------------------------------------------------------

#[test]
fn pbxproj_rejects_deeply_nested_arrays() {
    let input = "(".repeat(200_000);
    let err = pbxproj::parse(&input).expect_err("deep nesting should be rejected");
    assert!(
        err.message.contains("depth"),
        "expected a depth error, got: {err}"
    );
}

#[test]
fn pbxproj_rejects_deeply_nested_dicts() {
    let input = "{a = ".repeat(200_000);
    assert!(
        pbxproj::parse(&input).is_err(),
        "deep dict nesting should be rejected"
    );
}

#[test]
fn pbxproj_accepts_realistic_nesting() {
    // Well within the bound: must keep parsing.
    let depth = 64;
    let input = format!("{}x{}", "(".repeat(depth), ")".repeat(depth));
    pbxproj::parse(&input).expect("64 levels of nesting is legitimate");
}

// ---------------------------------------------------------------------------
// xcscheme: unbounded recursion in parse_element / write_element
// ---------------------------------------------------------------------------

#[test]
fn xcscheme_rejects_deeply_nested_elements() {
    let input = "<a>".repeat(200_000);
    let err = xcscheme::parse(&input).expect_err("deep XML nesting should be rejected");
    assert!(
        err.message.contains("depth"),
        "expected a depth error, got: {err}"
    );
}

#[test]
fn xcscheme_serializes_programmatic_deep_tree_without_overflow() {
    // The parser can never produce a tree this deep; a programmatically built
    // one must truncate instead of overflowing the serializer's stack.
    let mut e = xcscheme::Element {
        name: "leaf".into(),
        ..Default::default()
    };
    for _ in 0..10_000 {
        let mut parent = xcscheme::Element {
            name: "n".into(),
            ..Default::default()
        };
        parent.children.push(e);
        e = parent;
    }
    let out = xcscheme::serialize(&e);
    assert!(out.starts_with("<?xml"), "serializer should still emit XML");
}

// ---------------------------------------------------------------------------
// bplist: shared-reference expansion, extended-size truncation
// ---------------------------------------------------------------------------

/// A tiny `bplist00` file where object 0 is a scalar and each object *i* is a
/// two-element array referencing object *i−1* twice. Fully materialized, the
/// top object expands to 2^levels values; the parser must give up on a budget
/// instead. `levels` must keep every offset below 256 (1-byte offset table).
fn shared_ref_blowup_bplist(levels: usize) -> Vec<u8> {
    let mut data = b"bplist00".to_vec();
    let mut offsets = Vec::new();
    // Object 0: boolean false.
    offsets.push(data.len());
    data.push(0x08);
    // Objects 1..=levels: array of two refs to the previous object.
    for i in 1..=levels {
        offsets.push(data.len());
        data.push(0xA2);
        data.push(u8::try_from(i - 1).unwrap());
        data.push(u8::try_from(i - 1).unwrap());
    }
    let offset_table_offset = data.len();
    for &off in &offsets {
        data.push(u8::try_from(off).expect("offsets must fit in one byte"));
    }
    let mut trailer = [0u8; 32];
    trailer[6] = 1; // offset table entry size
    trailer[7] = 1; // object ref size
    trailer[8..16].copy_from_slice(&(offsets.len() as u64).to_be_bytes());
    trailer[16..24].copy_from_slice(&(offsets.len() as u64 - 1).to_be_bytes());
    trailer[24..32].copy_from_slice(&(offset_table_offset as u64).to_be_bytes());
    data.extend_from_slice(&trailer);
    data
}

#[test]
fn bplist_rejects_shared_ref_blowup_quickly() {
    // 2^80 values if fully expanded — must fail fast on the work budget.
    let data = shared_ref_blowup_bplist(80);
    assert!(data.len() < 600, "attack file should stay small");
    let start = Instant::now();
    let err = bplist::parse(&data).expect_err("blowup file should be rejected");
    assert!(
        start.elapsed().as_secs() < 5,
        "rejection must be fast, took {:?}",
        start.elapsed()
    );
    assert!(
        err.to_string().contains("budget"),
        "expected a budget error, got: {err}"
    );
}

#[test]
fn bplist_accepts_legitimate_shared_refs() {
    // Apple's writer dedups objects, so several containers referencing the
    // same object is normal — only the *total work* is bounded. Three levels
    // expand to 2^3 = 8 scalars, far under any budget.
    let data = shared_ref_blowup_bplist(3);
    let v = bplist::parse(&data).expect("small shared-ref plist is valid");
    assert!(v.as_array().is_some());
}

#[test]
fn bplist_rejects_16_byte_extended_size() {
    // One ASCII string object whose extended-size int claims a 16-byte width
    // (marker 0x14): wider than u64, it used to truncate silently.
    let mut data = b"bplist00".to_vec();
    data.push(0x5F); // ASCII string, extended size follows
    data.push(0x14); // int marker, 1 << 4 = 16 bytes
    data.extend_from_slice(&[0u8; 16]);
    let offset_table_offset = data.len();
    data.push(8); // object 0 starts right after the magic
    let mut trailer = [0u8; 32];
    trailer[6] = 1;
    trailer[7] = 1;
    trailer[8..16].copy_from_slice(&1u64.to_be_bytes());
    trailer[24..32].copy_from_slice(&(offset_table_offset as u64).to_be_bytes());
    data.extend_from_slice(&trailer);

    let err = bplist::parse(&data).expect_err("16-byte size marker should be rejected");
    assert!(
        err.to_string().contains("extended size"),
        "expected an extended-size error, got: {err}"
    );
}

// ---------------------------------------------------------------------------
// pbxproj_writer: comment derivation must not chase reference cycles
// ---------------------------------------------------------------------------

#[test]
fn writer_survives_self_referential_build_file() {
    let src = "{ objects = { AAA = { isa = PBXBuildFile; fileRef = AAA; }; }; }";
    let v = pbxproj::parse(src).expect("syntactically valid pbxproj");
    let out = pbxproj_writer::serialize(&v, "Demo");
    // The cyclic reference resolves like any failed lookup.
    assert!(
        out.contains("(null)"),
        "expected the (null) fallback comment, got: {out}"
    );
}

#[test]
fn writer_survives_mutually_referential_build_files() {
    let src = "{ objects = { \
        AAA = { isa = PBXBuildFile; fileRef = BBB; }; \
        BBB = { isa = PBXBuildFile; fileRef = AAA; }; \
    }; }";
    let v = pbxproj::parse(src).expect("syntactically valid pbxproj");
    let out = pbxproj_writer::serialize(&v, "Demo");
    assert!(out.contains("(null)"));
}

// ---------------------------------------------------------------------------
// Round-trip sanity: the guards must not disturb normal documents
// ---------------------------------------------------------------------------

#[test]
fn normal_pbxproj_still_round_trips() {
    let src = "// !$*UTF8*$!\n{\n\tarchiveVersion = 1;\n\tclasses = {\n\t};\n\tobjectVersion = 77;\n\tobjects = {\n\n/* Begin PBXProject section */\n\t\tABC /* Project object */ = {\n\t\t\tisa = PBXProject;\n\t\t};\n/* End PBXProject section */\n\t};\n\trootObject = ABC /* Project object */;\n}\n";
    let v = pbxproj::parse(src).unwrap();
    assert_eq!(pbxproj_writer::serialize(&v, "Demo"), src);
}

#[test]
fn normal_xcscheme_still_round_trips() {
    let src = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
               <Scheme\n   version = \"1.7\">\n\
               \x20\x20\x20<BuildAction>\n\
               \x20\x20\x20</BuildAction>\n\
               </Scheme>\n";
    let e = xcscheme::parse(src).unwrap();
    assert_eq!(xcscheme::serialize(&e), src);
}
