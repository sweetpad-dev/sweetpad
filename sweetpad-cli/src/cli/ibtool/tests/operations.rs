use super::*;

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("fixtures/ibtool")
        .join(name)
}

#[test]
fn storyboard_identifiers_cannot_write_outside_the_package() {
    let directory = TempDir::new("sweetpad-ib-controller-name");
    let source = directory.join("Invalid.storyboard");
    let xml = std::fs::read_to_string(fixture("MacViewController.storyboard")).unwrap();
    std::fs::write(
        &source,
        xml.replace(
            "storyboardIdentifier=\"AuditController\"",
            "storyboardIdentifier=\"../Escaped\"",
        ),
    )
    .unwrap();
    let output = directory.join("products");
    let result = compile(
        &source,
        &Args {
            compilation_directory: Some(output.clone()),
            ..Args::default()
        },
    );
    assert!(
        result
            .unwrap_err()
            .contains("invalid storyboard filename component")
    );
    assert!(!output.exists());
    assert!(!directory.join("Escaped.nib").exists());
}

#[test]
fn window_content_loading_follows_storyboard_outlet_connections() {
    fn loading_order(data: &[u8]) -> Vec<&'static str> {
        let objects = read::parse(data);
        let reference = |object: &read::Object, key: &str| {
            object.values.iter().find_map(|(k, value)| match value {
                Read::Object(id) if k == key => Some(*id),
                _ => None,
            })
        };
        let metadata = objects
            .iter()
            .find(|object| object.class == "NSIBObjectData")
            .unwrap();
        let connections = reference(metadata, "NSConnections").unwrap();
        objects[connections].values.iter().filter_map(|(_, value)| {
            let Read::Object(id) = value else { return None };
            let object = &objects[*id];
            if object.class == "NSIBUserDefinedRuntimeAttributesConnector" {
                Some("runtime-attributes")
            } else if object.class == "NSNibOutletConnector" && reference(object, "NSLabel").is_some_and(|label| {
                objects[label].values.iter().any(|(_, value)| matches!(value, Read::Data(bytes) if bytes == b"storyboard"))
            }) {
                Some("storyboard")
            } else { None }
        }).collect()
    }
    let directory = TempDir::new("sweetpad-window-content-order");
    compile(
        &fixture("MacWindowRelationship.storyboard"),
        &Args {
            compilation_directory: Some(directory.to_path_buf()),
            module: "App".into(),
            ..Args::default()
        },
    )
    .unwrap();
    let expected = std::fs::read(fixture(
        "expected/MacWindowRelationship.storyboardc/WindowController.nib",
    ))
    .unwrap();
    let actual =
        std::fs::read(directory.join("MacWindowRelationship.storyboardc/WindowController.nib"))
            .unwrap();
    assert_eq!(
        loading_order(&expected),
        ["storyboard", "storyboard", "runtime-attributes"]
    );
    assert_eq!(loading_order(&actual), loading_order(&expected));
}

#[test]
fn companion_strings_follow_xcodes_compile_and_link_workflow() {
    let directory = TempDir::new("sweetpad-ib-localization");
    let output = directory.join("compiled/Base.lproj/Localization.storyboardc");
    let translation = fixture("fr.lproj/Localization.strings");
    let arguments = [
        "--module".to_string(),
        "App".to_string(),
        "--compile".to_string(),
        output.display().to_string(),
        "--companion-strings-file".to_string(),
        format!("fr:{}", translation.display()),
    ];
    let args = parse_args(&arguments).unwrap();
    assert!(args.reference_external_strings);
    compile(&fixture("Base.lproj/Localization.storyboard"), &args).unwrap();
    let mapping =
        keyed_archive::read::parse(&std::fs::read(output.join("StringsFiles.plist")).unwrap());
    assert_eq!(
        mapping,
        Plist::Dict(vec![(
            "fr".into(),
            Plist::String(translation.display().to_string())
        )])
    );
    let destination = directory.join("product");
    link(&output, &destination).unwrap();
    assert_eq!(
        std::fs::read(destination.join("fr.lproj/Localization.strings")).unwrap(),
        std::fs::read(translation).unwrap()
    );
    assert!(
        destination
            .join("Base.lproj/Localization.storyboardc/Info.plist")
            .is_file()
    );
    assert!(
        !destination
            .join("Base.lproj/Localization.storyboardc/StringsFiles.plist")
            .exists()
    );
    assert!(output.join("StringsFiles.plist").is_file());
}

