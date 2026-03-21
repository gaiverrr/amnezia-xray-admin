use crate::error::{AppError, Result};
use async_trait::async_trait;
use russh::client;
use russh::ChannelMsg;
use russh_keys::load_secret_key;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::net::ToSocketAddrs;

/// Parsed entry from ~/.ssh/config
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SshHostConfig {
    pub hostname: Option<String>,
    pub port: Option<u16>,
    pub user: Option<String>,
    pub identity_file: Option<PathBuf>,
}

/// Result of a remote command execution
#[derive(Debug, Clone)]
pub struct CommandOutput {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: u32,
}

impl CommandOutput {
    pub fn success(&self) -> bool {
        self.exit_code == 0
    }
}

/// SSH client handler for russh
struct SshHandler;

#[async_trait]
impl client::Handler for SshHandler {
    type Error = russh::Error;

    async fn check_server_key(
        &mut self,
        _server_public_key: &russh_keys::ssh_key::PublicKey,
    ) -> std::result::Result<bool, Self::Error> {
        // Accept all host keys for now.
        // TODO: implement known_hosts verification
        Ok(true)
    }
}

/// An active SSH session that can execute remote commands.
pub struct SshSession {
    handle: client::Handle<SshHandler>,
    container: String,
}

impl SshSession {
    /// Connect to a remote host and authenticate.
    ///
    /// Tries key file authentication first, then falls back to ssh-agent.
    pub async fn connect<A: ToSocketAddrs>(
        addr: A,
        user: &str,
        key_path: Option<&Path>,
        container: &str,
    ) -> Result<Self> {
        let config = Arc::new(client::Config {
            inactivity_timeout: Some(std::time::Duration::from_secs(60)),
            keepalive_interval: Some(std::time::Duration::from_secs(15)),
            ..<_>::default()
        });

        let mut handle = client::connect(config, addr, SshHandler)
            .await
            .map_err(|e| AppError::Ssh(format!("connection failed: {}", e)))?;

        let authenticated = if let Some(key_path) = key_path {
            let key = load_secret_key(key_path, None)
                .map_err(|e| AppError::Ssh(format!("failed to load key {:?}: {}", key_path, e)))?;
            handle
                .authenticate_publickey(user, Arc::new(key))
                .await
                .map_err(|e| AppError::Ssh(format!("key auth failed: {}", e)))?
        } else {
            Self::try_agent_auth(&mut handle, user).await?
        };

        if !authenticated {
            return Err(AppError::Ssh("authentication failed".to_string()));
        }

        Ok(Self {
            handle,
            container: container.to_string(),
        })
    }

    /// Try authenticating via ssh-agent.
    async fn try_agent_auth(handle: &mut client::Handle<SshHandler>, user: &str) -> Result<bool> {
        let mut agent = russh_keys::agent::client::AgentClient::connect_env()
            .await
            .map_err(|e| AppError::Ssh(format!("ssh-agent connect failed: {}", e)))?;

        let identities = agent
            .request_identities()
            .await
            .map_err(|e| AppError::Ssh(format!("ssh-agent list keys failed: {}", e)))?;

        for key in identities {
            match handle
                .authenticate_publickey_with(user, key.clone(), &mut agent)
                .await
            {
                Ok(true) => return Ok(true),
                Ok(false) => continue,
                Err(_) => continue,
            }
        }

        Ok(false)
    }

    /// Execute a command on the remote host and return its output.
    pub async fn exec_command(&self, command: &str) -> Result<CommandOutput> {
        let mut channel = self
            .handle
            .channel_open_session()
            .await
            .map_err(|e| AppError::Ssh(format!("channel open failed: {}", e)))?;

        channel
            .exec(true, command)
            .await
            .map_err(|e| AppError::Ssh(format!("exec failed: {}", e)))?;

        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let mut exit_code = 0u32;

        loop {
            match channel.wait().await {
                Some(ChannelMsg::Data { ref data }) => {
                    stdout.extend_from_slice(data);
                }
                Some(ChannelMsg::ExtendedData { ref data, ext }) => {
                    if ext == 1 {
                        stderr.extend_from_slice(data);
                    }
                }
                Some(ChannelMsg::ExitStatus { exit_status }) => {
                    exit_code = exit_status;
                }
                None => break,
                _ => {}
            }
        }

        Ok(CommandOutput {
            stdout: String::from_utf8_lossy(&stdout).to_string(),
            stderr: String::from_utf8_lossy(&stderr).to_string(),
            exit_code,
        })
    }

    /// Execute a command inside the Docker container.
    pub async fn exec_in_container(&self, command: &str) -> Result<CommandOutput> {
        let full_cmd = format!("docker exec {} {}", self.container, command);
        self.exec_command(&full_cmd).await
    }

    /// Close the SSH session.
    pub async fn close(self) -> Result<()> {
        self.handle
            .disconnect(russh::Disconnect::ByApplication, "", "")
            .await
            .map_err(|e| AppError::Ssh(format!("disconnect failed: {}", e)))?;
        Ok(())
    }

