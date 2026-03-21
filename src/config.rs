use crate::error::{AppError, Result};
use clap::Parser;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

/// Default SSH port
const DEFAULT_SSH_PORT: u16 = 22;
/// Default SSH user
const DEFAULT_SSH_USER: &str = "root";
/// Default container name
const DEFAULT_CONTAINER: &str = "amnezia-xray";

/// Validate that a container name contains only safe characters.
/// Docker container names allow `[a-zA-Z0-9][a-zA-Z0-9_.-]`.
fn is_valid_container_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 128
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.' || c == '-')
}

/// CLI arguments for amnezia-xray-admin
#[derive(Parser, Debug)]
#[command(name = "amnezia-xray-admin")]
#[command(about = "Hacker-aesthetic TUI dashboard for managing Amnezia VPN's Xray server")]
#[command(
    long_about = "Hacker-aesthetic TUI dashboard for managing Amnezia VPN's Xray (VLESS + XTLS-Reality) server.\n\nConnects to your VPS via SSH, talks to the Xray gRPC API for live user management\nand traffic stats. No container restarts needed.\n\nOn first run, a setup wizard guides you through the SSH connection.\nConfig is saved to ~/.config/amnezia-xray-admin/config.toml.\n\nExamples:\n  amnezia-xray-admin                        # Use saved config or start setup wizard\n  amnezia-xray-admin --ssh-host vps-vpn     # Connect using SSH config alias\n  amnezia-xray-admin --host 1.2.3.4 --key ~/.ssh/id_ed25519"
)]
#[command(version)]
pub struct Cli {
    /// SSH host (IP or hostname) to connect to
    #[arg(long = "host")]
    pub host: Option<String>,

    /// SSH port
    #[arg(long = "port")]
    pub port: Option<u16>,

    /// SSH user
    #[arg(long = "user")]
    pub user: Option<String>,

    /// Path to SSH private key
    #[arg(long = "key")]
    pub key_path: Option<PathBuf>,

    /// SSH config Host alias (e.g. vps-vpn). If set, host/port/user/key are resolved from ~/.ssh/config
    #[arg(long = "ssh-host")]
    pub ssh_host: Option<String>,

    /// Docker container name running Xray
    #[arg(long = "container")]
    pub container: Option<String>,
}

/// Application configuration
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Config {
    /// SSH host (IP or hostname)
    pub host: Option<String>,
    /// SSH port (default: 22)
    #[serde(default = "default_port")]
    pub port: u16,
    /// SSH user (default: root)
    #[serde(default = "default_user")]
    pub user: String,
    /// Path to SSH private key
    pub key_path: Option<PathBuf>,
    /// SSH config Host alias (e.g. vps-vpn)
    pub ssh_host: Option<String>,
    /// Docker container name (default: amnezia-xray)
    #[serde(default = "default_container")]
    pub container: String,
}

fn default_port() -> u16 {
    DEFAULT_SSH_PORT
}

fn default_user() -> String {
    DEFAULT_SSH_USER.to_string()
}

fn default_container() -> String {
    DEFAULT_CONTAINER.to_string()
}

impl Default for Config {
    fn default() -> Self {
        Self {
            host: None,
            port: DEFAULT_SSH_PORT,
            user: DEFAULT_SSH_USER.to_string(),
            key_path: None,
            ssh_host: None,
            container: DEFAULT_CONTAINER.to_string(),
        }
    }
}

impl Config {
    /// Returns the config file path: ~/.config/amnezia-xray-admin/config.toml
    pub fn config_path() -> Result<PathBuf> {
        let config_dir = dirs::config_dir()
            .ok_or_else(|| AppError::Config("cannot determine config directory".to_string()))?;
        Ok(config_dir.join("amnezia-xray-admin").join("config.toml"))
    }

    /// Load config from the default config file path.
    /// Returns default config if the file does not exist.
    pub fn load() -> Result<Self> {
        let path = Self::config_path()?;
        Self::load_from(&path)
    }