#[test]
fn xib_companions_use_the_compiled_document_name() {
    let directory = TempDir::new("sweetpad-xib-localization");
    let output = directory.join("Base.lproj/Renamed.nib");
    let translation = fixture("fr.lproj/Localization.strings");
    compile(
        &fixture("Plain.xib"),
        &Args {
            compile: Some(output),
            companion_strings: vec![("fr".into(), translation.clone())],
            ..Args::default()
        },
    )
    .unwrap();
    assert_eq!(
        std::fs::read(directory.join("fr.lproj/Renamed.strings")).unwrap(),
        std::fs::read(translation).unwrap()
    );
    assert!(
        parse_args(&[
            "--companion-strings-file".into(),
            "../fr:table.strings".into()
        ])
        .is_err()
    );
}

#[test]
fn editable_xibs_round_trip_and_strip_without_runtime_changes() {
    let directory = TempDir::new("sweetpad-editable-nib");
    let output = directory.join("Editable.nib");
    compile(
        &fixture("Plain.xib"),
        &Args {
            compile: Some(output.clone()),
            flatten: Some(false),
            ..Args::default()
        },
    )
    .unwrap();
    assert!(output.join("designtime.nib").is_file());
    let document = sweetpad_lib::xcscheme::parse_file(&output.join("designtime.nib")).unwrap();
    assert_eq!(
        document.attr("type"),
        Some("com.apple.InterfaceBuilder3.CocoaTouch.XIB")
    );
    let stripped = directory.join("Stripped.nib");
    package::strip(&output, &stripped).unwrap();
    assert_eq!(
        std::fs::read(&stripped).unwrap(),
        std::fs::read(output.join("runtime.nib")).unwrap()
    );
    assert_eq!(
        canonical(&std::fs::read(&stripped).unwrap()),
        canonical(&std::fs::read(fixture("expected/Plain.nib")).unwrap())
    );
    let recompiled = directory.join("Recompiled.nib");
    compile(
        &output,
        &Args {
            compile: Some(recompiled.clone()),
            ..Args::default()
        },
    )
    .unwrap();
    assert_eq!(
        canonical(&std::fs::read(&recompiled).unwrap()),
        canonical(&std::fs::read(&stripped).unwrap())
    );
    package::strip(&stripped, &stripped).unwrap();
    assert_eq!(
        std::fs::read(&stripped).unwrap(),
        std::fs::read(recompiled).unwrap()
    );
}

#[test]
fn editable_storyboards_keep_source_and_linking_in_place_preserves_inputs() {
    let directory = TempDir::new("sweetpad-editable-storyboard");
    let output = directory.join("Target.storyboardc");
    compile(
        &fixture("Target.storyboard"),
        &Args {
            module: "App".into(),
            compile: Some(output.clone()),
            flatten: Some(false),
            ..Args::default()
        },
    )
    .unwrap();
    assert!(output.join("designable.storyboard").is_file());
    link(&output, &directory).unwrap();
    assert!(output.join("Info.plist").is_file());
    let stripped = directory.join("Stripped.storyboardc");
    package::strip(&output, &stripped).unwrap();
    assert!(!stripped.join("designable.storyboard").exists());
    assert_eq!(
        std::fs::read(output.join("Info.plist")).unwrap(),
        std::fs::read(stripped.join("Info.plist")).unwrap()
    );
}

#[test]
fn version_output_formats_match_xcode() {
    let data = serde_json::json!({"com.apple.ibtool.version": {"bundle-version": "25098", "short-bundle-version": "27.0"}});
    assert_eq!(
        output::encode(output::Format::Xml, &data),
        std::fs::read(fixture("expected/version.plist")).unwrap()
    );
    assert_eq!(
        sorted(keyed_archive::read::parse(&output::encode(
            output::Format::Binary,
            &data
        ))),
        sorted(keyed_archive::read::parse(
            &std::fs::read(fixture("expected/version.binary")).unwrap()
        ))
    );
    assert_eq!(output::encode(output::Format::Human, &data), b"/* com.apple.ibtool.version */\nibtool does not support human readable output for this option.\n");
}

#[test]
fn validation_checks_a_document_without_writing_a_product() {
    compile(&fixture("AdditionalViews.xib"), &Args::default()).unwrap();
    let directory = TempDir::new("sweetpad-ibtool-validation");
    let invalid = directory.join("Unknown.xib");
    let source = std::fs::read_to_string(fixture("Plain.xib"))
        .unwrap()
        .replace("<view ", "<unknownView ")
        .replace("</view>", "</unknownView>");
    std::fs::write(&invalid, source).unwrap();
    assert!(compile(&invalid, &Args::default()).is_err());
    assert!(!directory.join("Unknown.nib").exists());
}
