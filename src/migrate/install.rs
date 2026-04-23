//! Shared install primitives: apt, xray-install.sh, systemd helpers.

use crate::backend_trait::XrayBackend;
use crate::error::{AppError, Result};

pub async fn preflight(backend: &dyn XrayBackend, required_free_ports: &[u16]) -> Result<()> {
    let sudo = backend.exec_on_host("sudo -n true").await?;
    if !sudo.success() {
        return Err(AppError::Config("sudo requires password — configure NOPASSWD".into()));
    }
    let os = backend.exec_on_host("grep PRETTY_NAME /etc/os-release").await?;
    if !os.success() || !os.stdout.contains("Ubuntu 2") {
        return Err(AppError::Config(format!(
            "unsupported OS (need Ubuntu 22+/24+): {}", os.stdout.trim()
        )));
    }
    let mem = backend.exec_on_host("grep MemAvailable /proc/meminfo").await?;
    let mem_kb: u64 = mem.stdout.split_whitespace().nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    if mem_kb < 500_000 {
        return Err(AppError::Config(format!(
            "insufficient memory: {mem_kb} KB available, need ≥ 500_000"
        )));
    }
    for port in required_free_ports {
        let check = backend.exec_on_host(&format!("ss -tln | grep -E \":{port}\\b\" | head -1")).await?;
        if !check.stdout.trim().is_empty() {
            return Err(AppError::Config(format!("port {port} is already in use on new host")));
        }
    }
    Ok(())
}

pub async fn install_xray(backend: &dyn XrayBackend) -> Result<String> {
    let install_cmd = "sudo bash -c \"$(curl -Ls https://github.com/XTLS/Xray-install/raw/main/install-release.sh)\" @ install";
    let install = backend.exec_on_host(install_cmd).await?;
    if !install.success() {
        return Err(AppError::Config(format!("xray install failed: {}", install.stderr)));
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
    let major: u32 = token.split('.').next()
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| AppError::Config(format!("cannot parse major version from: {token}")))?;
    if major < 25 {
        return Err(AppError::Config(format!("xray version {token} is too old; need 25+ for XHTTP")));
    }
    Ok(token.to_string())
}

pub async fn apt_install(backend: &dyn XrayBackend, packages: &[&str]) -> Result<()> {
    let update = backend.exec_on_host(
        "sudo DEBIAN_FRONTEND=noninteractive apt-get update -qq"
    ).await?;
    if !update.success() {
        return Err(AppError::Config(format!(
            "apt-get update failed: {}", update.stderr
        )));
    }
    let install = backend.exec_on_host(&format!(
        "sudo DEBIAN_FRONTEND=noninteractive apt-get install -y -qq {}",
        packages.join(" ")
    )).await?;
    if !install.success() {
        return Err(AppError::Config(format!(
            "apt-get install failed: {}", install.stderr
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
        fn container_name(&self) -> &str { "mock" }
        fn hostname(&self) -> &str { "mock.example.com" }
    }

    #[tokio::test]
    async fn apt_install_calls_correct_commands() {
        let backend = MockBackend {
            calls: Mutex::new(vec![]),
            responses: Mutex::new(vec![
                CommandOutput { stdout: "ok".into(), stderr: "".into(), exit_code: 0 },
                CommandOutput { stdout: "ok".into(), stderr: "".into(), exit_code: 0 },
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
            responses: Mutex::new(vec![
                CommandOutput { stdout: "".into(), stderr: "E: broken".into(), exit_code: 100 },
            ]),
        };
        let result = apt_install(&backend, &["nginx"]).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn install_xray_runs_official_script() {
        let backend = MockBackend {
            calls: Mutex::new(vec![]),
            responses: Mutex::new(vec![
                CommandOutput { stdout: "Xray installed".into(), stderr: "".into(), exit_code: 0 },
                CommandOutput { stdout: "Xray 26.3.27 ...\nA unified platform...".into(), stderr: "".into(), exit_code: 0 },
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
                CommandOutput { stdout: "ok".into(), stderr: "".into(), exit_code: 0 },
                CommandOutput { stdout: "Xray 1.8.4 ...".into(), stderr: "".into(), exit_code: 0 },
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
                CommandOutput { stdout: "".into(), stderr: "".into(), exit_code: 0 },
                CommandOutput { stdout: "PRETTY_NAME=\"Ubuntu 24.04.4 LTS\"".into(), stderr: "".into(), exit_code: 0 },
                CommandOutput { stdout: "MemAvailable:    1500000 kB".into(), stderr: "".into(), exit_code: 0 },
                CommandOutput { stdout: "".into(), stderr: "".into(), exit_code: 1 },
            ]),
        };
        preflight(&backend, &[443]).await.unwrap();
    }

    #[tokio::test]
    async fn preflight_fails_on_busy_port() {
        let backend = MockBackend {
            calls: Mutex::new(vec![]),
            responses: Mutex::new(vec![
                CommandOutput { stdout: "".into(), stderr: "".into(), exit_code: 0 },
                CommandOutput { stdout: "PRETTY_NAME=\"Ubuntu 24.04.4 LTS\"".into(), stderr: "".into(), exit_code: 0 },
                CommandOutput { stdout: "MemAvailable:    1500000 kB".into(), stderr: "".into(), exit_code: 0 },
                CommandOutput { stdout: "LISTEN 0 128 *:443 *:* users:((something))".into(), stderr: "".into(), exit_code: 0 },
            ]),
        };
        let err = preflight(&backend, &[443]).await.unwrap_err();
        assert!(err.to_string().contains("443"));
    }
}