    /// Load config from a specific path.
    /// Returns default config if the file does not exist.
    pub fn load_from(path: &PathBuf) -> Result<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let content = fs::read_to_string(path)?;
        let config: Config = toml::from_str(&content)?;
        if !is_valid_container_name(&config.container) {
            return Err(AppError::Config(format!(
                "invalid container name '{}': only alphanumeric, hyphen, underscore, and dot allowed",
                config.container
            )));
        }
        Ok(config)
    }

    /// Save config to the default config file path with 0600 permissions.
    pub fn save(&self) -> Result<()> {
        let path = Self::config_path()?;
        self.save_to(&path)
    }

    /// Save config to a specific path with 0600 permissions.
    pub fn save_to(&self, path: &PathBuf) -> Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let content = toml::to_string_pretty(self)
            .map_err(|e| AppError::Config(format!("TOML serialize error: {}", e)))?;
        fs::write(path, &content)?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let perms = fs::Permissions::from_mode(0o600);
            fs::set_permissions(path, perms)?;
        }

        Ok(())
    }

    /// Merge CLI arguments into this config. CLI args take precedence.
    pub fn merge_cli(&mut self, cli: &Cli) {
        if let Some(ref host) = cli.host {
            self.host = Some(host.clone());
        }
        if let Some(port) = cli.port {
            self.port = port;
        }
        if let Some(ref user) = cli.user {
            self.user = user.clone();
        }
        if let Some(ref key_path) = cli.key_path {
            self.key_path = Some(key_path.clone());
        }
        if let Some(ref ssh_host) = cli.ssh_host {
            self.ssh_host = Some(ssh_host.clone());
        }
        if let Some(ref container) = cli.container {
            if is_valid_container_name(container) {
                self.container = container.clone();
            }
        }
    }

    /// Returns true if this config has enough info to attempt an SSH connection.
    /// Either ssh_host (config alias) or host must be set.
    pub fn has_connection_info(&self) -> bool {
        self.ssh_host.is_some() || self.host.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    #[test]
    fn test_default_config() {
        let config = Config::default();
        assert_eq!(config.host, None);
        assert_eq!(config.port, 22);
        assert_eq!(config.user, "root");
        assert_eq!(config.key_path, None);
        assert_eq!(config.ssh_host, None);
        assert_eq!(config.container, "amnezia-xray");
    }

    #[test]
    fn test_load_nonexistent_returns_default() {
        let path = PathBuf::from("/tmp/nonexistent-amnezia-test-config.toml");
        let config = Config::load_from(&path).unwrap();
        assert_eq!(config, Config::default());
    }

    #[test]
    fn test_load_full_config() {
        let mut f = NamedTempFile::new().unwrap();
        write!(
            f,
            r#"
host = "192.168.1.100"
port = 2222
user = "admin"
key_path = "/home/admin/.ssh/id_ed25519"
ssh_host = "vps-vpn"
container = "my-xray"
"#
        )
        .unwrap();

        let config = Config::load_from(&f.path().to_path_buf()).unwrap();
        assert_eq!(config.host.as_deref(), Some("192.168.1.100"));
        assert_eq!(config.port, 2222);
        assert_eq!(config.user, "admin");
        assert_eq!(
            config.key_path,
            Some(PathBuf::from("/home/admin/.ssh/id_ed25519"))
        );
        assert_eq!(config.ssh_host.as_deref(), Some("vps-vpn"));
        assert_eq!(config.container, "my-xray");
    }

    #[test]
    fn test_load_partial_config_uses_defaults() {
        let mut f = NamedTempFile::new().unwrap();
        write!(
            f,
            r#"
host = "10.0.0.1"
"#
        )
        .unwrap();

        let config = Config::load_from(&f.path().to_path_buf()).unwrap();
        assert_eq!(config.host.as_deref(), Some("10.0.0.1"));
        assert_eq!(config.port, 22);
        assert_eq!(config.user, "root");
        assert_eq!(config.container, "amnezia-xray");
    }

    #[test]
    fn test_save_and_reload() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");

        let config = Config {
            host: Some("1.2.3.4".to_string()),
            port: 2222,
            user: "deployer".to_string(),
            key_path: Some(PathBuf::from("/keys/id_rsa")),
            ssh_host: Some("my-server".to_string()),
            container: "xray-test".to_string(),
        };
        config.save_to(&path).unwrap();

        let loaded = Config::load_from(&path).unwrap();
        assert_eq!(config, loaded);
    }

    #[cfg(unix)]
    #[test]
    fn test_save_permissions_0600() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");

        let config = Config::default();
        config.save_to(&path).unwrap();

        let metadata = fs::metadata(&path).unwrap();
        let mode = metadata.permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }

    #[test]
    fn test_save_creates_parent_dirs() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("dir").join("config.toml");

        let config = Config::default();
        config.save_to(&path).unwrap();
        assert!(path.exists());
    }

    #[test]
    fn test_merge_cli_all_fields() {
        let mut config = Config::default();
        let cli = Cli {
            host: Some("5.6.7.8".to_string()),
            port: Some(3333),
            user: Some("testuser".to_string()),
            key_path: Some(PathBuf::from("/tmp/key")),
            ssh_host: Some("alias".to_string()),
            container: Some("ctr".to_string()),
        };
        config.merge_cli(&cli);

        assert_eq!(config.host.as_deref(), Some("5.6.7.8"));
        assert_eq!(config.port, 3333);
        assert_eq!(config.user, "testuser");
        assert_eq!(config.key_path, Some(PathBuf::from("/tmp/key")));
        assert_eq!(config.ssh_host.as_deref(), Some("alias"));
        assert_eq!(config.container, "ctr");
    }

    #[test]
    fn test_merge_cli_partial_preserves_config() {
        let mut config = Config {
            host: Some("original.host".to_string()),
            port: 2222,
            user: "original".to_string(),
            key_path: Some(PathBuf::from("/original/key")),
            ssh_host: Some("original-alias".to_string()),
            container: "original-ctr".to_string(),
        };
        let cli = Cli {
            host: None,
            port: Some(4444),
            user: None,
            key_path: None,
            ssh_host: None,
            container: None,
        };
        config.merge_cli(&cli);

        assert_eq!(config.host.as_deref(), Some("original.host"));
        assert_eq!(config.port, 4444);
        assert_eq!(config.user, "original");
        assert_eq!(config.key_path, Some(PathBuf::from("/original/key")));
        assert_eq!(config.ssh_host.as_deref(), Some("original-alias"));
        assert_eq!(config.container, "original-ctr");
    }

    #[test]
    fn test_merge_cli_none_preserves_defaults() {
        let mut config = Config::default();
        let cli = Cli {
            host: None,
            port: None,
            user: None,
            key_path: None,
            ssh_host: None,
            container: None,
        };
        config.merge_cli(&cli);
        assert_eq!(config, Config::default());
    }

    #[test]
    fn test_has_connection_info() {
        let mut config = Config::default();
        assert!(!config.has_connection_info());

        config.host = Some("1.2.3.4".to_string());
        assert!(config.has_connection_info());

        config.host = None;
        config.ssh_host = Some("alias".to_string());
        assert!(config.has_connection_info());
    }

    #[test]
    fn test_load_invalid_toml() {
        let mut f = NamedTempFile::new().unwrap();
        write!(f, "this is {{{{ not valid toml").unwrap();

        let result = Config::load_from(&f.path().to_path_buf());
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.to_string().contains("TOML parse error"));
    }

    #[test]
    fn test_config_roundtrip_toml() {
        let config = Config {
            host: Some("example.com".to_string()),
            port: 22,
            user: "root".to_string(),
            key_path: None,
            ssh_host: Some("vps-vpn".to_string()),
            container: "amnezia-xray".to_string(),
        };
        let toml_str = toml::to_string_pretty(&config).unwrap();
        let parsed: Config = toml::from_str(&toml_str).unwrap();
        assert_eq!(config, parsed);
    }

    #[test]
    fn test_config_path_returns_valid_path() {
        let path = Config::config_path()
            .expect("config_path should succeed on systems with a home directory");
        assert!(path.ends_with("amnezia-xray-admin/config.toml"));
    }
}
