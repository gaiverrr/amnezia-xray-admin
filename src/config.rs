use crate::error::{AppError, Result};
use clap::Parser;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

/// Default SSH port
const DEFAULT_SSH_PORT: u16 = 22;
/// Default SSH user
const DEFAULT_SSH_USER: &str = "root";

/// CLI arguments for amnezia-xray-admin
#[derive(Parser, Debug)]
#[command(name = "amnezia-xray-admin")]
#[command(about = "Personal CLI + Telegram bot for managing a native-systemd Xray VPN")]
#[command(
    long_about = "Personal CLI + Telegram bot for managing a native-systemd Xray (VLESS + XHTTP + Reality) VPN.\n\nEdits /usr/local/etc/xray/config.json on the bridge host directly — either over SSH\n(default) or locally via --local. No Docker, no gRPC API.\n\nConfig is saved to ~/.config/amnezia-xray-admin/config.toml."
)]
#[command(after_help = "\
EXAMPLES:
  User management:
    amnezia-xray-admin --list-users
    amnezia-xray-admin --add-user Friend
    amnezia-xray-admin --delete-user Friend --yes
    amnezia-xray-admin --user-url Friend
    amnezia-xray-admin --user-qr Friend

  Server info:
    amnezia-xray-admin --server-info
    amnezia-xray-admin --online-status

  Telegram bot:
    amnezia-xray-admin --telegram-bot --local --admin-id <ID>")]
#[command(version)]
pub struct Cli {
    /// SSH host (IP or hostname) to connect to (used for bot URL generation)
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

    /// List users and exit (non-interactive mode)
    #[arg(long = "list-users")]
    pub list_users: bool,

    /// Get vless:// URL for a user by name and exit
    #[arg(long = "user-url")]
    pub user_url: Option<String>,

    /// Show QR code for a user's vless:// URL in terminal and exit
    #[arg(long = "user-qr")]
    pub user_qr: Option<String>,

    /// Show online status for all users and exit
    #[arg(long = "online-status")]
    pub online_status: bool,

    /// Show server info (version, traffic, user count, API status) and exit
    #[arg(long = "server-info")]
    pub server_info: bool,

    /// Use local backend (direct exec) — for running on the bridge VPS
    #[arg(long = "local")]
    pub local: bool,

    /// Run as Telegram bot daemon (requires --telegram-token or TELEGRAM_TOKEN env var)
    #[arg(long = "telegram-bot")]
    pub telegram_bot: bool,

    /// Telegram bot token (can also be set via TELEGRAM_TOKEN env var)
    #[arg(long = "telegram-token", env = "TELEGRAM_TOKEN")]
    pub telegram_token: Option<String>,

    /// Telegram admin chat ID (your Telegram user ID; send /start to @userinfobot to find it)
    #[arg(long = "admin-id", env = "ADMIN_ID")]
    pub admin_id: Option<i64>,

    /// Add a new user and print their vless:// URL
    #[arg(long = "add-user")]
    pub add_user: Option<String>,

    /// Delete a user by name
    #[arg(long = "delete-user")]
    pub delete_user: Option<String>,

    /// Skip interactive confirmation prompts.
    #[arg(long = "yes")]
    pub yes: bool,
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
    /// Telegram bot token
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub telegram_token: Option<String>,
    /// Telegram bot admin chat ID (set via --admin-id or ADMIN_ID env var)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub telegram_admin_chat_id: Option<i64>,
}

fn default_port() -> u16 {
    DEFAULT_SSH_PORT
}

fn default_user() -> String {
    DEFAULT_SSH_USER.to_string()
}

