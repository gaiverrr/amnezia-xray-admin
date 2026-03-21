use std::fmt;

#[derive(Debug)]
pub enum AppError {
    Ssh(String),
    Xray(String),
    Config(String),
    Io(std::io::Error),
}

impl fmt::Display for AppError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AppError::Ssh(msg) => write!(f, "SSH error: {}", msg),
            AppError::Xray(msg) => write!(f, "Xray error: {}", msg),
            AppError::Config(msg) => write!(f, "Config error: {}", msg),
            AppError::Io(err) => write!(f, "IO error: {}", err),
        }
    }
}

impl std::error::Error for AppError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            AppError::Io(err) => Some(err),
            _ => None,
        }
    }
}

impl From<std::io::Error> for AppError {
    fn from(err: std::io::Error) -> Self {
        AppError::Io(err)
    }
}

impl From<serde_json::Error> for AppError {
    fn from(err: serde_json::Error) -> Self {
        AppError::Config(format!("JSON parse error: {}", err))
    }
}

impl From<toml::de::Error> for AppError {
    fn from(err: toml::de::Error) -> Self {
        AppError::Config(format!("TOML parse error: {}", err))
    }
}

pub type Result<T> = std::result::Result<T, AppError>;

/// Enrich an error message with an actionable hint based on known failure patterns.
/// Returns the original message with a hint appended, or unchanged if no pattern matches.
pub fn add_hint(msg: &str) -> String {
    let lower = msg.to_lowercase();

    // Xray API errors (check before generic SSH patterns to avoid false matches)
    if lower.contains("adu failed") || lower.contains("rmu failed") {
        return format!(
            "{}. Xray API may not be responding. Run '--check-server' to diagnose",
            msg
        );
    }

    // Public key missing
    if lower.contains("failed to read public key") {
        return format!(
            "{}. Is Amnezia Xray properly installed? The public key should be at /opt/amnezia/xray/xray_public.key inside the container",
            msg
        );
    }

    // Container errors
    if (lower.contains("no such container") || lower.contains("is not running"))
        && lower.contains("docker")
    {
        return format!(
            "{}. Check container name with 'docker ps' on your VPS",
            msg
        );
    }

    // SSH connection errors
    if lower.contains("connection refused") || lower.contains("connection reset") {
        return format!(
            "{}. Check: 1) SSH host alias or IP is correct 2) VPS is reachable 3) SSH port is correct",
            msg
        );
    }
    if lower.contains("auth") && lower.contains("failed") {
        return format!(
            "{}. Check your SSH key or ssh-agent (is the key loaded? try: ssh-add -l)",
            msg
        );
    }
    if lower.contains("ssh-agent connect failed") || lower.contains("ssh-agent list keys failed") {
        return format!(
            "{}. Is ssh-agent running? Try: eval $(ssh-agent) && ssh-add",
            msg
        );
    }

    msg.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_display_ssh_error() {
        let err = AppError::Ssh("connection refused".to_string());
        assert_eq!(err.to_string(), "SSH error: connection refused");
    }

    #[test]
    fn test_display_xray_error() {
        let err = AppError::Xray("gRPC unavailable".to_string());
        assert_eq!(err.to_string(), "Xray error: gRPC unavailable");
    }

    #[test]
    fn test_display_config_error() {
        let err = AppError::Config("missing field".to_string());
        assert_eq!(err.to_string(), "Config error: missing field");
    }

    #[test]
    fn test_display_io_error() {
        let io_err = std::io::Error::new(std::io::ErrorKind::NotFound, "file not found");
        let err = AppError::Io(io_err);
        assert_eq!(err.to_string(), "IO error: file not found");
    }

    #[test]
    fn test_from_io_error() {
        let io_err = std::io::Error::new(std::io::ErrorKind::PermissionDenied, "access denied");
        let err: AppError = io_err.into();
        assert!(matches!(err, AppError::Io(_)));
        assert_eq!(err.to_string(), "IO error: access denied");
    }

    #[test]
    fn test_from_serde_json_error() {
        let json_err = serde_json::from_str::<serde_json::Value>("invalid").unwrap_err();
        let err: AppError = json_err.into();
        assert!(matches!(err, AppError::Config(_)));
        assert!(err.to_string().contains("JSON parse error"));
    }

    #[test]
    fn test_from_toml_error() {
        let toml_err: toml::de::Error = toml::from_str::<toml::Value>("= invalid").unwrap_err();
        let err: AppError = toml_err.into();
        assert!(matches!(err, AppError::Config(_)));
        assert!(err.to_string().contains("TOML parse error"));
    }

    #[test]
    fn test_error_source_io() {
        let io_err = std::io::Error::new(std::io::ErrorKind::NotFound, "not found");
        let err = AppError::Io(io_err);
        assert!(std::error::Error::source(&err).is_some());
    }

    #[test]
    fn test_error_source_none_for_string_variants() {
        let err = AppError::Ssh("test".to_string());
        assert!(std::error::Error::source(&err).is_none());

        let err = AppError::Xray("test".to_string());
        assert!(std::error::Error::source(&err).is_none());

        let err = AppError::Config("test".to_string());
        assert!(std::error::Error::source(&err).is_none());
    }

    #[test]
    fn test_debug_impl() {
        let err = AppError::Ssh("test".to_string());
        let debug = format!("{:?}", err);
        assert!(debug.contains("Ssh"));
        assert!(debug.contains("test"));
    }

    // -- add_hint tests --

    #[test]
    fn test_hint_connection_refused() {
        let msg = add_hint("connection failed: Connection refused");
        assert!(msg.contains("Connection refused"));
        assert!(msg.contains("SSH host alias or IP is correct"));
    }

    #[test]
    fn test_hint_connection_reset() {
        let msg = add_hint("connection failed: Connection reset by peer");
        assert!(msg.contains("VPS is reachable"));
    }

    #[test]
    fn test_hint_auth_failed() {
        let msg = add_hint("authentication failed");
        assert!(msg.contains("SSH key"));
        assert!(msg.contains("ssh-add -l"));
    }

    #[test]
    fn test_hint_key_auth_failed() {
        let msg = add_hint("key auth failed: something went wrong");
        assert!(msg.contains("SSH key"));
    }

    #[test]
    fn test_hint_agent_connect_failed() {
        let msg = add_hint("ssh-agent connect failed: No such file");
        assert!(msg.contains("ssh-agent running"));
    }

    #[test]
    fn test_hint_agent_list_keys_failed() {
        let msg = add_hint("ssh-agent list keys failed: timeout");
        assert!(msg.contains("ssh-agent running"));
    }

    #[test]
    fn test_hint_no_such_container() {
        let msg = add_hint("docker exec failed: Error: No such container: amnezia-xray");
        assert!(msg.contains("docker ps"));
    }

    #[test]
    fn test_hint_container_not_running() {
        let msg = add_hint("docker exec failed: Error response: Container abc is not running");
        assert!(msg.contains("docker ps"));
    }

    #[test]
    fn test_hint_adu_failed() {
        let msg = add_hint("adu failed: gRPC connection error");
        assert!(msg.contains("--check-server"));
    }

    #[test]
    fn test_hint_rmu_failed() {
        let msg = add_hint("rmu failed: connection refused");
        assert!(msg.contains("--check-server"));
    }

    #[test]
    fn test_hint_public_key_missing() {
        let msg = add_hint("failed to read public key: No such file");
        assert!(msg.contains("Amnezia Xray properly installed"));
        assert!(msg.contains("xray_public.key"));
    }

    #[test]
    fn test_hint_no_match_unchanged() {
        let msg = add_hint("some random error");
        assert_eq!(msg, "some random error");
    }

    #[test]
    fn test_hint_preserves_original_message() {
        let original = "connection failed: Connection refused (os error 111)";
        let msg = add_hint(original);
        assert!(msg.starts_with(original));
    }
}
