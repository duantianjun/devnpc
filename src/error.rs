//! 统一错误类型
//!
//! 库层用 DevnpcError,CLI 层用 anyhow。

use thiserror::Error;

#[derive(Error, Debug)]
pub enum DevnpcError {
    #[error("配置错误: {0}")]
    Config(String),

    #[error("环境变量缺失: {var}")]
    MissingEnv { var: String },

    #[error("GitLab API 错误: {status} {body}")]
    GitlabApi { status: u16, body: String },

    #[error("GitLab 资源不存在: {resource}")]
    GitlabNotFound { resource: String },

    #[error("Git 命令失败: {cmd} (exit {code})")]
    GitCommand { cmd: String, code: i32 },

    #[error("分支保护: 不允许操作 {branch}")]
    BranchProtected { branch: String },

    #[error("LLM 调用失败: {0}")]
    Llm(String),

    #[error("Agent 达到迭代上限 ({max})")]
    MaxIterations { max: u32 },

    #[error("任务被取消")]
    Cancelled,

    #[error("CI 修复失败,重试 {attempts} 次未通过")]
    CiFixExhausted { attempts: u8 },

    #[error("Pipeline 超时 ({stage})")]
    PipelineTimeout { stage: String },

    #[error("工具调用错误: {tool}: {msg}")]
    Tool { tool: String, msg: String },

    #[error("路径越界: {path} 不在 workspace 内")]
    PathTraversal { path: String },

    #[error("GitLab API 请求失败: {0}")]
    Reqwest(#[from] reqwest::Error),

    #[error("YAML 解析错误: {0}")]
    Yaml(#[from] serde_yaml::Error),

    #[error(transparent)]
    Io(#[from] std::io::Error),
}

pub type Result<T> = std::result::Result<T, DevnpcError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_error_displays_message() {
        let err = DevnpcError::Config("缺少 api_key".into());
        assert_eq!(err.to_string(), "配置错误: 缺少 api_key");
    }

    #[test]
    fn missing_env_error_includes_var_name() {
        let err = DevnpcError::MissingEnv {
            var: "DEVNPC_API_KEY".into(),
        };
        assert!(err.to_string().contains("DEVNPC_API_KEY"));
    }

    #[test]
    fn gitlab_api_error_formats_status_and_body() {
        let err = DevnpcError::GitlabApi {
            status: 404,
            body: "Not Found".into(),
        };
        assert_eq!(err.to_string(), "GitLab API 错误: 404 Not Found");
    }

    #[test]
    fn io_error_converts_via_from() {
        let io_err = std::io::Error::new(std::io::ErrorKind::NotFound, "file missing");
        let err: DevnpcError = io_err.into();
        assert!(matches!(err, DevnpcError::Io(_)));
    }

    #[test]
    fn path_traversal_error_includes_path() {
        let err = DevnpcError::PathTraversal {
            path: "../etc/passwd".into(),
        };
        assert!(err.to_string().contains("../etc/passwd"));
    }
}
