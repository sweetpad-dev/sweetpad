//! App Store Connect API tokens: ES256-signed JSON Web Tokens.

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use p256::ecdsa::Signature;
use p256::ecdsa::signature::Signer as _;

use super::key::ApiKey;

/// How long a token lives. Apple refuses one that expires more than 20
/// minutes after it was issued.
pub(super) const LIFETIME_SECS: u64 = 15 * 60;

/// A token issued at `now` (Unix seconds).
pub(super) fn token(key: &ApiKey, now: u64) -> String {
    let header = serde_json::json!({"alg": "ES256", "kid": key.id, "typ": "JWT"});
    let mut claims = serde_json::json!({
        "iat": now,
        "exp": now + LIFETIME_SECS,
        "aud": "appstoreconnect-v1",
    });
    match &key.issuer {
        Some(issuer) => claims["iss"] = issuer.clone().into(),
        None => claims["sub"] = "user".into(),
    }
    let input = format!(
        "{}.{}",
        URL_SAFE_NO_PAD.encode(header.to_string()),
        URL_SAFE_NO_PAD.encode(claims.to_string())
    );
    let signature: Signature = key.signing.sign(input.as_bytes());
    format!("{input}.{}", URL_SAFE_NO_PAD.encode(signature.to_bytes()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use p256::ecdsa::signature::Verifier as _;
    use p256::ecdsa::{SigningKey, VerifyingKey};

    fn key(issuer: Option<&str>) -> ApiKey {
        ApiKey {
            id: "ABC123DEFG".into(),
            issuer: issuer.map(str::to_string),
            signing: SigningKey::from_slice(&[7u8; 32]).unwrap(),
        }
    }

    fn decode(part: &str) -> serde_json::Value {
        serde_json::from_slice(&URL_SAFE_NO_PAD.decode(part).unwrap()).unwrap()
    }

    #[test]
    fn a_team_token_names_the_issuer_and_verifies() {
        let key = key(Some("69a6de7e-0000-47e3-e053-5b8c7c11a4d1"));
        let token = token(&key, 1_800_000_000);
        let parts: Vec<&str> = token.split('.').collect();
        assert_eq!(parts.len(), 3);
        assert_eq!(
            decode(parts[0]),
            serde_json::json!({"alg": "ES256", "kid": "ABC123DEFG", "typ": "JWT"})
        );
        let claims = decode(parts[1]);
        assert_eq!(claims["iss"], "69a6de7e-0000-47e3-e053-5b8c7c11a4d1");
        assert_eq!(claims["aud"], "appstoreconnect-v1");
        assert_eq!(
            claims["exp"].as_u64().unwrap() - claims["iat"].as_u64().unwrap(),
            LIFETIME_SECS
        );
        assert!(claims.get("sub").is_none());
        let signature = Signature::from_slice(&URL_SAFE_NO_PAD.decode(parts[2]).unwrap()).unwrap();
        VerifyingKey::from(&key.signing)
            .verify(format!("{}.{}", parts[0], parts[1]).as_bytes(), &signature)
            .unwrap();
    }

    #[test]
    fn an_individual_token_is_for_the_user() {
        let token = token(&key(None), 1_800_000_000);
        let claims = decode(token.split('.').nth(1).unwrap());
        assert_eq!(claims["sub"], "user");
        assert!(claims.get("iss").is_none());
    }
}
