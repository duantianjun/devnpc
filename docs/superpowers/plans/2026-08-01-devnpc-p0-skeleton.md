# devnpc P0 骨架 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 搭建 devnpc 项目骨架,所有依赖编译通过,所有模块文件就位,`cargo build` 与 `cargo test` 成功,为 P1 起步奠定基础。

**Architecture:** Rust 单 crate 二进制项目,lib + bin 双 target。`src/lib.rs` 导出所有模块,`src/main.rs` 用 clap 提供 CLI 骨架。错误类型用 thiserror 统一定义。模块按设计文档第 10.5 节目录结构创建,每个模块文件含最小占位实现 (空函数或 stub trait),确保编译通过。

**Tech Stack:** Rust 2021 edition, tokio 异步运行时, rig-core (LLM 抽象), agent-file-tools (代码感知), reqwest (GitLab API), clap (CLI), thiserror/anyhow (错误处理), tracing (日志), serde/serde_yaml (序列化), askama (HTML 模板), mockall (测试 mock)。

**参考文档:** [2026-08-01-devnpc-design.md](../specs/2026-08-01-devnpc-design.md) 第 10.4 节依赖清单、第 10.5 节目录结构。

---

## File Structure

P0 创建以下文件,每个文件单一职责:

| 文件 | 职责 |
|---|---|
| `Cargo.toml` | 项目元数据 + 依赖 + dev-dependencies |
| `src/main.rs` | CLI 入口 (clap),调用 lib |
| `src/lib.rs` | 库导出,声明所有 pub mod |
| `src/error.rs` | 统一错误类型 DevnpcError + Result 别名 |
| `src/config/mod.rs` | 配置模块入口 |
| `src/config/env.rs` | 环境变量读取 |
| `src/config/loader.rs` | 配置加载 (env + .devnpc.md + YAML) |
| `src/gitlab_api/mod.rs` | GitLab API 模块入口 + GitlabApi trait |
| `src/gitlab_api/client.rs` | reqwest HTTP 客户端 |
| `src/gitlab_api/issues.rs` | Issue 操作 |
| `src/gitlab_api/mrs.rs` | MR 操作 |
| `src/gitlab_api/pipelines.rs` | Pipeline 操作 |
| `src/gitlab_api/notes.rs` | 评论操作 |
| `src/gitlab_api/repo.rs` | 仓库元数据/文件 |
| `src/git/mod.rs` | Git 模块入口 |
| `src/git/ops.rs` | Git 命令封装 |
| `src/memory/mod.rs` | 记忆模块入口 |
| `src/memory/context.rs` | 上下文聚合器 |
| `src/memory/repo_index.rs` | 仓库索引 |
| `src/agent/mod.rs` | Agent 模块入口 |
| `src/agent/loop_.rs` | ReAct 循环 (避开关键字 loop) |
| `src/agent/prompt.rs` | 提示词模板 |
| `src/agent/llm_client.rs` | rig LLM 客户端封装 |
| `src/agent/sop.rs` | SOP 偏离检测 |
| `src/tools/mod.rs` | 工具模块入口 + Tool trait |
| `src/tools/file_io.rs` | AFT 文件操作工具 |
| `src/tools/shell.rs` | Shell 命令工具 |
| `src/tools/git_tool.rs` | Git 工具 |
| `src/tools/gitlab_tool.rs` | GitLab API 工具 |
| `src/ci/mod.rs` | CI 模块入口 |
| `src/ci/controller.rs` | CI 闭环控制器 |
| `src/ci/log_parser.rs` | CI 日志解析器 |
| `src/npc/mod.rs` | NPC 模块入口 |
| `src/npc/role.rs` | Role 定义 |
| `src/npc/sop.rs` | SOP 定义 |
| `src/npc/runner.rs` | 单 NPC 执行器 |
| `src/team/mod.rs` | Team 模块入口 |
| `src/team/orchestrator.rs` | 多 NPC 编排器 |
| `src/team/comm.rs` | NPC 间通信 |
| `src/trigger/mod.rs` | 触发模块入口 |
| `src/trigger/parser.rs` | 事件解析器 |
| `src/report/mod.rs` | 报告模块入口 |
| `src/report/collector.rs` | 轨迹采集器 |
| `src/report/html.rs` | HTML 生成 |
| `src/report/publisher.rs` | 报告推送 |
| `src/bin/devnpc.rs` | (可选) 独立二进制入口,P0 不创建,用 main.rs |

注: `agent/loop_.rs` 文件名用 `loop_` 避开 Rust 关键字 `loop`,模块导出时用 `pub mod loop_;` 但公开符号用 `pub use loop_ as react_loop;` 或直接在 loop_.rs 内定义 `ReactLoop` 结构体。

---

### Task 1: 初始化 Cargo 项目

**Files:**
- Create: `Cargo.toml`
- Create: `src/main.rs`
- Create: `src/lib.rs`

- [ ] **Step 1: 初始化 git 仓库与 Cargo 项目**

在项目根目录 `c:\Users\Administrator\Documents\devnpc` 执行:

```bash
git init
cargo init --name devnpc
```

`cargo init` 会创建 `Cargo.toml` 和 `src/main.rs`。验证:

```bash
cargo build
```

Expected: 编译成功,产生 `target/debug/devnpc.exe`。

- [ ] **Step 2: 创建 lib.rs**

Rust 默认 `cargo init` 只创建 bin。手动创建 lib 入口:

Create `src/lib.rs`:

```rust
//! devnpc - 基于 GitLab 的企业级研发流程 AI 智能体

pub mod error;
```

修改 `src/main.rs` 为:

```rust
use devnpc::error::Result;

fn main() -> Result<()> {
    println!("devnpc skeleton - P0");
    Ok(())
}
```

- [ ] **Step 3: 创建 error.rs 占位**

Create `src/error.rs`:

```rust
//! 统一错误类型

use thiserror::Error;

#[derive(Error, Debug)]
pub enum DevnpcError {
    #[error("占位错误 - P0 骨架")]
    Placeholder,
}

pub type Result<T> = std::result::Result<T, DevnpcError>;
```

- [ ] **Step 4: 验证编译并提交**

```bash
cargo build
```

Expected: 编译成功。

```bash
git add Cargo.toml Cargo.lock src/main.rs src/lib.rs src/error.rs
git commit -m "feat: 初始化 devnpc 项目骨架"
```

---

### Task 2: 完整 Cargo.toml 依赖配置

**Files:**
- Modify: `Cargo.toml`

- [ ] **Step 1: 写入完整 Cargo.toml**

Replace `Cargo.toml` 内容为:

```toml
[package]
name = "devnpc"
version = "0.1.0"
edition = "2021"
description = "基于 GitLab 的企业级研发流程 AI 智能体"
license = "MIT"

[[bin]]
name = "devnpc"
path = "src/main.rs"

[lib]
name = "devnpc"
path = "src/lib.rs"

[dependencies]
# 异步运行时
tokio = { version = "1", features = ["full"] }
futures = "0.3"

# HTTP / GitLab API
reqwest = { version = "0.12", default-features = false, features = ["json", "rustls-tls", "stream"] }

# LLM 抽象层 (第 1 层: agent 用 rig)
rig-core = "0.10"

# 代码感知工具 (第 2 层: tools/file_io 用 AFT)
agent-file-tools = "0.49"
tree-sitter = "0.25"

# 序列化
serde = { version = "1", features = ["derive"] }
serde_json = "1"
serde_yaml = "0.9"

# CLI
clap = { version = "4", features = ["derive", "env"] }

# 日志与错误
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter", "json"] }
thiserror = "2"
anyhow = "1"

# 报告 (编译期 HTML 模板)
askama = "0.12"

# 工具
url = "2"
chrono = { version = "0.4", features = ["serde"] }
regex = "1"
dotenvy = "0.15"
async-trait = "0.1"

[dev-dependencies]
tokio-test = "0.4"
mockall = "0.13"
tempfile = "3"
```

