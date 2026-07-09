use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use rand::{rngs::OsRng, RngCore};
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PkcePair {
    pub code_verifier: String,
    pub code_challenge: String,
}

pub fn generate_pkce_pair() -> PkcePair {
    let mut bytes = [0u8; 32];
    OsRng.fill_bytes(&mut bytes);
    let code_verifier = URL_SAFE_NO_PAD.encode(bytes);
    challenge_for_verifier(&code_verifier)
}

pub fn challenge_for_verifier(code_verifier: &str) -> PkcePair {
    let digest = Sha256::digest(code_verifier.as_bytes());
    PkcePair {
        code_verifier: code_verifier.to_string(),
        code_challenge: URL_SAFE_NO_PAD.encode(digest),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_pkce_pair_uses_s256_challenge() {
        let pair = generate_pkce_pair();
        let expected = challenge_for_verifier(&pair.code_verifier);

        assert_eq!(pair, expected);
        assert_ne!(pair.code_verifier, pair.code_challenge);
        assert!(pair.code_verifier.len() >= 43);
    }
}
