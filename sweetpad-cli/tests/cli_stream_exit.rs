//! Regression net for the merged-pipe streaming path: a streaming command
//! must exit once the child tool does. The parent holding its own copies of
//! the pipe's write ends (inside the spawned `Command`) starves the reader of
//! EOF and hangs every human/ndjson build-family command forever — caught
//! here with a stub xcodebuild and a hard deadline.

use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

fn tmp(tag: &str) -> PathBuf {
    let n = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("sweetpad-stream-{tag}-{n}"));
    std::fs::create_dir_all(&dir).unwrap();
    // Stop walk-up discovery at this directory.
    std::fs::create_dir_all(dir.join(".git")).unwrap();
    dir
}

#[test]
fn streaming_command_exits_after_the_tool_does() {
    use std::os::unix::fs::PermissionsExt;

    let home = tmp("home");
    let cwd = tmp("cwd");
    let bin = cwd.join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let stub = bin.join("xcodebuild");
    std::fs::write(
        &stub,
        "#!/bin/sh\necho 'resolving packages'\necho 'done'\nexit 0\n",
    )
    .unwrap();
    std::fs::set_permissions(&stub, std::fs::Permissions::from_mode(0o755)).unwrap();
    let proj = cwd.join("Fixture.xcodeproj");
    std::fs::create_dir_all(&proj).unwrap();

    let path_env = format!(
        "{}:{}",
        bin.display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let modes: &[&[&str]] = &[&[], &["-o", "ndjson"]];
    for mode in modes {
        let mut args: Vec<&str> = vec!["dep", "resolve"];
        args.extend_from_slice(mode);
        let proj_str = proj.to_str().unwrap();
        args.extend_from_slice(&["--project", proj_str, "--non-interactive"]);
        let mut child = Command::new(env!("CARGO_BIN_EXE_sweetpad"))
            .args(&args)
            .current_dir(&cwd)
            .env("HOME", &home)
            .env("XDG_STATE_HOME", &home)
            .env("XDG_CONFIG_HOME", &home)
            .env("PATH", &path_env)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            match child.try_wait().unwrap() {
                Some(status) => {
                    assert!(status.success(), "{args:?}: expected success, got {status}");
                    break;
                }
                None if Instant::now() > deadline => {
                    let _ = child.kill();
                    panic!(
                        "{args:?}: still running 20s after the stub exited — \
                         the merged-pipe stream never saw EOF"
                    );
                }
                None => std::thread::sleep(Duration::from_millis(100)),
            }
        }
    }
}