- [ ] **Step 2: 验证依赖编译**

```bash
cargo build
```

Expected: 下载所有依赖并编译成功。首次编译较慢 (5-15 分钟,依赖网络)。

**注意**: 如果 `rig-core`、`agent-file-tools` 版本号与 crates.io 实际不符,编译会报错。此时用 `cargo search rig-core` 和 `cargo search agent-file-tools` 查实际最新版本,更新 Cargo.toml 后重试。若 `agent-file-tools` crate 不存在或 API 差异大,临时注释掉该依赖与 `tree-sitter`,在 P3 实现工具层时再解决 (P0 目标是骨架编译通过)。

- [ ] **Step 3: 提交**

```bash
git add Cargo.toml Cargo.lock
git commit -m "feat: 配置完整依赖清单"
```

---

### Task 3: 统一错误类型 (完整版)

**Files:**
- Modify: `src/error.rs`
- Test: `src/error.rs` (内联 #[cfg(test)])

- [ ] **Step 1: 写失败测试 - 错误显示**

Replace `src/error.rs` with:

```rust
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
        let err = DevnpcError::MissingEnv { var: "DEVNPC_API_KEY".into() };
        assert!(err.to_string().contains("DEVNPC_API_KEY"));
    }

    #[test]
    fn gitlab_api_error_formats_status_and_body() {
        let err = DevnpcError::GitlabApi { status: 404, body: "Not Found".into() };
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
        let err = DevnpcError::PathTraversal { path: "../etc/passwd".into() };
        assert!(err.to_string().contains("../etc/passwd"));
    }
}
```

- [ ] **Step 2: 运行测试验证通过**

```bash
cargo test --lib error
```

Expected: 5 个测试全部 PASS。

- [ ] **Step 3: 提交**

```bash
git add src/error.rs
git commit -m "feat: 定义统一错误类型 DevnpcError"
```

---

### Task 4: 配置模块骨架

**Files:**
- Create: `src/config/mod.rs`
- Create: `src/config/env.rs`
- Create: `src/config/loader.rs`
- Modify: `src/lib.rs`

- [ ] **Step 1: 创建 config 模块文件**

Create `src/config/mod.rs`:

```rust
//! 配置系统: 三层来源 (env > .devnpc.md > 内置)

pub mod env;
pub mod loader;

use std::collections::HashMap;

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
#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum SopMode {
    Soft,
    Strict,
}

impl Default for SopMode {
    fn default() -> Self {
        Self::Soft
    }
}

/// .devnpc.md 解析结果
#[derive(Debug, Clone, Default)]
pub struct ProjectConfig {
    pub sop_mode: SopMode,
    pub forbidden_paths: Vec<String>,
    pub required_checks: Vec<String>,
    pub branch_prefix: String,
    pub guidelines_markdown: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ReportConfig {
    pub target: ReportTarget,
}

#[derive(Debug, Clone, Deserialize)]
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
    /// 加载配置 (P1 完整实现,P0 返回占位)
    pub fn load() -> Result<Self> {
        Err(crate::error::DevnpcError::Config(
            "Config::load 尚未实现 - P1 将完成".into(),
        ))
    }
}
```

Create `src/config/env.rs`:

```rust
//! 环境变量读取 (P1 完整实现)

use crate::error::{DevnpcError, Result};

/// 从环境变量读取,缺失则返回错误
pub fn get_required(var: &str) -> Result<String> {
    std::env::var(var).map_err(|_| DevnpcError::MissingEnv { var: var.into() })
}

/// 从环境变量读取,缺失返回默认值
pub fn get_or_default(var: &str, default: &str) -> String {
    std::env::var(var).unwrap_or_else(|_| default.into())
}
```

Create `src/config/loader.rs`:

```rust
//! 配置加载器 (P1 完整实现: env + .devnpc.md + YAML)

// P1 将实现:
// - read_devnpc_md()
// - parse_devnpc_md()
// - load_roles()
// - load_sops()
// - Config::merge()
```

- [ ] **Step 2: 更新 lib.rs 导出 config 模块**

Replace `src/lib.rs`:

```rust
//! devnpc - 基于 GitLab 的企业级研发流程 AI 智能体

pub mod config;
pub mod error;
```

- [ ] **Step 3: 验证编译**

```bash
cargo build
```

Expected: 编译成功。可能有未使用代码警告 (正常,P0 是骨架)。

- [ ] **Step 4: 提交**

```bash
git add src/config/ src/lib.rs
git commit -m "feat: 配置模块骨架 (Config/LlmConfig/GitlabConfig/ProjectConfig)"
```

---

### Task 5: GitLab API 模块骨架

**Files:**
- Create: `src/gitlab_api/mod.rs`
- Create: `src/gitlab_api/client.rs`
- Create: `src/gitlab_api/issues.rs`
- Create: `src/gitlab_api/mrs.rs`
- Create: `src/gitlab_api/pipelines.rs`
- Create: `src/gitlab_api/notes.rs`
- Create: `src/gitlab_api/repo.rs`
- Modify: `src/lib.rs`

- [ ] **Step 1: 创建 gitlab_api/mod.rs 含数据模型与 trait**

Create `src/gitlab_api/mod.rs`:

```rust
//! GitLab REST API v4 客户端

pub mod client;
pub mod issues;
pub mod mrs;
pub mod notes;
pub mod pipelines;
pub mod repo;

use async_trait::async_trait;
use serde::Deserialize;

use crate::error::Result;

/// GitLab API 抽象 trait (便于 mock 测试)
#[async_trait]
pub trait GitlabApi: Send + Sync {
    async fn get_issue(&self, project_id: u64, iid: u64) -> Result<Issue>;
    async fn get_mr(&self, project_id: u64, iid: u64) -> Result<MergeRequest>;
    async fn create_mr(&self, project_id: u64, req: CreateMrReq) -> Result<MergeRequest>;
    async fn get_pipelines(&self, project_id: u64) -> Result<Vec<Pipeline>>;
    async fn get_issue_notes(&self, project_id: u64, iid: u64) -> Result<Vec<Note>>;
    async fn get_mr_notes(&self, project_id: u64, iid: u64) -> Result<Vec<Note>>;
    async fn create_mr_note(&self, project_id: u64, mr_iid: u64, body: &str) -> Result<Note>;
}

// === 数据模型 ===

#[derive(Debug, Clone, Deserialize)]
pub struct Issue {
    pub iid: u64,
    pub title: String,
    pub description: Option<String>,
    pub state: String,
    pub web_url: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct MergeRequest {
    pub iid: u64,
    pub title: String,
    pub description: Option<String>,
    pub state: String,
    pub source_branch: String,
    pub target_branch: String,
    pub web_url: String,
    pub draft: bool,
}

#[derive(Debug, Clone)]
pub struct CreateMrReq {
    pub source_branch: String,
    pub target_branch: String,
    pub title: String,
    pub description: String,
    pub draft: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Pipeline {
    pub id: u64,
    pub status: String,
    pub ref_: Option<String>,
    pub sha: Option<String>,
    pub web_url: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Note {
    pub id: u64,
    pub body: String,
    pub author: NoteAuthor,
    pub created_at: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct NoteAuthor {
    pub id: u64,
    pub username: String,
    pub name: String,
}
```

- [ ] **Step 2: 创建 client.rs (reqwest 封装,P1 完整实现)**

Create `src/gitlab_api/client.rs`:

```rust
//! GitLab HTTP 客户端 (P1 完整实现)

use async_trait::async_trait;

use crate::error::Result;
use super::{
    CreateMrReq, GitlabApi, Issue, MergeRequest, Note, Pipeline,
};

/// reqwest 实现
pub struct GitlabClient {
    #[allow(dead_code)]
    base_url: String,
    #[allow(dead_code)]
    token: String,
    #[allow(dead_code)]
    http: reqwest::Client,
}

impl GitlabClient {
    pub fn new(base_url: impl Into<String>, token: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into(),
            token: token.into(),
            http: reqwest::Client::new(),
        }
    }
}

#[async_trait]
impl GitlabApi for GitlabClient {
    async fn get_issue(&self, _project_id: u64, _iid: u64) -> Result<Issue> {
        unimplemented!("P1 将实现")
    }

    async fn get_mr(&self, _project_id: u64, _iid: u64) -> Result<MergeRequest> {
        unimplemented!("P1 将实现")
    }

    async fn create_mr(&self, _project_id: u64, _req: CreateMrReq) -> Result<MergeRequest> {
        unimplemented!("P1 将实现")
    }

    async fn get_pipelines(&self, _project_id: u64) -> Result<Vec<Pipeline>> {
        unimplemented!("P1 将实现")
    }

    async fn get_issue_notes(&self, _project_id: u64, _iid: u64) -> Result<Vec<Note>> {
        unimplemented!("P1 将实现")
    }

    async fn get_mr_notes(&self, _project_id: u64, _iid: u64) -> Result<Vec<Note>> {
        unimplemented!("P1 将实现")
    }

    async fn create_mr_note(&self, _project_id: u64, _mr_iid: u64, _body: &str) -> Result<Note> {
        unimplemented!("P1 将实现")
    }
}
```

- [ ] **Step 3: 创建其余 gitlab_api 子模块 (占位)**

Create `src/gitlab_api/issues.rs`:

```rust
//! Issue 操作 (P1 实现: get_issue, get_related_mrs, get_issue_notes)
```

Create `src/gitlab_api/mrs.rs`:

```rust
//! MR 操作 (P1 实现: get_mr, create_mr, get_mr_notes, create_mr_note)
```

Create `src/gitlab_api/pipelines.rs`:

```rust
//! Pipeline 操作 (P1 实现: get_pipelines, get_pipeline, get_job_logs)
```

Create `src/gitlab_api/notes.rs`:

```rust
//! 评论操作 (P1 实现)
```

Create `src/gitlab_api/repo.rs`:

```rust
//! 仓库元数据/文件 (P1 实现: get_file, list_tree)
```

- [ ] **Step 4: 更新 lib.rs 并验证编译**

Replace `src/lib.rs`:

```rust
//! devnpc - 基于 GitLab 的企业级研发流程 AI 智能体

pub mod config;
pub mod error;
pub mod gitlab_api;
```

```bash
cargo build
```

Expected: 编译成功。

- [ ] **Step 5: 提交**

```bash
git add src/gitlab_api/ src/lib.rs
git commit -m "feat: GitLab API 模块骨架 (GitlabApi trait + 数据模型)"
```

---

### Task 6: Git 操作模块骨架

**Files:**
- Create: `src/git/mod.rs`
- Create: `src/git/ops.rs`
- Modify: `src/lib.rs`

- [ ] **Step 1: 创建 git 模块**

Create `src/git/mod.rs`:

```rust
//! Git 操作层 (调用系统 git 命令)

pub mod ops;
```

Create `src/git/ops.rs`:

```rust
//! Git 命令封装 (P2 完整实现)
//!
//! 通过 std::process::Command 调用系统 git,避免 libgit2 C 依赖。

use std::path::PathBuf;

use crate::error::Result;

/// Git 操作封装
pub struct GitOps {
    /// 工作目录
    pub workspace: PathBuf,
}

impl GitOps {
    pub fn new(workspace: impl Into<PathBuf>) -> Self {
        Self {
            workspace: workspace.into(),
        }
    }

    /// clone 仓库 (P2 实现)
    pub async fn clone_repo(&self, _url: &str, _branch: &str) -> Result<()> {
        unimplemented!("P2 将实现")
    }

    /// 创建并切换分支 (P2 实现)
    pub async fn checkout_branch(&self, _branch: &str) -> Result<()> {
        unimplemented!("P2 将实现")
    }

    /// 提交 (P2 实现)
    pub async fn commit(&self, _message: &str) -> Result<()> {
        unimplemented!("P2 将实现")
    }

    /// 推送 (P2 实现)
    pub async fn push(&self, _branch: &str) -> Result<()> {
        unimplemented!("P2 将实现")
    }

    /// 获取最近提交 (P2 实现)
    pub async fn recent_commits(&self, _count: usize) -> Result<Vec<String>> {
        unimplemented!("P2 将实现")
    }
}
```

- [ ] **Step 2: 更新 lib.rs 并验证编译**

Replace `src/lib.rs`:

```rust
//! devnpc - 基于 GitLab 的企业级研发流程 AI 智能体

pub mod config;
pub mod error;
pub mod git;
pub mod gitlab_api;
```

```bash
cargo build
```

Expected: 编译成功。

- [ ] **Step 3: 提交**

```bash
git add src/git/ src/lib.rs
git commit -m "feat: Git 操作模块骨架 (GitOps)"
```

---

### Task 7: 记忆模块骨架

**Files:**
- Create: `src/memory/mod.rs`
- Create: `src/memory/context.rs`
- Create: `src/memory/repo_index.rs`
- Modify: `src/lib.rs`

- [ ] **Step 1: 创建 memory 模块**

Create `src/memory/mod.rs`:

```rust
//! 研发记忆聚合 (Git 仓库 + Issue + PR + CI)

pub mod context;
pub mod repo_index;
```

Create `src/memory/context.rs`:

```rust
//! 上下文聚合器 (P2 完整实现)
//!
//! 并行获取仓库结构、Issue、PR、CI 历史,聚合为 Context。

use crate::config::ProjectConfig;
use crate::error::Result;
use crate::git::ops::GitOps;
use crate::gitlab_api::{Issue, MergeRequest, Note, Pipeline};

/// 聚合的研发记忆
#[derive(Debug, Clone)]
pub struct Context {
    pub repo_tree: RepoTree,
    pub key_files: Vec<KeyFile>,
    pub issue: Issue,
    pub related_prs: Vec<MergeRequest>,
    pub issue_notes: Vec<Note>,
    pub recent_commits: Vec<String>,
    pub ci_failures: Vec<CiFailure>,
    pub project_config: ProjectConfig,
}

/// 仓库目录树 (精简)
#[derive(Debug, Clone, Default)]
pub struct RepoTree {
    pub entries: Vec<TreeEntry>,
}

#[derive(Debug, Clone)]
pub struct TreeEntry {
    pub path: String,
    pub kind: TreeKind,
    pub size: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TreeKind {
    File,
    Dir,
}

/// 关键文件摘要
#[derive(Debug, Clone)]
pub struct KeyFile {
    pub path: String,
    pub summary: String,
}

/// CI 失败记录
#[derive(Debug, Clone)]
pub struct CiFailure {
    pub pipeline_id: u64,
    pub job_name: String,
    pub failure_type: FailureType,
    pub root_cause: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FailureType {
    Compile,
    Test,
    Lint,
    Other,
}

impl Context {
    /// 构建上下文 (P2 完整实现)
    pub async fn build(
        _gitlab: &dyn crate::gitlab_api::GitlabApi,
        _git: &GitOps,
        _issue_iid: u64,
    ) -> Result<Self> {
        unimplemented!("P2 将实现")
    }
}
```

Create `src/memory/repo_index.rs`:

```rust
//! 仓库索引 (P2 实现: 目录树构建、关键文件选择、摘要生成)

use crate::error::Result;
use super::context::{KeyFile, RepoTree};

/// 构建仓库目录树 (P2 实现)
pub fn build_repo_tree(_workspace: &std::path::Path) -> Result<RepoTree> {
    unimplemented!("P2 将实现")
}

/// 选择关键文件 (P2 实现)
pub fn select_key_files(_tree: &RepoTree) -> Vec<KeyFile> {
    unimplemented!("P2 将实现")
}
```

- [ ] **Step 2: 更新 lib.rs 并验证编译**

Replace `src/lib.rs`:

```rust
//! devnpc - 基于 GitLab 的企业级研发流程 AI 智能体

pub mod config;
pub mod error;
pub mod git;
pub mod gitlab_api;
pub mod memory;
```

```bash
cargo build
```

Expected: 编译成功。

- [ ] **Step 3: 提交**

```bash
git add src/memory/ src/lib.rs
git commit -m "feat: 记忆模块骨架 (Context/RepoTree/CiFailure)"
```

---

### Task 8: Agent 模块骨架

**Files:**
- Create: `src/agent/mod.rs`
- Create: `src/agent/loop_.rs`
- Create: `src/agent/prompt.rs`
- Create: `src/agent/llm_client.rs`
- Create: `src/agent/sop.rs`
- Modify: `src/lib.rs`

- [ ] **Step 1: 创建 agent/mod.rs**

Create `src/agent/mod.rs`:

```rust
//! Agent 核心: ReAct 循环 + SOP 双层 (方案 C)

pub mod llm_client;
pub mod loop_;
pub mod prompt;
pub mod sop;
```

- [ ] **Step 2: 创建 loop_.rs (ReAct 循环骨架)**

Create `src/agent/loop_.rs`:

```rust
//! ReAct 循环 (P3 完整实现)
//!
//! Plan-Act-Observe 循环,带 SOP 偏离检测与迭代上限。

use crate::error::Result;
use super::sop::Sop;

/// Agent 运行结果
#[derive(Debug, Clone)]
pub enum RunResult {
    /// LLM 返回无 tool_call,任务完成
    Finished { text: String, trajectory: Trajectory },
    /// 达到迭代上限
    MaxIterationsReached(Trajectory),
}

/// 执行轨迹 (供 report 模块消费)
#[derive(Debug, Clone, Default)]
pub struct Trajectory {
    pub events: Vec<TrajectoryEvent>,
}

#[derive(Debug, Clone)]
pub enum TrajectoryEvent {
    LlmCall { iteration: u32 },
    ToolCall { name: String, success: bool },
}

/// ReAct 循环执行器 (P3 完整实现)
pub struct ReactLoop {
    pub max_iterations: u32,
}

impl ReactLoop {
    pub fn new(max_iterations: u32) -> Self {
        Self { max_iterations }
    }

    /// 运行循环 (P3 实现)
    pub async fn run(
        &self,
        _sop: Option<&Sop>,
    ) -> Result<RunResult> {
        unimplemented!("P3 将实现")
    }
}
```

- [ ] **Step 3: 创建 prompt.rs**

Create `src/agent/prompt.rs`:

```rust
//! 提示词模板 (P3 完整实现)

/// 构建初始消息 (P3 实现)
pub fn build_initial_messages() -> Vec<String> {
    unimplemented!("P3 将实现")
}
```

- [ ] **Step 4: 创建 llm_client.rs**

Create `src/agent/llm_client.rs`:

```rust
//! LLM 客户端封装 (P3 完整实现,基于 rig-core)

use crate::config::LlmConfig;
use crate::error::Result;

/// LLM 客户端 (P3 实现)
pub struct LlmClient {
    #[allow(dead_code)]
    config: LlmConfig,
}

impl LlmClient {
    pub fn new(config: LlmConfig) -> Self {
        Self { config }
    }

    /// 调用 LLM (P3 实现)
    pub async fn complete(&self, _messages: &[String]) -> Result<String> {
        unimplemented!("P3 将实现")
    }
}
```

- [ ] **Step 5: 创建 sop.rs**

Create `src/agent/sop.rs`:

```rust
//! SOP 偏离检测 (方案 C 核心)
//!
//! 软约束: 偏离只警告;strict 模式: 偏离即阻断。

use serde::Deserialize;

use super::loop_::Trajectory;

/// SOP 定义
#[derive(Debug, Clone, Deserialize)]
pub struct Sop {
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub steps: Vec<SopStep>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SopStep {
    pub name: String,
    pub expected_tools: Vec<String>,
    #[serde(default)]
    pub hint: String,
}

/// 偏离报告
#[derive(Debug, Clone)]
pub enum DeviationReport {
    None,
    Soft {
        step: String,
        unexpected_tools: Vec<String>,
    },
}

impl Sop {
    /// 估算当前步骤 (P3 完整实现)
    pub fn estimate_current_step(&self, _trajectory: &Trajectory) -> &SopStep {
        &self.steps[0]
    }

    /// 检查偏离 (P3 完整实现)
    pub fn check_deviation(
        &self,
        _tool_calls: &[String],
        _trajectory: &Trajectory,
    ) -> DeviationReport {
        DeviationReport::None
    }
}
```

- [ ] **Step 6: 更新 lib.rs 并验证编译**

Replace `src/lib.rs`:

```rust
//! devnpc - 基于 GitLab 的企业级研发流程 AI 智能体

pub mod agent;
pub mod config;
pub mod error;
pub mod git;
pub mod gitlab_api;
pub mod memory;
```

```bash
cargo build
```

Expected: 编译成功。

- [ ] **Step 7: 提交**

```bash
git add src/agent/ src/lib.rs
git commit -m "feat: Agent 模块骨架 (ReactLoop/Sop/Trajectory)"
```

---

### Task 9: 工具模块骨架

**Files:**
- Create: `src/tools/mod.rs`
- Create: `src/tools/file_io.rs`
- Create: `src/tools/shell.rs`
- Create: `src/tools/git_tool.rs`
- Create: `src/tools/gitlab_tool.rs`
- Modify: `src/lib.rs`

- [ ] **Step 1: 创建 tools/mod.rs 含 Tool trait**

Create `src/tools/mod.rs`:

```rust
//! Agent 工具集 (唯一副作用出口)
//!
//! P3 完整实现 13 个工具:
//! view_symbol, edit_symbol, ast_replace, outline, search_symbols (AFT),
//! read_file, write_file, list_files, git_diff (自建文件工具),
//! run_command (shell), git_commit (git), create_mr_note (gitlab), finish。

pub mod file_io;
pub mod git_tool;
pub mod gitlab_tool;
pub mod shell;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::error::Result;

/// 工具调用请求
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub name: String,
    pub arguments: serde_json::Value,
}

/// 工具调用结果
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResult {
    pub success: bool,
    pub output: String,
}

/// 工具 trait (P3 完整实现)
#[async_trait]
pub trait Tool: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    async fn call(&self, arguments: &serde_json::Value) -> Result<ToolResult>;
}

/// 工具注册表 (P3 完整实现)
pub struct ToolRegistry {
    #[allow(dead_code)]
    tools: Vec<Box<dyn Tool>>,
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self { tools: Vec::new() }
    }

    pub fn register(&mut self, tool: Box<dyn Tool>) {
        self.tools.push(tool);
    }
}

impl Default for ToolRegistry {
    fn default() -> Self {
        Self::new()
    }
}
```

- [ ] **Step 2: 创建工具子模块 (占位)**

Create `src/tools/file_io.rs`:

```rust
//! AFT 文件操作工具 (P3 实现: view_symbol, edit_symbol, ast_replace, outline, search_symbols)
//!
//! 基于 agent-file-tools (tree-sitter) 实现符号级读改。

use std::path::PathBuf;
use crate::error::{DevnpcError, Result};
use super::{Tool, ToolResult};

pub struct FileIo {
    pub workspace: PathBuf,
}

impl FileIo {
    pub fn new(workspace: impl Into<PathBuf>) -> Self {
        Self { workspace: workspace.into() }
    }

    /// 路径安全检查 (防 path traversal)
    pub fn validate_path(&self, path: &str) -> Result<PathBuf> {
        let full = self.workspace.join(path);
        let canonical = full.canonicalize().unwrap_or(full.clone());
        let ws = self.workspace.canonicalize().unwrap_or(self.workspace.clone());
        if !canonical.starts_with(&ws) {
            return Err(DevnpcError::PathTraversal { path: path.into() });
        }
        Ok(canonical)
    }
}
```

Create `src/tools/shell.rs`:

```rust
//! Shell 命令工具 (P3 实现: run_command)
//!
//! 沙箱内执行,带白名单/黑名单 + 超时。

use std::path::PathBuf;

pub struct Shell {
    #[allow(dead_code)]
    pub workspace: PathBuf,
}

impl Shell {
    pub fn new(workspace: impl Into<PathBuf>) -> Self {
        Self { workspace: workspace.into() }
    }
}
```

Create `src/tools/git_tool.rs`:

```rust
//! Git 工具 (P3 实现: git_commit, git_diff)

use std::path::PathBuf;

pub struct GitTool {
    #[allow(dead_code)]
    pub workspace: PathBuf,
}

impl GitTool {
    pub fn new(workspace: impl Into<PathBuf>) -> Self {
        Self { workspace: workspace.into() }
    }
}
```

Create `src/tools/gitlab_tool.rs`:

```rust
//! GitLab API 工具 (P3 实现: create_mr_note)

use std::sync::Arc;

pub struct GitlabTool {
    #[allow(dead_code)]
    pub client: Arc<dyn crate::gitlab_api::GitlabApi>,
    #[allow(dead_code)]
    pub project_id: u64,
}

impl GitlabTool {
    pub fn new(client: Arc<dyn crate::gitlab_api::GitlabApi>, project_id: u64) -> Self {
        Self { client, project_id }
    }
}
```

- [ ] **Step 3: 写路径检查测试**

在 `src/tools/file_io.rs` 末尾追加:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn validate_path_blocks_traversal() {
        let dir = tempdir().unwrap();
        let file_io = FileIo::new(dir.path());
        let result = file_io.validate_path("../etc/passwd");
        assert!(matches!(result, Err(DevnpcError::PathTraversal { .. })));
    }

    #[test]
    fn validate_path_allows_inside_workspace() {
        let dir = tempdir().unwrap();
        let sub = dir.path().join("src");
        fs::create_dir_all(&sub).unwrap();
        let file_io = FileIo::new(dir.path());
        let result = file_io.validate_path("src/main.rs");
        // 不报 PathTraversal 错误即通过 (文件可能不存在,canonicalize 会失败)
        assert!(!matches!(result, Err(DevnpcError::PathTraversal { .. })));
    }
}
```

- [ ] **Step 4: 更新 lib.rs 并验证**

Replace `src/lib.rs`:

```rust
//! devnpc - 基于 GitLab 的企业级研发流程 AI 智能体

