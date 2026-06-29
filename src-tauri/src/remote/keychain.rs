#[cfg(target_os = "macos")]
const SERVICE: &str = "com.jean.desktop.remote-server-ssh";

#[cfg(target_os = "macos")]
pub fn store_passphrase(server_id: &str, passphrase: &str) -> Result<(), String> {
    security_framework::passwords::set_generic_password(SERVICE, server_id, passphrase.as_bytes())
        .map_err(|error| format!("Failed to store SSH key passphrase in macOS Keychain: {error}"))
}

#[cfg(not(target_os = "macos"))]
pub fn store_passphrase(_server_id: &str, _passphrase: &str) -> Result<(), String> {
    Err("SSH key passphrase storage is currently supported on macOS only".to_string())
}

#[cfg(target_os = "macos")]
pub fn load_passphrase(server_id: &str) -> Result<Option<String>, String> {
    const ERR_SEC_ITEM_NOT_FOUND: i32 = -25300;

    match security_framework::passwords::get_generic_password(SERVICE, server_id) {
        Ok(passphrase) => String::from_utf8(passphrase)
            .map(Some)
            .map_err(|_| "The SSH key passphrase in macOS Keychain is not valid UTF-8".to_string()),
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
