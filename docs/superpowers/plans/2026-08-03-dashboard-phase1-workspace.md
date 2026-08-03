# Dashboard Phase 1: Workspace 拆分 + core crate 实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 将 devnpc 单 crate 改造成 Cargo workspace，创建 devnpc-core 共享类型 crate，现有 350 个测试全部通过。

**Architecture:** workspace 根管理三个 crate：devnpc-core（共享数据类型 + 事件协议）、devnpc（现有 bin，依赖 core）、devnpc-dashboard（Phase 3 新建）。本阶段只创建 core 并迁移纯数据类型。

**Tech Stack:** Rust 2024 edition, Cargo workspace, serde, chrono, uuid, thiserror

**关联 spec:** [2026-08-03-devnpc-dashboard-design.md](../specs/2026-08-03-devnpc-dashboard-design.md)

---

## 文件结构总览

本阶段完成后目录结构：

```
devnpc/
├── Cargo.toml                    # workspace 根 (移除 [package], 新增 [workspace])
├── crates/
│   ├── devnpc-core/
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── report/
│   │       │   ├── mod.rs
│   │       │   ├── types.rs      # 迁移自 collector.rs 的纯数据类型
│   │       │   └── event_schema.rs  # 新增: dashboard 事件协议
│   │       └── error.rs          # core 错误类型
│   └── devnpc/
│       ├── Cargo.toml
│       └── src/                  # 从现有 src/ 移入
│           ├── lib.rs
│           ├── main.rs
│           ├── adapter/ ci/ config/ git/ gitlab_api/ memory/ report/ trigger/
│           └── error.rs
├── tests/
│   └── integration_e2e.rs
└── (其他文件不变: docs/, npc-config/, Dockerfile 等)
```

**迁移策略**：core 只包含纯数据类型（无 IO 依赖）。`TrajectoryCollector` 和 `build_report` 留在 devnpc（依赖 `CiOutcome`/`CostConfig`/`UsageStats`）。devnpc 的 `report/collector.rs` 通过 `pub use devnpc_core::report::types::*` re-export core 类型，现有代码 `use crate::report::collector::*` 无需改动。

---

### Task 1: 创建 workspace 根 Cargo.toml

**Files:**
- Modify: `Cargo.toml` (根)

- [ ] **Step 1: 备份当前 Cargo.toml 内容**

读取当前 `Cargo.toml` 全部内容，保存到临时变量（下一步需要用到 `[dependencies]` 和 `[dev-dependencies]` 部分）。

- [ ] **Step 2: 重写根 Cargo.toml 为 workspace 定义**

将根 `Cargo.toml` 替换为：

```toml
[workspace]
members = ["crates/devnpc-core", "crates/devnpc"]
resolver = "2"

[workspace.package]
edition = "2024"
license = "MIT"

[workspace.dependencies]
# 异步运行时
tokio = { version = "1", features = ["full"] }
futures = "0.3"

# 序列化
serde = { version = "1", features = ["derive"] }
serde_json = "1"
serde_yaml = "0.9"

# 日志与错误
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter", "json"] }
thiserror = "2"

# 工具
chrono = { version = "0.4", features = ["serde"] }
uuid = { version = "1", features = ["v4"] }
regex = "1"
async-trait = "0.1"

# HTTP
reqwest = { version = "0.12", default-features = false, features = ["json", "rustls-tls", "stream"] }
axum = "0.7"
tower = "0.5"

# 数据库
rusqlite = { version = "0.32", features = ["bundled"] }

# CLI
clap = { version = "4", features = ["derive", "env"] }

# 代码感知
tree-sitter = "0.26"
```

- [ ] **Step 3: 验证 workspace 配置可解析**

Run: `cargo metadata --no-deps --format-version 1 > NUL 2>&1 || echo "expected: members not found yet"`
Expected: 报错 "failed to load manifest for workspace member"，因为 `crates/devnpc-core` 和 `crates/devnpc` 尚不存在。这是正常的，后续 Task 创建后即可。

---

### Task 2: 创建 devnpc-core crate 骨架