pub mod agent;
pub mod config;
pub mod error;
pub mod git;
pub mod gitlab_api;
pub mod memory;
pub mod tools;
```

```bash
cargo build
cargo test --lib tools
```

Expected: 编译成功,2 个路径检查测试 PASS。

- [ ] **Step 5: 提交**

```bash
git add src/tools/ src/lib.rs
git commit -m "feat: 工具模块骨架 (Tool trait/ToolRegistry/FileIo 路径检查)"
```

---

### Task 10: CI 模块骨架

**Files:**
- Create: `src/ci/mod.rs`
- Create: `src/ci/controller.rs`
- Create: `src/ci/log_parser.rs`
- Modify: `src/lib.rs`

- [ ] **Step 1: 创建 ci/mod.rs**

Create `src/ci/mod.rs`:

```rust
//! CI 闭环控制器: MR → Pipeline → 日志 → 修复

pub mod controller;
pub mod log_parser;
```

- [ ] **Step 2: 创建 controller.rs**

Create `src/ci/controller.rs`:

```rust
//! CI 闭环控制器 (P4 完整实现)

use std::time::Duration;

use serde::Deserialize;

use crate::error::Result;

#[derive(Debug, Clone, Deserialize)]
pub struct CiConfig {
    pub poll_interval: Duration,
    pub poll_timeout: Duration,
    pub pipeline_timeout: Duration,
    pub max_retries: u8,
}

