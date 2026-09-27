//! Thin wrapper over `xcrun devicectl` — listing and driving physical devices.
//! Shared by the `device` command and the `app … --device` path. `devicectl`
//! writes its listing to a `--json-output` file rather than stdout, so [`list`]
//! routes through a temp file. The JSON is read by
//! `sweetpad_core::devices::devicectl`, which the VS Code extension reads
//! through the addon too; `device info details` returns the same record for
//! one device, after connecting to it ([`details`]).

use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use sweetpad_core::devices::devicectl as parse;
pub use sweetpad_core::devices::devicectl::{Details, Device, find};

use crate::cli::{CliError, ErrorContext, process};

/// A fresh temp path for one `devicectl` `--json-output` / `--log-output`, so
/// concurrent runs never share a file.
fn temp_file(stem: &str, extension: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    std::env::temp_dir().join(format!(
        "sweetpad-{stem}-{}-{nanos}.{extension}",
        std::process::id()
    ))
}

/// Enumerate the physical devices paired with this Mac.
pub fn list() -> Result<Vec<Device>, CliError> {
    let tmp = temp_file("devices", "json");

    let ok = process::run(
        "xcrun",
        &[
            "devicectl",
            "list",
            "devices",
            "--json-output",
            &tmp.to_string_lossy(),
            "--timeout",
            "10",
        ],
        None,
        true,
    )?;
    if !ok {
        let _ = std::fs::remove_file(&tmp);
        return Err(CliError::new("'xcrun devicectl list devices' failed"));
    }

    let raw = std::fs::read_to_string(&tmp)
        .map_err(|e| CliError::new(format!("reading devicectl output: {e}")))?;
    let _ = std::fs::remove_file(&tmp);

    parse::parse_list(&raw).map_err(CliError::new)
}

/// How long past its own `--timeout` a devicectl call may run before it is
/// killed.
const KILL_GRACE: Duration = Duration::from_secs(5);

/// The shortest `--timeout` devicectl accepts.
pub const MIN_TIMEOUT_SECS: u64 = 5;

/// Connect to a device and read its details, waiting at most `timeout` for it
/// to answer. A device that cannot be reached still yields details (devicectl
/// returns the best it has); an error means devicectl itself failed or had to
/// be killed.
pub fn details(udid: &str, timeout: Duration) -> Result<Details, CliError> {
    let json = temp_file("details", "json");
    let log = temp_file("details", "log");
    let seconds = timeout.as_secs().max(MIN_TIMEOUT_SECS).to_string();
    let finished = process::run_quiet_within(
        "xcrun",
        &[
            "devicectl",
            "device",
            "info",
            "details",
            "--device",
            udid,
            "--json-output",
            &json.to_string_lossy(),
            "--log-output",
            &log.to_string_lossy(),
            "--timeout",
            &seconds,
        ],
        timeout + KILL_GRACE,
    )?;
    let raw = std::fs::read_to_string(&json).unwrap_or_default();
    let log_text = std::fs::read_to_string(&log).unwrap_or_default();
    let _ = std::fs::remove_file(&json);
    let _ = std::fs::remove_file(&log);
    if finished.is_none() {
        return Err(CliError::new(format!(
            "devicectl did not finish within {seconds}s"
        )));
    }
    if raw.trim().is_empty() {
        return Err(CliError::new("devicectl exited without a result"));
    }
    parse::parse_details(&raw, &log_text).map_err(CliError::new)
}

/// Whether the device is locked right now (`device info lockState`'s
/// `passcodeRequired`), or `None` when it cannot say. A locked device cannot
/// have its developer services started or its apps launched.
#[must_use]
pub fn locked(udid: &str, timeout: Duration) -> Option<bool> {
    let json = temp_file("lock-state", "json");
    let seconds = timeout.as_secs().max(MIN_TIMEOUT_SECS).to_string();
    let finished = process::run_quiet_within(
        "xcrun",
        &[
            "devicectl",
            "device",
            "info",
            "lockState",
            "--device",
            udid,
            "--json-output",
            &json.to_string_lossy(),
            "--timeout",
            &seconds,
        ],
        timeout + KILL_GRACE,
    )
    .ok()
    .flatten();
    let raw = std::fs::read_to_string(&json).unwrap_or_default();
    let _ = std::fs::remove_file(&json);
    finished?;
    parse::parse_lock_state(&raw)
}

/// Install an `.app` bundle onto a device. Captures stdout (stderr stays
/// visible): `devicectl` prints install progress there, which is noise under
/// the caller's step line — and in `--json` mode it would interleave with the
/// envelope on the same stream and break the consumer's parse.
pub fn install(device_id: &str, app_path: &str) -> Result<(), CliError> {
    process::capture(
        "xcrun",
        &[
            "devicectl",
            "device",
            "install",
            "app",
            "--device",
            device_id,
            app_path,
        ],
        None,
    )
    .map(|_| ())
    .context("installing the app on the device")
}