**Files:**
- Create: `crates/devnpc-core/Cargo.toml`
- Create: `crates/devnpc-core/src/lib.rs`

- [ ] **Step 1: 创建目录结构**

Run: `mkdir crates\devnpc-core\src\report`

- [ ] **Step 2: 创建 devnpc-core/Cargo.toml**

```toml
[package]
name = "devnpc-core"
version = "0.1.0"
edition.workspace = true
license.workspace = true
description = "devnpc 共享类型: 报告数据结构 + dashboard 事件协议"

[dependencies]
serde = { workspace = true }
serde_json = { workspace = true }
chrono = { workspace = true }
uuid = { workspace = true }
thiserror = { workspace = true }
```

- [ ] **Step 3: 创建 src/lib.rs**

```rust
//! devnpc-core - 共享类型库
//!
//! 提供 devnpc 和 devnpc-dashboard 共用的数据结构:
//! - 报告类型 (Trajectory/ReportData/CostEstimate)
//! - dashboard 事件协议 (TaskStartedEvent/ExecutionEvent/TaskFinishedEvent)

pub mod error;
pub mod report;
```

- [ ] **Step 4: 创建 src/report/mod.rs**

```rust
//! 报告相关共享类型

pub mod event_schema;
pub mod types;

// 便捷 re-export
pub use types::*;
```

- [ ] **Step 5: 创建 src/error.rs（core 错误类型）**

```rust
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
```

- [ ] **Step 6: 创建 src/report/types.rs 占位**

```rust
//! 报告数据类型 (从 devnpc 迁移)
//! Task 3 中填充实际类型定义

// 占位: Task 3 迁移实际类型
```

- [ ] **Step 7: 创建 src/report/event_schema.rs 占位**

```rust
//! dashboard 事件协议类型
//! Task 4 中填充实际类型定义

// 占位: Task 4 定义事件协议
```

- [ ] **Step 8: 验证 core crate 编译**

Run: `cargo check -p devnpc-core`
Expected: 编译通过（可能有 unused warnings，正常）

---

### Task 3: 迁移纯数据类型到 core

**Files:**
- Create: `crates/devnpc-core/src/report/types.rs` (替换占位)
- Source: `src/report/collector.rs` (读取类型定义，不修改原文件)

**迁移范围**：以下类型从 `collector.rs` 迁移到 `types.rs`（纯数据结构，无 devnpc 依赖）：
- `TrajectoryEvent` (enum)
- `Trajectory` (struct + `new()` + `record_*` 方法)
- `ReportData` (struct + `Default` impl)
- `TeamStepSummary` (struct + `Default`)
- `TrajectorySummary` (struct + `Default`)
- `TrajectoryEventSummary` (struct)
- `CostEstimate` (struct + `Default`)

**不迁移**（留在 devnpc 的 `collector.rs`）：
- `TrajectoryCollector` (依赖 `CiOutcome`)
- `TrajectoryCollector::build_report` (依赖 `crate::adapter::orchestrator::UsageStats` 和 `crate::config::CostConfig`)

- [ ] **Step 1: 编写 types.rs 类型定义测试**

在 `crates/devnpc-core/src/report/types.rs` 顶部编写测试：