impl Default for CiConfig {
    fn default() -> Self {
        Self {
            poll_interval: Duration::from_secs(10),
            poll_timeout: Duration::from_secs(300),       // 5 min
            pipeline_timeout: Duration::from_secs(1800),  // 30 min
            max_retries: 3,
        }
    }
}

#[derive(Debug, Clone)]
pub enum CiOutcome {
    Passed { mr_iid: u64, pipeline_id: u64, attempts: u8 },
    Failed { mr_iid: u64, last_error: String, attempts: u8 },
    Timeout { mr_iid: u64, stage: String },
}

/// CI 闭环控制器 (P4 实现)
pub struct CiController {
    #[allow(dead_code)]
    config: CiConfig,
}

impl CiController {
    pub fn new(config: CiConfig) -> Self {
        Self { config }
    }

    /// 运行 CI 闭环 (P4 实现)
    pub async fn run(&self, _mr_iid: u64) -> Result<CiOutcome> {
        unimplemented!("P4 将实现")
    }
}
```

- [ ] **Step 3: 创建 log_parser.rs (含可工作测试)**

Create `src/ci/log_parser.rs`:

```rust
//! CI 日志解析器 (P4 完整实现)
//!
//! MVP 支持 Rust,识别:
//! - 编译错误: error[E####]:
//! - 测试失败: test ... FAILED / panicked at
//! - Lint: warning:
//! - 超时: timed out / killed

