use std::collections::HashMap;
use std::sync::Mutex;

use once_cell::sync::Lazy;

// Cache passphrases for the app session so macOS only prompts once per server.
// auth_secret() is called for every SSH invocation — including multiplexed
// ones that reuse an existing ControlMaster and don't actually need the secret.
static PASSPHRASE_CACHE: Lazy<Mutex<HashMap<String, String>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

#[cfg(target_os = "macos")]
const SERVICE: &str = "com.jean.desktop.remote-server-ssh";

#[cfg(target_os = "macos")]
pub fn store_passphrase(server_id: &str, passphrase: &str) -> Result<(), String> {
    if let Ok(mut cache) = PASSPHRASE_CACHE.lock() {
        cache.insert(server_id.to_string(), passphrase.to_string());
    }
    security_framework::passwords::set_generic_password(SERVICE, server_id, passphrase.as_bytes())
        .map_err(|error| format!("Failed to store SSH key passphrase in macOS Keychain: {error}"))
}

#[cfg(not(target_os = "macos"))]
pub fn store_passphrase(_server_id: &str, _passphrase: &str) -> Result<(), String> {
    Err("SSH key passphrase storage is currently supported on macOS only".to_string())
}

#[cfg(target_os = "macos")]
pub fn load_passphrase(server_id: &str) -> Result<Option<String>, String> {
    if let Ok(cache) = PASSPHRASE_CACHE.lock() {
        if let Some(passphrase) = cache.get(server_id) {
            return Ok(Some(passphrase.clone()));
        }
    }

    const ERR_SEC_ITEM_NOT_FOUND: i32 = -25300;

    match security_framework::passwords::get_generic_password(SERVICE, server_id) {
        Ok(passphrase) => {
            let passphrase = String::from_utf8(passphrase)
                .map_err(|_| "The SSH key passphrase in macOS Keychain is not valid UTF-8".to_string())?;
            if let Ok(mut cache) = PASSPHRASE_CACHE.lock() {
                cache.insert(server_id.to_string(), passphrase.clone());
            }
            Ok(Some(passphrase))
        }
        Err(error) if error.code() == ERR_SEC_ITEM_NOT_FOUND => Ok(None),
        Err(error) => Err(format!(
            "Failed to load SSH key passphrase from macOS Keychain: {error}"
        )),
    }
}

#[cfg(not(target_os = "macos"))]
pub fn load_passphrase(_server_id: &str) -> Result<Option<String>, String> {
    Ok(None)
}

#[cfg(target_os = "macos")]
pub fn delete_passphrase(server_id: &str) -> Result<(), String> {
    if let Ok(mut cache) = PASSPHRASE_CACHE.lock() {
        cache.remove(server_id);
    }

    const ERR_SEC_ITEM_NOT_FOUND: i32 = -25300;

    match security_framework::passwords::delete_generic_password(SERVICE, server_id) {
        Ok(()) => Ok(()),
        Err(error) if error.code() == ERR_SEC_ITEM_NOT_FOUND => Ok(()),
        Err(error) => Err(format!(
            "Failed to delete SSH key passphrase from macOS Keychain: {error}"
        )),
    }
}

#[cfg(not(target_os = "macos"))]
pub fn delete_passphrase(_server_id: &str) -> Result<(), String> {
    Ok(())
}