```rust
//! 报告数据类型 (从 devnpc report/collector.rs 迁移)
//!
//! 纯数据结构,无 devnpc 业务依赖。

use serde::{Deserialize, Serialize};

// ============================================================
// 轨迹类型
// ============================================================

/// 轨迹事件
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TrajectoryEvent {
    /// LLM 调用
    LlmCall { iteration: usize },
    /// 工具调用
    ToolCall { name: String, success: bool },
    /// SOP 偏离
    Deviation { step: String, unexpected: Vec<String> },
}

/// 轨迹
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Trajectory {
    pub events: Vec<TrajectoryEvent>,
}

impl Trajectory {
    pub fn new() -> Self {
        Self { events: Vec::new() }
    }

    /// 记录 LLM 调用
    pub fn record_llm_call(&mut self, iteration: usize) {
        self.events.push(TrajectoryEvent::LlmCall { iteration });
    }

    /// 记录工具调用
    pub fn record_tool_call(&mut self, name: &str, success: bool) {
        self.events.push(TrajectoryEvent::ToolCall {
            name: name.to_string(),
            success,
        });
    }
}

// ============================================================
// 报告数据
// ============================================================

/// 报告数据 (供 HTML 生成使用)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReportData {
    pub status: String,
    pub duration_secs: u64,
    pub token_total: u64,
    pub llm_calls: u32,
    pub tool_calls: u32,
    pub ci_retries: u8,
    pub mr_url: Option<String>,
    pub ci_url: Option<String>,
    pub summary: String,
    pub task_description: String,
    pub trajectory: TrajectorySummary,
    pub cost_estimate: CostEstimate,
    pub mr_iid: Option<u64>,
    pub pipeline_id: Option<u64>,
    pub started_at: String,
    pub finished_at: String,
    /// Team 协作流程步骤 (仅在 Team 编排模式下填充)
    pub team_steps: Vec<TeamStepSummary>,
}

/// Team 协作步骤摘要
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TeamStepSummary {
    pub role: String,
    pub instruction: String,
    pub output: String,
    pub signals: Vec<String>,
}

/// 轨迹摘要
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TrajectorySummary {
    pub events: Vec<TrajectoryEventSummary>,
}

/// 轨迹事件摘要
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrajectoryEventSummary {
    pub kind: String,
    pub detail: String,
    pub success: Option<bool>,
}

/// 成本估算
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CostEstimate {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub estimated_cost_usd: f64,
}

impl Default for CostEstimate {
    fn default() -> Self {
        Self {
            input_tokens: 0,
            output_tokens: 0,
            estimated_cost_usd: 0.0,
        }
    }
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
            ci_url: None,
            summary: String::new(),
            task_description: String::new(),
            trajectory: TrajectorySummary::default(),
            cost_estimate: CostEstimate::default(),
            mr_iid: None,
            pipeline_id: None,
            started_at: chrono::Utc::now().to_rfc3339(),
            finished_at: chrono::Utc::now().to_rfc3339(),
            team_steps: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trajectory_new_is_empty() {
        let t = Trajectory::new();
        assert!(t.events.is_empty());
    }

    #[test]
    fn trajectory_record_llm_call() {
        let mut t = Trajectory::new();
        t.record_llm_call(1);
        assert_eq!(t.events.len(), 1);
        assert!(matches!(t.events[0], TrajectoryEvent::LlmCall { iteration: 1 }));
    }

    #[test]
    fn trajectory_record_tool_call() {
        let mut t = Trajectory::new();
        t.record_tool_call("read_file", true);
        assert_eq!(t.events.len(), 1);
        assert!(matches!(
            &t.events[0],
            TrajectoryEvent::ToolCall { name, success } if name == "read_file" && *success
        ));
    }

    #[test]
    fn report_data_default_has_unknown_status() {
        let d = ReportData::default();
        assert_eq!(d.status, "unknown");
    }

    #[test]
    fn cost_estimate_default_is_zero() {
        let c = CostEstimate::default();
        assert_eq!(c.input_tokens, 0);
        assert_eq!(c.output_tokens, 0);
        assert_eq!(c.estimated_cost_usd, 0.0);
    }

    #[test]
    fn trajectory_event_serializes_to_json() {
        let event = TrajectoryEvent::LlmCall { iteration: 5 };
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("LlmCall"));
        assert!(json.contains("5"));
    }

    #[test]
    fn report_data_serializes_to_json() {
        let data = ReportData::default();
        let json = serde_json::to_string(&data).unwrap();
        assert!(json.contains("unknown"));
    }
}
```

- [ ] **Step 2: 运行测试验证类型定义正确**

Run: `cargo test -p devnpc-core`
Expected: 8 个测试全部 PASS

- [ ] **Step 3: 提交 core 类型迁移**

Run: `git add crates/devnpc-core/ ; git commit -m "feat(core): 迁移报告数据类型到 devnpc-core (Trajectory/ReportData/CostEstimate)"`

---

### Task 4: 定义事件协议类型

**Files:**
- Modify: `crates/devnpc-core/src/report/event_schema.rs` (替换占位)