use serde::Deserialize;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FailureType {
    Compile,
    Test,
    Lint,
    Build,
    Timeout,
    Other,
}

#[derive(Debug, Clone)]
pub struct ParsedFailure {
    pub failure_type: FailureType,
    pub job_name: String,
    pub file: Option<String>,
    pub line: Option<u32>,
    pub error_message: String,
    pub context_lines: Vec<String>,
}

/// 解析日志 (MVP: Rust)
pub fn parse_log(job_name: &str, log: &str) -> Vec<ParsedFailure> {
    let mut failures = Vec::new();

    for (i, line) in log.lines().enumerate() {
        // 编译错误: error[E0277]: ...
        if let Some(msg) = line.strip_prefix("error[") {
            if let Some(end) = msg.find("]:") {
                let error_message = msg[end + 2..].trim().to_string();
                let context = extract_context(log, i);
                failures.push(ParsedFailure {
                    failure_type: FailureType::Compile,
                    job_name: job_name.into(),
                    file: extract_file(&error_message),
                    line: extract_line(&error_message),
                    error_message,
                    context_lines: context,
                });
            }
        }

        // 测试失败: panicked at '...', src/file.rs:42:13
        if line.contains("panicked at") {
            let error_message = line.trim().to_string();
            failures.push(ParsedFailure {
                failure_type: FailureType::Test,
                job_name: job_name.into(),
                file: extract_file_from_panic(line),
                line: extract_line_from_panic(line),
                error_message,
                context_lines: extract_context(log, i),
            });
        }

        // 超时
        if line.contains("timed out") || line.contains("killed (signal 9)") {
            failures.push(ParsedFailure {
                failure_type: FailureType::Timeout,
                job_name: job_name.into(),
                file: None,
                line: None,
                error_message: line.trim().to_string(),
                context_lines: extract_context(log, i),
            });
        }
    }

    // 去重 + 限 10 条
    failures.dedup_by(|a, b| a.error_message == b.error_message);
    failures.truncate(10);
    failures
}

