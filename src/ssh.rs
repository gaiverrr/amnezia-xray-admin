use crate::error::{AppError, Result};
use async_trait::async_trait;
use russh::client;
use russh::ChannelMsg;
use russh_keys::load_secret_key;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::net::ToSocketAddrs;

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

    /// Return stdout + stderr combined, preferring whichever is non-empty.
    /// Useful when commands use `2>&1` which merges stderr into stdout.
    pub fn combined_output(&self) -> String {
        let stdout = self.stdout.trim();
        let stderr = self.stderr.trim();
        if !stderr.is_empty() && !stdout.is_empty() {
            format!("{}\n{}", stdout, stderr)
        } else if !stdout.is_empty() {
            stdout.to_string()
        } else {
            stderr.to_string()
        }
    }
}

/// Path to the known_hosts file
fn known_hosts_path() -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join(".ssh").join("known_hosts"))
}

/// Check a server's public key against the known_hosts file.
/// Returns Ok(true) if the key is known or was added (TOFU), Err if there's a mismatch.
fn check_known_host(
    host: &str,
    port: u16,
    key: &russh_keys::ssh_key::PublicKey,
) -> std::result::Result<bool, String> {
    use base64::Engine;

    let key_algo = key.algorithm();
    let key_type = key_algo.as_str();
    // Use the ssh-key crate's Encode impl to serialize in wire format.
    // This handles all key types (Ed25519, RSA, ECDSA, etc.) generically.
    let key_data = key
        .to_bytes()
        .map_err(|e| format!("failed to encode host key: {}", e))?;

    let key_b64 = base64::engine::general_purpose::STANDARD.encode(&key_data);

    // Build the host pattern for known_hosts lookup.
    // Strip brackets from IPv6 host before formatting to avoid double-brackets.
    let bare_host = host
        .strip_prefix('[')
        .and_then(|h| h.strip_suffix(']'))
        .unwrap_or(host);
    let host_pattern = if port == 22 {
        host.to_string()
    } else {
        format!("[{}]:{}", bare_host, port)
    };

    let kh_path = match known_hosts_path() {
        Some(p) => p,
        None => return Ok(true), // No home dir — accept
    };

    // Read existing known_hosts (distinguish "not found" from "unreadable")
    let content = match std::fs::read_to_string(&kh_path) {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => {
            return Err(format!(
                "cannot read {}: {} — host key verification skipped, refusing connection",
                kh_path.display(),
                e
            ));
        }
    };

    let mut has_hashed_entries = false;
    let mut has_cert_authority = false;
    let mut found_host = false;
    let mut same_type_mismatch = false;

    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        // Detect hashed known_hosts entries (format: |1|<salt>|<hash>)
        if line.starts_with("|1|") {
            has_hashed_entries = true;
            continue;
        }

        // Handle marker-prefixed lines: @revoked, @cert-authority
        // Use split_whitespace to handle tabs and multiple spaces (valid in known_hosts)
        let mut words = line.split_whitespace().peekable();
        let first = match words.peek() {
            Some(w) => *w,
            None => continue,
        };
        let line_without_marker: Option<()> = if first == "@revoked" {
            words.next(); // consume marker
            let rev_parts: Vec<&str> = words.collect();
            if rev_parts.len() >= 3 {
                let rev_hosts = rev_parts[0];
                let rev_key_type = rev_parts[1];
                let rev_key_b64 = rev_parts[2];
                let matches_host = rev_hosts.split(',').any(|h| h.trim() == host_pattern);
                if matches_host && rev_key_type == key_type && rev_key_b64 == key_b64 {
                    return Err(format!(
                        "HOST KEY REJECTED for '{}': the server's {} key is explicitly \
                         revoked in {}. Refusing connection.",
                        host_pattern,
                        key_type,
                        kh_path.display()
                    ));
                }
            }
            continue;
        } else if first == "@cert-authority" {
            // We cannot verify certificate authority entries; track their presence
            // so we can fail closed if no plaintext match is found.
            has_cert_authority = true;
            continue;
        } else {
            // Not a marker line; re-split to get the three standard fields
            None
        };
        let _ = line_without_marker; // unused, just for control flow clarity

        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() < 3 {
            continue;
        }
        let hosts = parts[0];
        let line_key_type = parts[1];
        let line_key_b64 = parts[2];

        // Check if this line matches our host
        let matches_host = hosts.split(',').any(|h| h.trim() == host_pattern);
        if !matches_host {
            continue;
        }

        found_host = true;

        // Only compare keys of the same algorithm type.
        // A host may have entries for multiple key types (e.g. ssh-rsa AND ssh-ed25519).
        if line_key_type != key_type {
            continue;
        }

        // Same host and same key type — check if the key data matches
        if line_key_b64 == key_b64 {
            return Ok(true); // Known and matches
        }

        // Same host, same key type, but different key data — potential MITM
        same_type_mismatch = true;
    }

    if same_type_mismatch {
        return Err(format!(
            "HOST KEY VERIFICATION FAILED for '{}': the server's {} key has changed. \
             This could indicate a MITM attack. Remove the old key from {} to proceed.",
            host_pattern,
            key_type,
            kh_path.display()
        ));
    }

    if has_hashed_entries && !found_host {
        // Hashed entries may contain a pin for this host that we cannot verify.
        // Since no plaintext entry was found for this host, a hashed entry might
        // exist with a different key — fail closed rather than silently accepting.
        return Err(format!(
            "{} contains hashed entries which this tool cannot verify. \
             The host key for '{}' cannot be checked — refusing connection. \
             Add a plaintext entry or use `ssh-keygen -R` and reconnect to re-pin.",
            kh_path.display(),
            host_pattern
        ));
    }

    if has_cert_authority && !found_host {
        // @cert-authority entries may cover this host via wildcard or domain matching.
        // Since this tool cannot verify certificate-based trust, fail closed rather
        // than silently bypassing a CA trust policy with TOFU.
        return Err(format!(
            "{} contains @cert-authority entries which this tool cannot verify. \
             The host key for '{}' cannot be checked — refusing connection. \
             Add a plaintext host key entry or use ssh-keyscan to pin the key.",
            kh_path.display(),
            host_pattern
        ));
    }

    // Host not found — Trust On First Use: add it
    if let Some(parent) = kh_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    // Ensure we don't corrupt the last line if file lacks a trailing newline
    let prefix = if content.ends_with('\n') || content.is_empty() {
        ""
    } else {
        "\n"
    };
    let entry = format!("{}{} {} {}\n", prefix, host_pattern, key_type, key_b64);
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&kh_path);
    match file {
        Ok(ref mut f) => {
            use std::io::Write;
            if let Err(e) = f.write_all(entry.as_bytes()) {
                return Err(format!(
                    "could not write TOFU entry to {}: {} — refusing connection (host key would be unpinned)",
                    kh_path.display(),
                    e
                ));
            }
        }
        Err(e) => {
            return Err(format!(
                "could not open {} for writing: {} — refusing connection (host key would be unpinned)",
                kh_path.display(),
                e
            ));
        }
    }
    Ok(true)
}

