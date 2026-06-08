//! `sweetpad-lib config` writes the `buildServer.json` that sourcekit-lsp reads
//! to discover and launch our server. Its decoder **silently skips** a config
//! missing any required field (`name` / `version` / `bspVersion` / `languages` /
//! `argv`) — a dropped field disables editor intelligence with no error — so this
//! hermetic oracle pins the full shape, including an `argv` that re-launches the
//! server (`bsp --project <abs>`) at the same project. No Xcode / build needed.

use std::process::{Command, Stdio};

use serde_json::Value;

#[test]
fn config_writes_complete_build_server_json() {
    let root = env!("CARGO_MANIFEST_DIR");
    let project = format!("{root}/fixtures/_synthetic-multimodule/project/MultiModule.xcodeproj");
    // Write outside the fixture tree so it can't pollute the project dir.
    let out = std::env::temp_dir().join(format!("sweetpad-bsp-config-{}.json", std::process::id()));
    let _ = std::fs::remove_file(&out);

    let status = Command::new(env!("CARGO_BIN_EXE_sweetpad-lib"))
        .args(["config", "--project", &project, "--output"])
        .arg(&out)
        .stderr(Stdio::null())
        .status()
        .expect("run config subcommand");
    assert!(status.success(), "config subcommand exited non-zero");

    let raw = std::fs::read_to_string(&out).expect("buildServer.json was written");
    let _ = std::fs::remove_file(&out);
    let cfg: Value = serde_json::from_str(&raw).expect("buildServer.json is valid JSON");

    // The five fields sourcekit-lsp's decoder requires.
    assert!(cfg.get("name").and_then(Value::as_str).is_some(), "missing `name`: {cfg}");
    assert!(cfg.get("version").and_then(Value::as_str).is_some(), "missing `version`: {cfg}");
    assert_eq!(cfg.get("bspVersion").and_then(Value::as_str), Some("2.2.0"), "wrong/absent `bspVersion`: {cfg}");

    let langs: Vec<&str> = cfg
        .get("languages")
        .and_then(Value::as_array)
        .expect("`languages` array")
        .iter()
        .filter_map(Value::as_str)
        .collect();
    for lang in ["swift", "objective-c", "objective-cpp", "c", "cpp"] {
        assert!(langs.contains(&lang), "`languages` missing {lang}: {langs:?}");
    }

    // `argv` must re-launch this server pointed at the (canonicalized) project,
    // so the editor can spawn it — server exe first, then the `bsp` subcommand.
    let argv: Vec<&str> = cfg
        .get("argv")
        .and_then(Value::as_array)
        .expect("`argv` array")
        .iter()
        .filter_map(Value::as_str)
        .collect();
    assert!(
        argv.first().is_some_and(|a| a.ends_with("sweetpad-lib")),
        "argv[0] should be the server executable: {argv:?}"
    );
    assert!(argv.contains(&"bsp"), "argv missing the `bsp` subcommand: {argv:?}");
    assert!(
        argv.windows(2).any(|w| w[0] == "--project" && w[1].ends_with("/MultiModule.xcodeproj")),
        "argv missing `--project <path>`: {argv:?}"
    );
}
