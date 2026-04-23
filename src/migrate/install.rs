//! Shared install primitives: apt, xray-install.sh, systemd helpers.

use crate::backend_trait::XrayBackend;
use crate::error::{AppError, Result};

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
}
