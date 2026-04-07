//! Reusable snapshot/restore/upgrade logic for Xray server.
//!
//! Snapshots are stored on the HOST filesystem (not inside the container)
//! at `/data/projects/xray-backup/<tag>/`. Each snapshot contains:
//! - server.json, clientsTable, key files
//! - xray binary copy
//! - xray_version.txt with the version string

use base64::Engine;

use crate::backend_trait::XrayBackend;
use crate::error::{AppError, Result};

/// Host directory where snapshots are stored (outside the container).
const SNAPSHOT_HOST_DIR: &str = "/data/projects/xray-backup";
/// Config directory inside the Xray container.
const XRAY_CONFIG_DIR: &str = "/opt/amnezia/xray";

/// Information about a snapshot.
pub struct SnapshotInfo {
    pub tag: String,
    pub version: String,
    pub users_count: usize,
}

/// Result of an Xray upgrade operation.
pub struct UpgradeResult {
    pub old_version: String,
    pub new_version: String,
    pub snapshot_tag: String,
}

/// Create a snapshot on the HOST filesystem using docker cp + exec_on_host.
///
/// Steps:
/// 1. Generate tag from current timestamp
/// 2. Create snapshot directory on host
/// 3. Copy config files from container to host via `docker cp`
/// 4. Save xray version info
///
/// Returns a `SnapshotInfo` with the tag, xray version, and user count.
pub async fn create_snapshot(backend: &dyn XrayBackend) -> Result<SnapshotInfo> {
    let container = backend.container_name();

    // Generate tag from current timestamp (on host)
    let tag_result = backend.exec_on_host("date +%Y%m%d-%H%M%S").await?;
    let tag = tag_result.stdout.trim().to_string();
    if tag.is_empty() {
        return Err(AppError::Xray("failed to generate snapshot tag".to_string()));
    }

    let snapshot_path = format!("{}/{}", SNAPSHOT_HOST_DIR, tag);

    // Create snapshot directory on host
    backend
        .exec_on_host(&format!("mkdir -p {}", snapshot_path))
        .await?;

    // Copy config files from container to host via docker cp
    let files = [
        "server.json",
        "clientsTable",
        "xray_private.key",
        "xray_public.key",
        "xray_short_id.key",
        "xray_uuid.key",
    ];
    for file in &files {
        let src = format!("{}:{}/{}", container, XRAY_CONFIG_DIR, file);
        let dst = format!("{}/{}", snapshot_path, file);
        // Use 2>/dev/null for optional files (keys may not exist)
        backend
            .exec_on_host(&format!("docker cp {} {} 2>/dev/null; true", src, dst))
            .await?;
    }

    // Copy xray binary
    let bin_src = format!("{}:/usr/bin/xray", container);
    let bin_dst = format!("{}/xray", snapshot_path);
    let result = backend
        .exec_on_host(&format!("docker cp {} {} && echo OK", bin_src, bin_dst))
        .await?;
    if !result.stdout.contains("OK") {
        return Err(AppError::Xray(format!(
            "failed to copy xray binary: {}",
            result.stderr.trim()
        )));
    }

    // Save xray version
    let ver_result = backend
        .exec_in_container("sh -c 'xray version 2>/dev/null | head -1'")
        .await
        .ok()
        .map(|r| r.stdout.trim().to_string())
        .unwrap_or_else(|| "unknown".to_string());
    backend
        .exec_on_host(&format!(
            "echo '{}' > {}/xray_version.txt",
            ver_result, snapshot_path
        ))
        .await?;

    // Extract version (e.g. "Xray 1.8.4 (Xray, Penetrates Everything.)" -> "1.8.4")
    let version = ver_result
        .split_whitespace()
        .nth(1)
        .unwrap_or("unknown")
        .to_string();

    // Count users from clientsTable
    let users_count = backend
        .exec_on_host(&format!(
            "grep -c 'clientName' {}/clientsTable 2>/dev/null || echo 0",
            snapshot_path
        ))
        .await
        .ok()
        .and_then(|r| r.stdout.trim().parse::<usize>().ok())
        .unwrap_or(0);

    Ok(SnapshotInfo {
        tag,
        version,
        users_count,
    })
}

