//! Backend abstraction for xray server operations.
//!
//! Defines the `XrayBackend` trait that enables the same xray management code
//! to work over SSH (remote) or local docker exec (on-VPS Telegram bot).

use async_trait::async_trait;

use crate::error::{AppError, Result};
use crate::ssh::{CommandOutput, SshSession};

/// Trait abstracting command execution against an xray server.
///
/// Implementations handle the transport layer (SSH or local shell),
/// while xray-specific logic in `XrayApiClient` and `xray::config`
/// works against this trait.
#[async_trait]
pub trait XrayBackend: Send + Sync {
    /// Execute a command inside the xray Docker container.
    async fn exec_in_container(&self, cmd: &str) -> Result<CommandOutput>;

    /// Execute a command on the host (outside the container).
    async fn exec_on_host(&self, cmd: &str) -> Result<CommandOutput>;

    /// The Docker container name for xray.
    fn container_name(&self) -> &str;

    /// The server's hostname or IP (used for vless:// URL generation).
    fn hostname(&self) -> &str;
}

/// SSH-based backend for remote xray management.
pub struct SshBackend {
    session: SshSession,
    hostname: String,
}

impl SshBackend {
    pub fn new(session: SshSession, hostname: String) -> Self {
        Self { session, hostname }
    }

    /// Close the underlying SSH session.
    pub async fn close(self) -> Result<()> {
        self.session.close().await
    }
}

#[async_trait]
impl XrayBackend for SshBackend {
    async fn exec_in_container(&self, cmd: &str) -> Result<CommandOutput> {
        self.session.exec_in_container(cmd).await
    }

    async fn exec_on_host(&self, cmd: &str) -> Result<CommandOutput> {
        self.session.exec_command(cmd).await
    }

    fn container_name(&self) -> &str {
        self.session.container_name()
    }

    fn hostname(&self) -> &str {
        &self.hostname
    }
}

/// Local backend for on-VPS usage (runs docker exec directly without SSH).
pub struct LocalBackend {
    container: String,
    hostname: String,
}

impl LocalBackend {
    pub fn new(container: String, hostname: String) -> Self {
        Self {
            container,
            hostname,
        }
    }

    /// Build the command arguments for `docker exec`.
    fn docker_exec_args(&self, cmd: &str) -> Vec<String> {
        let args = vec![
            "exec".to_string(),
            self.container.clone(),
            "sh".to_string(),
            "-c".to_string(),
            cmd.to_string(),
        ];
        args
    }

    /// Run a shell command locally via `tokio::process::Command` and collect output.
    async fn run_local(program: &str, args: &[String]) -> Result<CommandOutput> {
        let output = tokio::process::Command::new(program)
            .args(args)
            .output()
            .await
            .map_err(AppError::Io)?;

        let exit_code = output.status.code().unwrap_or(1) as u32;
        Ok(CommandOutput {
            stdout: String::from_utf8_lossy(&output.stdout).to_string(),
            stderr: String::from_utf8_lossy(&output.stderr).to_string(),
            exit_code,
        })
    }
}

#[async_trait]
impl XrayBackend for LocalBackend {
    async fn exec_in_container(&self, cmd: &str) -> Result<CommandOutput> {
        // Defense-in-depth: verify container name is shell-safe
        if !self
            .container
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-'))
        {
            return Err(AppError::Xray(format!(
                "unsafe container name: {}",
                self.container
            )));
        }
        let args = self.docker_exec_args(cmd);
        Self::run_local("docker", &args).await
    }

    async fn exec_on_host(&self, cmd: &str) -> Result<CommandOutput> {
        Self::run_local("sh", &["-c".to_string(), cmd.to_string()]).await
    }

    fn container_name(&self) -> &str {
        &self.container
    }