impl Default for Config {
    fn default() -> Self {
        Self {
            host: None,
            port: DEFAULT_SSH_PORT,
            user: DEFAULT_SSH_USER.to_string(),
            key_path: None,
            telegram_token: None,
            telegram_admin_chat_id: None,
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
        Ok(config)
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
        if let Some(admin_id) = cli.admin_id {
            self.telegram_admin_chat_id = Some(admin_id);
        }
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
    }

    #[test]
    fn test_merge_cli_all_fields() {
        let mut config = Config::default();
        let cli = Cli {
            host: Some("5.6.7.8".to_string()),
            port: Some(3333),
            user: Some("testuser".to_string()),
            key_path: Some(PathBuf::from("/tmp/key")),
            list_users: false,
            user_url: None,
            user_qr: None,
            online_status: false,
            server_info: false,
            local: false,
            telegram_bot: false,
            telegram_token: None,
            admin_id: None,
            add_user: None,
            delete_user: None,
            yes: false,
        };
        config.merge_cli(&cli);

        assert_eq!(config.host.as_deref(), Some("5.6.7.8"));
        assert_eq!(config.port, 3333);
        assert_eq!(config.user, "testuser");
        assert_eq!(config.key_path, Some(PathBuf::from("/tmp/key")));
    }

    #[test]
    fn test_merge_cli_partial_preserves_config() {
        let mut config = Config {
            host: Some("original.host".to_string()),
            port: 2222,
            user: "original".to_string(),
            key_path: Some(PathBuf::from("/original/key")),
            telegram_token: None,
            telegram_admin_chat_id: None,
        };
        let cli = Cli {
            host: None,
            port: Some(4444),
            user: None,
            key_path: None,
            list_users: false,
            user_url: None,
            user_qr: None,
            online_status: false,
            server_info: false,
            local: false,
            telegram_bot: false,
            telegram_token: None,
            admin_id: None,
            add_user: None,
            delete_user: None,
            yes: false,
        };
        config.merge_cli(&cli);

        assert_eq!(config.host.as_deref(), Some("original.host"));
        assert_eq!(config.port, 4444);
        assert_eq!(config.user, "original");
        assert_eq!(config.key_path, Some(PathBuf::from("/original/key")));
    }

    #[test]
    fn test_merge_cli_none_preserves_defaults() {
        let mut config = Config::default();
        let cli = Cli {
            host: None,
            port: None,
            user: None,
            key_path: None,
            list_users: false,
            user_url: None,
            user_qr: None,
            online_status: false,
            server_info: false,
            local: false,
            telegram_bot: false,
            telegram_token: None,
            admin_id: None,
            add_user: None,
            delete_user: None,
            yes: false,
        };
        config.merge_cli(&cli);
        assert_eq!(config, Config::default());
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
            telegram_token: None,
            telegram_admin_chat_id: None,
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

    #[test]
    fn test_config_telegram_token_serialization() {
        let config = Config {
            telegram_token: Some("123456:ABCdef".to_string()),
            ..Default::default()
        };
        let toml_str = toml::to_string_pretty(&config).unwrap();
        assert!(toml_str.contains("telegram_token = \"123456:ABCdef\""));

        let parsed: Config = toml::from_str(&toml_str).unwrap();
        assert_eq!(parsed.telegram_token, Some("123456:ABCdef".to_string()));
    }

    #[test]
    fn test_config_telegram_token_omitted_when_none() {
        let config = Config::default();
        let toml_str = toml::to_string_pretty(&config).unwrap();
        assert!(!toml_str.contains("telegram_token"));
    }

    #[test]
    fn test_config_telegram_fields_roundtrip() {
        let config = Config {
            host: Some("1.2.3.4".to_string()),
            port: 22,
            user: "root".to_string(),
            key_path: None,
            telegram_token: Some("123:abc".to_string()),
            telegram_admin_chat_id: Some(987654321),
        };
        let toml_str = toml::to_string_pretty(&config).unwrap();
        let parsed: Config = toml::from_str(&toml_str).unwrap();
        assert_eq!(config, parsed);
        assert_eq!(parsed.telegram_token, Some("123:abc".to_string()));
        assert_eq!(parsed.telegram_admin_chat_id, Some(987654321));
    }

    #[test]
    fn test_config_load_with_telegram_fields() {
        let mut f = NamedTempFile::new().unwrap();
        write!(
            f,
            r#"
host = "10.0.0.1"
port = 22
user = "root"
telegram_token = "999:xyz"
telegram_admin_chat_id = 12345
"#
        )
        .unwrap();

        let config = Config::load_from(&f.path().to_path_buf()).unwrap();
        assert_eq!(config.telegram_token, Some("999:xyz".to_string()));
        assert_eq!(config.telegram_admin_chat_id, Some(12345));
    }

    #[test]
    fn test_config_load_without_telegram_fields_defaults_to_none() {
        let mut f = NamedTempFile::new().unwrap();
        write!(
            f,
            r#"
host = "10.0.0.1"
"#
        )
        .unwrap();

        let config = Config::load_from(&f.path().to_path_buf()).unwrap();
        assert_eq!(config.telegram_token, None);
        assert_eq!(config.telegram_admin_chat_id, None);
    }

    #[test]
    fn test_cli_parse_delete_user() {
        let cli = Cli::parse_from(["app", "--delete-user", "Alice"]);
        assert_eq!(cli.delete_user, Some("Alice".to_string()));
        assert!(!cli.yes);
    }

    #[test]
    fn test_cli_parse_delete_user_with_yes() {
        let cli = Cli::parse_from(["app", "--delete-user", "Bob", "--yes"]);
        assert_eq!(cli.delete_user, Some("Bob".to_string()));
        assert!(cli.yes);
    }

    #[test]
    fn test_cli_parse_yes_without_delete() {
        let cli = Cli::parse_from(["app", "--yes"]);
        assert!(cli.yes);
        assert_eq!(cli.delete_user, None);
    }

    #[test]
    fn test_cli_delete_user_with_brackets() {
        let cli = Cli::parse_from(["app", "--delete-user", "Admin [macOS Tahoe]", "--yes"]);
        assert_eq!(cli.delete_user, Some("Admin [macOS Tahoe]".to_string()));
    }
}
