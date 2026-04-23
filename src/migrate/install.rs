//! Shared install primitives: apt, xray-install.sh, systemd helpers.

use crate::backend_trait::XrayBackend;
use crate::error::{AppError, Result};
use crate::xray::client::NATIVE_CONFIG_PATH;
use base64::prelude::*;

pub async fn write_xray_config(backend: &dyn XrayBackend, content: &str) -> Result<()> {
    let encoded = BASE64_STANDARD.encode(content);
    let cmd = format!(
        "echo '{encoded}' | base64 -d | sudo tee {NATIVE_CONFIG_PATH} > /dev/null && sudo chmod 644 {NATIVE_CONFIG_PATH}"
    );
    let out = backend.exec_on_host(&cmd).await?;
    if !out.success() {
        return Err(AppError::Config(format!(
            "write config failed: {}",
            out.stderr
        )));
    }
    Ok(())
}

pub async fn restart_xray(backend: &dyn XrayBackend) -> Result<()> {
    let out = backend.exec_on_host("sudo systemctl restart xray").await?;
    if !out.success() {
        return Err(AppError::Config(format!(
            "systemctl restart xray: {}",
            out.stderr
        )));
    }
    tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
    let status = backend
        .exec_on_host("sudo systemctl is-active xray")
        .await?;
    if !status.stdout.trim().eq("active") {
        return Err(AppError::Config(format!(
            "xray not active after restart: {}",
            status.stdout
        )));
    }
    Ok(())
}

#[derive(Debug, Clone)]
pub struct Secrets {
    pub reality_private: String,
    pub reality_public: String,
    pub short_id: String,
    pub path: String,
}

pub async fn generate_secrets(backend: &dyn XrayBackend) -> Result<Secrets> {
    let keys = backend.exec_on_host("xray x25519").await?;
    if !keys.success() {
        return Err(AppError::Config(format!(
            "xray x25519 failed: {}",
            keys.stderr
        )));
    }
    let mut priv_key = String::new();
    let mut pub_key = String::new();
    for line in keys.stdout.lines() {
        if let Some(rest) = line.strip_prefix("PrivateKey:") {
            priv_key = rest.trim().to_string();
        }
        if line.starts_with("Password") {
            if let Some(idx) = line.find(':') {
                pub_key = line[idx + 1..].trim().to_string();
            }
        }
    }
    if priv_key.is_empty() || pub_key.is_empty() {
        return Err(AppError::Config(format!(
            "cannot parse x25519 output: {}",
            keys.stdout
        )));
    }
    let sid = backend.exec_on_host("openssl rand -hex 8").await?;
    let path = backend.exec_on_host("openssl rand -hex 6").await?;
    Ok(Secrets {
        reality_private: priv_key,
        reality_public: pub_key,
        short_id: sid.stdout.trim().to_string(),
        path: format!("/{}", path.stdout.trim()),
    })
}

pub async fn preflight(backend: &dyn XrayBackend, required_free_ports: &[u16]) -> Result<()> {
    let sudo = backend.exec_on_host("sudo -n true").await?;
    if !sudo.success() {
        return Err(AppError::Config(
            "sudo requires password — configure NOPASSWD".into(),
        ));
    }
    let os = backend
        .exec_on_host("grep PRETTY_NAME /etc/os-release")
        .await?;
    if !os.success() || !os.stdout.contains("Ubuntu 2") {
        return Err(AppError::Config(format!(
            "unsupported OS (need Ubuntu 22+/24+): {}",
            os.stdout.trim()
        )));
    }
    let mem = backend
        .exec_on_host("grep MemAvailable /proc/meminfo")
        .await?;
    let mem_kb: u64 = mem
        .stdout
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    if mem_kb < 500_000 {
        return Err(AppError::Config(format!(
            "insufficient memory: {mem_kb} KB available, need ≥ 500_000"
        )));
    }
    for port in required_free_ports {
        let check = backend
            .exec_on_host(&format!("ss -tln | grep -E \":{port}\\b\" | head -1"))
            .await?;
        if !check.stdout.trim().is_empty() {
            return Err(AppError::Config(format!(
                "port {port} is already in use on new host"
            )));
        }
    }
    Ok(())
}

