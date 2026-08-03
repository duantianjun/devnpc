//! devnpc-core 错误类型
//!
//! 仅包含 core 层错误。devnpc 的 DevnpcError 通过 #[from] 转换。

use thiserror::Error;

#[derive(Error, Debug)]
pub enum CoreError {
    #[error("序列化错误: {0}")]
    Serde(#[from] serde_json::Error),

    #[error("Dashboard 推送失败: {0}")]
    DashboardPush(String),

    #[error("Dashboard 配置错误: {0}")]
    DashboardConfig(String),

    #[error("IO 错误: {0}")]
    Io(#[from] std::io::Error),
}

pub type Result<T> = std::result::Result<T, CoreError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serde_error_displays_message() {
        let err = CoreError::DashboardPush("connection refused".into());
        assert!(err.to_string().contains("connection refused"));
    }

    #[test]
    fn io_error_converts_via_from() {
        let io_err = std::io::Error::new(std::io::ErrorKind::NotFound, "file missing");
        let err: CoreError = io_err.into();
        assert!(matches!(err, CoreError::Io(_)));
    }
}
