//! 配置系统: 三层来源 (env > .devnpc.md > 内置)

pub mod devnpc_md;
pub mod env;
pub mod loader;

use serde::Deserialize;

use crate::error::Result;

/// 顶层配置
#[derive(Debug, Clone)]
pub struct Config {
    pub llm: LlmConfig,
    pub gitlab: GitlabConfig,
    pub limits: Limits,
    pub project: ProjectConfig,
    pub report: ReportConfig,
    // P6 引入: pub roles: HashMap<String, Role>,
    // P6 引入: pub sops: HashMap<String, Sop>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LlmConfig {
    pub api_key: String,
    pub base_url: String,
    pub model: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct GitlabConfig {
    pub url: String,
    pub token: String,
    pub project_id: u64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Limits {
    pub max_iterations: u32,
    pub max_ci_retries: u8,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_iterations: 20,
            max_ci_retries: 3,
        }
    }
}

/// SOP 约束模式
#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum SopMode {
    #[default]
    Soft,
    Strict,
}

/// .devnpc.md 解析结果
#[derive(Debug, Clone, Default)]
pub struct ProjectConfig {
    pub sop_mode: SopMode,
    pub forbidden_paths: Vec<String>,
    pub required_checks: Vec<String>,
    pub branch_prefix: String,
    pub max_ci_retries: Option<u8>,
    pub guidelines_markdown: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ReportConfig {
    pub target: ReportTarget,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ReportTarget {
    Artifact,
    Pages,
    None,
}

impl Default for ReportConfig {
    fn default() -> Self {
        Self {
            target: ReportTarget::Artifact,
        }
    }
}

impl Config {
    /// 加载配置 (env + .devnpc.md + 默认值三层合并)
    pub fn load() -> Result<Self> {
        loader::load()
    }
}
