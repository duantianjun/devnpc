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
    /// 成本估算配置 (orchestrator.rs / collector.rs / main.rs)
    pub cost: CostConfig,
    /// AFT 代码感知工具配置 (adapter/tools.rs)
    pub tools: ToolsConfig,
    /// 触发源配置 (trigger/parser.rs / main.rs)
    pub trigger: TriggerConfig,
    /// Dashboard 推送配置 (spec §4.1)
    pub dashboard: DashboardConfig,
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
#[derive(Debug, Clone, Serialize)]
pub struct ProjectConfig {
    pub sop_mode: SopMode,
    pub forbidden_paths: Vec<String>,
    pub required_checks: Vec<String>,
    pub branch_prefix: String,
    pub max_ci_retries: Option<u8>,
    pub guidelines_markdown: String,
    /// 默认 Git 分支名 (用于读取仓库文件,默认 "main")
    pub default_branch: String,
    /// 创建 MR 时的目标分支 (默认 "main")
    pub target_branch: String,
    /// 主 Agent 系统指令 (可通过 DEVNPC_MAIN_INSTRUCTION 环境变量覆盖)
    pub main_instruction: String,
}

impl Default for ProjectConfig {
    fn default() -> Self {
        Self {
            sop_mode: SopMode::default(),
            forbidden_paths: Vec::new(),
            required_checks: Vec::new(),
            branch_prefix: "npc".to_string(),
            max_ci_retries: None,
            guidelines_markdown: String::new(),
            default_branch: "main".to_string(),
            target_branch: "main".to_string(),
            main_instruction: default_main_instruction(),
        }
    }
}

/// 默认主 Agent 系统指令
pub fn default_main_instruction() -> String {
    "你是一个软件开发工程师。使用 devnpc 工具链完成研发任务。\n\
     遵循以下原则:\n\
     1. 修改前先理解上下文 (read_file / list_files / aft_outline)\n\
     2. 改完后用对应的构建工具验证编译 (如 cargo build / mvn compile / gradle build / npm run build 等)\n\
     3. 完成后总结你的工作成果\n\
     4. 禁止修改工作目录外的文件"
        .to_string()
}

#[derive(Debug, Clone, Deserialize)]
pub struct ReportConfig {
    pub target: ReportTarget,
    /// 报告输出目录 (默认 ".devnpc-report")
    pub output_dir: String,
    /// 报告文件名 (默认 "report.html")
    pub output_file: String,
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
            output_dir: ".devnpc-report".to_string(),
            output_file: "report.html".to_string(),
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
#[derive(Debug, Clone, Deserialize)]
pub struct MemoryConfig {
    /// 是否启用长期记忆
    pub enabled: bool,
    /// SQLite 存储路径 (默认 ".devnpc-memory.db")
    pub db_path: String,
    /// 搜索任务记录/修复经验时返回的最大条数 (默认 10)
    pub max_search_results: usize,
}

impl Default for MemoryConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            db_path: ".devnpc-memory.db".to_string(),
            max_search_results: 10,
        }
    }
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
#[derive(Debug, Clone, Deserialize)]
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
    /// 触发事件 channel 缓冲区大小 (默认 32)
    pub channel_buffer_size: usize,
}

impl Default for WebhookConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            host: "0.0.0.0".to_string(),
            port: 8080,
            secret: String::new(),
            path: "/webhook".to_string(),
            channel_buffer_size: 32,
        }
    }
}

/// 成本估算配置 (orchestrator.rs / collector.rs / main.rs)
#[derive(Debug, Clone, Deserialize)]
pub struct CostConfig {
    /// 默认 input token 费率 (USD/token, 默认 $1.5/M = 0.0000015)
    pub input_rate: f64,
    /// 默认 output token 费率 (USD/token, 默认 $2.0/M = 0.0000020)
    pub output_rate: f64,
    /// 回退估算: 每次 LLM 调用假设的 input token 数 (默认 500)
    pub est_input_tokens_per_call: u64,
    /// 回退估算: 每次 LLM 调用假设的 output token 数 (默认 200)
    pub est_output_tokens_per_call: u64,
}

impl Default for CostConfig {
    fn default() -> Self {
        Self {
            input_rate: 0.000_001_5,
            output_rate: 0.000_002_0,
            est_input_tokens_per_call: 500,
            est_output_tokens_per_call: 200,
        }
    }
}

