//! Experimental Linux Metal compiler. Embed the frontend so a distributed CLI
//! uses the same version as its Swift Build tool links.

use std::process::{Command, ExitCode};

const LOADER: &str = r#"
import sys, types
for name in ("msl", "library", "stages"):
    module = types.ModuleType(name)
    sys.modules[name] = module
    exec(compile(sys.argv.pop(1), name + ".py", "exec"), module.__dict__)
driver = sys.argv.pop(1)
exec(compile(driver, "sweetpad-metal.py", "exec"), {"__name__": "__main__"})
"#;

pub(crate) fn enabled() -> bool {
    std::env::var_os("SWEETPAD_NATIVE_METAL").is_some_and(|value| value == "1")
}

pub fn main(tool: &str, args: &[String]) -> ExitCode {
    let result = Command::new("python3")
        .args([
            "-I",
            "-B",
            "-c",
            LOADER,
            include_str!("msl.py"),
            include_str!("library.py"),
            include_str!("stages.py"),
            include_str!("driver.py"),
            tool,
        ])
        .args(args)
        .status();
    match result {
        Ok(status) => ExitCode::from(
            status
                .code()
                .and_then(|code| u8::try_from(code).ok())
                .unwrap_or(1),
        ),
        Err(error) => {
            eprintln!("sweetpad {tool}: starting python3: {error}");
            ExitCode::FAILURE
        }
    }
}
