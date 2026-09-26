//! Regression net for the log follow's SIGTERM path: the CLI forwards one
//! SIGINT to its `log stream` child and waits for it to end. A CLI started as
//! a script's background job inherits SIGINT ignored, and a child that
//! inherits the same ignore drops the forwarded signal, so the CLI waits on a
//! stream that never ends. Caught with a stub `xcrun` whose `simctl spawn`
//! installs no handler of its own, and a hard deadline.

use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

fn tmp(tag: &str) -> PathBuf {
    let n = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("sweetpad-sigint-{tag}-{n}"));
    std::fs::create_dir_all(&dir).unwrap();
    // Stop walk-up discovery at this directory.
    std::fs::create_dir_all(dir.join(".git")).unwrap();
    dir
}

fn kill(pid: i32, sig: libc::c_int) {
    // Safety: signalling a process this test started.
    unsafe {
        libc::kill(pid, sig);
    }
}

#[test]
fn sigterm_ends_a_log_follow_started_with_sigint_ignored() {
    use std::os::unix::fs::PermissionsExt;

    let home = tmp("home");
    let cwd = tmp("cwd");
    let bin = cwd.join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let marker = cwd.join("stream.pid");
    // `simctl spawn` records its pid and becomes a process that ends on SIGINT
    // only if it was started with SIGINT at its default disposition.
    let stub = bin.join("xcrun");
    std::fs::write(
        &stub,
        "#!/bin/sh\n\
         case \"$1 $2\" in\n\
         \"simctl boot\") exit 0 ;;\n\
         \"simctl spawn\") echo $$ > \"$SWEETPAD_TEST_MARKER\"; exec sleep 30 ;;\n\
         esac\n\
         exit 1\n",
    )
    .unwrap();
    std::fs::set_permissions(&stub, std::fs::Permissions::from_mode(0o755)).unwrap();
    let proj = cwd.join("Fixture.xcodeproj");
    std::fs::create_dir_all(&proj).unwrap();

    // A recorded simulator launch sends 'app logs' straight to the stream,
    // with no build settings to resolve.
    let key = std::fs::canonicalize(&proj).unwrap();
    let state_dir = home.join("sweetpad");
    std::fs::create_dir_all(&state_dir).unwrap();
    std::fs::write(
        state_dir.join("state.toml"),
        format!(
            "[projects.{key:?}.last_launched_app]\n\
             kind = \"simulator\"\n\
             app_path = \"/nonexistent/Stub.app\"\n\
             bundle_identifier = \"dev.sweetpad.stub\"\n\
             executable_name = \"Stub\"\n\
             simulator_udid = \"STUB-UDID\"\n",
            key = key.display().to_string()
        ),
    )
    .unwrap();

    let path_env = format!(
        "{}:{}",
        bin.display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_sweetpad"));
    cmd.args(["app", "logs", "--project", proj.to_str().unwrap()])
        .current_dir(&cwd)
        .env("HOME", &home)
        .env("XDG_STATE_HOME", &home)
        .env("XDG_CONFIG_HOME", &home)
        .env("PATH", &path_env)
        .env("SWEETPAD_TEST_MARKER", &marker)
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    // Safety: signal(2) in the forked child before exec, as a shell does for a
    // background job when job control is off.
    unsafe {
        cmd.pre_exec(|| {
            libc::signal(libc::SIGINT, libc::SIG_IGN);
            Ok(())
        });
    }
    let mut child = cmd.spawn().unwrap();
    let cli = i32::try_from(child.id()).unwrap();

    let deadline = Instant::now() + Duration::from_secs(20);
    let stream: i32 = loop {
        if let Some(pid) = std::fs::read_to_string(&marker)
            .ok()
            .and_then(|s| s.trim().parse().ok())
        {
            break pid;
        }
        if let Some(status) = child.try_wait().unwrap() {
            panic!("app logs exited with {status} before starting its stream");
        }
        assert!(
            Instant::now() < deadline,
            "app logs never started its stream"
        );
        std::thread::sleep(Duration::from_millis(50));
    };
    // Past the spawn, so the CLI is waiting on the stream in forward-only mode.
    std::thread::sleep(Duration::from_millis(300));
    kill(cli, libc::SIGTERM);

    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        match child.try_wait().unwrap() {
            Some(status) => {
                assert!(status.success(), "expected a clean stop, got {status}");
                break;
            }
            None if Instant::now() > deadline => {
                kill(stream, libc::SIGKILL);
                let _ = child.kill();
                panic!(
                    "app logs still running 10s after SIGTERM: the stream dropped the \
                     SIGINT it was forwarded"
                );
            }
            None => std::thread::sleep(Duration::from_millis(100)),
        }
    }
}