/// AFT 代码感知工具配置 (adapter/tools.rs)
#[derive(Debug, Clone, Deserialize)]
pub struct ToolsConfig {
    /// AST 符号收集/查找的递归最大深度 (默认 20)
    pub max_symbol_depth: usize,
    /// 源码文件收集的递归最大深度 (默认 10)
    pub max_file_depth: usize,
    /// 文件收集时跳过的目录名 (默认 ["target", ".git", "node_modules"])
    pub ignore_dirs: Vec<String>,
    /// SOP 严格模式允许的工具白名单 (默认全部业务工具)
    pub allowed_tools: Vec<String>,
}

impl Default for ToolsConfig {
    fn default() -> Self {
        Self {
            max_symbol_depth: 20,
            max_file_depth: 10,
            ignore_dirs: vec![
                "target".to_string(),
                ".git".to_string(),
                "node_modules".to_string(),
            ],
            allowed_tools: default_allowed_tools_list(),
        }
    }
}

/// 默认 SOP 允许工具列表 (用于 loader.rs fallback 和 ToolsConfig::default)
pub fn default_allowed_tools_list() -> Vec<String> {
    [
        "read_file", "write_file", "edit_file", "delete_file",
        "list_files", "search_files", "grep_files", "run_command",
        "aft_outline", "aft_view_symbol", "aft_edit_symbol",
        "aft_search_symbols", "aft_ast_replace",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect()
}

/// 触发源配置 (trigger/parser.rs / main.rs)
#[derive(Debug, Clone, Deserialize)]
pub struct TriggerConfig {
    /// @devnpc 提及的正则表达式 (默认 r"@devnpc\s*(.*)")
    pub mention_regex: String,
    /// CI 环境变量名: MR IID (默认 "CI_MERGE_REQUEST_IID")
    pub ci_mr_iid_var: String,
    /// CI 环境变量名: Issue IID (默认 "CI_ISSUE_IID")
    pub ci_issue_iid_var: String,
}

impl Default for TriggerConfig {
    fn default() -> Self {
        Self {
            mention_regex: r"@devnpc\s*(.*)".to_string(),
            ci_mr_iid_var: "CI_MERGE_REQUEST_IID".to_string(),
            ci_issue_iid_var: "CI_ISSUE_IID".to_string(),
        }
    }
}

/// Dashboard 推送配置 (spec §4.1)
///
/// 通过 .env 配置,未配置 URL 时 enabled=false,不推送。
/// local_event_log 默认 true,即使 dashboard 未启用也保存本地事件文件。
#[derive(Debug, Clone, Deserialize)]
pub struct DashboardConfig {
    /// 是否启用 dashboard 推送 (默认 false,未配置 URL 时不推送)
    pub enabled: bool,
    /// Dashboard 服务地址 (DEVNPC_DASHBOARD_URL)
    pub url: String,
    /// 推送鉴权 token (DEVNPC_DASHBOARD_TOKEN)
    pub token: String,
    /// 批量推送阈值,事件数累积到此次数触发 POST (默认 20)
    pub batch_size: usize,
    /// 批量推送时间阈值,距上次推送超过此秒数触发 POST (默认 3)
    pub batch_interval_secs: u64,
    /// 是否保存本地 .jsonl 事件文件 (默认 true,独立于 enabled)
    pub local_event_log: bool,
}

impl Default for DashboardConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            url: String::new(),
            token: String::new(),
            batch_size: 20,
            batch_interval_secs: 3,
            local_event_log: true,
        }
    }
}

impl Config {
    /// 加载配置 (env + .devnpc.md + 默认值三层合并)
    pub fn load() -> Result<Self> {
        loader::load()
    }
}

#[cfg(test)]
mod dashboard_config_tests {
    use super::*;

    #[test]
    fn dashboard_config_default_has_safe_values() {
        let cfg = DashboardConfig::default();
        // 默认不启用推送 (降级安全)
        assert!(!cfg.enabled);
        // 默认保存本地事件文件
        assert!(cfg.local_event_log);
        // 批量阈值
        assert_eq!(cfg.batch_size, 20);
        assert_eq!(cfg.batch_interval_secs, 3);
        // URL/token 默认空
        assert!(cfg.url.is_empty());
        assert!(cfg.token.is_empty());
    }

    #[test]
    fn dashboard_config_can_be_enabled() {
        let cfg = DashboardConfig {
            enabled: true,
            url: "http://dashboard:8080".into(),
            token: "secret".into(),
            batch_size: 50,
            batch_interval_secs: 10,
            local_event_log: false,
        };
        assert!(cfg.enabled);
        assert_eq!(cfg.url, "http://dashboard:8080");
        assert!(!cfg.local_event_log);
    }
}