pub async fn install_xray(backend: &dyn XrayBackend) -> Result<String> {
    let install_cmd = "sudo bash -c \"$(curl -Ls https://github.com/XTLS/Xray-install/raw/main/install-release.sh)\" @ install";
    let install = backend.exec_on_host(install_cmd).await?;
    if !install.success() {
        return Err(AppError::Config(format!(
            "xray install failed: {}",
            install.stderr
        )));
    }

    let version = backend.exec_on_host("xray version 2>&1 | head -1").await?;
    if !version.success() {
        return Err(AppError::Config("xray version check failed".into()));
    }

    let stdout = version.stdout.trim();
    let token = stdout
        .split_whitespace()
        .nth(1)
        .ok_or_else(|| AppError::Config(format!("cannot parse xray version from: {stdout}")))?;
    let major: u32 = token
        .split('.')
        .next()
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| AppError::Config(format!("cannot parse major version from: {token}")))?;
    if major < 25 {
        return Err(AppError::Config(format!(
            "xray version {token} is too old; need 25+ for XHTTP"
        )));
    }
    Ok(token.to_string())
}

pub async fn apt_install(backend: &dyn XrayBackend, packages: &[&str]) -> Result<()> {
    let update = backend
        .exec_on_host("sudo DEBIAN_FRONTEND=noninteractive apt-get update -qq")
        .await?;
    if !update.success() {
        return Err(AppError::Config(format!(
            "apt-get update failed: {}",
            update.stderr
        )));
    }
    let install = backend
        .exec_on_host(&format!(
            "sudo DEBIAN_FRONTEND=noninteractive apt-get install -y -qq {}",
            packages.join(" ")
        ))
        .await?;
    if !install.success() {
        return Err(AppError::Config(format!(
            "apt-get install failed: {}",
            install.stderr
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ssh::CommandOutput;
    use async_trait::async_trait;
    use std::sync::Mutex;

    pub(super) struct MockBackend {
        pub calls: Mutex<Vec<String>>,
        pub responses: Mutex<Vec<CommandOutput>>,
    }

    #[async_trait]
    impl XrayBackend for MockBackend {
        async fn exec_in_container(&self, cmd: &str) -> Result<CommandOutput> {
            self.exec_on_host(cmd).await
        }
        async fn exec_on_host(&self, cmd: &str) -> Result<CommandOutput> {
            self.calls.lock().unwrap().push(cmd.to_string());
            Ok(self.responses.lock().unwrap().remove(0))
        }
        fn container_name(&self) -> &str {
            "mock"
        }
        fn hostname(&self) -> &str {
            "mock.example.com"
        }
    }

    #[tokio::test]
    async fn apt_install_calls_correct_commands() {
        let backend = MockBackend {
            calls: Mutex::new(vec![]),
            responses: Mutex::new(vec![
                CommandOutput {
                    stdout: "ok".into(),
                    stderr: "".into(),
                    exit_code: 0,
                },
                CommandOutput {
                    stdout: "ok".into(),
                    stderr: "".into(),
                    exit_code: 0,
                },
            ]),
        };
        apt_install(&backend, &["nginx", "certbot"]).await.unwrap();
        let calls = backend.calls.lock().unwrap();
        assert!(calls[0].contains("apt-get update"));
        assert!(calls[1].contains("apt-get install"));
        assert!(calls[1].contains("nginx"));
        assert!(calls[1].contains("certbot"));
    }

    #[tokio::test]
    async fn apt_install_fails_on_nonzero_exit() {
        let backend = MockBackend {
            calls: Mutex::new(vec![]),
            responses: Mutex::new(vec![CommandOutput {
                stdout: "".into(),
                stderr: "E: broken".into(),
                exit_code: 100,
            }]),
        };
        let result = apt_install(&backend, &["nginx"]).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn install_xray_runs_official_script() {
        let backend = MockBackend {
            calls: Mutex::new(vec![]),
            responses: Mutex::new(vec![
                CommandOutput {
                    stdout: "Xray installed".into(),
                    stderr: "".into(),
                    exit_code: 0,
                },
                CommandOutput {
                    stdout: "Xray 26.3.27 ...\nA unified platform...".into(),
                    stderr: "".into(),
                    exit_code: 0,
                },
            ]),
        };
        let version = install_xray(&backend).await.unwrap();
        assert!(version.starts_with("26."));
        let calls = backend.calls.lock().unwrap();
        assert!(calls[0].contains("Xray-install"));
        assert!(calls[1].contains("xray version"));
    }

    #[tokio::test]
    async fn install_xray_rejects_old_version() {
        let backend = MockBackend {
            calls: Mutex::new(vec![]),
            responses: Mutex::new(vec![
                CommandOutput {
                    stdout: "ok".into(),
                    stderr: "".into(),
                    exit_code: 0,
                },
                CommandOutput {
                    stdout: "Xray 1.8.4 ...".into(),
                    stderr: "".into(),
                    exit_code: 0,
                },
            ]),
        };
        let err = install_xray(&backend).await.unwrap_err();
        assert!(err.to_string().contains("too old") || err.to_string().contains("1.8"));
    }

    #[tokio::test]
    async fn preflight_passes_on_healthy_host() {
        let backend = MockBackend {
            calls: Mutex::new(vec![]),
            responses: Mutex::new(vec![
                CommandOutput {
                    stdout: "".into(),
                    stderr: "".into(),
                    exit_code: 0,
                },
                CommandOutput {
                    stdout: "PRETTY_NAME=\"Ubuntu 24.04.4 LTS\"".into(),
                    stderr: "".into(),
                    exit_code: 0,
                },
                CommandOutput {
                    stdout: "MemAvailable:    1500000 kB".into(),
                    stderr: "".into(),
                    exit_code: 0,
                },
                CommandOutput {
                    stdout: "".into(),
                    stderr: "".into(),
                    exit_code: 1,
                },
            ]),
        };
        preflight(&backend, &[443]).await.unwrap();
    }

    #[tokio::test]
    async fn preflight_fails_on_busy_port() {
        let backend = MockBackend {
            calls: Mutex::new(vec![]),
            responses: Mutex::new(vec![
                CommandOutput {
                    stdout: "".into(),
                    stderr: "".into(),
                    exit_code: 0,
                },
                CommandOutput {
                    stdout: "PRETTY_NAME=\"Ubuntu 24.04.4 LTS\"".into(),
                    stderr: "".into(),
                    exit_code: 0,
                },
                CommandOutput {
                    stdout: "MemAvailable:    1500000 kB".into(),
                    stderr: "".into(),
                    exit_code: 0,
                },
                CommandOutput {
                    stdout: "LISTEN 0 128 *:443 *:* users:((something))".into(),
                    stderr: "".into(),
                    exit_code: 0,
                },
            ]),
        };
        let err = preflight(&backend, &[443]).await.unwrap_err();
        assert!(err.to_string().contains("443"));
    }

    #[tokio::test]
    async fn write_xray_config_uses_base64() {
        let backend = MockBackend {
            calls: Mutex::new(vec![]),
            responses: Mutex::new(vec![CommandOutput {
                stdout: "".into(),
                stderr: "".into(),
                exit_code: 0,
            }]),
        };
        write_xray_config(&backend, "{\"a\":1}").await.unwrap();
        let calls = backend.calls.lock().unwrap();
        assert!(calls[0].contains("base64 -d"));
        assert!(calls[0].contains("/usr/local/etc/xray/config.json"));
    }

    #[tokio::test]
    async fn generate_secrets_parses_output() {
        let backend = MockBackend {
            calls: Mutex::new(vec![]),
            responses: Mutex::new(vec![
                CommandOutput {
                    stdout: "PrivateKey: ABC_PRIV\nPassword (PublicKey): ABC_PUB\nHash32: HASH"
                        .into(),
                    stderr: "".into(),
                    exit_code: 0,
                },
                CommandOutput {
                    stdout: "833552e201595cd4".into(),
                    stderr: "".into(),
                    exit_code: 0,
                },
                CommandOutput {
                    stdout: "0e1fa74ddc24".into(),
                    stderr: "".into(),
                    exit_code: 0,
                },
            ]),
        };
        let s = generate_secrets(&backend).await.unwrap();
        assert_eq!(s.reality_private, "ABC_PRIV");
        assert_eq!(s.reality_public, "ABC_PUB");
        assert_eq!(s.short_id, "833552e201595cd4");
        assert_eq!(s.path, "/0e1fa74ddc24");
    }
}