    fn hostname(&self) -> &str {
        &self.hostname
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// Mock backend for testing xray operations without SSH.
    pub struct MockBackend {
        container: String,
        host: String,
        responses: Mutex<Vec<CommandOutput>>,
    }

    impl MockBackend {
        pub fn new(container: &str, hostname: &str) -> Self {
            Self {
                container: container.to_string(),
                host: hostname.to_string(),
                responses: Mutex::new(Vec::new()),
            }
        }

        /// Queue a response that will be returned by the next exec call.
        pub fn queue_response(&self, output: CommandOutput) {
            self.responses.lock().unwrap().push(output);
        }
    }

    #[async_trait]
    impl XrayBackend for MockBackend {
        async fn exec_in_container(&self, _cmd: &str) -> Result<CommandOutput> {
            let mut responses = self.responses.lock().unwrap();
            if responses.is_empty() {
                Ok(CommandOutput {
                    stdout: String::new(),
                    stderr: String::new(),
                    exit_code: 0,
                })
            } else {
                Ok(responses.remove(0))
            }
        }

        async fn exec_on_host(&self, _cmd: &str) -> Result<CommandOutput> {
            let mut responses = self.responses.lock().unwrap();
            if responses.is_empty() {
                Ok(CommandOutput {
                    stdout: String::new(),
                    stderr: String::new(),
                    exit_code: 0,
                })
            } else {
                Ok(responses.remove(0))
            }
        }

        fn container_name(&self) -> &str {
            &self.container
        }

        fn hostname(&self) -> &str {
            &self.host
        }
    }

    #[test]
    fn test_mock_backend_accessors() {
        let backend = MockBackend::new("test-container", "1.2.3.4");
        assert_eq!(backend.container_name(), "test-container");
        assert_eq!(backend.hostname(), "1.2.3.4");
    }

    #[tokio::test]
    async fn test_mock_backend_queued_response() {
        let backend = MockBackend::new("test", "host");
        backend.queue_response(CommandOutput {
            stdout: "hello".to_string(),
            stderr: String::new(),
            exit_code: 0,
        });
        let result = backend.exec_in_container("echo hello").await.unwrap();
        assert_eq!(result.stdout, "hello");
        assert!(result.success());
    }

    #[tokio::test]
    async fn test_mock_backend_default_empty_response() {
        let backend = MockBackend::new("test", "host");
        let result = backend.exec_on_host("test").await.unwrap();
        assert_eq!(result.stdout, "");
        assert!(result.success());
    }

    #[tokio::test]
    async fn test_mock_backend_multiple_responses() {
        let backend = MockBackend::new("test", "host");
        backend.queue_response(CommandOutput {
            stdout: "first".to_string(),
            stderr: String::new(),
            exit_code: 0,
        });
        backend.queue_response(CommandOutput {
            stdout: "second".to_string(),
            stderr: String::new(),
            exit_code: 0,
        });

        let r1 = backend.exec_in_container("cmd1").await.unwrap();
        let r2 = backend.exec_in_container("cmd2").await.unwrap();
        assert_eq!(r1.stdout, "first");
        assert_eq!(r2.stdout, "second");
    }

    // --- LocalBackend tests ---

    #[test]
    fn test_local_backend_accessors() {
        let backend = LocalBackend::new("amnezia-xray".to_string(), "10.0.0.1".to_string());
        assert_eq!(backend.container_name(), "amnezia-xray");
        assert_eq!(backend.hostname(), "10.0.0.1");
    }

    #[test]
    fn test_local_backend_docker_exec_args() {
        let backend = LocalBackend::new("amnezia-xray".to_string(), "10.0.0.1".to_string());
        let args = backend.docker_exec_args("xray api statsquery");
        assert_eq!(
            args,
            vec!["exec", "amnezia-xray", "sh", "-c", "xray api statsquery"]
        );
    }

    #[test]
    fn test_local_backend_docker_exec_args_complex_command() {
        let backend = LocalBackend::new("my-container".to_string(), "host".to_string());
        let args = backend.docker_exec_args("cat /opt/amnezia/xray/server.json");
        assert_eq!(
            args,
            vec![
                "exec",
                "my-container",
                "sh",
                "-c",
                "cat /opt/amnezia/xray/server.json"
            ]
        );
    }

    #[tokio::test]
    async fn test_local_backend_exec_on_host_echo() {
        let backend = LocalBackend::new("test".to_string(), "localhost".to_string());
        let result = backend.exec_on_host("echo hello").await.unwrap();
        assert!(result.success());
        assert_eq!(result.stdout.trim(), "hello");
    }

    #[tokio::test]
    async fn test_local_backend_exec_on_host_nonzero_exit() {
        let backend = LocalBackend::new("test".to_string(), "localhost".to_string());
        let result = backend.exec_on_host("false").await.unwrap();
        assert!(!result.success());
    }

    #[tokio::test]
    async fn test_local_backend_unsafe_container_name() {
        let backend = LocalBackend::new("bad;name".to_string(), "localhost".to_string());
        let result = backend.exec_in_container("echo test").await;
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("unsafe container name"));
    }

    #[test]
    fn test_local_backend_as_dyn_trait() {
        let backend = LocalBackend::new("ctr".to_string(), "1.2.3.4".to_string());
        let dyn_ref: &dyn XrayBackend = &backend;
        assert_eq!(dyn_ref.container_name(), "ctr");
        assert_eq!(dyn_ref.hostname(), "1.2.3.4");
    }
}
