//! Backend abstraction for xray server operations.
//!
//! Defines the `XrayBackend` trait that enables the same xray management code
//! to work over SSH (remote) or local docker exec (on-VPS Telegram bot).

use async_trait::async_trait;

use crate::error::Result;
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
}
