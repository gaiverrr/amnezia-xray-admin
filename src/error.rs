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
}
