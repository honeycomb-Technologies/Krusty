//! PKCE (Proof Key for Code Exchange) implementation
//!
//! Implements RFC 7636 for OAuth 2.0 PKCE, which provides protection
//! against authorization code interception attacks.

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use rand::RngCore;
use sha2::{Digest, Sha256};

/// PKCE code verifier - a cryptographically random string
///
/// The verifier is generated as 64 random bytes, base64url encoded.
/// This exceeds the minimum 43-character requirement from RFC 7636.
#[derive(Debug, Clone)]
pub struct PkceVerifier(String);

/// PKCE code challenge - SHA256 hash of the verifier
///
/// The challenge is computed as `base64url(sha256(verifier))` per RFC 7636.
#[derive(Debug, Clone)]
pub struct PkceChallenge(String);

impl PkceVerifier {
    /// Generate a new random PKCE verifier
    ///
    /// Creates a 64-byte random value, base64url encoded without padding.
    /// The resulting string is ~86 characters, well within the 43-128 range.
    pub fn new() -> Self {
        let mut bytes = [0u8; 64];
        rand::thread_rng().fill_bytes(&mut bytes);
        let encoded = URL_SAFE_NO_PAD.encode(bytes);
        Self(encoded)
    }

    /// Generate the PKCE challenge from this verifier
    ///
    /// Uses the S256 (SHA-256) challenge method as recommended by RFC 7636.
    pub fn challenge(&self) -> PkceChallenge {
        let hash = Sha256::digest(self.0.as_bytes());
        let encoded = URL_SAFE_NO_PAD.encode(hash);
        PkceChallenge(encoded)
    }

    /// Reconstruct a verifier from a previously stored string
    pub fn from_string(verifier: String) -> Self {
        Self(verifier)
    }

    /// Get the verifier string for use in token exchange
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Default for PkceVerifier {
    fn default() -> Self {
        Self::new()
    }
}

impl PkceChallenge {
    /// Get the challenge string for use in authorization request
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Get the challenge method (always "S256")
    pub fn method(&self) -> &'static str {
        "S256"
    }
}

impl std::fmt::Display for PkceVerifier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::fmt::Display for PkceChallenge {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_verifier_length() {
        let verifier = PkceVerifier::new();
        let len = verifier.as_str().len();
        // 64 bytes base64 encoded = ~86 characters
        assert!(len >= 43, "Verifier must be at least 43 characters");
        assert!(len <= 128, "Verifier must be at most 128 characters");
    }

    #[test]
    fn test_verifier_is_random() {
        let v1 = PkceVerifier::new();
        let v2 = PkceVerifier::new();
        assert_ne!(
            v1.as_str(),
            v2.as_str(),
            "Two verifiers should be different"
        );
    }

    #[test]
    fn test_challenge_is_deterministic() {
        let verifier = PkceVerifier::new();
        let c1 = verifier.challenge();
        let c2 = verifier.challenge();
        assert_eq!(
            c1.as_str(),
            c2.as_str(),
            "Same verifier should produce same challenge"
        );
    }

    #[test]
    fn test_challenge_length() {
        let verifier = PkceVerifier::new();
        let challenge = verifier.challenge();
        // SHA256 produces 32 bytes, base64 encoded = 43 characters
        assert_eq!(challenge.as_str().len(), 43);
    }

    #[test]
    fn test_challenge_method() {
        let verifier = PkceVerifier::new();
        let challenge = verifier.challenge();
        assert_eq!(challenge.method(), "S256");
    }

    #[test]
    fn test_url_safe_characters() {
        let verifier = PkceVerifier::new();
        let challenge = verifier.challenge();

        // Check both contain only URL-safe base64 characters
        for s in [verifier.as_str(), challenge.as_str()] {
            for c in s.chars() {
                assert!(
                    c.is_ascii_alphanumeric() || c == '-' || c == '_',
                    "Character '{}' is not URL-safe base64",
                    c
                );
            }
        }
    }

    #[test]
    fn test_known_vector() {
        // Test with a known verifier to ensure our implementation is correct
        // This test verifies the S256 method produces expected output
        let test_verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
        let expected_challenge = "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM";

        // Manually create a verifier with known value
        let hash = Sha256::digest(test_verifier.as_bytes());
        let actual_challenge = URL_SAFE_NO_PAD.encode(hash);

        assert_eq!(actual_challenge, expected_challenge);
    }
}