/// SSH client handler for russh, with known_hosts verification.
struct SshHandler {
    /// Host string for known_hosts lookup (set before connecting)
    host: String,
    /// Port for known_hosts lookup
    port: u16,
}

#[async_trait]
impl client::Handler for SshHandler {
    type Error = russh::Error;

    async fn check_server_key(
        &mut self,
        server_public_key: &russh_keys::ssh_key::PublicKey,
    ) -> std::result::Result<bool, Self::Error> {
        match check_known_host(&self.host, self.port, server_public_key) {
            Ok(accepted) => Ok(accepted),
            Err(msg) => {
                // Log the error and reject the connection
                eprintln!("{}", msg);
                Ok(false)
            }
        }
    }
}

/// An active SSH session that can execute remote commands.
pub struct SshSession {
    handle: client::Handle<SshHandler>,
}

impl SshSession {
    /// Connect to a remote host and authenticate.
    ///
    /// Tries key file authentication first, then falls back to ssh-agent.
    pub async fn connect<A: ToSocketAddrs + ToString>(
        addr: A,
        user: &str,
        key_path: Option<&Path>,
    ) -> Result<Self> {
        let config = Arc::new(client::Config {
            inactivity_timeout: Some(std::time::Duration::from_secs(60)),
            keepalive_interval: Some(std::time::Duration::from_secs(15)),
            ..<_>::default()
        });

        // Parse host:port from the address for known_hosts verification
        let addr_str = addr.to_string();
        let (host, port) = parse_host_port(&addr_str);

        let handler = SshHandler {
            host: host.to_string(),
            port,
        };

        let mut handle = client::connect(config, addr, handler).await.map_err(|e| {
            let msg = format!("connection failed: {}", e);
            AppError::Ssh(crate::error::add_hint(&msg))
        })?;

        let authenticated = if let Some(key_path) = key_path {
            let key = load_secret_key(key_path, None)
                .map_err(|e| AppError::Ssh(format!("failed to load key {:?}: {}", key_path, e)))?;
            handle
                .authenticate_publickey(user, Arc::new(key))
                .await
                .map_err(|e| {
                    let msg = format!("key auth failed: {}", e);
                    AppError::Ssh(crate::error::add_hint(&msg))
                })?
        } else {
            Self::try_agent_auth(&mut handle, user).await?
        };

        if !authenticated {
            return Err(AppError::Ssh(crate::error::add_hint(
                "authentication failed",
            )));
        }

        Ok(Self { handle })
    }

    /// Try authenticating via ssh-agent.
    async fn try_agent_auth(handle: &mut client::Handle<SshHandler>, user: &str) -> Result<bool> {
        let mut agent = russh_keys::agent::client::AgentClient::connect_env()
            .await
            .map_err(|e| {
                let msg = format!("ssh-agent connect failed: {}", e);
                AppError::Ssh(crate::error::add_hint(&msg))
            })?;

        let identities = agent.request_identities().await.map_err(|e| {
            let msg = format!("ssh-agent list keys failed: {}", e);
            AppError::Ssh(crate::error::add_hint(&msg))
        })?;

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
                Some(ChannelMsg::ExtendedData { ref data, ext: 1 }) => {
                    stderr.extend_from_slice(data);
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
}

/// Parse host and port from an address string like "1.2.3.4:22" or "[::1]:22".
fn parse_host_port(addr: &str) -> (&str, u16) {
    // Handle bracketed IPv6: [::1]:22
    if let Some(bracket_end) = addr.find("]:") {
        let host = &addr[..bracket_end + 1]; // includes the closing bracket
        let port = addr[bracket_end + 2..].parse().unwrap_or(22);
        return (host, port);
    }
    // Non-bracketed: only split on colon if there is exactly one (avoids bare IPv6)
    if let Some(colon_idx) = addr.rfind(':') {
        if addr[..colon_idx].contains(':') {
            // Multiple colons means bare IPv6 address with no port
            return (addr, 22);
        }
        let host = &addr[..colon_idx];
        let port = addr[colon_idx + 1..].parse().unwrap_or(22);
        (host, port)
    } else {
        (addr, 22)
    }
}

/// Expand ~ to the user's home directory.
pub fn expand_tilde(path: &str) -> String {
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
}
