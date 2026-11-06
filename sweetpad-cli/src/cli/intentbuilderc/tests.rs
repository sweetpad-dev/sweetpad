use super::*;
use crate::cli::testdir::TempDir;
fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/intentbuilderc")
}
fn declarations(source: &str) -> String {
    let source = source
        .lines()
        .filter(|line| !line.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n");
    let mut plain = String::new();
    let mut rest = source.as_str();
    while let Some(start) = rest.find("/*") {
        plain.push_str(&rest[..start]);
        rest = &rest[start + 2..];
        rest = &rest[rest.find("*/").expect("closed comment") + 2..];
    }
    plain.push_str(rest);
    plain.chars().filter(|c| !c.is_whitespace()).collect()
}
#[test]
fn generated_intent_declarations_match_xcode() {
    let root =
        sweetpad_lib::xcscheme::parse_file(&fixtures().join("Intents.intentdefinition")).unwrap();
    let sources = swift::generate(root.child("dict").unwrap()).unwrap();
    assert_eq!(sources.len(), 1);
    for (name, source) in sources {
        let expected = std::fs::read_to_string(fixtures().join("expected").join(name)).unwrap();
        assert_eq!(declarations(&source), declarations(&expected));
    }
}
#[test]
fn swift_builds_empty_prefix_and_dry_run_are_accepted() {
    let output = TempDir::new("sweetpad-intent").join("Generated");
    let values = [
        "generate",
        "-dryRun",
        "-input",
        "Model.intentdefinition",
        "-output",
        output.to_str().unwrap(),
        "-classPrefix",
        "",
        "-language",
        "Swift",
        "-swiftVersion",
        "6.2",
        "-visibility",
        "public",
    ];
    let args = parse(&values.map(str::to_string)).unwrap();
    assert!(args.generate && args.dry_run);
    assert!(args.prefix.is_empty());
    assert_eq!(args.output, output);
}
#[test]
fn compilation_preserves_the_runtime_resource() {
    let output = TempDir::new("sweetpad-intent");
    let args = Args {
        generate: false,
        dry_run: false,
        input: fixtures().join("Intents.intentdefinition"),
        output: output.to_path_buf(),
        prefix: String::new(),
        visibility: "public".into(),
    };
    run(&args).unwrap();
    assert_eq!(
        std::fs::read(&args.input).unwrap(),
        std::fs::read(output.join("Intents.intentdefinition")).unwrap()
    );
}
