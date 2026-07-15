use base64::Engine;
use rand::RngCore;

pub fn generate_token() -> String {
    let mut bytes = [0_u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

pub fn validate_token(provided: &str, expected: &str) -> bool {
    if provided.len() != expected.len() {
        return false;
    }
    provided
        .as_bytes()
        .iter()
        .zip(expected.as_bytes())
        .fold(0_u8, |difference, (left, right)| {
            difference | (left ^ right)
        })
        == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_tokens_are_url_safe_and_unique() {
        let first = generate_token();
        let second = generate_token();
        assert_eq!(first.len(), 43);
        assert_ne!(first, second);
        assert!(first
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_')));
    }

    #[test]
    fn token_validation_rejects_different_values_and_lengths() {
        assert!(validate_token("secret", "secret"));
        assert!(!validate_token("secret", "secrex"));
        assert!(!validate_token("short", "longer"));
    }
}