    pub fn is_closed(&self) -> bool {
        self.handle.is_closed()
    }
}

/// Parse an SSH config file and return a map of Host -> SshHostConfig.
///
/// Supports a subset of OpenSSH config: Host, HostName, Port, User, IdentityFile.
/// Wildcard hosts (e.g. Host *) are ignored.
pub fn parse_ssh_config(content: &str) -> HashMap<String, SshHostConfig> {
    let mut hosts: HashMap<String, SshHostConfig> = HashMap::new();
    let mut current_host: Option<String> = None;

    for line in content.lines() {
        let line = line.trim();

        // Skip empty lines and comments
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        // Split on first whitespace or '='
        let (key, value) = match line.split_once(|c: char| c.is_whitespace() || c == '=') {
            Some((k, v)) => (k.trim().to_lowercase(), v.trim().to_string()),
            None => continue,
        };

        match key.as_str() {
            "host" => {
                // Skip wildcard hosts
                if value.contains('*') || value.contains('?') {
                    current_host = None;
                } else {
                    // Handle multiple hosts on one line — take the first
                    let host_name = value
                        .split_whitespace()
                        .next()
                        .unwrap_or(&value)
                        .to_string();
                    current_host = Some(host_name.clone());
                    hosts.entry(host_name).or_default();
                }
            }
            "hostname" => {
                if let Some(ref host) = current_host {
                    if let Some(entry) = hosts.get_mut(host) {
                        entry.hostname = Some(value);
                    }
                }
            }
            "port" => {
                if let Some(ref host) = current_host {
                    if let Ok(port) = value.parse::<u16>() {
                        if let Some(entry) = hosts.get_mut(host) {
                            entry.port = Some(port);
                        }
                    }
                }
            }
            "user" => {
                if let Some(ref host) = current_host {
                    if let Some(entry) = hosts.get_mut(host) {
                        entry.user = Some(value);
                    }
                }
            }
            "identityfile" => {
                if let Some(ref host) = current_host {
                    if let Some(entry) = hosts.get_mut(host) {
                        let expanded = expand_tilde(&value);
                        entry.identity_file = Some(PathBuf::from(expanded));
                    }
                }
            }
            _ => {}
        }
    }

    hosts
}

/// Load and parse the user's ~/.ssh/config file.
/// Returns an empty map if the file doesn't exist.
pub fn load_ssh_config() -> HashMap<String, SshHostConfig> {
    let home = match dirs::home_dir() {
        Some(h) => h,
        None => return HashMap::new(),
    };
    let config_path = home.join(".ssh").join("config");
    match std::fs::read_to_string(&config_path) {
        Ok(content) => parse_ssh_config(&content),
        Err(_) => HashMap::new(),
    }
}

/// Resolve an SSH config host alias into connection parameters.
pub fn resolve_ssh_host(alias: &str) -> Option<SshHostConfig> {
    let configs = load_ssh_config();
    configs.get(alias).cloned()
}

