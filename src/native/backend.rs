//! `NativeBackend`: XrayBackend impl for native-systemd xray (no Docker).
//!
//! Unlike `SshBackend`/`LocalBackend` which wrap commands in `docker exec
//! <container> sh -c ...`, this backend runs commands directly on the host
//! (either over SSH or as local subprocess). Used by the new bridge
//! (yc-vm) and egress (vps-vpn:8444) where xray is a native systemd service.

use crate::backend_trait::XrayBackend;
use crate::error::{AppError, Result};
use crate::ssh::{CommandOutput, SshSession};
use async_trait::async_trait;

/// SSH-based `NativeBackend` — runs commands on a remote host without Docker wrapping.
pub struct NativeSshBackend {
    session: SshSession,
    hostname: String,
}

impl NativeSshBackend {
    pub fn new(session: SshSession, hostname: String) -> Self {
        Self { session, hostname }
    }
}

#[async_trait]
impl XrayBackend for NativeSshBackend {
    async fn exec_in_container(&self, cmd: &str) -> Result<CommandOutput> {
        // No container — pass straight to host.
        self.session.exec_command(cmd).await
    }
    async fn exec_on_host(&self, cmd: &str) -> Result<CommandOutput> {
        self.session.exec_command(cmd).await
    }
    fn container_name(&self) -> &str {
        ""
    }
    fn hostname(&self) -> &str {
        &self.hostname
    }
}

/// Local `NativeBackend` — bot running on the same host as xray.
pub struct NativeLocalBackend {
    hostname: String,
}

impl NativeLocalBackend {
    pub fn new(hostname: String) -> Self {
        Self { hostname }
    }

    async fn run_shell(&self, cmd: &str) -> Result<CommandOutput> {
        let output = tokio::process::Command::new("sh")
            .arg("-c")
            .arg(cmd)
            .output()
            .await
            .map_err(AppError::Io)?;
        Ok(CommandOutput {
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
            exit_code: output.status.code().unwrap_or(-1) as u32,
        })
    }
}

#[async_trait]
impl XrayBackend for NativeLocalBackend {
    async fn exec_in_container(&self, cmd: &str) -> Result<CommandOutput> {
        self.run_shell(cmd).await
    }
    async fn exec_on_host(&self, cmd: &str) -> Result<CommandOutput> {
        self.run_shell(cmd).await
    }
    fn container_name(&self) -> &str {
        ""
    }
    fn hostname(&self) -> &str {
        &self.hostname
    }
}
