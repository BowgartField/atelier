use serde::{Deserialize, Serialize};

fn default_ssh_port() -> u16 {
    22
}

fn default_remote_port() -> u16 {
    3456
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RemoteServerAuth {
    SshKeyPath {
        path: String,
        #[serde(default, skip_serializing)]
        passphrase: Option<String>,
    },
    Password {
        password: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum RemoteServerStatus {
    Disconnected,
    Connecting,
    Connected,
    Provisioning,
    Error,
}

impl Default for RemoteServerStatus {
    fn default() -> Self {
        Self::Disconnected
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RemoteServerConfig {
    pub id: String,
    pub name: String,
    pub host: String,
    #[serde(default = "default_ssh_port")]
    pub port: u16,
    pub username: String,
    pub auth: RemoteServerAuth,
    #[serde(default)]
    pub default: bool,
    #[serde(default = "default_remote_port")]
    pub remote_port: u16,
    #[serde(default)]
    pub status: RemoteServerStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub http_token: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub installed_version: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RemoteServerInput {
    pub name: String,
    pub host: String,
    #[serde(default = "default_ssh_port")]
    pub port: u16,
    pub username: String,
    pub auth: RemoteServerAuth,
    #[serde(default)]
    pub default: bool,
    #[serde(default = "default_remote_port")]
    pub remote_port: u16,
}

impl RemoteServerInput {
    pub fn validate(&self) -> Result<(), String> {
        validate_required("Server name", &self.name, 80)?;
        validate_required("Host", &self.host, 255)?;
        validate_required("Username", &self.username, 128)?;
        if self.port == 0 {
            return Err("SSH port must be greater than zero".to_string());
        }
        if self.remote_port == 0 {
            return Err("Remote Jean port must be greater than zero".to_string());
        }
        if self.host.starts_with('-') || self.username.starts_with('-') {
            return Err("Host and username cannot start with '-'".to_string());
        }
        if !self
            .username
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || "._-".contains(character))
        {
            return Err(
                "Username may contain only letters, numbers, dots, underscores, and dashes"
                    .to_string(),
            );
        }
        match &self.auth {
            RemoteServerAuth::SshKeyPath { path, passphrase } => {
                validate_required("SSH key path", path, 4096)?;
                if let Some(passphrase) = passphrase {
                    validate_secret("SSH key passphrase", passphrase)?;
                }
            }
            RemoteServerAuth::Password { password } => {
                validate_required("SSH password", password, 4096)?;
            }
        }
        Ok(())
    }

    pub fn into_config(self, id: String) -> RemoteServerConfig {
        RemoteServerConfig {
            id,
            name: self.name.trim().to_string(),
            host: self.host.trim().to_string(),
            port: self.port,
            username: self.username.trim().to_string(),
            auth: self.auth,
            default: self.default,
            remote_port: self.remote_port,
            status: RemoteServerStatus::Disconnected,
            http_token: None,
            installed_version: None,
        }
    }
}

fn validate_required(label: &str, value: &str, max_len: usize) -> Result<(), String> {
    if value.trim().is_empty() {
        return Err(format!("{label} cannot be empty"));
    }
    if value.len() > max_len {
        return Err(format!("{label} is too long (max {max_len} characters)"));
    }
    if value.contains(['\r', '\n', '\0']) {
        return Err(format!("{label} contains invalid characters"));
    }
    Ok(())
}

fn validate_secret(label: &str, value: &str) -> Result<(), String> {
    if value.len() > 4096 {
        return Err(format!("{label} is too long (max 4096 characters)"));
    }
    if value.contains(['\r', '\n', '\0']) {
        return Err(format!("{label} contains invalid characters"));
    }
    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RemoteConnectionTest {
    pub success: bool,
    pub message: String,
    pub hostname: Option<String>,
    pub os: Option<String>,
    pub architecture: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ProvisionResult {
    pub success: bool,
    pub version: String,
    pub remote_port: u16,
    pub service_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RemoteConnection {
    pub server_id: String,
    pub local_port: u16,
    pub remote_port: u16,
    pub token: String,
    pub url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RemoteServerStatusInfo {
    pub server_id: String,
    pub status: RemoteServerStatus,
    pub local_port: Option<u16>,
    pub remote_port: u16,
    pub last_error: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn input_validation_rejects_option_injection_and_newlines() {
        let input = RemoteServerInput {
            name: "Cloud".to_string(),
            host: "-oProxyCommand=bad".to_string(),
            port: 22,
            username: "jean".to_string(),
            auth: RemoteServerAuth::SshKeyPath {
                path: "/tmp/key".to_string(),
                passphrase: None,
            },
            default: false,
            remote_port: 3456,
        };
        assert!(input.validate().is_err());

        let mut valid = input;
        valid.host = "example.com".to_string();
        valid.name = "bad\nname".to_string();
        assert!(valid.validate().is_err());
    }

    #[test]
    fn auth_serialization_uses_snake_case_tag() {
        let auth = RemoteServerAuth::SshKeyPath {
            path: "~/.ssh/id_ed25519".to_string(),
            passphrase: Some("never serialize me".to_string()),
        };
        let value = serde_json::to_value(auth).unwrap();
        assert_eq!(value["type"], "ssh_key_path");
        assert!(value.get("passphrase").is_none());
    }
}