- [ ] **Step 1: 编写 event_schema.rs**

```rust
//! dashboard 事件协议类型
//!
//! devnpc 任务执行过程中推送到 dashboard 的事件结构。
//! 同时用于本地 .jsonl 文件存储 (兜底机制)。

use serde::{Deserialize, Serialize};

// ============================================================
// 枚举辅助类型
// ============================================================

/// SOP 步骤状态
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum SopStepStatus {
    Started,
    Completed,
    Deviated,
}

/// CI 状态
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum CiStatus {
    Running,
    Passed,
    Failed,
}

/// 任务最终状态
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    Success,
    Failed,
    CiFailed,
    Timeout,
}

// ============================================================
// 事件类型
// ============================================================

/// 任务启动事件 (任务开始时推送一次)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskStartedEvent {
    /// UUID v4,贯穿任务全生命周期
    pub task_id: String,
    /// GitLab 项目路径
    pub project: String,
    pub mr_iid: Option<u64>,
    pub pipeline_id: Option<u64>,
    pub task_description: String,
    /// issue/mr_comment/manual
    pub task_kind: String,
    /// RFC3339
    pub started_at: String,
    /// 使用的 LLM 模型名
    pub model: String,
}

/// 执行过程事件 (任务执行中持续推送)
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ExecutionEvent {
    LlmCall {
        iteration: u32,
        prompt_tokens: u64,
        completion_tokens: u64,
        latency_ms: u64,
    },
    ToolCall {
        name: String,
        success: bool,
        latency_ms: u64,
        /// 工具调用摘要(非完整参数)
        detail: String,
    },
    SopStep {
        step: String,
        status: SopStepStatus,
        note: Option<String>,
    },
    CiStatus {
        pipeline_id: u64,
        status: CiStatus,
        /// 第几次重试
        attempt: u8,
    },
    TeamHandoff {
        /// pm/developer/tester
        from_role: String,
        to_role: String,
        /// decomposed/implemented/tested
        signal: String,
    },
}

/// 任务结束事件 (任务完成时推送一次)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskFinishedEvent {
    pub task_id: String,
    pub status: TaskStatus,
    pub duration_secs: u64,
    pub total_tokens: u64,
    pub estimated_cost_usd: f64,
    pub mr_url: Option<String>,
    pub ci_url: Option<String>,
    /// LLM 生成的验收摘要
    pub summary: String,
    /// 失败原因
    pub error: Option<String>,
    pub finished_at: String,
}

// ============================================================
// JSONL 文件行类型 (本地兜底文件格式)
// ============================================================

/// .jsonl 文件每行的包装类型
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum EventLogEntry {
    TaskStarted {
        task_id: String,
        #[serde(flatten)]
        data: TaskStartedEvent,
    },
    Execution {
        task_id: String,
        event: ExecutionEvent,
    },
    TaskFinished {
        task_id: String,
        #[serde(flatten)]
        data: TaskFinishedEvent,
    },
}

/// 批量推送请求体
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchEventsRequest {
    pub task_id: String,
    pub events: Vec<ExecutionEvent>,
}

/// 导入结果
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportResult {
    pub task_id: String,
    pub events_count: usize,
    /// true=因已 finish 而跳过
    pub skipped: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn task_started_event_serializes() {
        let e = TaskStartedEvent {
            task_id: "abc-123".into(),
            project: "my-group/my-project".into(),
            mr_iid: Some(42),
            pipeline_id: Some(100),
            task_description: "修复 bug".into(),
            task_kind: "mr_comment".into(),
            started_at: "2026-08-03T10:00:00Z".into(),
            model: "deepseek-chat".into(),
        };
        let json = serde_json::to_string(&e).unwrap();
        assert!(json.contains("abc-123"));
        assert!(json.contains("deepseek-chat"));
        // round-trip
        let e2: TaskStartedEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(e2.task_id, "abc-123");
    }

    #[test]
    fn execution_event_llm_call_serializes() {
        let e = ExecutionEvent::LlmCall {
            iteration: 1,
            prompt_tokens: 500,
            completion_tokens: 200,
            latency_ms: 1500,
        };
        let json = serde_json::to_string(&e).unwrap();
        assert!(json.contains("llm_call"));
        let e2: ExecutionEvent = serde_json::from_str(&json).unwrap();
        match e2 {
            ExecutionEvent::LlmCall { iteration, .. } => assert_eq!(iteration, 1),
            _ => panic!("应为 LlmCall"),
        }
    }

    #[test]
    fn execution_event_tool_call_serializes() {
        let e = ExecutionEvent::ToolCall {
            name: "read_file".into(),
            success: true,
            latency_ms: 50,
            detail: "src/main.rs".into(),
        };
        let json = serde_json::to_string(&e).unwrap();
        assert!(json.contains("tool_call"));
        assert!(json.contains("read_file"));
    }

    #[test]
    fn execution_event_sop_step_serializes() {
        let e = ExecutionEvent::SopStep {
            step: "analyze".into(),
            status: SopStepStatus::Completed,
            note: None,
        };
        let json = serde_json::to_string(&e).unwrap();
        assert!(json.contains("sop_step"));
        assert!(json.contains("completed"));
    }

    #[test]
    fn execution_event_ci_status_serializes() {
        let e = ExecutionEvent::CiStatus {
            pipeline_id: 100,
            status: CiStatus::Failed,
            attempt: 2,
        };
        let json = serde_json::to_string(&e).unwrap();
        assert!(json.contains("ci_status"));
        assert!(json.contains("failed"));
    }

    #[test]
    fn execution_event_team_handoff_serializes() {
        let e = ExecutionEvent::TeamHandoff {
            from_role: "pm".into(),
            to_role: "developer".into(),
            signal: "decomposed".into(),
        };
        let json = serde_json::to_string(&e).unwrap();
        assert!(json.contains("team_handoff"));
        assert!(json.contains("decomposed"));
    }

    #[test]
    fn task_finished_event_serializes() {
        let e = TaskFinishedEvent {
            task_id: "abc-123".into(),
            status: TaskStatus::Success,
            duration_secs: 45,
            total_tokens: 12000,
            estimated_cost_usd: 0.05,
            mr_url: Some("https://gitlab.com/mr/42".into()),
            ci_url: Some("https://gitlab.com/pipeline/100".into()),
            summary: "已修复 bug".into(),
            error: None,
            finished_at: "2026-08-03T10:01:00Z".into(),
        };
        let json = serde_json::to_string(&e).unwrap();
        assert!(json.contains("success"));
        let e2: TaskFinishedEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(e2.status, TaskStatus::Success);
    }

    #[test]
    fn event_log_entry_task_started_serializes() {
        let entry = EventLogEntry::TaskStarted {
            task_id: "abc-123".into(),
            data: TaskStartedEvent {
                task_id: "abc-123".into(),
                project: "proj".into(),
                mr_iid: None,
                pipeline_id: None,
                task_description: "test".into(),
                task_kind: "manual".into(),
                started_at: "2026-08-03T10:00:00Z".into(),
                model: "gpt-4".into(),
            },
        };
        let json = serde_json::to_string(&entry).unwrap();
        assert!(json.contains("task_started"));
        let entry2: EventLogEntry = serde_json::from_str(&json).unwrap();
        match entry2 {
            EventLogEntry::TaskStarted { task_id, .. } => assert_eq!(task_id, "abc-123"),
            _ => panic!("应为 TaskStarted"),
        }
    }

    #[test]
    fn event_log_entry_execution_serializes() {
        let entry = EventLogEntry::Execution {
            task_id: "abc-123".into(),
            event: ExecutionEvent::LlmCall {
                iteration: 1,
                prompt_tokens: 100,
                completion_tokens: 50,
                latency_ms: 500,
            },
        };
        let json = serde_json::to_string(&entry).unwrap();
        assert!(json.contains("execution"));
        assert!(json.contains("llm_call"));
    }

    #[test]
    fn event_log_entry_task_finished_serializes() {
        let entry = EventLogEntry::TaskFinished {
            task_id: "abc-123".into(),
            data: TaskFinishedEvent {
                task_id: "abc-123".into(),
                status: TaskStatus::Failed,
                duration_secs: 100,
                total_tokens: 5000,
                estimated_cost_usd: 0.02,
                mr_url: None,
                ci_url: None,
                summary: "失败".into(),
                error: Some("CI 超时".into()),
                finished_at: "2026-08-03T10:02:00Z".into(),
            },
        };
        let json = serde_json::to_string(&entry).unwrap();
        assert!(json.contains("task_finished"));
        assert!(json.contains("failed"));
    }

    #[test]
    fn batch_events_request_serializes() {
        let req = BatchEventsRequest {
            task_id: "abc-123".into(),
            events: vec![
                ExecutionEvent::LlmCall {
                    iteration: 1,
                    prompt_tokens: 100,
                    completion_tokens: 50,
                    latency_ms: 500,
                },
                ExecutionEvent::ToolCall {
                    name: "read_file".into(),
                    success: true,
                    latency_ms: 10,
                    detail: "test.rs".into(),
                },
            ],
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains("abc-123"));
        assert!(json.contains("llm_call"));
        assert!(json.contains("tool_call"));
    }

    #[test]
    fn import_result_serializes() {
        let r = ImportResult {
            task_id: "abc-123".into(),
            events_count: 10,
            skipped: false,
        };
        let json = serde_json::to_string(&r).unwrap();
        assert!(json.contains("10"));
        assert!(json.contains("false"));
    }

    #[test]
    fn jsonl_line_round_trip() {
        // 模拟 .jsonl 文件中的一行
        let entry = EventLogEntry::Execution {
            task_id: "t1".into(),
            event: ExecutionEvent::ToolCall {
                name: "git_commit".into(),
                success: true,
                latency_ms: 200,
                detail: "commit message".into(),
            },
        };
        let line = serde_json::to_string(&entry).unwrap();
        // 模拟从文件读取一行并解析
        let parsed: EventLogEntry = serde_json::from_str(&line).unwrap();
        match parsed {
            EventLogEntry::Execution { task_id, event } => {
                assert_eq!(task_id, "t1");
                match event {
                    ExecutionEvent::ToolCall { name, success, .. } => {
                        assert_eq!(name, "git_commit");
                        assert!(success);
                    }
                    _ => panic!("应为 ToolCall"),
                }
            }
            _ => panic!("应为 Execution"),
        }
    }
}
```