/// Expand ~ to the user's home directory.
fn expand_tilde(path: &str) -> String {
    if path.starts_with("~/") {
        if let Some(home) = dirs::home_dir() {
            return format!("{}{}", home.display(), &path[1..]);
        }
    }
    path.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_ssh_config_basic() {
        let content = r#"
Host vps-vpn
    HostName 192.168.1.100
    Port 2222
    User admin
    IdentityFile ~/.ssh/id_ed25519
"#;
        let configs = parse_ssh_config(content);
        let entry = configs.get("vps-vpn").expect("vps-vpn should be present");
        assert_eq!(entry.hostname.as_deref(), Some("192.168.1.100"));
        assert_eq!(entry.port, Some(2222));
        assert_eq!(entry.user.as_deref(), Some("admin"));
        assert!(entry.identity_file.is_some());
    }

    #[test]
    fn test_parse_ssh_config_multiple_hosts() {
        let content = r#"
Host server1
    HostName 10.0.0.1
    User root

Host server2
    HostName 10.0.0.2
    Port 22
    User deploy
"#;
        let configs = parse_ssh_config(content);
        assert_eq!(configs.len(), 2);

        let s1 = configs.get("server1").unwrap();
        assert_eq!(s1.hostname.as_deref(), Some("10.0.0.1"));
        assert_eq!(s1.user.as_deref(), Some("root"));
        assert_eq!(s1.port, None);

        let s2 = configs.get("server2").unwrap();
        assert_eq!(s2.hostname.as_deref(), Some("10.0.0.2"));
        assert_eq!(s2.user.as_deref(), Some("deploy"));
        assert_eq!(s2.port, Some(22));
    }

    #[test]
    fn test_parse_ssh_config_wildcard_ignored() {
        let content = r#"
Host *
    ServerAliveInterval 60

Host myhost
    HostName example.com
"#;
        let configs = parse_ssh_config(content);
        assert_eq!(configs.len(), 1);
        assert!(configs.contains_key("myhost"));
        assert!(!configs.contains_key("*"));
    }

    #[test]
    fn test_parse_ssh_config_empty() {
        let configs = parse_ssh_config("");
        assert!(configs.is_empty());
    }

    #[test]
    fn test_parse_ssh_config_comments_only() {
        let content = r#"
# This is a comment
# Another comment
"#;
        let configs = parse_ssh_config(content);
        assert!(configs.is_empty());
    }

    #[test]
    fn test_parse_ssh_config_equals_separator() {
        let content = r#"
Host myhost
    HostName=example.com
    Port=22
    User=testuser
"#;
        let configs = parse_ssh_config(content);
        let entry = configs.get("myhost").unwrap();
        assert_eq!(entry.hostname.as_deref(), Some("example.com"));
        assert_eq!(entry.port, Some(22));
        assert_eq!(entry.user.as_deref(), Some("testuser"));
    }

    #[test]
    fn test_parse_ssh_config_case_insensitive_keys() {
        let content = r#"
HOST myhost
    HOSTNAME example.com
    PORT 3333
    USER admin
    IDENTITYFILE ~/.ssh/my_key
"#;
        let configs = parse_ssh_config(content);
        let entry = configs.get("myhost").unwrap();
        assert_eq!(entry.hostname.as_deref(), Some("example.com"));
        assert_eq!(entry.port, Some(3333));
        assert_eq!(entry.user.as_deref(), Some("admin"));
        assert!(entry.identity_file.is_some());
    }

    #[test]
    fn test_parse_ssh_config_no_hostname() {
        let content = r#"
Host myalias
    User root
    Port 22
"#;
        let configs = parse_ssh_config(content);
        let entry = configs.get("myalias").unwrap();
        assert_eq!(entry.hostname, None);
        assert_eq!(entry.user.as_deref(), Some("root"));
    }

    #[test]
    fn test_parse_ssh_config_unknown_keys_ignored() {
        let content = r#"
Host myhost
    HostName example.com
    ForwardAgent yes
    ProxyCommand ssh -W %h:%p bastion
    ServerAliveInterval 60
"#;
        let configs = parse_ssh_config(content);
        let entry = configs.get("myhost").unwrap();
        assert_eq!(entry.hostname.as_deref(), Some("example.com"));
        assert_eq!(entry.port, None);
        assert_eq!(entry.user, None);
    }

    #[test]
    fn test_expand_tilde() {
        // With home dir available
        let result = expand_tilde("~/test/path");
        assert!(!result.starts_with("~/"));
        assert!(result.ends_with("/test/path"));

        // Without tilde
        let result = expand_tilde("/absolute/path");
        assert_eq!(result, "/absolute/path");

        // Just tilde slash
        let result = expand_tilde("~/");
        assert!(!result.starts_with("~/"));
    }

    #[test]
    fn test_expand_tilde_no_expand_for_non_tilde() {
        assert_eq!(expand_tilde("relative/path"), "relative/path");
        assert_eq!(expand_tilde("~notapath"), "~notapath");
    }

    #[test]
    fn test_command_output_success() {
        let output = CommandOutput {
            stdout: "hello".to_string(),
            stderr: String::new(),
            exit_code: 0,
        };
        assert!(output.success());
    }

    #[test]
    fn test_command_output_failure() {
        let output = CommandOutput {
            stdout: String::new(),
            stderr: "error".to_string(),
            exit_code: 1,
        };
        assert!(!output.success());
    }

    #[test]
    fn test_ssh_host_config_default() {
        let config = SshHostConfig::default();
        assert_eq!(config.hostname, None);
        assert_eq!(config.port, None);
        assert_eq!(config.user, None);
        assert_eq!(config.identity_file, None);
    }

    #[test]
    fn test_load_ssh_config_returns_map() {
        // Verify load_ssh_config returns a valid HashMap without panicking.
        let configs = load_ssh_config();
        // On any system, the result should be a HashMap (possibly empty)
        // Verify it's actually a usable collection
        assert!(configs.len() < 10_000, "unreasonably large SSH config");
    }

    #[test]
    fn test_parse_ssh_config_invalid_port() {
        let content = r#"
Host myhost
    HostName example.com
    Port notanumber
"#;
        let configs = parse_ssh_config(content);
        let entry = configs.get("myhost").unwrap();
        assert_eq!(entry.port, None); // Invalid port is ignored
    }

    #[test]
    fn test_parse_ssh_config_multiple_hosts_on_line() {
        let content = r#"
Host host1 host2
    HostName example.com
"#;
        let configs = parse_ssh_config(content);
        // We take the first host from the line
        let entry = configs.get("host1").unwrap();
        assert_eq!(entry.hostname.as_deref(), Some("example.com"));
    }

    #[test]
    fn test_parse_ssh_config_identity_file_tilde_expanded() {
        let content = r#"
Host myhost
    IdentityFile ~/.ssh/id_rsa
"#;
        let configs = parse_ssh_config(content);
        let entry = configs.get("myhost").unwrap();
        let id_path = entry.identity_file.as_ref().unwrap();
        // Should not start with ~
        assert!(!id_path.to_string_lossy().starts_with("~"));
        assert!(id_path.to_string_lossy().ends_with(".ssh/id_rsa"));
    }
}
