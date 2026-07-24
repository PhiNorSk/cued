use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use sha2::{Digest, Sha256};

use crate::error::AuthError;

fn random_base64url(num_bytes: usize) -> Result<String, AuthError> {
    let mut buf = vec![0u8; num_bytes];
    getrandom::fill(&mut buf)
        .map_err(|_| AuthError::Config("OS random number generator unavailable".into()))?;
    Ok(URL_SAFE_NO_PAD.encode(buf))
}

/// Generate a PKCE `code_verifier`: 43 base64url chars from 32 CSPRNG bytes.
pub fn generate_verifier() -> Result<String, AuthError> {
    random_base64url(32)
}

/// PKCE S256: BASE64URL-ENCODE(SHA256(ASCII(code_verifier))), no padding (RFC 7636 §4.2).
pub fn challenge_s256(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

/// Generate the OAuth `state` value: 22 base64url chars from 16 CSPRNG bytes.
pub fn generate_state() -> Result<String, AuthError> {
    random_base64url(16)
}

/// Abort the flow unless the callback returned exactly the state we sent.
pub fn validate_state(expected: &str, received: &str) -> Result<(), AuthError> {
    if !expected.is_empty() && expected == received {
        Ok(())
    } else {
        Err(AuthError::StateMismatch)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn is_base64url(s: &str) -> bool {
        s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    }

    #[test]
    fn challenge_matches_rfc7636_test_vector() {
        // Appendix B of RFC 7636
        let verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
        assert_eq!(
            challenge_s256(verifier),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
    }

    #[test]
    fn verifier_has_valid_length_and_charset() {
        let v = generate_verifier().expect("verifier generation must not fail");
        // RFC 7636 §4.1: 43–128 chars of [A-Za-z0-9-._~]; base64url is a subset
        assert!((43..=128).contains(&v.len()), "len was {}", v.len());
        assert!(is_base64url(&v));
    }

    #[test]
    fn verifier_is_random() {
        let a = generate_verifier().expect("verifier generation must not fail");
        let b = generate_verifier().expect("verifier generation must not fail");
        assert_ne!(a, b);
    }

    #[test]
    fn state_has_valid_length_and_charset_and_is_random() {
        let a = generate_state().expect("state generation must not fail");
        let b = generate_state().expect("state generation must not fail");
        assert!(a.len() >= 22, "len was {}", a.len());
        assert!(is_base64url(&a));
        assert_ne!(a, b);
    }

    #[test]
    fn state_validation_accepts_match() {
        assert!(validate_state("abc123", "abc123").is_ok());
    }

    #[test]
    fn state_validation_rejects_mismatch() {
        let err = validate_state("abc123", "abc124").expect_err("mismatch must be rejected");
        assert!(matches!(err, AuthError::StateMismatch));
    }

    #[test]
    fn state_validation_rejects_empty_received() {
        let err = validate_state("abc123", "").expect_err("empty state must be rejected");
        assert!(matches!(err, AuthError::StateMismatch));
    }
}