/// List snapshots from the host directory.
///
/// Returns a list of `SnapshotInfo` sorted by tag (oldest first).
pub async fn list_snapshots(backend: &dyn XrayBackend) -> Result<Vec<SnapshotInfo>> {
    let cmd = format!(
        "ls -1d {}/*/  2>/dev/null | while read d; do \
           tag=$(basename \"$d\"); \
           ver=$(cat \"$d/xray_version.txt\" 2>/dev/null | head -1 || echo unknown); \
           users=$(grep -c clientName \"$d/clientsTable\" 2>/dev/null || echo 0); \
           echo \"$tag|$ver|$users\"; \
         done",
        SNAPSHOT_HOST_DIR
    );
    let result = backend.exec_on_host(&cmd).await?;
    let snapshots: Vec<SnapshotInfo> = result
        .stdout
        .lines()
        .filter(|l| l.contains('|'))
        .map(|l| {
            let parts: Vec<&str> = l.splitn(3, '|').collect();
            let tag = parts.first().unwrap_or(&"").trim().to_string();
            let ver_raw = parts.get(1).unwrap_or(&"unknown").trim().to_string();
            // Extract version number from "Xray 1.8.4 ..." or keep as-is
            let version = ver_raw
                .split_whitespace()
                .nth(1)
                .unwrap_or(&ver_raw)
                .to_string();
            let users_count = parts
                .get(2)
                .unwrap_or(&"0")
                .trim()
                .parse::<usize>()
                .unwrap_or(0);
            SnapshotInfo {
                tag,
                version,
                users_count,
            }
        })
        .collect();
    Ok(snapshots)
}

/// Restore a snapshot from the host back into the container.
///
/// Copies files from host to container via `docker cp`, then restarts the container.
pub async fn restore_snapshot(backend: &dyn XrayBackend, tag: &str) -> Result<()> {
    let container = backend.container_name();
    let snapshot_path = format!("{}/{}", SNAPSHOT_HOST_DIR, tag);

    // Verify snapshot exists
    let check = backend
        .exec_on_host(&format!("test -d {} && echo OK", snapshot_path))
        .await?;
    if !check.stdout.contains("OK") {
        return Err(AppError::Xray(format!("snapshot '{}' not found", tag)));
    }

    // Copy config files from host to container
    let files = [
        ("server.json", XRAY_CONFIG_DIR),
        ("clientsTable", XRAY_CONFIG_DIR),
        ("xray_private.key", XRAY_CONFIG_DIR),
        ("xray_public.key", XRAY_CONFIG_DIR),
        ("xray_short_id.key", XRAY_CONFIG_DIR),
        ("xray_uuid.key", XRAY_CONFIG_DIR),
    ];
    for (file, dest) in &files {
        let src = format!("{}/{}", snapshot_path, file);
        let dst = format!("{}:{}/{}", container, dest, file);
        backend
            .exec_on_host(&format!("docker cp {} {} 2>/dev/null; true", src, dst))
            .await?;
    }

    // Copy xray binary
    let bin_src = format!("{}/xray", snapshot_path);
    let bin_dst = format!("{}:/usr/bin/xray", container);
    let result = backend
        .exec_on_host(&format!(
            "docker cp {} {} && docker exec {} chmod +x /usr/bin/xray && echo OK",
            bin_src, bin_dst, container
        ))
        .await?;
    if !result.stdout.contains("OK") {
        return Err(AppError::Xray(format!(
            "failed to restore xray binary: {}",
            result.stderr.trim()
        )));
    }

    // Restart container
    backend
        .exec_on_host(&format!("docker restart {} 2>&1", container))
        .await?;

    Ok(())
}

/// Get the latest Xray release version from GitHub API.
pub async fn get_latest_xray_version(backend: &dyn XrayBackend) -> Result<String> {
    let result = backend
        .exec_on_host("curl -sf --max-time 10 https://api.github.com/repos/XTLS/Xray-core/releases/latest | grep tag_name | cut -d'\"' -f4 | tr -d 'v'")
        .await
        .map_err(|e| AppError::Xray(format!("failed to check latest version: {}", e)))?;

    let version = result.stdout.trim().to_string();
    if version.is_empty() {
        return Err(AppError::Xray(
            "failed to fetch latest version from GitHub".to_string(),
        ));
    }

    Ok(version)
}