fn extract_context(log: &str, center: usize) -> Vec<String> {
    let lines: Vec<&str> = log.lines().collect();
    let start = center.saturating_sub(2);
    let end = (center + 3).min(lines.len());
    lines[start..end].iter().map(|s| s.to_string()).collect()
}

fn extract_file(msg: &str) -> Option<String> {
    // 简化: 找 --> src/xxx.rs 模式
    let re = regex::Regex::new(r"-->\s*([^\s:]+\.rs)").ok()?;
    re.captures(msg).map(|c| c[1].to_string())
}

fn extract_line(msg: &str) -> Option<u32> {
    regex::Regex::new(r":(\d+):\d+").ok()?
        .captures(msg)
        .and_then(|c| c[1].parse().ok())
}

fn extract_file_from_panic(line: &str) -> Option<String> {
    let re = regex::Regex::new(r"'[^']*',\s*([^:]+\.rs)").ok()?;
    re.captures(line).map(|c| c[1].to_string())
}

fn extract_line_from_panic(line: &str) -> Option<u32> {
    regex::Regex::new(r"\.rs:(\d+)").ok()?
        .captures(line)
        .and_then(|c| c[1].parse().ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_compile_error_extracts_file_line() {
        let log = r#"
error[E0277]: cannot find value `password_raw` in this scope
  --> src/handler/login.rs:45:13
   |
45|     if password_raw.contains('!') {
   |        ^^^^^^^^^^^^^ not found
"#;
        let failures = parse_log("test", log);
        assert_eq!(failures.len(), 1);
        assert_eq!(failures[0].failure_type, FailureType::Compile);
        assert!(failures[0].error_message.contains("password_raw"));
        assert_eq!(failures[0].file.as_deref(), Some("src/handler/login.rs"));
    }

    #[test]
    fn parse_test_failure_panicked() {
        let log = "thread 'main' panicked at 'assertion failed', src/handler/login.rs:42:13";
        let failures = parse_log("test", log);
        assert_eq!(failures.len(), 1);
        assert_eq!(failures[0].failure_type, FailureType::Test);
        assert_eq!(failures[0].file.as_deref(), Some("src/handler/login.rs"));
        assert_eq!(failures[0].line, Some(42));
    }

    #[test]
    fn parse_timeout() {
        let log = "Job was killed (signal 9)";
        let failures = parse_log("test", log);
        assert_eq!(failures.len(), 1);
        assert_eq!(failures[0].failure_type, FailureType::Timeout);
    }

    #[test]
    fn parse_empty_log_returns_empty() {
        let failures = parse_log("test", "");
        assert!(failures.is_empty());
    }

    #[test]
    fn parse_dedup_duplicate_errors() {
        let log = "error[E0277]: same error\nerror[E0277]: same error";
        let failures = parse_log("test", log);
        assert_eq!(failures.len(), 1);
    }
}
```

- [ ] **Step 4: 更新 lib.rs 并验证**

Replace `src/lib.rs`:

```rust
//! devnpc - 基于 GitLab 的企业级研发流程 AI 智能体

pub mod agent;
pub mod ci;
pub mod config;
pub mod error;
pub mod git;
pub mod gitlab_api;
pub mod memory;
pub mod tools;
```

```bash
cargo build
cargo test --lib ci
```

Expected: 编译成功,5 个日志解析测试 PASS。

- [ ] **Step 5: 提交**

```bash
git add src/ci/ src/lib.rs
git commit -m "feat: CI 模块骨架 + 日志解析器 (Rust 编译/测试/超时)"
```

---

### Task 11: NPC 模块骨架

**Files:**
- Create: `src/npc/mod.rs`
- Create: `src/npc/role.rs`
- Create: `src/npc/sop.rs`
- Create: `src/npc/runner.rs`
- Modify: `src/lib.rs`

- [ ] **Step 1: 创建 npc/mod.rs**

Create `src/npc/mod.rs`:

```rust
//! NPC 角色系统: Role + SOP 加载 + 单 NPC 执行器

pub mod role;
pub mod runner;
pub mod sop;
```

- [ ] **Step 2: 创建 role.rs**

Create `src/npc/role.rs`:

```rust
//! Role 定义 (P6 完整实现: 从 YAML 加载)

use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct Role {
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub system_prompt: String,
    #[serde(default = "default_max_iterations")]
    pub max_iterations: u32,
    pub default_sop: Option<String>,
    pub tools: Vec<String>,
}

fn default_max_iterations() -> u32 {
    20
}
```

- [ ] **Step 3: 创建 sop.rs (NPC 侧 SOP 定义,引用 agent::sop)**

Create `src/npc/sop.rs`:

```rust
//! NPC SOP 定义 (P6 完整实现: 从 YAML 加载)
//!
//! 复用 agent::sop::Sop 结构

pub use crate::agent::sop::{Sop, SopStep};
```

- [ ] **Step 4: 创建 runner.rs**

Create `src/npc/runner.rs`:

```rust
//! 单 NPC 执行器 (P3-P5 完整实现)

use crate::error::Result;
use crate::memory::context::Context;
use super::role::Role;

/// NPC 执行器
pub struct NpcRunner {
    pub role: Role,
}

impl NpcRunner {
    pub fn new(role: Role) -> Self {
        Self { role }
    }

    /// 执行任务 (P3+ 实现)
    pub async fn execute(&self, _context: &Context) -> Result<()> {
        unimplemented!("P3+ 将实现")
    }
}
```

- [ ] **Step 5: 更新 lib.rs 并验证**

Replace `src/lib.rs`:

```rust
//! devnpc - 基于 GitLab 的企业级研发流程 AI 智能体

pub mod agent;
pub mod ci;
pub mod config;
pub mod error;
pub mod git;
pub mod gitlab_api;
pub mod memory;
pub mod npc;
pub mod tools;
```

```bash
cargo build
```

Expected: 编译成功。

- [ ] **Step 6: 提交**

```bash
git add src/npc/ src/lib.rs
git commit -m "feat: NPC 模块骨架 (Role/NpcRunner)"
```

---

### Task 12: Team 模块骨架

**Files:**
- Create: `src/team/mod.rs`
- Create: `src/team/orchestrator.rs`
- Create: `src/team/comm.rs`
- Modify: `src/lib.rs`

- [ ] **Step 1: 创建 team 模块**

Create `src/team/mod.rs`:

```rust
//! 多 NPC 协同: 编排器 + GitLab 评论总线

pub mod comm;
pub mod orchestrator;
```

Create `src/team/orchestrator.rs`:

```rust
//! 多 NPC 编排器 (P7 完整实现)
//!
//! 任务分解 → 并行执行 → 联调 → 单 MR 汇总

use crate::error::Result;

/// Team 编排器 (P7 实现)
pub struct Orchestrator;

impl Orchestrator {
    pub fn new() -> Self {
        Self
    }

    /// 运行 Team 任务 (P7 实现)
    pub async fn run(&self, _goal: &str) -> Result<()> {
        unimplemented!("P7 将实现")
    }
}

impl Default for Orchestrator {
    fn default() -> Self {
        Self::new()
    }
}
```

Create `src/team/comm.rs`:

```rust
//! NPC 间通信 (P7 完整实现: GitLab 评论总线)
//!
//! 协议头: [devnpc:handoff] ... [/devnpc:handoff]

use crate::error::Result;

/// 解析 handoff 消息 (P7 实现)
pub fn parse_handoff(_body: &str) -> Result<Option<Handoff>> {
    unimplemented!("P7 将实现")
}

/// Handoff 消息
#[derive(Debug, Clone)]
pub struct Handoff {
    pub from: String,
    pub to: Vec<String>,
    pub signal: String,
}
```

- [ ] **Step 2: 更新 lib.rs 并验证**

Replace `src/lib.rs`:

```rust
//! devnpc - 基于 GitLab 的企业级研发流程 AI 智能体

pub mod agent;
pub mod ci;
pub mod config;
pub mod error;
pub mod git;
pub mod gitlab_api;
pub mod memory;
pub mod npc;
pub mod team;
pub mod tools;
```

```bash
cargo build
```

Expected: 编译成功。

- [ ] **Step 3: 提交**

```bash
git add src/team/ src/lib.rs
git commit -m "feat: Team 模块骨架 (Orchestrator/Handoff)"
```

---

### Task 13: Trigger 模块骨架

**Files:**
- Create: `src/trigger/mod.rs`
- Create: `src/trigger/parser.rs`
- Modify: `src/lib.rs`

- [ ] **Step 1: 创建 trigger 模块**

Create `src/trigger/mod.rs`:

```rust
//! 事件触发: 解析 @devnpc 提及

pub mod parser;
```

Create `src/trigger/parser.rs`:

```rust
//! 触发解析器 (P5 完整实现)
//!
//! MVP: MR 评论 + 手动触发
//! P5+: Issue 评论 + Issue 创建

use serde::Deserialize;

#[derive(Debug, Clone)]
pub enum Trigger {
    IssueTask { issue_iid: u64, task: TaskSpec },
    MrTask { mr_iid: u64, task: TaskSpec },
    Manual { task: TaskSpec },
    None,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TaskSpec {
    pub kind: TaskKind,
    pub description: String,
    pub target_issue: Option<u64>,
    #[serde(default)]
    pub acceptance_criteria: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub enum TaskKind {
    Implement,
    Fix,
    Test,
    Refactor,
    Review,
}

/// 从评论中查找 @devnpc 提及并解析任务 (P5 实现)
pub fn parse_mention(_body: &str) -> Option<TaskSpec> {
    unimplemented!("P5 将实现")
}

/// 根据关键字识别任务类型 (P5 实现)
pub fn classify_task(_text: &str) -> TaskKind {
    unimplemented!("P5 将实现")
}
```

- [ ] **Step 2: 更新 lib.rs 并验证**

Replace `src/lib.rs`:

```rust
//! devnpc - 基于 GitLab 的企业级研发流程 AI 智能体

pub mod agent;
pub mod ci;
pub mod config;
pub mod error;
pub mod git;
pub mod gitlab_api;
pub mod memory;
pub mod npc;
pub mod team;
pub mod tools;
pub mod trigger;
```

```bash
cargo build
```

Expected: 编译成功。

- [ ] **Step 3: 提交**

```bash
git add src/trigger/ src/lib.rs
git commit -m "feat: Trigger 模块骨架 (Trigger/TaskSpec/TaskKind)"
```

---

### Task 14: Report 模块骨架

**Files:**
- Create: `src/report/mod.rs`
- Create: `src/report/collector.rs`
- Create: `src/report/html.rs`
- Create: `src/report/publisher.rs`
- Modify: `src/lib.rs`

- [ ] **Step 1: 创建 report/mod.rs**

Create `src/report/mod.rs`:

```rust
//! 运维报告: 轨迹采集 + HTML 生成 + 推送

pub mod collector;
pub mod html;
pub mod publisher;
```

- [ ] **Step 2: 创建 collector.rs**

Create `src/report/collector.rs`:

```rust
//! 轨迹采集器 (P4 完整实现)
//!
//! 通过 tracing 事件订阅,不侵入业务逻辑。

use std::sync::{Arc, Mutex};

use crate::agent::loop_::Trajectory;

/// 轨迹采集器
pub struct TrajectoryCollector {
    #[allow(dead_code)]
    events: Arc<Mutex<Vec<String>>>,
}

impl TrajectoryCollector {
    pub fn new() -> Self {
        Self {
            events: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// 从 Agent Trajectory 生成报告数据 (P4 实现)
    pub fn from_trajectory(_trajectory: &Trajectory) -> Self {
        Self::new()
    }
}

impl Default for TrajectoryCollector {
    fn default() -> Self {
        Self::new()
    }
}
```

- [ ] **Step 3: 创建 html.rs (含 askama 模板)**

Create `src/report/html.rs`:

```rust
//! HTML 报告生成 (P4 完整实现,基于 askama)

/// 报告数据 (P4 完整实现)
#[derive(Debug, Clone)]
pub struct ReportData {
    pub status: String,
    pub duration_secs: u64,
    pub token_total: u64,
    pub llm_calls: u32,
    pub tool_calls: u32,
    pub ci_retries: u8,
    pub mr_url: Option<String>,
    pub summary: String,
}

impl Default for ReportData {
    fn default() -> Self {
        Self {
            status: "unknown".into(),
            duration_secs: 0,
            token_total: 0,
            llm_calls: 0,
            tool_calls: 0,
            ci_retries: 0,
            mr_url: None,
            summary: String::new(),
        }
    }
}

/// 生成 HTML (P4 完整实现,改用 askama 模板)
pub fn generate_html(data: &ReportData) -> String {
    format!(
        r#"<!DOCTYPE html>
<html lang="zh">
<head><meta charset="UTF-8"><title>devnpc 报告</title></head>
<body>
<h1>devnpc 运维报告</h1>
<p>状态: {status}</p>
<p>耗时: {duration_secs}s</p>
<p>Token: {token_total}</p>
<p>LLM 调用: {llm_calls}</p>
<p>工具调用: {tool_calls}</p>
<p>CI 重试: {ci_retries}</p>
<p>摘要: {summary}</p>
</body>
</html>"#,
        status = data.status,
        duration_secs = data.duration_secs,
        token_total = data.token_total,
        llm_calls = data.llm_calls,
        tool_calls = data.tool_calls,
        ci_retries = data.ci_retries,
        summary = data.summary,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generate_html_contains_status() {
        let data = ReportData {
            status: "success".into(),
            ..Default::default()
        };
        let html = generate_html(&data);
        assert!(html.contains("success"));
        assert!(html.contains("<!DOCTYPE html>"));
    }
}
```

注: P0 用 `format!` 简化生成,P4 再改用 askama 模板文件 `templates/report.html.askama`。

- [ ] **Step 4: 创建 publisher.rs**

Create `src/report/publisher.rs`:

```rust
//! 报告推送 (P4 完整实现)

use crate::config::ReportTarget;
use crate::error::Result;

/// 推送报告
pub async fn publish(_html: &str, _target: &ReportTarget) -> Result<String> {
    unimplemented!("P4 将实现")
}
```

- [ ] **Step 5: 更新 lib.rs 并验证**

Replace `src/lib.rs`:

```rust
//! devnpc - 基于 GitLab 的企业级研发流程 AI 智能体

pub mod agent;
pub mod ci;
pub mod config;
pub mod error;
pub mod git;
pub mod gitlab_api;
pub mod memory;
pub mod npc;
pub mod report;
pub mod team;
pub mod tools;
pub mod trigger;
```

```bash
cargo build
cargo test --lib report
```

Expected: 编译成功,HTML 生成测试 PASS。

- [ ] **Step 6: 提交**

```bash
git add src/report/ src/lib.rs
git commit -m "feat: Report 模块骨架 (TrajectoryCollector/ReportData/HTML 生成)"
```

---

### Task 15: CLI 入口 (clap) + 集成验证

**Files:**
- Modify: `src/main.rs`

- [ ] **Step 1: 写 CLI 骨架**

Replace `src/main.rs`:

```rust
use clap::{Parser, Subcommand};

use devnpc::error::Result;

/// devnpc - 基于 GitLab 的企业级研发流程 AI 智能体
#[derive(Parser, Debug)]
#[command(name = "devnpc", version, about)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// 运行 NPC 任务 (CI 内调用)
    Run {
        /// 手动指定任务描述 (调试用)
        #[arg(long)]
        task: Option<String>,

        /// 干跑模式,不真正改码 (冒烟测试用)
        #[arg(long)]
        dry_run: bool,
    },

    /// 打印当前配置 (调试用)
    Config,

    /// 打印版本与构建信息
    Info,
}

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let cli = Cli::parse();

    match cli.command {
        Some(Commands::Run { task, dry_run }) => {
            run(task.as_deref(), dry_run)
        }
        Some(Commands::Config) => {
            print_config()
        }
        Some(Commands::Info) | None => {
            print_info();
            Ok(())
        }
    }
}

fn run(task: Option<&str>, dry_run: bool) -> Result<()> {
    tracing::info!(task = ?task, dry_run, "启动 devnpc (P0 骨架)");
    println!("devnpc P0 骨架 - run 命令");
    if dry_run {
        println!("dry_run 模式: 不执行实际任务");
    }
    println!("P1+ 将实现完整功能");
    Ok(())
}

fn print_config() -> Result<()> {
    println!("devnpc P0 骨架 - 配置加载待 P1 实现");
    Ok(())
}

fn print_info() {
    println!("devnpc {}", env!("CARGO_PKG_VERSION"));
    println!("基于 GitLab 的企业级研发流程 AI 智能体");
    println!();
    println!("阶段: P0 骨架");
    println!("后续: P1 配置+API → P2 记忆 → P3 Agent → P4 CI闭环 → P5 触发");
}
```

- [ ] **Step 2: 验证编译与运行**

```bash
cargo build
```

Expected: 编译成功。

```bash
cargo run -- info
```

Expected: 打印版本与阶段信息。

```bash
cargo run -- run --dry-run
```

Expected: 打印 "dry_run 模式: 不执行实际任务"。

- [ ] **Step 3: 提交**

```bash
git add src/main.rs
git commit -m "feat: CLI 入口 (clap) - run/config/info 子命令"
```

---

### Task 16: .gitignore + 全量测试 + 最终验证

**Files:**
- Create: `.gitignore`

- [ ] **Step 1: 创建 .gitignore**

Create `.gitignore`:

```
/target
/.env
*.swp
*.swo
.devnpc-report/
```

- [ ] **Step 2: 全量编译与测试**

```bash
cargo build
cargo test
cargo build --release
```

Expected:
- `cargo build`: 成功
- `cargo test`: 所有测试 PASS (error: 5, tools: 2, ci: 5, report: 1 = 共 13 个测试)
- `cargo build --release`: 成功

- [ ] **Step 3: 运行 clippy (可选但推荐)**

```bash
cargo clippy -- -D warnings
```

Expected: 无 warning (或有少量 dead_code warning,P0 骨架阶段可接受)。

- [ ] **Step 4: 提交 .gitignore 与最终验证**

```bash
git add .gitignore
git commit -m "chore: 添加 .gitignore"
```

- [ ] **Step 5: P0 验收检查**

逐项确认:

- [ ] `cargo build` 成功
- [ ] `cargo test` 全部 PASS
- [ ] `cargo run -- info` 正常输出
- [ ] `cargo run -- run --dry-run` 正常输出
- [ ] 所有 12 个模块文件就位 (config/gitlab_api/git/memory/agent/tools/ci/npc/team/trigger/report/error)
- [ ] `src/lib.rs` 导出所有模块
- [ ] git 历史清晰 (16 个提交,每个 Task 一个)

---

## P0 完成后的 Next Steps

P0 骨架完成后,基于实际代码产出,为 **P1 (配置系统 + GitLab API 客户端)** 编写下一份实施计划。P1 将:

1. 实现 `Config::load()` 完整逻辑 (env + .devnpc.md 解析 + YAML 加载)
2. 实现 `GitlabClient` 所有 API 方法 (reqwest 调用 GitLab REST v4)
3. 补充配置加载与 GitLab API 的单元/集成测试
4. 验收: 能从真实 GitLab 读取 Issue/MR/Pipeline

每个后续阶段 (P2-P5) 同样在上一阶段完成后编写计划,确保计划基于实际代码、精确可执行。
