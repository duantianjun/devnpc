//! 配置系统: 三层来源 (env > .devnpc.md > 内置)

pub mod devnpc_md;
pub mod env;
pub mod loader;
pub mod npc_config;
pub mod skill;

use serde::{Deserialize, Serialize};

use crate::error::Result;

// ── 新增集中配置结构体 ──

/// 命令执行配置 (shell.rs)
#[derive(Debug, Clone, Deserialize)]
pub struct CommandConfig {
    pub allowlist: Vec<String>,
    pub denylist: Vec<String>,
    pub default_timeout_secs: u64,
}

impl Default for CommandConfig {
    fn default() -> Self {
        Self {
            allowlist: vec![
                // Rust
                "cargo".into(), "rustc".into(),
                // Java / JVM
                "mvn".into(), "mvnw".into(), "gradle".into(), "gradlew".into(),
                "java".into(), "javac".into(), "jar".into(),
                // Node.js
                "npm".into(), "npx".into(), "node".into(),
                // Python
                "python".into(), "pip".into(), "pip3".into(), "poetry".into(),
                // Go
                "go".into(), "gofmt".into(),
                // .NET
                "dotnet".into(),
                // 通用
                "make".into(), "just".into(), "fmt".into(),
                "clippy".into(), "echo".into(),
            ],
            denylist: vec![
                "rm".into(), "mv".into(), "cp".into(), "curl".into(),
                "wget".into(), "ssh".into(), "scp".into(),
            ],
            default_timeout_secs: 120,
        }
    }
}

/// 文件读取配置 (file_io.rs)
#[derive(Debug, Clone, Deserialize)]
pub struct ReadFileConfig {
    pub max_lines: usize,
}

impl Default for ReadFileConfig {
    fn default() -> Self {
        Self { max_lines: 200 }
    }
}

/// 日志解析配置 (log_parser.rs)
#[derive(Debug, Clone, Deserialize)]
pub struct LogParserConfig {
    pub max_failures: usize,
}

impl Default for LogParserConfig {
    fn default() -> Self {
        Self { max_failures: 10 }
    }
}

/// 关键文件摘要配置 (repo_index.rs)
#[derive(Debug, Clone, Deserialize)]
pub struct SummaryConfig {
    pub key_file_patterns: Vec<String>,
    pub readme_lines: usize,
    pub main_rs_lines: usize,
    pub other_lines: usize,
}

impl Default for SummaryConfig {
    fn default() -> Self {
        Self {
            key_file_patterns: vec![
                // Rust
                "Cargo.toml".into(), "src/main.rs".into(), "src/lib.rs".into(),
                // Java / JVM
                "pom.xml".into(), "build.gradle".into(), "build.gradle.kts".into(),
                "settings.gradle".into(), "gradlew".into(), "mvnw".into(),
                // Node.js
                "package.json".into(), "package-lock.json".into(),
                "yarn.lock".into(), "tsconfig.json".into(),
                // Python
                "pyproject.toml".into(), "requirements.txt".into(),
                "setup.py".into(), "setup.cfg".into(),
                // Go
                "go.mod".into(), "go.sum".into(),
                // 通用
                "README.md".into(), ".devnpc.md".into(),
                ".gitlab-ci.yml".into(),
                "Makefile".into(), "justfile".into(),
            ],
            readme_lines: 30,
            main_rs_lines: 50,
            other_lines: 20,
        }
    }
}

/// 上下文构建配置 (context.rs)
#[derive(Debug, Clone, Deserialize)]
pub struct ContextConfig {
    pub max_recent_commits: usize,
    pub max_recent_pipelines: usize,
    pub max_ci_history_failures: usize,
}

impl Default for ContextConfig {
    fn default() -> Self {
        Self {
            max_recent_commits: 20,
            max_recent_pipelines: 5,
            max_ci_history_failures: 5,
        }
    }
}

/// CI 闭环配置 (controller.rs, 从 ci/controller.rs 迁移)
#[derive(Debug, Clone, Deserialize)]
pub struct CiConfig {
    pub poll_interval_secs: u64,
    pub poll_timeout_secs: u64,
    pub pipeline_timeout_secs: u64,
    pub max_retries: u8,
}

