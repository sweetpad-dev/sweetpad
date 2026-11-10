//! Linux Wi-Fi and USB services through the pinned pymobiledevice3 Python API.
use std::process::{Child, Command, Stdio};

use serde_json::{Value, json};

use crate::cli::CliError;

const ADAPTER: &str = include_str!("device_network.py");

#[must_use]
pub fn configured() -> bool {
    std::env::var_os("SWEETPAD_DEVICE_NETWORK_CONFIG").is_some()
        || std::env::var_os("SWEETPAD_DEVICE_PYTHON").is_some()
}

#[must_use]
pub fn command(request: &Value) -> Command {
    let python = std::env::var_os("SWEETPAD_DEVICE_PYTHON").unwrap_or_else(|| "python3".into());
    let mut command = Command::new(python);
    command.args(["-u", "-c", ADAPTER, &request.to_string()]);
    command.stdin(Stdio::null());
    command
}

pub fn capture(request: &Value) -> Result<String, CliError> {
    let output = command(request).output().map_err(|e| {
        CliError::new(format!("starting Linux device transport: {e}; set SWEETPAD_DEVICE_PYTHON to Python with pymobiledevice3==11.50.0"))
    })?;
    if !output.status.success() {
        return Err(CliError::new(
            String::from_utf8_lossy(&output.stderr).trim().to_string(),
        ));
    }
    if !output.stderr.is_empty() {
        use std::io::Write;
        let _ = std::io::stderr().write_all(&output.stderr);
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

pub fn spawn(request: &Value) -> Result<Child, CliError> {
    command(request)
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .map_err(|e| CliError::new(format!("starting Linux device stream: {e}")))
}

#[must_use]
pub fn action(action: &str, device: &str) -> Value {
    json!({"action": action, "device": device})
}
