//! Dashboard 错误类型
//!
//! 统一错误枚举,实现 IntoResponse 自动转换为 HTTP 响应。

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;
use thiserror::Error;

/// Dashboard 全局错误类型
#[derive(Debug, Error)]
pub enum DashboardError {
    #[error("SQLite 错误: {0}")]
    Sqlite(#[from] rusqlite::Error),

    #[error("序列化错误: {0}")]
    Serde(#[from] serde_json::Error),

    #[error("任务不存在: {0}")]
    TaskNotFound(String),

    #[error("任务状态冲突: {0}")]
    TaskConflict(String),

    #[error("导入文件格式错误: {0}")]
    ImportFormat(String),

    #[error("模板渲染错误: {0}")]
    Template(#[from] askama::Error),

    #[error("IO 错误: {0}")]
    Io(#[from] std::io::Error),
}

/// 便捷 Result 别名
pub type Result<T> = std::result::Result<T, DashboardError>;

/// 将错误映射为 HTTP 状态码 + JSON body
impl IntoResponse for DashboardError {
    fn into_response(self) -> Response {
        let (status, msg) = match &self {
            DashboardError::Sqlite(e) => {
                tracing::error!(error = %e, "sqlite 写入失败");
                (StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
            }
            DashboardError::Serde(e) => (StatusCode::BAD_REQUEST, e.to_string()),
            DashboardError::TaskNotFound(id) => {
                (StatusCode::NOT_FOUND, format!("任务不存在: {}", id))
            }
            DashboardError::TaskConflict(msg) => (StatusCode::CONFLICT, msg.clone()),
            DashboardError::ImportFormat(msg) => (StatusCode::BAD_REQUEST, msg.clone()),
            DashboardError::Template(e) => {
                tracing::error!(error = %e, "模板渲染失败");
                (StatusCode::INTERNAL_SERVER_ERROR, "页面渲染失败".to_string())
            }
            DashboardError::Io(e) => {
                tracing::error!(error = %e, "IO 错误");
                (StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
            }
        };
        (status, Json(json!({ "error": msg }))).into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sqlite_error_displays() {
        let err = DashboardError::TaskNotFound("abc".into());
        assert!(err.to_string().contains("abc"));
    }

    #[test]
    fn task_conflict_displays() {
        let err = DashboardError::TaskConflict("任务已存在".into());
        assert!(err.to_string().contains("任务已存在"));
    }

    #[test]
    fn import_format_displays() {
        let err = DashboardError::ImportFormat("第 3 行解析失败".into());
        assert!(err.to_string().contains("第 3 行"));
    }

    #[tokio::test]
    async fn task_not_found_maps_to_404() {
        let err = DashboardError::TaskNotFound("no-such-task".into());
        let resp = err.into_response();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn task_conflict_maps_to_409() {
        let err = DashboardError::TaskConflict("已存在".into());
        let resp = err.into_response();
        assert_eq!(resp.status(), StatusCode::CONFLICT);
    }

    #[tokio::test]
    async fn import_format_maps_to_400() {
        let err = DashboardError::ImportFormat("坏格式".into());
        let resp = err.into_response();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }
}