- [ ] **Step 2: 运行测试验证事件协议**

Run: `cargo test -p devnpc-core`
Expected: 之前 8 个 + 新增 13 个 = 21 个测试全部 PASS

- [ ] **Step 3: 提交事件协议**

Run: `git add crates/devnpc-core/src/report/event_schema.rs ; git commit -m "feat(core): 定义 dashboard 事件协议类型 (TaskStartedEvent/ExecutionEvent/TaskFinishedEvent + JSONL 格式)"`

---

### Task 5: 移动 devnpc src 到 crates/devnpc

**Files:**
- Move: `src/` → `crates/devnpc/src/`
- Move: `tests/` → `crates/devnpc/tests/`
- Create: `crates/devnpc/Cargo.toml`

- [ ] **Step 1: 创建 crates/devnpc 目录**

Run: `mkdir crates\devnpc`

- [ ] **Step 2: 移动 src/ 和 tests/ 到 crates/devnpc/**

Run: `move src crates\devnpc\src`
Run: `move tests crates\devnpc\tests`

- [ ] **Step 3: 创建 crates/devnpc/Cargo.toml**

从根 Cargo.toml 的原始内容中提取 `[package]`、`[[bin]]`、`[lib]`、`[features]`、`[dependencies]`、`[dev-dependencies]`、`[lints.rust]`、`[profile.release]` 部分，添加 `devnpc-core` 依赖：

```toml
[package]
name = "devnpc"
version = "0.1.0"
edition.workspace = true
license.workspace = true
description = "基于 GitLab 的企业级研发流程 AI 智能体"

[[bin]]
name = "devnpc"
path = "src/main.rs"

[lib]
name = "devnpc"
path = "src/lib.rs"

[features]
default = ["deepseek", "openai", "anthropic", "gemini"]
deepseek = ["adk-rust/deepseek"]
openai = ["adk-rust/openai"]
anthropic = ["adk-rust/anthropic"]
gemini = ["adk-rust/gemini"]

[dependencies]
devnpc-core = { path = "../devnpc-core" }

# 异步运行时
tokio = { workspace = true }
futures = { workspace = true }

# Agent 框架 (adk-rust)
adk-rust = { version = "1", features = [
    "minimal",
    "openai", "anthropic", "deepseek",
    "tools",
    "graph",
    "mcp",
    "mcp-http",
    "guardrail",
    "memory",
    "rag",
    "code",
    "eval",
] }

# HTTP / GitLab API
reqwest = { workspace = true }

# Webhook 服务器
axum = { workspace = true }
tower = { workspace = true }

# 代码感知工具 (AFT)
tree-sitter = { workspace = true }
tree-sitter-rust = "0.23"
tree-sitter-java = "0.23"
tree-sitter-python = "0.23"
tree-sitter-javascript = "0.23"
tree-sitter-typescript = "0.23"
tree-sitter-go = "0.23"
tree-sitter-c = "0.23"
tree-sitter-cpp = "0.23"
tree-sitter-ruby = "0.23"
tree-sitter-php = "0.24"
tree-sitter-swift = "0.7"
tree-sitter-kotlin-sg = "0.4"

# 序列化
serde = { workspace = true }
serde_json = { workspace = true }
serde_yaml = { workspace = true }

# CLI
clap = { workspace = true }

# 日志与错误
tracing = { workspace = true }
tracing-subscriber = { workspace = true }
thiserror = { workspace = true }

# 工具
uuid = { workspace = true }
chrono = { workspace = true }
regex = { workspace = true }
async-trait = { workspace = true }

# 长期记忆存储
rusqlite = { workspace = true }

# MCP 协议
rmcp = "1"

[dev-dependencies]
tokio-test = "0.4"
mockall = "0.13"
tempfile = "3"
wiremock = "0.6"

[lints.rust]
unexpected_cfgs = { level = "warn", check-cfg = ['cfg(feature, values("deepseek", "openai", "anthropic", "gemini"))'] }

[profile.release]
opt-level = 3
lto = "thin"
strip = true
```

- [ ] **Step 4: 验证 workspace 解析**

Run: `cargo metadata --no-deps --format-version 1 > NUL`
Expected: 成功输出 JSON（不再报错）

---

### Task 6: 更新 devnpc report 模块 re-export core 类型

**Files:**
- Modify: `crates/devnpc/src/report/collector.rs`
- Modify: `crates/devnpc/src/report/mod.rs`

**策略**：`collector.rs` 删除已迁移到 core 的类型定义，改为 re-export。保留 `TrajectoryCollector` 和 `build_report`（依赖 devnpc 类型）。

- [ ] **Step 1: 更新 collector.rs 顶部，删除已迁移的类型，添加 re-export**

在 `crates/devnpc/src/report/collector.rs` 文件开头，删除以下类型定义（已迁移到 core）：
- `TrajectoryEvent` enum
- `Trajectory` struct + `new()` + `record_*` 方法
- `ReportData` struct + `Default` impl
- `TeamStepSummary` struct + `Default`
- `TrajectorySummary` struct + `Default`
- `TrajectoryEventSummary` struct
- `CostEstimate` struct + `Default` impl

在文件顶部添加 re-export：

```rust
//! 轨迹采集器
//!
//! 从执行轨迹提取事件,聚合为报告数据。
//! 纯数据类型已迁移到 devnpc-core,这里通过 re-export 保持向后兼容。

// re-export core 类型 (向后兼容现有 use crate::report::collector::* 路径)
pub use devnpc_core::report::types::{
    CostEstimate, ReportData, TeamStepSummary, Trajectory, TrajectoryEvent,
    TrajectoryEventSummary, TrajectorySummary,
};

use std::sync::{Arc, Mutex};

use chrono::Utc;

use crate::ci::controller::CiOutcome;
```

保留 `TrajectoryCollector` struct 和 `build_report` / `from_trajectory` 方法不变。

- [ ] **Step 2: 更新 report/mod.rs**

`crates/devnpc/src/report/mod.rs` 保持不变（已有 `pub mod collector;`）。

- [ ] **Step 3: 验证编译**

Run: `cargo check -p devnpc`
Expected: 编译通过。如果报错 `cannot find type`，检查 re-export 路径是否正确。

---

### Task 7: 更新 devnpc error.rs 添加 CoreError 转换

**Files:**
- Modify: `crates/devnpc/src/error.rs`

- [ ] **Step 1: 在 DevnpcError 中添加 CoreError 转换**

在 `crates/devnpc/src/error.rs` 的 `DevnpcError` enum 中添加：

```rust
    #[error("core 错误: {0}")]
    Core(#[from] devnpc_core::error::CoreError),
```

放在 `Sqlite(String)` 变体之后。

- [ ] **Step 2: 验证编译**

Run: `cargo check -p devnpc`
Expected: 编译通过

---

### Task 8: 运行全部测试验证

**Files:**
- 无修改

- [ ] **Step 1: 运行 core 测试**

Run: `cargo test -p devnpc-core`
Expected: 21 个测试全部 PASS

- [ ] **Step 2: 运行 devnpc 单元测试**

Run: `cargo test -p devnpc --lib`
Expected: 现有 ~338 个单元测试全部 PASS（可能有少量测试因 use 路径变化需要调整）

- [ ] **Step 3: 运行 devnpc 集成测试**

Run: `cargo test -p devnpc --test integration_e2e`
Expected: 12 个集成测试全部 PASS

- [ ] **Step 4: 如果有测试失败，修复 use 路径**

搜索失败的测试，将 `use crate::report::collector::TypeName` 确认通过 re-export 仍然可用。re-export 策略下，现有 `use crate::report::collector::*` 路径不需要修改。

如果出现 `cannot find type CiOutcome in crate devnpc_core`，说明 `build_report` 或 `TrajectoryCollector` 的代码引用了 core 中不存在的类型。检查这些函数是否仍引用 devnpc 的 `crate::ci::controller::CiOutcome` 等——它们应该保留在 devnpc 的 collector.rs 中。

- [ ] **Step 5: 运行 clippy**

Run: `cargo clippy --workspace -- -D warnings`
Expected: 无 warning

- [ ] **Step 6: 提交 workspace 拆分**

Run: `git add -A ; git commit -m "refactor: 拆分 Cargo workspace,创建 devnpc-core 共享类型 crate (向后兼容,350 测试通过)"`

---

### Task 9: 更新 .gitignore 和文档引用

**Files:**
- Modify: `Dockerfile` (如有路径引用)
- Modify: `.gitignore` (如有 target 路径引用)

- [ ] **Step 1: 检查 Dockerfile 路径引用**

Run: `findstr /n "src/" Dockerfile`
Expected: 如果有 `COPY src/` 之类的引用，需要更新为 `COPY crates/devnpc/src/`

- [ ] **Step 2: 检查 .gitlab-ci.yml.example 路径引用**

Run: `findstr /n "src/" .gitlab-ci.yml.example`
Expected: 如有引用则更新

- [ ] **Step 3: 提交路径修复**

Run: `git add -A ; git commit -m "chore: 更新 Dockerfile/CI 配置适配 workspace 目录结构"`

---

## Self-Review 检查清单

- [ ] spec §二 Workspace 结构：三 crate 布局已建立（core + devnpc，dashboard 在 Phase 3）
- [ ] spec §三 事件协议：event_schema.rs 定义了所有事件类型 + JSONL 格式
- [ ] spec §四 devnpc 侧改造：re-export 保持向后兼容，现有测试无需改 use 路径
- [ ] 向后兼容：未配置 dashboard 时 devnpc 行为与现状一致（本阶段不涉及推送逻辑）
- [ ] 无占位符：所有代码步骤包含完整实现
- [ ] 类型一致：core 的 types.rs 与现有 collector.rs 类型定义一致（字段名/类型/derive）
- [ ] 测试覆盖：core 21 个测试覆盖类型定义和序列化 round-trip