impl Default for CiConfig {
    fn default() -> Self {
        Self {
            poll_interval_secs: 10,
            poll_timeout_secs: 300,
            pipeline_timeout_secs: 1800,
            max_retries: 3,
        }
    }
}

// ── 原有配置 ──

/// 顶层配置
#[derive(Debug, Clone)]
pub struct Config {
    pub llm: LlmConfig,
    pub gitlab: GitlabConfig,
    pub limits: Limits,
    pub project: ProjectConfig,
    pub model_routing: ModelRoutingConfig,
    pub report: ReportConfig,
    /// 命令执行配置 (shell.rs)
    pub command: CommandConfig,
    /// 文件读取配置 (file_io.rs)
    pub read_file: ReadFileConfig,
    /// 日志解析配置 (log_parser.rs)
    pub log_parser: LogParserConfig,
    /// 关键文件摘要配置 (repo_index.rs)
    pub summary: SummaryConfig,
    /// 上下文构建配置 (context.rs)
    pub context: ContextConfig,
    /// CI 闭环配置 (controller.rs)
    pub ci: CiConfig,
    /// MCP 服务器配置
    pub mcp: McpConfig,
    /// 长期记忆配置
    pub memory: MemoryConfig,
    /// npc-config 角色与 SOP 配置
    pub npc_config: NpcConfigSection,
    /// Webhook 服务器配置 (trigger/webhook.rs)
    pub webhook: WebhookConfig,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LlmConfig {
    pub api_key: String,
    pub base_url: String,
    pub model: String,
    /// 模型提供商: "deepseek" | "openai" | "anthropic" | "gemini"
    #[serde(default = "default_provider")]
    pub provider: String,
}

fn default_provider() -> String {
    "deepseek".to_string()
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
#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum SopMode {
    #[default]
    Soft,
    Strict,
}

/// .devnpc.md 解析结果
#[derive(Debug, Clone, Default, Serialize)]
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

/// 模型路由配置
#[derive(Debug, Clone, Default, Deserialize)]
pub struct ModelRoutingConfig {
    /// 简单任务使用的模型 (Fix, Test)
    pub simple_model: String,
    /// 复杂任务使用的模型 (Implement, Refactor)
    pub complex_model: String,
}

/// MCP 服务器配置
#[derive(Debug, Clone, Default, Deserialize)]
pub struct McpConfig {
    /// 是否启用 MCP Gateway
    pub enabled: bool,
    /// codemap 二进制路径 (默认 "codemap")
    pub codemap_path: String,
    /// codemap 数据目录 (默认 ".codemap")
    pub codemap_data_dir: String,
}

/// 长期记忆配置
#[derive(Debug, Clone, Default, Deserialize)]
pub struct MemoryConfig {
    /// 是否启用长期记忆
    pub enabled: bool,
    /// SQLite 存储路径 (默认 ".devnpc-memory.db")
    pub db_path: String,
}

/// npc-config 角色与 SOP 配置
#[derive(Debug, Clone, Deserialize)]
pub struct NpcConfigSection {
    /// 是否启用 npc-config 加载 (默认 true)
    pub enabled: bool,
    /// npc-config 目录路径 (默认 "npc-config")
    pub base_dir: String,
}

impl Default for NpcConfigSection {
    fn default() -> Self {
        Self {
            enabled: true,
            base_dir: "npc-config".to_string(),
        }
    }
}

/// Webhook 服务器配置
///
/// 用于接收 GitLab webhook 事件 (Note/MergeRequest/Issue),自动触发任务执行。
/// 替代 `@devnpc` 评论轮询模式,减少触发延迟。
#[derive(Debug, Clone, Default, Deserialize)]
pub struct WebhookConfig {
    /// 是否启用 webhook 服务器 (默认 false,仅在 `serve` 子命令时启动)
    pub enabled: bool,
    /// 监听地址 (默认 "0.0.0.0")
    pub host: String,
    /// 监听端口 (默认 8080)
    pub port: u16,
    /// GitLab webhook secret (用于校验 X-Gitlab-Token header,空则不校验)
    pub secret: String,
    /// webhook 路径 (默认 "/webhook")
    pub path: String,
}

impl Config {
    /// 加载配置 (env + .devnpc.md + 默认值三层合并)
    pub fn load() -> Result<Self> {
        loader::load()
    }
}
