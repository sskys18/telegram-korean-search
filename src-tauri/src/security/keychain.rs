use rand::RngCore;
use security_framework::passwords::{
    delete_generic_password, get_generic_password, set_generic_password,
};

const SERVICE_NAME: &str = "com.sskys18.telegram-korean-search";
const ACCOUNT_NAME: &str = "session-key";
const KEY_SIZE: usize = 32;

/// Retrieve the AES-256 key from the macOS Keychain, or create one if it doesn't exist.
pub fn get_or_create_key() -> Result<[u8; KEY_SIZE], KeychainError> {
    match get_generic_password(SERVICE_NAME, ACCOUNT_NAME) {
        Ok(key_data) => {
            if key_data.len() != KEY_SIZE {
                return Err(KeychainError::InvalidKeyLength(key_data.len()));
            }
            let mut key = [0u8; KEY_SIZE];
            key.copy_from_slice(&key_data);
            Ok(key)
        }
        Err(e) if e.code() == -25300 => {
            // errSecItemNotFound — create a new key
            let key = generate_key();
            set_generic_password(SERVICE_NAME, ACCOUNT_NAME, &key)
                .map_err(KeychainError::Framework)?;
            Ok(key)
        }
        Err(e) => Err(KeychainError::Framework(e)),
    }
}

/// Delete the AES key from the Keychain (for logout/reset).
pub fn delete_key() -> Result<(), KeychainError> {
    match delete_generic_password(SERVICE_NAME, ACCOUNT_NAME) {
        Ok(()) => Ok(()),
        Err(e) if e.code() == -25300 => Ok(()), // already gone
        Err(e) => Err(KeychainError::Framework(e)),
    }
}

fn generate_key() -> [u8; KEY_SIZE] {
    let mut key = [0u8; KEY_SIZE];
    rand::thread_rng().fill_bytes(&mut key);
    key
}

#[derive(Debug)]
pub enum KeychainError {
    Framework(security_framework::base::Error),
    InvalidKeyLength(usize),
}

impl std::fmt::Display for KeychainError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            KeychainError::Framework(e) => write!(f, "keychain error: {}", e),
            KeychainError::InvalidKeyLength(len) => {
                write!(
                    f,
                    "invalid key length in keychain: {} (expected {})",
                    len, KEY_SIZE
                )
            }
        }
    }
}

// Note: Keychain tests require macOS Keychain access and may prompt for permission.
// They are placed behind a feature gate and should be run manually.
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_key_length() {
        let key = generate_key();
        assert_eq!(key.len(), KEY_SIZE);
    }

    #[test]
    fn test_generate_key_randomness() {
        let key1 = generate_key();
        let key2 = generate_key();
        assert_ne!(key1, key2);
    }

    // Integration test: requires macOS Keychain access.
    // Run with: cargo test -- --ignored test_keychain_roundtrip
    #[test]
    #[ignore]
    fn test_keychain_roundtrip() {
        // Clean up from any previous failed run
        let _ = delete_key();

        // First call should create a new key
        let key1 = get_or_create_key().unwrap();
        assert_eq!(key1.len(), KEY_SIZE);

        // Second call should return the same key
        let key2 = get_or_create_key().unwrap();
        assert_eq!(key1, key2);

        // Clean up
        delete_key().unwrap();

        // After deletion, a new key should be generated
        let key3 = get_or_create_key().unwrap();
        assert_ne!(key1, key3);

        // Final cleanup
        delete_key().unwrap();
    }
}