/// Launch an installed app on a device, terminating any existing instance.
pub fn launch(
    device_id: &str,
    bundle_id: &str,
    args: &[String],
    env: &[(String, String)],
    wait_for_debugger: bool,
) -> Result<String, CliError> {
    let mut cmd_args: Vec<&str> = vec![
        "devicectl",
        "device",
        "process",
        "launch",
        "--terminate-existing",
    ];
    if wait_for_debugger {
        cmd_args.push("--start-stopped");
    }
    cmd_args.extend_from_slice(&["--device", device_id, bundle_id]);
    // Trailing arguments go to the app, exactly as `devicectl … <bundle-id>
    // [<command-line-arguments> ...]` documents.
    cmd_args.extend(args.iter().map(String::as_str));
    // devicectl forwards `DEVICECTL_CHILD_*` from its own environment to the
    // app, the same shape simctl uses for `SIMCTL_CHILD_*`.
    process::capture_env("xcrun", &cmd_args, None, env).context("launching the app on the device")
}

/// Launch with the console attached, streaming the app's stdout/stderr and
/// os_log output to the terminal until it exits (Xcode 16+). This is how device
/// log following works — `devicectl` has no attach-to-running-process console.
pub fn launch_console(
    device_id: &str,
    bundle_id: &str,
    args: &[String],
    env: &[(String, String)],
) -> Result<(), CliError> {
    let mut cmd_args = console_args(device_id, bundle_id);
    cmd_args.extend(args.iter().map(String::as_str));
    process::stream_env("xcrun", &cmd_args, None, env)
}

/// The shared `devicectl … launch --console` prefix; the app's own arguments
/// are appended by the caller.
fn console_args<'a>(device_id: &'a str, bundle_id: &'a str) -> Vec<&'a str> {
    vec![
        "devicectl",
        "device",
        "process",
        "launch",
        "--console",
        "--terminate-existing",
        "--device",
        device_id,
        bundle_id,
    ]
}

/// Like [`launch_console`] but spawned in the background with stdout/stderr piped,
/// handing back the child so the interactive `app run` session can render the device
/// console (the app's own output) while watching for the rebuild key.
pub fn spawn_console(
    device_id: &str,
    bundle_id: &str,
    args: &[String],
    env: &[(String, String)],
) -> Result<std::process::Child, CliError> {
    let mut cmd_args = console_args(device_id, bundle_id);
    cmd_args.extend(args.iter().map(String::as_str));
    process::spawn_piped_both_env("xcrun", &cmd_args, None, env)
}

/// Uninstall an app from a device. Stdout is captured for the same reason as
/// [`install`] — keep `devicectl`'s progress chatter off the stream the
/// `--json` envelope goes to.
pub fn uninstall(device_id: &str, bundle_id: &str) -> Result<(), CliError> {
    process::capture(
        "xcrun",
        &[
            "devicectl",
            "device",
            "uninstall",
            "app",
            "--device",
            device_id,
            bundle_id,
        ],
        None,
    )
    .map(|_| ())
    .context("uninstalling the app from the device")
}

/// Terminate a running app on a device. `devicectl` has no
/// terminate-by-bundle-id — processes are addressed by pid — so the app's
/// pid(s) are looked up in the device's process list by the `.app` directory
/// name in the executable path, then signalled. Nothing running is success,
/// mirroring `simctl terminate`'s idempotence.
pub fn terminate(device_id: &str, app_dir_name: &str) -> Result<(), CliError> {
    if app_dir_name.is_empty() {
        return Err(CliError::new(
            "cannot terminate: the app bundle's name is unknown",
        ));
    }
    for pid in app_pids(device_id, app_dir_name)? {
        // Best-effort per pid: the process may have exited between the list
        // and the signal, which devicectl reports as a failure.
        let _ = process::run(
            "xcrun",
            &[
                "devicectl",
                "device",
                "process",
                "signal",
                "--signal",
                "15",
                "--pid",
                &pid.to_string(),
                "--device",
                device_id,
            ],
            None,
            true,
        );
    }
    Ok(())
}

/// Pids of running processes whose executable lives inside the named `.app`
/// directory. `devicectl device info processes` routes through a
/// `--json-output` temp file like [`list`].
fn app_pids(device_id: &str, app_dir_name: &str) -> Result<Vec<i64>, CliError> {
    let tmp = temp_file("processes", "json");
    let ok = process::run(
        "xcrun",
        &[
            "devicectl",
            "device",
            "info",
            "processes",
            "--device",
            device_id,
            "--json-output",
            &tmp.to_string_lossy(),
        ],
        None,
        true,
    )?;
    if !ok {
        let _ = std::fs::remove_file(&tmp);
        return Err(CliError::new(
            "'xcrun devicectl device info processes' failed",
        ));
    }
    let raw = std::fs::read_to_string(&tmp)
        .map_err(|e| CliError::new(format!("reading devicectl output: {e}")))?;
    let _ = std::fs::remove_file(&tmp);
    let processes = parse::parse_app_processes(&raw, app_dir_name).map_err(CliError::new)?;
    Ok(processes.into_iter().map(|p| p.pid).collect())
}
