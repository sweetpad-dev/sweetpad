//! The App Store Connect API key a command signs its requests with.

use std::path::{Path, PathBuf};

use p256::ecdsa::SigningKey;
use p256::pkcs8::DecodePrivateKey;

use crate::cli::{CliError, ErrorKind};

/// The key flags shared by the commands that call App Store Connect.
#[derive(Debug, Clone, Default, clap::Args)]
pub struct KeyArgs {
    /// App Store Connect API key ID: the KEYID in 'AuthKey_KEYID.p8'.
    #[arg(long = "api-key", value_name = "KEY_ID", env = "SWEETPAD_ASC_KEY_ID")]
    pub key_id: Option<String>,
    /// Issuer ID of a team API key. Leave it out for an individual key.
    #[arg(
        long = "api-issuer",
        value_name = "ISSUER_ID",
        env = "SWEETPAD_ASC_ISSUER_ID"
    )]
    pub issuer: Option<String>,
    /// The key's .p8 file. Without it, sweetpad reads the key's text from
    /// SWEETPAD_ASC_KEY, or looks for 'AuthKey_KEY_ID.p8' where altool does:
    /// ./private_keys, ~/private_keys, ~/.private_keys,
    /// ~/.appstoreconnect/private_keys and $API_PRIVATE_KEYS_DIR.
    #[arg(
        long = "api-key-path",
        value_name = "FILE",
        env = "SWEETPAD_ASC_KEY_PATH"
    )]
    pub key_path: Option<PathBuf>,
}

/// A loaded key.
pub(crate) struct ApiKey {
    pub(crate) id: String,
    /// `None` for an individual key.
    pub(crate) issuer: Option<String>,
    pub(crate) signing: SigningKey,
}

fn non_empty(value: Option<&String>) -> Option<String> {
    value
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

impl ApiKey {
    /// Load the key the flags and environment name. Every failure is a usage
    /// error, decided before any request.
    pub(crate) fn resolve(args: &KeyArgs) -> Result<Self, CliError> {
        let usage = |message: String| CliError::new(message).kind(ErrorKind::Usage);
        let id = non_empty(args.key_id.as_ref()).ok_or_else(|| {
            usage(
                "no App Store Connect API key: pass '--api-key KEY_ID' (or set \
                 SWEETPAD_ASC_KEY_ID), with '--api-issuer' for a team key"
                    .into(),
            )
        })?;
        let issuer = non_empty(args.issuer.as_ref());
        let (pem, origin) = if let Some(path) =
            args.key_path.as_ref().filter(|p| !p.as_os_str().is_empty())
        {
            let text = std::fs::read_to_string(path)
                .map_err(|e| usage(format!("can't read the API key {}: {e}", path.display())))?;
            (text, path.display().to_string())
        } else if let Some(text) = non_empty(std::env::var("SWEETPAD_ASC_KEY").ok().as_ref()) {
            (text, "SWEETPAD_ASC_KEY".to_string())
        } else {
            let folders = search_folders();
            let name = format!("AuthKey_{id}.p8");
            let found = folders.iter().map(|f| f.join(&name)).find(|p| p.is_file());
            let Some(path) = found else {
                let list: Vec<String> = folders.iter().map(|f| f.display().to_string()).collect();
                return Err(usage(format!(
                    "can't find {name}: pass '--api-key-path FILE' or put it in one of {}",
                    list.join(", ")
                )));
            };
            let text = std::fs::read_to_string(&path)
                .map_err(|e| usage(format!("can't read the API key {}: {e}", path.display())))?;
            (text, path.display().to_string())
        };
        let signing = SigningKey::from_pkcs8_pem(pem.trim()).map_err(|_| {
            usage(format!(
                "{origin} isn't an App Store Connect API key (a PEM-encoded P-256 private key)"
            ))
        })?;
        Ok(Self {
            id,
            issuer,
            signing,
        })
    }
}

/// Where altool looks for `AuthKey_<id>.p8`, in its order.
fn search_folders() -> Vec<PathBuf> {
    let mut folders = vec![PathBuf::from("./private_keys")];
    if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
        for sub in [
            "private_keys",
            ".private_keys",
            ".appstoreconnect/private_keys",
        ] {
            folders.push(home.join(sub));
        }
    }
    if let Some(dir) = std::env::var_os("API_PRIVATE_KEYS_DIR") {
        folders.push(Path::new(&dir).to_path_buf());
    }
    folders
}