/// Upgrade Xray to the latest version.
///
/// Full cycle:
/// 1. Check current vs latest version
/// 2. Create snapshot (pre-upgrade backup)
/// 3. Download new binary
/// 4. Replace binary in container
/// 5. Restart and verify
///
/// Returns an `UpgradeResult` with old/new versions and the snapshot tag.
pub async fn upgrade_xray(backend: &dyn XrayBackend) -> Result<UpgradeResult> {
    let container = backend.container_name();

    // Get current version
    let ver_result = backend
        .exec_in_container("sh -c 'xray version 2>&1 | head -1'")
        .await?;
    let old_version = ver_result
        .stdout
        .split_whitespace()
        .nth(1)
        .unwrap_or("unknown")
        .to_string();

    // Get latest version
    let latest = get_latest_xray_version(backend).await?;

    if latest == old_version {
        return Err(AppError::Xray(format!(
            "already on latest version v{}",
            latest
        )));
    }

    // Create snapshot before upgrade
    let snapshot = create_snapshot(backend).await?;

    // Download new binary on host
    let download_cmd = format!(
        "curl -sL https://github.com/XTLS/Xray-core/releases/download/v{}/Xray-linux-64.zip > /tmp/xray-upgrade.zip && \
         python3 -c \"import zipfile; z=zipfile.ZipFile('/tmp/xray-upgrade.zip'); z.extract('xray','/tmp'); z.close()\" && \
         chmod +x /tmp/xray && \
         echo OK",
        latest
    );
    let result = backend.exec_on_host(&download_cmd).await?;
    if !result.stdout.contains("OK") {
        return Err(AppError::Xray(format!(
            "download failed: {}",
            result.combined_output()
        )));
    }

    // Replace binary in container
    let replace_cmd = format!(
        "docker cp /tmp/xray {}:/usr/bin/xray && \
         docker exec {} chmod +x /usr/bin/xray && \
         echo OK",
        container, container
    );
    let result = backend.exec_on_host(&replace_cmd).await?;
    if !result.stdout.contains("OK") {
        return Err(AppError::Xray(format!(
            "binary replacement failed: {}",
            result.combined_output()
        )));
    }

    // Restart and wait
    backend
        .exec_on_host(&format!("docker restart {} 2>&1", container))
        .await?;
    backend.exec_on_host("sleep 2").await?;

    // Verify new version
    let new_ver_result = backend
        .exec_in_container("sh -c 'xray version 2>&1 | head -1'")
        .await
        .ok()
        .map(|r| r.stdout.trim().to_string())
        .unwrap_or_else(|| "unknown".to_string());
    let new_version = new_ver_result
        .split_whitespace()
        .nth(1)
        .unwrap_or("unknown")
        .to_string();

    Ok(UpgradeResult {
        old_version,
        new_version,
        snapshot_tag: snapshot.tag,
    })
}

/// Pack a snapshot into a tar.gz archive (returned as raw bytes).
///
/// Uses `tar czf` on the host, then reads it back via base64 encoding.
pub async fn pack_snapshot_zip(backend: &dyn XrayBackend, tag: &str) -> Result<Vec<u8>> {
    let snapshot_path = format!("{}/{}", SNAPSHOT_HOST_DIR, tag);
    let tmp_archive = format!("/tmp/snapshot-{}.tar.gz", tag);

    // Verify snapshot exists
    let check = backend
        .exec_on_host(&format!("test -d {} && echo OK", snapshot_path))
        .await?;
    if !check.stdout.contains("OK") {
        return Err(AppError::Xray(format!("snapshot '{}' not found", tag)));
    }

    // Create tar.gz and encode as base64
    let cmd = format!(
        "cd {} && tar czf {} . && base64 {} && rm -f {}",
        snapshot_path, tmp_archive, tmp_archive, tmp_archive
    );
    let result = backend.exec_on_host(&cmd).await?;
    if result.stdout.trim().is_empty() {
        return Err(AppError::Xray(format!(
            "failed to pack snapshot: {}",
            result.stderr.trim()
        )));
    }

    // Decode base64 to bytes
    let b64_clean: String = result.stdout.chars().filter(|c| !c.is_whitespace()).collect();
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(&b64_clean)
        .map_err(|e| AppError::Xray(format!("failed to decode snapshot archive: {}", e)))?;

    Ok(bytes)
}
