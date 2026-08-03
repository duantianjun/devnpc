# Dashboard Phase 3: devnpc-dashboard 服务端实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 创建 `devnpc-dashboard` crate，实现 SQLite 存储层、RealtimeHub、axum 服务端 API（推送 + 辅助 + SSE + 静态资源）和鉴权中间件，不涉及前端视图（阶段 4 实现）。

**Architecture:** `devnpc-dashboard` 是独立长驻 bin，依赖 `devnpc-core` 的事件协议类型。存储层用 `rusqlite`（`Arc<Mutex<Connection>>` 串行化写，WAL 模式读不阻塞写）；实时层用 `tokio::broadcast` + 环形缓冲；HTTP 层用 `axum` 0.7（multipart 支持文件导入）；鉴权用 `from_fn_with_state` 中间件校验 `X-Devnpc-Token`。lib + bin 双目标：lib 暴露模块供单元测试，bin 负责 CLI 启动。

**Tech Stack:** Rust 2024 edition, axum 0.7 (multipart), rusqlite 0.32 (bundled, WAL), tokio (broadcast/stream), serde, clap (derive+env), rust-embed (静态资源), thiserror, chrono, tracing

**关联 spec:** [2026-08-03-devnpc-dashboard-design.md](../specs/2026-08-03-devnpc-dashboard-design.md) §三/§五/§七

**前置条件:** 阶段 1 已完成——`crates/devnpc-core/` 存在，`devnpc_core::report::event_schema` 模块已定义 `TaskStartedEvent`/`ExecutionEvent`/`TaskFinishedEvent`/`EventLogEntry`/`BatchEventsRequest`/`ImportResult`/`TaskStatus` 等类型；根 `Cargo.toml` 已是 workspace，`members = ["crates/devnpc-core", "crates/devnpc"]`。

---

## 文件结构总览

本阶段完成后目录结构（仅展示新增/修改文件）：

```
devnpc/
├── Cargo.toml                              # Modify: workspace members 增加 dashboard
└── crates/
    └── devnpc-dashboard/                   # Create: 新 crate
        ├── Cargo.toml
        ├── static/                         # 静态资源目录 (Phase 4 放入 LayUI)
        │   └── .gitkeep
        └── src/
            ├── lib.rs                      # 模块导出 (供测试)
            ├── main.rs                     # CLI 入口 (clap + 启动)
            ├── error.rs                    # DashboardError + IntoResponse
            ├── state.rs                    # AppState
            ├── auth.rs                     # require_token 中间件
            ├── storage/
            │   ├── mod.rs                  # pub mod + re-export
            │   ├── schema.rs               # 建表 SQL + WAL + init_db
            │   └── queries.rs              # Storage 结构 + CRUD + 聚合 + 导入
            ├── realtime/
            │   └── mod.rs                  # RealtimeHub + 环形缓冲 + broadcast
            └── server/
                ├── mod.rs                  # build_router 组装 + 静态资源 handler
                └── api.rs                  # 所有 API handler
```

**职责划分**：`schema.rs` 只管建表；`queries.rs` 持有 `Storage` 与全部数据库方法及行类型；`realtime/mod.rs` 持有 `RealtimeHub`；`auth.rs` 仅中间件；`server/api.rs` 所有 handler；`server/mod.rs` 路由组装 + 静态资源；`state.rs` 共享状态；`error.rs` 错误类型与 HTTP 映射。

**依赖类型（来自 devnpc-core，阶段 1 已定义）**：
- `TaskStartedEvent { task_id, project, mr_iid, pipeline_id, task_description, task_kind, started_at, model }`
- `ExecutionEvent`（`#[serde(tag="type", rename_all="snake_case")]`，变体 LlmCall/ToolCall/SopStep/CiStatus/TeamHandoff）
- `TaskFinishedEvent { task_id, status, duration_secs, total_tokens, estimated_cost_usd, mr_url, ci_url, summary, error, finished_at }`
- `EventLogEntry`（`#[serde(tag="kind", rename_all="snake_case")]`，变体 TaskStarted/Execution/TaskFinished）
- `BatchEventsRequest { task_id, events }`、`ImportResult { task_id, events_count, skipped }`
- `TaskStatus`（Success/Failed/CiFailed/Timeout）、`SopStepStatus`、`CiStatus`

---

### Task 1: 创建 crate 骨架与错误类型

**Files:**
- Modify: `Cargo.toml` (workspace 根)
- Create: `crates/devnpc-dashboard/Cargo.toml`
- Create: `crates/devnpc-dashboard/src/lib.rs`
- Create: `crates/devnpc-dashboard/src/error.rs`
- Create: `crates/devnpc-dashboard/src/state.rs`
- Create: `crates/devnpc-dashboard/src/main.rs`
- Create: `crates/devnpc-dashboard/static/.gitkeep`

- [ ] **Step 1: 在 workspace members 中添加 dashboard**

修改根 `Cargo.toml` 的 `[workspace]` 段，将 `members` 改为：

```toml
[workspace]
members = ["crates/devnpc-core", "crates/devnpc", "crates/devnpc-dashboard"]
resolver = "2"
```

- [ ] **Step 2: 创建目录结构**

Run: `mkdir crates\devnpc-dashboard\src\storage ; mkdir crates\devnpc-dashboard\src\realtime ; mkdir crates\devnpc-dashboard\src\server ; mkdir crates\devnpc-dashboard\static`

- [ ] **Step 3: 创建 crates/devnpc-dashboard/Cargo.toml**

```toml
[package]
name = "devnpc-dashboard"
version = "0.1.0"
edition.workspace = true
license.workspace = true
description = "devnpc 可观测 Dashboard 服务端: 实时监控 + 历史分析"

[[bin]]
name = "devnpc-dashboard"
path = "src/main.rs"

[lib]
name = "devnpc_dashboard"
path = "src/lib.rs"

[dependencies]
devnpc-core = { path = "../devnpc-core" }

# 异步运行时
tokio = { workspace = true }
futures = { workspace = true }
tokio-stream = "0.1"

# Web 框架 (multipart 用于文件导入)
axum = { workspace = true, features = ["multipart"] }
tower = { workspace = true }

# 数据库
rusqlite = { workspace = true }

# 序列化
serde = { workspace = true }
serde_json = { workspace = true }

# CLI
clap = { workspace = true }

# 日志与错误
tracing = { workspace = true }
tracing-subscriber = { workspace = true }
thiserror = { workspace = true }

# 工具
chrono = { workspace = true }
uuid = { workspace = true }

# 配置加载
dotenvy = "0.15"

# 模板 (Phase 4 使用,此处仅为错误类型前置)
askama = "0.12"

# 静态资源嵌入
rust-embed = "8"
mime_guess = "2"

[dev-dependencies]
tempfile = "3"
tokio-test = "0.4"
tower = { version = "0.5", features = ["util"] }
```

- [ ] **Step 4: 创建 src/error.rs**

```rust
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
```

- [ ] **Step 5: 创建 src/state.rs**

```rust
//! 应用共享状态

use crate::realtime::RealtimeHub;
use crate::storage::queries::Storage;

/// axum 共享状态,通过 State<AppState> 注入 handler 与中间件
#[derive(Clone)]
pub struct AppState {
    /// SQLite 存储层
    pub storage: Storage,
    /// 实时事件中心
    pub hub: std::sync::Arc<RealtimeHub>,
    /// 推送鉴权 token (空字符串表示未配置)
    pub token: String,
}
```

- [ ] **Step 6: 创建 src/lib.rs**

```rust
//! devnpc-dashboard 库
//!
//! 暴露各模块供 bin 与测试使用。

pub mod auth;
pub mod error;
pub mod realtime;
pub mod server;
pub mod state;
pub mod storage;
```

- [ ] **Step 7: 创建 src/main.rs 占位**

```rust
//! devnpc-dashboard CLI 入口
//!
//! Task 11 中填充完整启动流程。

use clap::Parser;

#[derive(Parser)]
#[command(name = "devnpc-dashboard", about = "devnpc 可观测 Dashboard 服务")]
struct Cli {
    /// 监听端口 (默认 8080)
    #[arg(long, env = "DEVNPC_DASHBOARD_PORT")]
    port: Option<u16>,

    /// 监听地址 (默认 0.0.0.0)
    #[arg(long, env = "DEVNPC_DASHBOARD_HOST")]
    host: Option<String>,

    /// SQLite 数据库路径 (默认 ./devnpc-dashboard.db)
    #[arg(long, env = "DEVNPC_DASHBOARD_DB")]
    db: Option<String>,

    /// 推送鉴权 token
    #[arg(long, env = "DEVNPC_DASHBOARD_TOKEN")]
    token: Option<String>,

    /// 实时环形缓冲容量 (默认 1000)
    #[arg(long, env = "DEVNPC_DASHBOARD_REALTIME_BUFFER")]
    realtime_buffer: Option<usize>,
}

fn main() {
    let _cli = Cli::parse();
    eprintln!("devnpc-dashboard: 服务启动逻辑在 Task 11 实现");
}
```

- [ ] **Step 8: 创建 static/.gitkeep 占位**

`crates/devnpc-dashboard/static/.gitkeep` 文件内容：

```
# LayUI 静态资源占位,Phase 4 由用户放入 layui/ css/ js/ 子目录
```

- [ ] **Step 9: 创建各子模块占位文件以通过编译**

创建 `src/storage/mod.rs`：

```rust
//! 存储层模块

pub mod queries;
pub mod schema;
```

创建 `src/storage/schema.rs`：

```rust
//! SQLite schema 初始化 (Task 2 填充)
```

创建 `src/storage/queries.rs`：

```rust
//! Storage 结构与 CRUD/聚合/导入查询 (Task 3-6 填充)
```

创建 `src/realtime/mod.rs`：

```rust
//! RealtimeHub 实时事件中心 (Task 7 填充)

use std::sync::Arc;

/// 占位类型,Task 7 替换为完整实现
pub struct RealtimeHub;

impl RealtimeHub {
    pub fn new(_capacity: usize) -> Arc<Self> {
        Arc::new(RealtimeHub)
    }
}
```

创建 `src/auth.rs`：

```rust
//! 鉴权中间件 (Task 8 填充)
```

创建 `src/server/mod.rs`：

```rust
//! 路由组装 (Task 11 填充)
```

创建 `src/server/api.rs`：

```rust
//! API handler (Task 9-10 填充)
```

- [ ] **Step 10: 验证编译**

Run: `cargo check -p devnpc-dashboard`
Expected: 编译通过（可能有 unused warnings，正常）

- [ ] **Step 11: 运行错误类型测试**

Run: `cargo test -p devnpc-dashboard error::tests`
Expected: 6 个测试 PASS

- [ ] **Step 12: 提交**

Run: `git add Cargo.toml crates/devnpc-dashboard ; git commit -m "feat(dashboard): 创建 devnpc-dashboard crate 骨架与 DashboardError 错误类型"`

---

### Task 2: SQLite Schema 初始化

**Files:**
- Modify: `crates/devnpc-dashboard/src/storage/schema.rs`
- Modify: `crates/devnpc-dashboard/src/storage/queries.rs`

- [ ] **Step 1: 编写 schema.rs**

```rust
//! SQLite schema 初始化
//!
//! 建表 + 索引。WAL 模式由 Storage::open 在连接时设置。

use rusqlite::Connection;

use crate::error::Result;

/// 全部建表 SQL (IF NOT EXISTS 保证幂等)
pub const SCHEMA_SQL: &str = r#"
-- 任务主表
CREATE TABLE IF NOT EXISTS tasks (
    task_id TEXT PRIMARY KEY,
    project TEXT NOT NULL,
    mr_iid INTEGER,
    pipeline_id INTEGER,
    task_description TEXT NOT NULL,
    task_kind TEXT NOT NULL,
    model TEXT NOT NULL,
    status TEXT NOT NULL,
    started_at TEXT NOT NULL,
    finished_at TEXT,
    duration_secs INTEGER,
    total_tokens INTEGER DEFAULT 0,
    input_tokens INTEGER DEFAULT 0,
    output_tokens INTEGER DEFAULT 0,
    estimated_cost_usd REAL DEFAULT 0,
    mr_url TEXT,
    ci_url TEXT,
    summary TEXT,
    error TEXT,
    ci_retries INTEGER DEFAULT 0
);

-- 执行事件表
CREATE TABLE IF NOT EXISTS events (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    task_id TEXT NOT NULL,
    seq INTEGER NOT NULL,
    event_type TEXT NOT NULL,
    payload TEXT NOT NULL,
    created_at TEXT NOT NULL,
    FOREIGN KEY (task_id) REFERENCES tasks(task_id)
);

-- SOP 偏离记录表
CREATE TABLE IF NOT EXISTS sop_deviations (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    task_id TEXT NOT NULL,
    step TEXT NOT NULL,
    note TEXT,
    created_at TEXT NOT NULL,
    FOREIGN KEY (task_id) REFERENCES tasks(task_id)
);

-- 索引
CREATE INDEX IF NOT EXISTS idx_events_task_id ON events(task_id);
CREATE INDEX IF NOT EXISTS idx_tasks_status ON tasks(status);
CREATE INDEX IF NOT EXISTS idx_tasks_project ON tasks(project);
CREATE INDEX IF NOT EXISTS idx_tasks_started_at ON tasks(started_at);
"#;

/// 执行全部建表与索引 (幂等,可重复调用)
pub fn init_db(conn: &Connection) -> Result<()> {
    conn.execute_batch(SCHEMA_SQL)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn init_db_creates_tables() {
        let conn = Connection::open_in_memory().unwrap();
        init_db(&conn).unwrap();
        // 验证 tasks 表存在
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM tasks", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn init_db_creates_events_table() {
        let conn = Connection::open_in_memory().unwrap();
        init_db(&conn).unwrap();
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM events", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn init_db_creates_sop_deviations_table() {
        let conn = Connection::open_in_memory().unwrap();
        init_db(&conn).unwrap();
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM sop_deviations", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn init_db_is_idempotent() {
        let conn = Connection::open_in_memory().unwrap();
        init_db(&conn).unwrap();
        // 再次执行不应报错
        init_db(&conn).unwrap();
    }

    #[test]
    fn init_db_creates_indexes() {
        let conn = Connection::open_in_memory().unwrap();
        init_db(&conn).unwrap();
        // 验证索引存在 (查 sqlite_master)
        let idx_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='index' AND name LIKE 'idx_%'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(idx_count, 4);
    }
}
```

- [ ] **Step 2: 在 queries.rs 添加 Storage 结构与 open 方法**

将 `crates/devnpc-dashboard/src/storage/queries.rs` 替换为：

```rust
//! Storage 结构与数据库查询
//!
//! Arc<Mutex<Connection>> 串行化写;WAL 模式下读不阻塞写。

use std::sync::{Arc, Mutex};

use rusqlite::Connection;

use crate::error::Result;
use crate::storage::schema;

/// SQLite 存储层
#[derive(Clone)]
pub struct Storage {
    conn: Arc<Mutex<Connection>>,
}

impl Storage {
    /// 打开文件数据库,开启 WAL 并执行 schema 迁移
    pub fn open(path: &str) -> Result<Self> {
        let conn = Connection::open(path)?;
        // WAL 模式: 读不阻塞写
        conn.pragma_update(None, "journal_mode", "WAL")?;
        schema::init_db(&conn)?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    /// 打开内存数据库 (测试用)
    pub fn open_in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        schema::init_db(&conn)?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_in_memory_succeeds() {
        let s = Storage::open_in_memory();
        assert!(s.is_ok());
    }

    #[test]
    fn open_file_succeeds() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let s = Storage::open(tmp.path().to_str().unwrap());
        assert!(s.is_ok());
    }

    #[test]
    fn storage_is_clone() {
        let s = Storage::open_in_memory().unwrap();
        let _s2 = s.clone();
    }
}
```

- [ ] **Step 3: 运行 schema 测试验证通过**

Run: `cargo test -p devnpc-dashboard storage`
Expected: 8 个测试 PASS（schema 5 个 + storage 3 个）

- [ ] **Step 4: 提交**

Run: `git add crates/devnpc-dashboard/src/storage ; git commit -m "feat(dashboard): SQLite schema 初始化与 Storage 连接管理 (WAL 模式)"`

---

### Task 3: Storage 任务生命周期 CRUD

**Files:**
- Modify: `crates/devnpc-dashboard/src/storage/queries.rs`

实现 `start_task`/`insert_events`/`finish_task`/`task_exists`/`task_is_finished`/`delete_task` 及行类型 `TaskRow`/`EventRow`。`finish_task` 同时聚合 input/output tokens 与 ci_retries，并写入 sop_deviations。

- [ ] **Step 1: 在 queries.rs 顶部添加行类型定义**

在 `use` 语句之后、`Storage` 结构之前添加：

```rust
use devnpc_core::report::event_schema::{
    ExecutionEvent, TaskFinishedEvent, TaskStartedEvent,
};

// ============================================================
// 行类型
// ============================================================

/// tasks 表行映射
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TaskRow {
    pub task_id: String,
    pub project: String,
    pub mr_iid: Option<u64>,
    pub pipeline_id: Option<u64>,
    pub task_description: String,
    pub task_kind: String,
    pub model: String,
    pub status: String,
    pub started_at: String,
    pub finished_at: Option<String>,
    pub duration_secs: Option<u64>,
    pub total_tokens: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub estimated_cost_usd: f64,
    pub mr_url: Option<String>,
    pub ci_url: Option<String>,
    pub summary: Option<String>,
    pub error: Option<String>,
    pub ci_retries: u64,
}

/// events 表行映射
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct EventRow {
    pub id: i64,
    pub task_id: String,
    pub seq: i64,
    pub event_type: String,
    pub payload: serde_json::Value,
    pub created_at: String,
}
```

- [ ] **Step 2: 编写生命周期方法的失败测试**

在 `queries.rs` 的 `#[cfg(test)] mod tests` 中追加（保留已有测试）：

```rust
    use devnpc_core::report::event_schema::{
        CiStatus, ExecutionEvent, SopStepStatus, TaskFinishedEvent, TaskStartedEvent, TaskStatus,
    };

    fn sample_started(task_id: &str) -> TaskStartedEvent {
        TaskStartedEvent {
            task_id: task_id.into(),
            project: "group/proj".into(),
            mr_iid: Some(42),
            pipeline_id: Some(100),
            task_description: "修复 bug".into(),
            task_kind: "mr_comment".into(),
            started_at: "2026-08-03T10:00:00Z".into(),
            model: "deepseek-chat".into(),
        }
    }

    #[test]
    fn start_task_inserts_running_row() {
        let s = Storage::open_in_memory().unwrap();
        s.start_task(&sample_started("t1")).unwrap();
        let exists = s.task_exists("t1").unwrap();
        assert!(exists);
        let row = s.get_task("t1").unwrap().unwrap();
        assert_eq!(row.status, "running");
        assert_eq!(row.project, "group/proj");
    }

    #[test]
    fn start_task_duplicate_returns_conflict() {
        let s = Storage::open_in_memory().unwrap();
        s.start_task(&sample_started("t1")).unwrap();
        let err = s.start_task(&sample_started("t1")).unwrap_err();
        assert!(matches!(err, crate::error::DashboardError::TaskConflict(_)));
    }

    #[test]
    fn insert_events_stores_rows() {
        let s = Storage::open_in_memory().unwrap();
        s.start_task(&sample_started("t1")).unwrap();
        let events = vec![
            ExecutionEvent::LlmCall {
                iteration: 1,
                prompt_tokens: 500,
                completion_tokens: 200,
                latency_ms: 1500,
            },
            ExecutionEvent::ToolCall {
                name: "read_file".into(),
                success: true,
                latency_ms: 50,
                detail: "src/main.rs".into(),
            },
        ];
        s.insert_events("t1", &events).unwrap();
        let rows = s.list_events("t1").unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].seq, 1);
        assert_eq!(rows[1].seq, 2);
        assert_eq!(rows[0].event_type, "llm_call");
    }

    #[test]
    fn insert_events_unknown_task_returns_not_found() {
        let s = Storage::open_in_memory().unwrap();
        let events = vec![ExecutionEvent::LlmCall {
            iteration: 1,
            prompt_tokens: 10,
            completion_tokens: 5,
            latency_ms: 100,
        }];
        let err = s.insert_events("nope", &events).unwrap_err();
        assert!(matches!(err, crate::error::DashboardError::TaskNotFound(_)));
    }

    #[test]
    fn finish_task_updates_status_and_tokens() {
        let s = Storage::open_in_memory().unwrap();
        s.start_task(&sample_started("t1")).unwrap();
        s.insert_events(
            "t1",
            &vec![
                ExecutionEvent::LlmCall {
                    iteration: 1,
                    prompt_tokens: 500,
                    completion_tokens: 200,
                    latency_ms: 1500,
                },
                ExecutionEvent::LlmCall {
                    iteration: 2,
                    prompt_tokens: 300,
                    completion_tokens: 100,
                    latency_ms: 1200,
                },
            ],
        )
        .unwrap();
        let finished = TaskFinishedEvent {
            task_id: "t1".into(),
            status: TaskStatus::Success,
            duration_secs: 45,
            total_tokens: 1100,
            estimated_cost_usd: 0.05,
            mr_url: Some("https://gitlab.com/mr/42".into()),
            ci_url: None,
            summary: "已修复".into(),
            error: None,
            finished_at: "2026-08-03T10:01:00Z".into(),
        };
        s.finish_task(&finished).unwrap();
        let row = s.get_task("t1").unwrap().unwrap();
        assert_eq!(row.status, "success");
        assert_eq!(row.total_tokens, 1100);
        // 聚合: prompt_tokens 500+300=800, completion 200+100=300
        assert_eq!(row.input_tokens, 800);
        assert_eq!(row.output_tokens, 300);
        assert_eq!(row.duration_secs, Some(45));
        assert!(row.finished_at.is_some());
    }

    #[test]
    fn finish_task_aggregates_ci_retries() {
        let s = Storage::open_in_memory().unwrap();
        s.start_task(&sample_started("t1")).unwrap();
        s.insert_events(
            "t1",
            &vec![
                ExecutionEvent::CiStatus {
                    pipeline_id: 100,
                    status: CiStatus::Failed,
                    attempt: 1,
                },
                ExecutionEvent::CiStatus {
                    pipeline_id: 100,
                    status: CiStatus::Failed,
                    attempt: 2,
                },
                ExecutionEvent::CiStatus {
                    pipeline_id: 100,
                    status: CiStatus::Passed,
                    attempt: 3,
                },
            ],
        )
        .unwrap();
        let finished = TaskFinishedEvent {
            task_id: "t1".into(),
            status: TaskStatus::Success,
            duration_secs: 100,
            total_tokens: 0,
            estimated_cost_usd: 0.0,
            mr_url: None,
            ci_url: None,
            summary: "ok".into(),
            error: None,
            finished_at: "2026-08-03T10:02:00Z".into(),
        };
        s.finish_task(&finished).unwrap();
        let row = s.get_task("t1").unwrap().unwrap();
        assert_eq!(row.ci_retries, 3);
    }

    // 注意: finish_task_writes_sop_deviations 测试已移至 Task 5 (依赖 sop_stats 方法)

    #[test]
    fn finish_task_unknown_returns_not_found() {
        let s = Storage::open_in_memory().unwrap();
        let finished = TaskFinishedEvent {
            task_id: "nope".into(),
            status: TaskStatus::Failed,
            duration_secs: 0,
            total_tokens: 0,
            estimated_cost_usd: 0.0,
            mr_url: None,
            ci_url: None,
            summary: String::new(),
            error: None,
            finished_at: "2026-08-03T10:00:00Z".into(),
        };
        let err = s.finish_task(&finished).unwrap_err();
        assert!(matches!(err, crate::error::DashboardError::TaskNotFound(_)));
    }

    #[test]
    fn finish_task_twice_returns_conflict() {
        let s = Storage::open_in_memory().unwrap();
        s.start_task(&sample_started("t1")).unwrap();
        let finished = TaskFinishedEvent {
            task_id: "t1".into(),
            status: TaskStatus::Success,
            duration_secs: 10,
            total_tokens: 0,
            estimated_cost_usd: 0.0,
            mr_url: None,
            ci_url: None,
            summary: "ok".into(),
            error: None,
            finished_at: "2026-08-03T10:03:00Z".into(),
        };
        s.finish_task(&finished).unwrap();
        let err = s.finish_task(&finished).unwrap_err();
        assert!(matches!(err, crate::error::DashboardError::TaskConflict(_)));
    }

    #[test]
    fn task_is_finished_reports_correctly() {
        let s = Storage::open_in_memory().unwrap();
        s.start_task(&sample_started("t1")).unwrap();
        assert!(!s.task_is_finished("t1").unwrap());
        let finished = TaskFinishedEvent {
            task_id: "t1".into(),
            status: TaskStatus::Success,
            duration_secs: 10,
            total_tokens: 0,
            estimated_cost_usd: 0.0,
            mr_url: None,
            ci_url: None,
            summary: "ok".into(),
            error: None,
            finished_at: "2026-08-03T10:03:00Z".into(),
        };
        s.finish_task(&finished).unwrap();
        assert!(s.task_is_finished("t1").unwrap());
    }

    #[test]
    fn delete_task_removes_task_and_events() {
        let s = Storage::open_in_memory().unwrap();
        s.start_task(&sample_started("t1")).unwrap();
        s.insert_events(
            "t1",
            &vec![ExecutionEvent::LlmCall {
                iteration: 1,
                prompt_tokens: 10,
                completion_tokens: 5,
                latency_ms: 100,
            }],
        )
        .unwrap();
        s.delete_task("t1").unwrap();
        assert!(!s.task_exists("t1").unwrap());
        assert!(s.list_events("t1").unwrap().is_empty());
    }
```

注意：`get_task`/`list_events` 方法在 Step 4 中一并实现（从 Task 4 移入，因 Task 3 测试依赖它们）。`sop_stats` 相关测试已移至 Task 5。

- [ ] **Step 3: 运行测试验证编译失败**

Run: `cargo test -p devnpc-dashboard storage::queries`
Expected: 编译失败，报 `no method named start_task/insert_events/finish_task/task_exists/task_is_finished/delete_task/get_task/list_events`

- [ ] **Step 4: 实现 CRUD 方法与查询方法**

在 `Storage` 的 `impl` 块中（`open`/`open_in_memory` 之后）追加：

```rust
    /// 创建任务记录,状态=running (重复 task_id 返回 TaskConflict)
    pub fn start_task(&self, e: &TaskStartedEvent) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        if self.task_exists_locked(&conn, &e.task_id)? {
            return Err(crate::error::DashboardError::TaskConflict(format!(
                "任务 {} 已存在",
                e.task_id
            )));
        }
        conn.execute(
            "INSERT INTO tasks (task_id, project, mr_iid, pipeline_id, task_description, task_kind, model, status, started_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'running', ?8)",
            rusqlite::params![
                e.task_id, e.project, e.mr_iid, e.pipeline_id,
                e.task_description, e.task_kind, e.model, e.started_at,
            ],
        )?;
        Ok(())
    }

    /// 批量写入执行事件 (task_id 不存在返回 TaskNotFound)
    pub fn insert_events(&self, task_id: &str, events: &[ExecutionEvent]) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        if !self.task_exists_locked(&conn, task_id)? {
            return Err(crate::error::DashboardError::TaskNotFound(task_id.into()));
        }
        // 取当前最大 seq
        let max_seq: i64 = conn
            .query_row(
                "SELECT COALESCE(MAX(seq), 0) FROM events WHERE task_id = ?1",
                rusqlite::params![task_id],
                |r| r.get(0),
            )
            .unwrap_or(0);
        let now = chrono::Utc::now().to_rfc3339();
        let mut seq = max_seq;
        for ev in events {
            seq += 1;
            let payload = serde_json::to_string(ev)?;
            let event_type = match ev {
                ExecutionEvent::LlmCall { .. } => "llm_call",
                ExecutionEvent::ToolCall { .. } => "tool_call",
                ExecutionEvent::SopStep { .. } => "sop_step",
                ExecutionEvent::CiStatus { .. } => "ci_status",
                ExecutionEvent::TeamHandoff { .. } => "team_handoff",
            };
            conn.execute(
                "INSERT INTO events (task_id, seq, event_type, payload, created_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                rusqlite::params![task_id, seq, event_type, payload, now],
            )?;
        }
        Ok(())
    }

    /// 任务结束: 更新状态/汇总字段,聚合 tokens 与 ci_retries,写入 sop_deviations
    pub fn finish_task(&self, e: &TaskFinishedEvent) -> Result<()> {
        let mut conn = self.conn.lock().unwrap();
        if !self.task_exists_locked(&conn, &e.task_id)? {
            return Err(crate::error::DashboardError::TaskNotFound(e.task_id.clone()));
        }
        if self.task_is_finished_locked(&conn, &e.task_id)? {
            return Err(crate::error::DashboardError::TaskConflict(format!(
                "任务 {} 已结束",
                e.task_id
            )));
        }
        // 聚合 input/output tokens (从 llm_call 事件)
        let (input_tokens, output_tokens): (u64, u64) = conn
            .query_row(
                "SELECT \
                   COALESCE(SUM(json_extract(payload, '$.prompt_tokens')), 0), \
                   COALESCE(SUM(json_extract(payload, '$.completion_tokens')), 0) \
                 FROM events WHERE task_id = ?1 AND event_type = 'llm_call'",
                rusqlite::params![e.task_id],
                |r| Ok((r.get::<_, i64>(0)? as u64, r.get::<_, i64>(1)? as u64)),
            )
            .unwrap_or((0, 0));
        // 聚合 ci_retries (ci_status 事件的最大 attempt)
        let ci_retries: u64 = conn
            .query_row(
                "SELECT COALESCE(MAX(json_extract(payload, '$.attempt')), 0) \
                 FROM events WHERE task_id = ?1 AND event_type = 'ci_status'",
                rusqlite::params![e.task_id],
                |r| Ok(r.get::<_, i64>(0)? as u64),
            )
            .unwrap_or(0);
        let status_str = match e.status {
            devnpc_core::report::event_schema::TaskStatus::Success => "success",
            devnpc_core::report::event_schema::TaskStatus::Failed => "failed",
            devnpc_core::report::event_schema::TaskStatus::CiFailed => "ci_failed",
            devnpc_core::report::event_schema::TaskStatus::Timeout => "timeout",
        };
        conn.execute(
            "UPDATE tasks SET status = ?1, finished_at = ?2, duration_secs = ?3, \
             total_tokens = ?4, input_tokens = ?5, output_tokens = ?6, \
             estimated_cost_usd = ?7, mr_url = ?8, ci_url = ?9, summary = ?10, \
             error = ?11, ci_retries = ?12 WHERE task_id = ?13",
            rusqlite::params![
                status_str,
                e.finished_at,
                e.duration_secs as i64,
                e.total_tokens as i64,
                input_tokens as i64,
                output_tokens as i64,
                e.estimated_cost_usd,
                e.mr_url,
                e.ci_url,
                e.summary,
                e.error,
                ci_retries as i64,
                e.task_id,
            ],
        )?;
        // 写入 sop_deviations (从 sop_step 事件中 status=deviated 的)
        let now = chrono::Utc::now().to_rfc3339();
        let mut stmt = conn.prepare(
            "SELECT json_extract(payload, '$.step'), json_extract(payload, '$.note') \
             FROM events WHERE task_id = ?1 AND event_type = 'sop_step' \
             AND json_extract(payload, '$.status') = 'deviated'",
        )?;
        let deviations: Vec<(Option<String>, Option<String>)> = stmt
            .query_map(rusqlite::params![e.task_id], |r| {
                Ok((r.get::<_, Option<String>>(0)?, r.get::<_, Option<String>>(1)?))
            })?
            .filter_map(|r| r.ok())
            .collect();
        drop(stmt);
        for (step, note) in deviations {
            conn.execute(
                "INSERT INTO sop_deviations (task_id, step, note, created_at) \
                 VALUES (?1, ?2, ?3, ?4)",
                rusqlite::params![e.task_id, step, note, now],
            )?;
        }
        Ok(())
    }

    /// 任务是否存在
    pub fn task_exists(&self, task_id: &str) -> Result<bool> {
        let conn = self.conn.lock().unwrap();
        self.task_exists_locked(&conn, task_id)
    }

    /// 任务是否已结束 (status != running)
    pub fn task_is_finished(&self, task_id: &str) -> Result<bool> {
        let conn = self.conn.lock().unwrap();
        self.task_is_finished_locked(&conn, task_id)
    }

    /// 删除任务及其事件与偏离记录 (覆盖导入时使用)
    pub fn delete_task(&self, task_id: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute("DELETE FROM sop_deviations WHERE task_id = ?1", rusqlite::params![task_id])?;
        conn.execute("DELETE FROM events WHERE task_id = ?1", rusqlite::params![task_id])?;
        conn.execute("DELETE FROM tasks WHERE task_id = ?1", rusqlite::params![task_id])?;
        Ok(())
    }

    /// 查询单个任务
    pub fn get_task(&self, task_id: &str) -> Result<Option<TaskRow>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT task_id, project, mr_iid, pipeline_id, task_description, task_kind, \
             model, status, started_at, finished_at, duration_secs, total_tokens, \
             input_tokens, output_tokens, estimated_cost_usd, mr_url, ci_url, summary, \
             error, ci_retries FROM tasks WHERE task_id = ?1",
        )?;
        let row = stmt
            .query_row(rusqlite::params![task_id], |r| {
                Ok(TaskRow {
                    task_id: r.get(0)?,
                    project: r.get(1)?,
                    mr_iid: r.get::<_, Option<i64>>(2)?.map(|v| v as u64),
                    pipeline_id: r.get::<_, Option<i64>>(3)?.map(|v| v as u64),
                    task_description: r.get(4)?,
                    task_kind: r.get(5)?,
                    model: r.get(6)?,
                    status: r.get(7)?,
                    started_at: r.get(8)?,
                    finished_at: r.get(9)?,
                    duration_secs: r.get::<_, Option<i64>>(10)?.map(|v| v as u64),
                    total_tokens: r.get::<_, i64>(11)? as u64,
                    input_tokens: r.get::<_, i64>(12)? as u64,
                    output_tokens: r.get::<_, i64>(13)? as u64,
                    estimated_cost_usd: r.get(14)?,
                    mr_url: r.get(15)?,
                    ci_url: r.get(16)?,
                    summary: r.get(17)?,
                    error: r.get(18)?,
                    ci_retries: r.get::<_, i64>(19)? as u64,
                })
            })
            .ok();
        Ok(row)
    }

    /// 查询任务的事件列表 (按 seq 升序)
    pub fn list_events(&self, task_id: &str) -> Result<Vec<EventRow>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, task_id, seq, event_type, payload, created_at \
             FROM events WHERE task_id = ?1 ORDER BY seq ASC",
        )?;
        let rows: Vec<EventRow> = stmt
            .query_map(rusqlite::params![task_id], |r| {
                let payload_str: String = r.get(4)?;
                let payload: serde_json::Value =
                    serde_json::from_str(&payload_str).unwrap_or(serde_json::Value::Null);
                Ok(EventRow {
                    id: r.get(0)?,
                    task_id: r.get(1)?,
                    seq: r.get(2)?,
                    event_type: r.get(3)?,
                    payload,
                    created_at: r.get(5)?,
                })
            })?
            .filter_map(|r| r.ok())
            .collect();
        Ok(rows)
    }

    // ---- 内部辅助 (已持有锁) ----

    fn task_exists_locked(&self, conn: &Connection, task_id: &str) -> Result<bool> {
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM tasks WHERE task_id = ?1",
            rusqlite::params![task_id],
            |r| r.get(0),
        )?;
        Ok(count > 0)
    }

    fn task_is_finished_locked(&self, conn: &Connection, task_id: &str) -> Result<bool> {
        let status: Option<String> = conn.query_row(
            "SELECT status FROM tasks WHERE task_id = ?1",
            rusqlite::params![task_id],
            |r| r.get(0),
        ).ok();
        match status {
            Some(s) => Ok(s != "running"),
            None => Ok(false),
        }
    }
```

- [ ] **Step 5: 运行测试验证通过**

Run: `cargo test -p devnpc-dashboard storage::queries`
Expected: Task 3 全部测试 PASS（start/insert/finish/get_task/list_events/delete_task 等）

- [ ] **Step 6: 提交**

Run: `git add crates/devnpc-dashboard/src/storage/queries.rs ; git commit -m "feat(dashboard): Storage CRUD + 查询 (start/insert/finish/get_task/list_events/delete_task + token/ci 聚合)"`

---

### Task 4: Storage 列表查询接口

**Files:**
- Modify: `crates/devnpc-dashboard/src/storage/queries.rs`

实现 `list_tasks`（分页/过滤）及 `TaskFilter`/`TaskListResponse` 类型。（`get_task`/`list_events` 已在 Task 3 实现）

- [ ] **Step 1: 添加 TaskFilter 与 TaskListResponse 类型**

在 `queries.rs` 的行类型区域（`EventRow` 之后）追加：

```rust
/// 任务列表过滤条件
#[derive(Debug, Clone, Default)]
pub struct TaskFilter {
    pub status: Option<String>,
    pub project: Option<String>,
}

/// 任务列表响应 (带分页元信息)
#[derive(Debug, Clone, serde::Serialize)]
pub struct TaskListResponse {
    pub tasks: Vec<TaskRow>,
    pub total: usize,
    pub page: usize,
    pub size: usize,
}
```

- [ ] **Step 2: 编写 list_tasks 的失败测试**

在 `queries.rs` 测试模块中追加：

```rust
    #[test]
    fn list_tasks_pagination() {
        let s = Storage::open_in_memory().unwrap();
        for i in 0..15 {
            s.start_task(&sample_started(&format!("t{}", i))).unwrap();
        }
        let resp = s.list_tasks(1, 10, &TaskFilter::default()).unwrap();
        assert_eq!(resp.tasks.len(), 10);
        assert_eq!(resp.total, 15);
        assert_eq!(resp.page, 1);
        assert_eq!(resp.size, 10);
        let resp2 = s.list_tasks(2, 10, &TaskFilter::default()).unwrap();
        assert_eq!(resp2.tasks.len(), 5);
    }

    #[test]
    fn list_tasks_filter_by_status() {
        let s = Storage::open_in_memory().unwrap();
        s.start_task(&sample_started("t1")).unwrap();
        s.start_task(&sample_started("t2")).unwrap();
        // t2 设为 success
        let f = TaskFinishedEvent {
            task_id: "t2".into(),
            status: TaskStatus::Success,
            duration_secs: 5,
            total_tokens: 0,
            estimated_cost_usd: 0.0,
            mr_url: None,
            ci_url: None,
            summary: "ok".into(),
            error: None,
            finished_at: "2026-08-03T10:05:00Z".into(),
        };
        s.finish_task(&f).unwrap();
        let resp = s
            .list_tasks(1, 100, &TaskFilter { status: Some("running".into()), project: None })
            .unwrap();
        assert_eq!(resp.tasks.len(), 1);
        assert_eq!(resp.tasks[0].task_id, "t1");
    }

    #[test]
    fn list_tasks_filter_by_project() {
        let s = Storage::open_in_memory().unwrap();
        let mut a = sample_started("t1");
        a.project = "proj-a".into();
        let mut b = sample_started("t2");
        b.project = "proj-b".into();
        s.start_task(&a).unwrap();
        s.start_task(&b).unwrap();
        let resp = s
            .list_tasks(1, 100, &TaskFilter { status: None, project: Some("proj-a".into()) })
            .unwrap();
        assert_eq!(resp.tasks.len(), 1);
        assert_eq!(resp.tasks[0].task_id, "t1");
    }
```

- [ ] **Step 3: 运行测试验证编译失败**

Run: `cargo test -p devnpc-dashboard storage::queries`
Expected: 编译失败，`no method named list_tasks`

- [ ] **Step 4: 实现 list_tasks 方法**

在 `Storage` 的 `impl` 块中追加：

```rust
    /// 分页 + 过滤查询任务列表 (按 started_at 倒序)
    pub fn list_tasks(&self, page: usize, size: usize, filter: &TaskFilter) -> Result<TaskListResponse> {
        let conn = self.conn.lock().unwrap();
        let page = if page == 0 { 1 } else { page };
        let size = if size == 0 { 20 } else { size };
        let offset = ((page - 1) * size) as i64;

        // 动态拼接 WHERE
        let mut where_clauses: Vec<String> = Vec::new();
        let mut params: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
        if let Some(s) = &filter.status {
            where_clauses.push(format!("status = ?{}", where_clauses.len() + 1));
            params.push(Box::new(s.clone()));
        }
        if let Some(p) = &filter.project {
            where_clauses.push(format!("project = ?{}", where_clauses.len() + 1));
            params.push(Box::new(p.clone()));
        }
        let where_sql = if where_clauses.is_empty() {
            String::new()
        } else {
            format!("WHERE {}", where_clauses.join(" AND "))
        };

        // 总数
        let count_sql = format!("SELECT COUNT(*) FROM tasks {}", where_sql);
        let param_refs: Vec<&dyn rusqlite::ToSql> = params.iter().map(|p| p.as_ref()).collect();
        let total: i64 = conn.query_row(&count_sql, param_refs.as_slice(), |r| r.get(0))?;

        // 分页数据
        let list_sql = format!(
            "SELECT task_id, project, mr_iid, pipeline_id, task_description, task_kind, \
             model, status, started_at, finished_at, duration_secs, total_tokens, \
             input_tokens, output_tokens, estimated_cost_usd, mr_url, ci_url, summary, \
             error, ci_retries FROM tasks {} ORDER BY started_at DESC LIMIT ?{} OFFSET ?{}",
            where_sql,
            params.len() + 1,
            params.len() + 2,
        );
        let mut all_params: Vec<Box<dyn rusqlite::ToSql>> = params;
        all_params.push(Box::new(size as i64));
        all_params.push(Box::new(offset));
        let all_refs: Vec<&dyn rusqlite::ToSql> = all_params.iter().map(|p| p.as_ref()).collect();

        let mut stmt = conn.prepare(&list_sql)?;
        let tasks: Vec<TaskRow> = stmt
            .query_map(all_refs.as_slice(), |r| {
                Ok(TaskRow {
                    task_id: r.get(0)?,
                    project: r.get(1)?,
                    mr_iid: r.get::<_, Option<i64>>(2)?.map(|v| v as u64),
                    pipeline_id: r.get::<_, Option<i64>>(3)?.map(|v| v as u64),
                    task_description: r.get(4)?,
                    task_kind: r.get(5)?,
                    model: r.get(6)?,
                    status: r.get(7)?,
                    started_at: r.get(8)?,
                    finished_at: r.get(9)?,
                    duration_secs: r.get::<_, Option<i64>>(10)?.map(|v| v as u64),
                    total_tokens: r.get::<_, i64>(11)? as u64,
                    input_tokens: r.get::<_, i64>(12)? as u64,
                    output_tokens: r.get::<_, i64>(13)? as u64,
                    estimated_cost_usd: r.get(14)?,
                    mr_url: r.get(15)?,
                    ci_url: r.get(16)?,
                    summary: r.get(17)?,
                    error: r.get(18)?,
                    ci_retries: r.get::<_, i64>(19)? as u64,
                })
            })?
            .filter_map(|r| r.ok())
            .collect();

        Ok(TaskListResponse {
            tasks,
            total: total as usize,
            page,
            size,
        })
    }
```

- [ ] **Step 5: 运行测试验证通过**

Run: `cargo test -p devnpc-dashboard storage::queries`
Expected: Task 3/4 全部测试 PASS

- [ ] **Step 6: 提交**

Run: `git add crates/devnpc-dashboard/src/storage/queries.rs ; git commit -m "feat(dashboard): Storage list_tasks 分页过滤查询 + TaskFilter/TaskListResponse"`

---

### Task 5: Storage 聚合查询

**Files:**
- Modify: `crates/devnpc-dashboard/src/storage/queries.rs`

实现 `trends`/`cost_breakdown`/`ci_stats`/`sop_stats` 及结果类型。

- [ ] **Step 1: 添加聚合结果类型**

在 `queries.rs` 的类型区域追加：

```rust
/// 趋势统计单点
#[derive(Debug, Clone, serde::Serialize)]
pub struct TrendPoint {
    pub date: String,
    pub total: u64,
    pub success: u64,
    pub failed: u64,
    pub avg_duration_secs: f64,
    pub total_tokens: u64,
    pub total_cost_usd: f64,
}

/// 趋势统计结果
#[derive(Debug, Clone, serde::Serialize)]
pub struct TrendsData {
    pub days: u32,
    pub points: Vec<TrendPoint>,
}

/// 成本聚合桶
#[derive(Debug, Clone, serde::Serialize)]
pub struct CostBucket {
    pub key: String,
    pub total_cost_usd: f64,
    pub total_tokens: u64,
    pub task_count: u64,
}

/// CI 自愈统计
#[derive(Debug, Clone, serde::Serialize)]
pub struct CiStats {
    pub total_failed: u64,
    pub auto_healed: u64,
    pub heal_rate: f64,
    pub avg_retries: f64,
    pub failed_tasks: Vec<TaskRow>,
}

/// SOP 偏离记录行
#[derive(Debug, Clone, serde::Serialize)]
pub struct SopDeviationRow {
    pub id: i64,
    pub task_id: String,
    pub step: String,
    pub note: Option<String>,
    pub created_at: String,
}
```

- [ ] **Step 2: 编写聚合查询的失败测试**

在测试模块追加：

```rust
    #[test]
    fn finish_task_writes_sop_deviations() {
        let s = Storage::open_in_memory().unwrap();
        s.start_task(&sample_started("t1")).unwrap();
        s.insert_events(
            "t1",
            &vec![
                ExecutionEvent::SopStep {
                    step: "analyze".into(),
                    status: SopStepStatus::Completed,
                    note: None,
                },
                ExecutionEvent::SopStep {
                    step: "implement".into(),
                    status: SopStepStatus::Deviated,
                    note: Some("跳过单测".into()),
                },
            ],
        )
        .unwrap();
        let finished = TaskFinishedEvent {
            task_id: "t1".into(),
            status: TaskStatus::Success,
            duration_secs: 10,
            total_tokens: 0,
            estimated_cost_usd: 0.0,
            mr_url: None,
            ci_url: None,
            summary: "ok".into(),
            error: None,
            finished_at: "2026-08-03T10:03:00Z".into(),
        };
        s.finish_task(&finished).unwrap();
        let devs = s.sop_stats().unwrap();
        assert_eq!(devs.len(), 1);
        assert_eq!(devs[0].step, "implement");
    }

    #[test]
    fn trends_returns_points() {
        let s = Storage::open_in_memory().unwrap();
        s.start_task(&sample_started("t1")).unwrap();
        let f = TaskFinishedEvent {
            task_id: "t1".into(),
            status: TaskStatus::Success,
            duration_secs: 30,
            total_tokens: 1000,
            estimated_cost_usd: 0.05,
            mr_url: None,
            ci_url: None,
            summary: "ok".into(),
            error: None,
            finished_at: "2026-08-03T10:05:00Z".into(),
        };
        s.finish_task(&f).unwrap();
        let data = s.trends(7).unwrap();
        assert_eq!(data.days, 7);
        assert!(!data.points.is_empty());
        let total: u64 = data.points.iter().map(|p| p.total).sum();
        assert_eq!(total, 1);
        let success: u64 = data.points.iter().map(|p| p.success).sum();
        assert_eq!(success, 1);
    }

    #[test]
    fn cost_breakdown_by_project() {
        let s = Storage::open_in_memory().unwrap();
        let mut a = sample_started("t1");
        a.project = "proj-a".into();
        let mut b = sample_started("t2");
        b.project = "proj-b".into();
        s.start_task(&a).unwrap();
        s.start_task(&b).unwrap();
        s.finish_task(&TaskFinishedEvent {
            task_id: "t1".into(),
            status: TaskStatus::Success,
            duration_secs: 10,
            total_tokens: 500,
            estimated_cost_usd: 0.02,
            mr_url: None,
            ci_url: None,
            summary: "ok".into(),
            error: None,
            finished_at: "2026-08-03T10:05:00Z".into(),
        })
        .unwrap();
        s.finish_task(&TaskFinishedEvent {
            task_id: "t2".into(),
            status: TaskStatus::Success,
            duration_secs: 10,
            total_tokens: 300,
            estimated_cost_usd: 0.03,
            mr_url: None,
            ci_url: None,
            summary: "ok".into(),
            error: None,
            finished_at: "2026-08-03T10:06:00Z".into(),
        })
        .unwrap();
        let buckets = s.cost_breakdown("project").unwrap();
        assert_eq!(buckets.len(), 2);
        let total_cost: f64 = buckets.iter().map(|b| b.total_cost_usd).sum();
        assert!((total_cost - 0.05).abs() < 1e-9);
    }

    #[test]
    fn cost_breakdown_invalid_group_returns_error() {
        let s = Storage::open_in_memory().unwrap();
        assert!(s.cost_breakdown("invalid").is_err());
    }

    #[test]
    fn ci_stats_counts_failures_and_heals() {
        let s = Storage::open_in_memory().unwrap();
        s.start_task(&sample_started("t1")).unwrap();
        s.insert_events(
            "t1",
            &vec![ExecutionEvent::CiStatus {
                pipeline_id: 100,
                status: CiStatus::Failed,
                attempt: 2,
            }],
        )
        .unwrap();
        s.finish_task(&TaskFinishedEvent {
            task_id: "t1".into(),
            status: TaskStatus::Success,
            duration_secs: 10,
            total_tokens: 0,
            estimated_cost_usd: 0.0,
            mr_url: None,
            ci_url: None,
            summary: "ok".into(),
            error: None,
            finished_at: "2026-08-03T10:05:00Z".into(),
        })
        .unwrap();
        s.start_task(&sample_started("t2")).unwrap();
        s.finish_task(&TaskFinishedEvent {
            task_id: "t2".into(),
            status: TaskStatus::CiFailed,
            duration_secs: 10,
            total_tokens: 0,
            estimated_cost_usd: 0.0,
            mr_url: None,
            ci_url: None,
            summary: "fail".into(),
            error: Some("CI 失败".into()),
            finished_at: "2026-08-03T10:06:00Z".into(),
        })
        .unwrap();
        let stats = s.ci_stats().unwrap();
        // t1: ci_retries=2 + status=success -> auto_healed
        // t2: status=ci_failed -> total_failed
        assert_eq!(stats.total_failed, 1);
        assert_eq!(stats.auto_healed, 1);
        assert_eq!(stats.failed_tasks.len(), 1);
    }

    #[test]
    fn sop_stats_returns_deviations() {
        let s = Storage::open_in_memory().unwrap();
        s.start_task(&sample_started("t1")).unwrap();
        s.insert_events(
            "t1",
            &vec![ExecutionEvent::SopStep {
                step: "test".into(),
                status: SopStepStatus::Deviated,
                note: Some("跳过测试".into()),
            }],
        )
        .unwrap();
        s.finish_task(&TaskFinishedEvent {
            task_id: "t1".into(),
            status: TaskStatus::Success,
            duration_secs: 10,
            total_tokens: 0,
            estimated_cost_usd: 0.0,
            mr_url: None,
            ci_url: None,
            summary: "ok".into(),
            error: None,
            finished_at: "2026-08-03T10:05:00Z".into(),
        })
        .unwrap();
        let devs = s.sop_stats().unwrap();
        assert_eq!(devs.len(), 1);
        assert_eq!(devs[0].step, "test");
        assert_eq!(devs[0].note.as_deref(), Some("跳过测试"));
    }
```

- [ ] **Step 3: 运行测试验证编译失败**

Run: `cargo test -p devnpc-dashboard storage::queries`
Expected: 编译失败，`no method named trends/cost_breakdown/ci_stats/sop_stats`

- [ ] **Step 4: 实现聚合查询方法**

在 `Storage` 的 `impl` 块中追加：

```rust
    /// 趋势统计 (按天聚合最近 N 天)
    pub fn trends(&self, days: u32) -> Result<TrendsData> {
        let conn = self.conn.lock().unwrap();
        let modifier = format!("-{} days", days);
        let mut stmt = conn.prepare(
            "SELECT date(started_at) as d, COUNT(*), \
             SUM(CASE WHEN status='success' THEN 1 ELSE 0 END), \
             SUM(CASE WHEN status IN ('failed','ci_failed','timeout') THEN 1 ELSE 0 END), \
             AVG(duration_secs), \
             SUM(total_tokens), \
             SUM(estimated_cost_usd) \
             FROM tasks WHERE started_at >= datetime('now', ?1) \
             GROUP BY d ORDER BY d ASC",
        )?;
        let points: Vec<TrendPoint> = stmt
            .query_map(rusqlite::params![modifier], |r| {
                Ok(TrendPoint {
                    date: r.get::<_, Option<String>>(0)?.unwrap_or_default(),
                    total: r.get::<_, i64>(1)? as u64,
                    success: r.get::<_, i64>(2)? as u64,
                    failed: r.get::<_, i64>(3)? as u64,
                    avg_duration_secs: r.get::<_, Option<f64>>(4)?.unwrap_or(0.0),
                    total_tokens: r.get::<_, i64>(5)? as u64,
                    total_cost_usd: r.get::<_, Option<f64>>(6)?.unwrap_or(0.0),
                })
            })?
            .filter_map(|r| r.ok())
            .collect();
        Ok(TrendsData { days, points })
    }

    /// 成本聚合 (group_by: project/model/kind)
    pub fn cost_breakdown(&self, group_by: &str) -> Result<Vec<CostBucket>> {
        let column = match group_by {
            "project" => "project",
            "model" => "model",
            "kind" => "task_kind",
            _ => {
                return Err(crate::error::DashboardError::ImportFormat(format!(
                    "无效的 group_by: {} (允许 project/model/kind)",
                    group_by
                )))
            }
        };
        let conn = self.conn.lock().unwrap();
        let sql = format!(
            "SELECT {} as k, SUM(estimated_cost_usd), SUM(total_tokens), COUNT(*) \
             FROM tasks WHERE status != 'running' GROUP BY k ORDER BY SUM(estimated_cost_usd) DESC",
            column
        );
        let mut stmt = conn.prepare(&sql)?;
        let buckets: Vec<CostBucket> = stmt
            .query_map([], |r| {
                Ok(CostBucket {
                    key: r.get::<_, Option<String>>(0)?.unwrap_or_default(),
                    total_cost_usd: r.get::<_, Option<f64>>(1)?.unwrap_or(0.0),
                    total_tokens: r.get::<_, i64>(2)? as u64,
                    task_count: r.get::<_, i64>(3)? as u64,
                })
            })?
            .filter_map(|r| r.ok())
            .collect();
        Ok(buckets)
    }

    /// CI 自愈统计
    pub fn ci_stats(&self) -> Result<CiStats> {
        let conn = self.conn.lock().unwrap();
        let total_failed: u64 = conn
            .query_row(
                "SELECT COUNT(*) FROM tasks WHERE status IN ('failed','ci_failed','timeout')",
                [],
                |r| Ok(r.get::<_, i64>(0)? as u64),
            )
            .unwrap_or(0);
        let auto_healed: u64 = conn
            .query_row(
                "SELECT COUNT(*) FROM tasks WHERE ci_retries > 0 AND status = 'success'",
                [],
                |r| Ok(r.get::<_, i64>(0)? as u64),
            )
            .unwrap_or(0);
        let avg_retries: f64 = conn
            .query_row(
                "SELECT AVG(ci_retries) FROM tasks WHERE ci_retries > 0",
                [],
                |r| Ok(r.get::<_, Option<f64>>(0)?.unwrap_or(0.0)),
            )
            .unwrap_or(0.0);
        let heal_rate = if total_failed + auto_healed == 0 {
            0.0
        } else {
            auto_healed as f64 / (total_failed + auto_healed) as f64
        };
        // 失败任务列表
        let mut stmt = conn.prepare(
            "SELECT task_id, project, mr_iid, pipeline_id, task_description, task_kind, \
             model, status, started_at, finished_at, duration_secs, total_tokens, \
             input_tokens, output_tokens, estimated_cost_usd, mr_url, ci_url, summary, \
             error, ci_retries FROM tasks WHERE status IN ('failed','ci_failed','timeout') \
             ORDER BY started_at DESC LIMIT 100",
        )?;
        let failed_tasks: Vec<TaskRow> = stmt
            .query_map([], |r| {
                Ok(TaskRow {
                    task_id: r.get(0)?,
                    project: r.get(1)?,
                    mr_iid: r.get::<_, Option<i64>>(2)?.map(|v| v as u64),
                    pipeline_id: r.get::<_, Option<i64>>(3)?.map(|v| v as u64),
                    task_description: r.get(4)?,
                    task_kind: r.get(5)?,
                    model: r.get(6)?,
                    status: r.get(7)?,
                    started_at: r.get(8)?,
                    finished_at: r.get(9)?,
                    duration_secs: r.get::<_, Option<i64>>(10)?.map(|v| v as u64),
                    total_tokens: r.get::<_, i64>(11)? as u64,
                    input_tokens: r.get::<_, i64>(12)? as u64,
                    output_tokens: r.get::<_, i64>(13)? as u64,
                    estimated_cost_usd: r.get(14)?,
                    mr_url: r.get(15)?,
                    ci_url: r.get(16)?,
                    summary: r.get(17)?,
                    error: r.get(18)?,
                    ci_retries: r.get::<_, i64>(19)? as u64,
                })
            })?
            .filter_map(|r| r.ok())
            .collect();
        Ok(CiStats {
            total_failed,
            auto_healed,
            heal_rate,
            avg_retries,
            failed_tasks,
        })
    }

    /// SOP 偏离记录 (最近 100 条)
    pub fn sop_stats(&self) -> Result<Vec<SopDeviationRow>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, task_id, step, note, created_at \
             FROM sop_deviations ORDER BY created_at DESC LIMIT 100",
        )?;
        let rows: Vec<SopDeviationRow> = stmt
            .query_map([], |r| {
                Ok(SopDeviationRow {
                    id: r.get(0)?,
                    task_id: r.get(1)?,
                    step: r.get(2)?,
                    note: r.get(3)?,
                    created_at: r.get(4)?,
                })
            })?
            .filter_map(|r| r.ok())
            .collect();
        Ok(rows)
    }
```

- [ ] **Step 5: 运行全部 storage 测试验证通过**

Run: `cargo test -p devnpc-dashboard storage`
Expected: 全部 storage 测试 PASS（schema + queries 所有）

- [ ] **Step 6: 提交**

Run: `git add crates/devnpc-dashboard/src/storage/queries.rs ; git commit -m "feat(dashboard): Storage 聚合查询 (trends/cost_breakdown/ci_stats/sop_stats)"`

---

### Task 6: Storage JSONL 导入

**Files:**
- Modify: `crates/devnpc-dashboard/src/storage/queries.rs`

实现 `import_from_jsonl`：逐行解析 `EventLogEntry`，幂等处理（已 finish 跳过，未 finish 覆盖）。

- [ ] **Step 1: 编写导入的失败测试**

在测试模块追加：

```rust
    fn sample_jsonl(task_id: &str, finished: bool) -> String {
        let mut lines = Vec::new();
        // 注意: EventLogEntry::TaskStarted 用 #[serde(flatten)] data,
        // task_id 字段在外层, data 内也有 task_id, JSON 需保证两者一致
        let started_line = serde_json::json!({
            "kind": "task_started",
            "task_id": task_id,
            "project": "group/proj",
            "mr_iid": 42,
            "pipeline_id": 100,
            "task_description": "修复 bug",
            "task_kind": "mr_comment",
            "started_at": "2026-08-03T10:00:00Z",
            "model": "deepseek-chat",
        });
        lines.push(started_line.to_string());
        lines.push(serde_json::json!({
            "kind": "execution",
            "task_id": task_id,
            "event": { "type": "llm_call", "iteration": 1, "prompt_tokens": 100, "completion_tokens": 50, "latency_ms": 500 }
        }).to_string());
        lines.push(serde_json::json!({
            "kind": "execution",
            "task_id": task_id,
            "event": { "type": "tool_call", "name": "read_file", "success": true, "latency_ms": 10, "detail": "a.rs" }
        }).to_string());
        if finished {
            lines.push(serde_json::json!({
                "kind": "task_finished",
                "task_id": task_id,
                "status": "success",
                "duration_secs": 45,
                "total_tokens": 150,
                "estimated_cost_usd": 0.01,
                "mr_url": "https://gitlab.com/mr/42",
                "ci_url": null,
                "summary": "已修复",
                "error": null,
                "finished_at": "2026-08-03T10:01:00Z"
            }).to_string());
        }
        lines.join("\n")
    }

    #[test]
    fn import_new_task_succeeds() {
        let s = Storage::open_in_memory().unwrap();
        let content = sample_jsonl("imp-1", true);
        let result = s.import_from_jsonl(&content).unwrap();
        assert_eq!(result.task_id, "imp-1");
        assert!(!result.skipped);
        assert_eq!(result.events_count, 2);
        let row = s.get_task("imp-1").unwrap().unwrap();
        assert_eq!(row.status, "success");
        assert_eq!(row.total_tokens, 150);
    }

    #[test]
    fn import_finished_task_is_skipped() {
        let s = Storage::open_in_memory().unwrap();
        let content = sample_jsonl("imp-2", true);
        s.import_from_jsonl(&content).unwrap();
        // 再次导入 -> 已 finish,跳过
        let result = s.import_from_jsonl(&content).unwrap();
        assert!(result.skipped);
    }

    #[test]
    fn import_running_task_overwrites() {
        let s = Storage::open_in_memory().unwrap();
        // 先导入一个未 finish 的 (running)
        let running_content = sample_jsonl("imp-3", false);
        s.import_from_jsonl(&running_content).unwrap();
        assert_eq!(s.get_task("imp-3").unwrap().unwrap().status, "running");
        // 再导入完整版本 -> 覆盖
        let full_content = sample_jsonl("imp-3", true);
        let result = s.import_from_jsonl(&full_content).unwrap();
        assert!(!result.skipped);
        assert_eq!(s.get_task("imp-3").unwrap().unwrap().status, "success");
    }

    #[test]
    fn import_invalid_format_returns_error() {
        let s = Storage::open_in_memory().unwrap();
        let bad = "not a json line\n{also bad}";
        let err = s.import_from_jsonl(bad).unwrap_err();
        assert!(matches!(err, crate::error::DashboardError::ImportFormat(_)));
    }

    #[test]
    fn import_missing_task_started_returns_error() {
        let s = Storage::open_in_memory().unwrap();
        // 第一行不是 task_started
        let bad = serde_json::json!({
            "kind": "execution",
            "task_id": "x",
            "event": { "type": "llm_call", "iteration": 1, "prompt_tokens": 1, "completion_tokens": 1, "latency_ms": 1 }
        }).to_string();
        let err = s.import_from_jsonl(&bad).unwrap_err();
        assert!(matches!(err, crate::error::DashboardError::ImportFormat(_)));
    }
```

- [ ] **Step 2: 运行测试验证编译失败**

Run: `cargo test -p devnpc-dashboard storage::queries::tests::import`
Expected: 编译失败，`no method named import_from_jsonl`

- [ ] **Step 3: 实现 import_from_jsonl**

在 `Storage` 的 `impl` 块中追加：

```rust
    /// 从 JSONL 内容导入任务 (幂等: 已 finish 跳过, 未 finish 覆盖)
    pub fn import_from_jsonl(&self, content: &str) -> Result<ImportResult> {
        use devnpc_core::report::event_schema::EventLogEntry;

        // 先解析全部行
        let mut entries: Vec<EventLogEntry> = Vec::new();
        for (i, line) in content.lines().enumerate() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let entry: EventLogEntry = serde_json::from_str(line).map_err(|e| {
                crate::error::DashboardError::ImportFormat(format!("第 {} 行解析失败: {}", i + 1, e))
            })?;
            entries.push(entry);
        }
        if entries.is_empty() {
            return Err(crate::error::DashboardError::ImportFormat(
                "文件为空".into(),
            ));
        }
        // 第一行必须是 TaskStarted
        let task_id = match &entries[0] {
            EventLogEntry::TaskStarted { task_id, .. } => task_id.clone(),
            _ => {
                return Err(crate::error::DashboardError::ImportFormat(
                    "第一行必须是 task_started".into(),
                ))
            }
        };
        // 幂等: 已 finish -> 跳过
        if self.task_exists(&task_id)? && self.task_is_finished(&task_id)? {
            return Ok(ImportResult {
                task_id,
                events_count: 0,
                skipped: true,
            });
        }
        // 存在但未 finish -> 覆盖 (先删除)
        if self.task_exists(&task_id)? {
            self.delete_task(&task_id)?;
        }
        // 重新写入
        let mut events_count = 0usize;
        let mut execution_events: Vec<ExecutionEvent> = Vec::new();
        let mut finished_event: Option<TaskFinishedEvent> = None;
        for entry in &entries {
            match entry {
                EventLogEntry::TaskStarted { data, .. } => {
                    self.start_task(data)?;
                }
                EventLogEntry::Execution { event, .. } => {
                    execution_events.push(event.clone());
                    events_count += 1;
                }
                EventLogEntry::TaskFinished { data, .. } => {
                    finished_event = Some(data.clone());
                }
            }
        }
        if !execution_events.is_empty() {
            self.insert_events(&task_id, &execution_events)?;
        }
        if let Some(fin) = finished_event {
            self.finish_task(&fin)?;
        }
        Ok(ImportResult {
            task_id,
            events_count,
            skipped: false,
        })
    }
```

需要在文件顶部 `use` 中补充 `ImportResult`。将现有的 `use devnpc_core::report::event_schema::{...}` 改为：

```rust
use devnpc_core::report::event_schema::{
    ExecutionEvent, ImportResult, TaskFinishedEvent, TaskStartedEvent,
};
```

- [ ] **Step 4: 运行测试验证通过**

Run: `cargo test -p devnpc-dashboard storage`
Expected: 全部 storage 测试 PASS（含导入 5 个）

- [ ] **Step 5: 提交**

Run: `git add crates/devnpc-dashboard/src/storage/queries.rs ; git commit -m "feat(dashboard): Storage JSONL 导入 (幂等处理: 已 finish 跳过/未 finish 覆盖)"`

---

### Task 7: RealtimeHub 实时事件中心

**Files:**
- Modify: `crates/devnpc-dashboard/src/realtime/mod.rs`

实现环形缓冲（`VecDeque` 容量上限）+ `broadcast` 广播 + `subscribe` 返回历史 + 实时流。

- [ ] **Step 1: 替换 realtime/mod.rs 为完整实现（含测试）**

将 `crates/devnpc-dashboard/src/realtime/mod.rs` 替换为：

```rust
//! RealtimeHub 实时事件中心
//!
//! 内存环形缓冲 (VecDeque 容量上限) + broadcast 广播。
//! subscribe() 先回放缓冲历史,再推送实时事件。

use std::collections::{HashSet, VecDeque};
use std::sync::Arc;

use devnpc_core::report::event_schema::ExecutionEvent;
use futures::stream::{self, Stream};
use futures::StreamExt;
use tokio::sync::{broadcast, RwLock};
use tokio_stream::wrappers::BroadcastStream;

/// 推送到 SSE 订阅者的事件
#[derive(Debug, Clone, serde::Serialize)]
pub struct RealtimeEvent {
    pub task_id: String,
    pub event: ExecutionEvent,
    pub timestamp: String,
}

/// 实时事件中心
pub struct RealtimeHub {
    /// 环形缓冲 (最近 N 条事件)
    buffer: RwLock<VecDeque<RealtimeEvent>>,
    /// broadcast 发送端
    tx: broadcast::Sender<RealtimeEvent>,
    /// 缓冲容量
    buffer_capacity: usize,
    /// 当前 running 任务集合
    running_tasks: RwLock<HashSet<String>>,
}

impl RealtimeHub {
    pub fn new(buffer_capacity: usize) -> Self {
        let (tx, _) = broadcast::channel(buffer_capacity.max(16));
        Self {
            buffer: RwLock::new(VecDeque::with_capacity(buffer_capacity)),
            tx,
            buffer_capacity,
            running_tasks: RwLock::new(HashSet::new()),
        }
    }

    /// 标记任务启动 (加入 running 集合)
    pub async fn task_started(&self, task_id: &str) {
        self.running_tasks.write().await.insert(task_id.to_string());
    }

    /// 标记任务结束 (移出 running 集合)
    pub async fn task_finished(&self, task_id: &str) {
        self.running_tasks.write().await.remove(task_id);
    }

    /// 推送事件到缓冲与所有订阅者
    pub async fn push_events(&self, task_id: &str, events: &[ExecutionEvent]) {
        let now = chrono::Utc::now().to_rfc3339();
        let mut buf = self.buffer.write().await;
        for ev in events {
            let re = RealtimeEvent {
                task_id: task_id.to_string(),
                event: ev.clone(),
                timestamp: now.clone(),
            };
            // 环形缓冲: 满则淘汰最旧
            if buf.len() >= self.buffer_capacity {
                buf.pop_front();
            }
            buf.push_back(re.clone());
            // broadcast: 忽略无订阅者的错误
            let _ = self.tx.send(re);
        }
    }

    /// 订阅事件流: 先回放缓冲历史,再接收实时事件
    pub async fn subscribe(&self) -> impl Stream<Item = RealtimeEvent> {
        // 克隆历史快照
        let history: Vec<RealtimeEvent> = {
            let buf = self.buffer.read().await;
            buf.iter().cloned().collect()
        };
        let rx = self.tx.subscribe();
        let live = BroadcastStream::new(rx).filter_map(|r| async move { r.ok() });
        stream::iter(history).chain(live)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use devnpc_core::report::event_schema::ExecutionEvent;

    fn llm(i: u32) -> ExecutionEvent {
        ExecutionEvent::LlmCall {
            iteration: i,
            prompt_tokens: 10,
            completion_tokens: 5,
            latency_ms: 100,
        }
    }

    #[tokio::test]
    async fn push_events_stores_in_buffer() {
        let hub = RealtimeHub::new(100);
        hub.push_events("t1", &[llm(1), llm(2)]).await;
        let buf = hub.buffer.read().await;
        assert_eq!(buf.len(), 2);
        assert_eq!(buf[0].task_id, "t1");
    }

    #[tokio::test]
    async fn buffer_evicts_oldest_when_full() {
        let hub = RealtimeHub::new(2);
        hub.push_events("t1", &[llm(1)]).await;
        hub.push_events("t1", &[llm(2)]).await;
        hub.push_events("t1", &[llm(3)]).await;
        let buf = hub.buffer.read().await;
        assert_eq!(buf.len(), 2);
        // 最旧的 llm(1) 被淘汰
        match &buf[0].event {
            ExecutionEvent::LlmCall { iteration, .. } => assert_eq!(*iteration, 2),
            _ => panic!(),
        }
    }

    #[tokio::test]
    async fn task_started_and_finished() {
        let hub = RealtimeHub::new(100);
        hub.task_started("t1").await;
        assert!(hub.running_tasks.read().await.contains("t1"));
        hub.task_finished("t1").await;
        assert!(!hub.running_tasks.read().await.contains("t1"));
    }

    #[tokio::test]
    async fn subscribe_receives_history_then_live() {
        let hub = Arc::new(RealtimeHub::new(100));
        // 先推一条历史
        hub.push_events("t1", &[llm(1)]).await;
        // 订阅 (应包含历史)
        let mut stream = hub.subscribe().await;
        // 读取历史
        let first = stream.next().await;
        assert!(first.is_some());
        // 推送实时
        hub.push_events("t1", &[llm(2)]).await;
        let second = stream.next().await;
        assert!(second.is_some());
    }

    #[tokio::test]
    async fn multiple_subscribers_all_receive() {
        let hub = Arc::new(RealtimeHub::new(100));
        let mut s1 = hub.subscribe().await;
        let mut s2 = hub.subscribe().await;
        hub.push_events("t1", &[llm(1)]).await;
        assert!(s1.next().await.is_some());
        assert!(s2.next().await.is_some());
    }
}
```

注意：`subscribe()` 方法内部使用了 `filter_map` 和 `chain`（来自 `StreamExt`），已在文件顶部导入。测试模块中 `use super::*;` 会自动引入该 trait。

- [ ] **Step 2: 运行测试验证通过**

Run: `cargo test -p devnpc-dashboard realtime`
Expected: 5 个测试 PASS

- [ ] **Step 3: 提交**

Run: `git add crates/devnpc-dashboard/src/realtime/mod.rs ; git commit -m "feat(dashboard): RealtimeHub 环形缓冲 + broadcast 广播 + 历史回放订阅"`

---

### Task 8: 鉴权中间件

**Files:**
- Modify: `crates/devnpc-dashboard/src/auth.rs`

实现 `require_token` 中间件：校验 `X-Devnpc-Token` header，空配置返回 403，不匹配返回 401。

- [ ] **Step 1: 替换 auth.rs 为完整实现（含测试）**

将 `crates/devnpc-dashboard/src/auth.rs` 替换为：

```rust
//! 推送鉴权中间件
//!
//! 校验 X-Devnpc-Token header。token 未配置时返回 403,
//! 不匹配时返回 401。

use axum::body::Body;
use axum::extract::State;
use axum::http::{Request, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};

use crate::state::AppState;

/// 校验推送 token 的中间件
pub async fn require_token(
    State(state): State<AppState>,
    req: Request<Body>,
    next: Next,
) -> Response {
    if state.token.is_empty() {
        return (StatusCode::FORBIDDEN, "DEVNPC_DASHBOARD_TOKEN 未配置").into_response();
    }
    match req.headers().get("X-Devnpc-Token").and_then(|v| v.to_str().ok()) {
        Some(t) if t == state.token => next.run(req).await,
        _ => (StatusCode::UNAUTHORIZED, "无效的推送 token").into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::realtime::RealtimeHub;
    use crate::storage::queries::Storage;
    use axum::body::Body;
    use axum::http::Request;
    use std::sync::Arc;
    use tower::ServiceExt;

    fn make_state(token: &str) -> AppState {
        AppState {
            storage: Storage::open_in_memory().unwrap(),
            hub: Arc::new(RealtimeHub::new(100)),
            token: token.to_string(),
        }
    }

    async fn run_middleware(state: AppState, token_header: Option<&str>) -> StatusCode {
        let middleware = from_fn_with_state(state.clone(), require_token);
        let mut req = Request::builder()
            .uri("/api/events/start")
            .body(Body::empty())
            .unwrap();
        if let Some(t) = token_header {
            req.headers_mut().insert("X-Devnpc-Token", t.parse().unwrap());
        }
        // 用一个简单 handler 作为 next
        let app = Router::new()
            .route("/api/events/start", get(|| async { "ok" }))
            .layer(middleware)
            .with_state(state);
        let resp = app.oneshot(req).await.unwrap();
        resp.status()
    }

    use axum::middleware::from_fn_with_state;
    use axum::routing::get;
    use axum::Router;

    #[tokio::test]
    async fn empty_token_returns_403() {
        let state = make_state("");
        let status = run_middleware(state, None).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn missing_header_returns_401() {
        let state = make_state("secret");
        let status = run_middleware(state, None).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn wrong_token_returns_401() {
        let state = make_state("secret");
        let status = run_middleware(state, Some("wrong")).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn correct_token_passes_through() {
        let state = make_state("secret");
        let status = run_middleware(state, Some("secret")).await;
        assert_eq!(status, StatusCode::OK);
    }
}
```

- [ ] **Step 2: 运行测试验证通过**

Run: `cargo test -p devnpc-dashboard auth`
Expected: 4 个测试 PASS

- [ ] **Step 3: 提交**

Run: `git add crates/devnpc-dashboard/src/auth.rs ; git commit -m "feat(dashboard): 推送 token 鉴权中间件 (403 未配置/401 不匹配)"`

---

### Task 9: 推送 API 路由

**Files:**
- Modify: `crates/devnpc-dashboard/src/server/api.rs`

实现 4 个推送 API handler：`start_task`/`batch_events`/`finish_task`/`import_events`（multipart）。

- [ ] **Step 1: 编写 api.rs 推送 handler 与测试**

将 `crates/devnpc-dashboard/src/server/api.rs` 替换为：

```rust
//! Dashboard API handler
//!
//! 推送 API (token 鉴权): /api/events/*
//! 辅助 API (无鉴权): /api/tasks/*, /api/stats/* (Task 10)

use axum::extract::{Multipart, Path, Query, State};
use axum::response::Json;
use axum::Json as JsonExtractor;
use serde::Deserialize;

use devnpc_core::report::event_schema::{
    BatchEventsRequest, ImportResult, TaskFinishedEvent, TaskStartedEvent,
};

use crate::error::DashboardError;
use crate::state::AppState;
use crate::storage::queries::{EventRow, TaskFilter, TaskListResponse, TaskRow};

// ============================================================
// 推送 API
// ============================================================

/// POST /api/events/start - 创建任务记录
pub async fn start_task(
    State(state): State<AppState>,
    JsonExtractor(event): JsonExtractor<TaskStartedEvent>,
) -> Result<Json<serde_json::Value>, DashboardError> {
    state.storage.start_task(&event)?;
    state.hub.task_started(&event.task_id).await;
    Ok(Json(serde_json::json!({ "task_id": event.task_id, "status": "running" })))
}

/// POST /api/events/batch - 批量写入执行事件
pub async fn batch_events(
    State(state): State<AppState>,
    JsonExtractor(req): JsonExtractor<BatchEventsRequest>,
) -> Result<Json<serde_json::Value>, DashboardError> {
    let count = req.events.len();
    state.storage.insert_events(&req.task_id, &req.events)?;
    state.hub.push_events(&req.task_id, &req.events).await;
    Ok(Json(serde_json::json!({ "task_id": req.task_id, "received": count })))
}

/// POST /api/events/finish - 任务结束
pub async fn finish_task(
    State(state): State<AppState>,
    JsonExtractor(event): JsonExtractor<TaskFinishedEvent>,
) -> Result<Json<serde_json::Value>, DashboardError> {
    state.storage.finish_task(&event)?;
    state.hub.task_finished(&event.task_id).await;
    Ok(Json(serde_json::json!({ "task_id": event.task_id, "status": "finished" })))
}

/// POST /api/events/import - 导入本地 .jsonl 文件 (multipart)
pub async fn import_events(
    State(state): State<AppState>,
    mut multipart: Multipart,
) -> Result<Json<ImportResult>, DashboardError> {
    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| DashboardError::ImportFormat(format!("multipart 解析失败: {}", e)))?
    {
        let name = field.name().unwrap_or("").to_string();
        if name == "file" {
            let data = field
                .bytes()
                .await
                .map_err(|e| DashboardError::ImportFormat(format!("读取文件失败: {}", e)))?;
            if data.len() > 50 * 1024 * 1024 {
                return Err(DashboardError::ImportFormat("文件过大 (>50MB)".into()));
            }
            let content = String::from_utf8(data.to_vec())
                .map_err(|_| DashboardError::ImportFormat("文件非 UTF-8 编码".into()))?;
            let result = state.storage.import_from_jsonl(&content)?;
            if result.skipped {
                return Err(DashboardError::TaskConflict(format!(
                    "任务 {} 已存在，跳过导入",
                    result.task_id
                )));
            }
            return Ok(Json(result));
        }
    }
    Err(DashboardError::ImportFormat("未找到 file 字段".into()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::realtime::RealtimeHub;
    use crate::storage::queries::Storage;
    use std::sync::Arc;

    fn make_state() -> AppState {
        AppState {
            storage: Storage::open_in_memory().unwrap(),
            hub: Arc::new(RealtimeHub::new(100)),
            token: "secret".into(),
        }
    }

    #[tokio::test]
    async fn start_task_creates_running() {
        let state = make_state();
        let event = TaskStartedEvent {
            task_id: "api-t1".into(),
            project: "proj".into(),
            mr_iid: Some(1),
            pipeline_id: Some(2),
            task_description: "desc".into(),
            task_kind: "manual".into(),
            started_at: "2026-08-03T10:00:00Z".into(),
            model: "m".into(),
        };
        let _resp = start_task(State(state.clone()), JsonExtractor(event)).await.unwrap();
        assert!(state.storage.task_exists("api-t1").unwrap());
    }

    #[tokio::test]
    async fn batch_events_inserts_and_broadcasts() {
        let state = make_state();
        state.storage.start_task(&TaskStartedEvent {
            task_id: "api-t2".into(),
            project: "p".into(),
            mr_iid: None,
            pipeline_id: None,
            task_description: "d".into(),
            task_kind: "manual".into(),
            started_at: "2026-08-03T10:00:00Z".into(),
            model: "m".into(),
        }).unwrap();
        let req = BatchEventsRequest {
            task_id: "api-t2".into(),
            events: vec![devnpc_core::report::event_schema::ExecutionEvent::LlmCall {
                iteration: 1,
                prompt_tokens: 10,
                completion_tokens: 5,
                latency_ms: 100,
            }],
        };
        let _resp = batch_events(State(state.clone()), JsonExtractor(req)).await.unwrap();
        let events = state.storage.list_events("api-t2").unwrap();
        assert_eq!(events.len(), 1);
    }

    #[tokio::test]
    async fn finish_task_updates_status() {
        let state = make_state();
        state.storage.start_task(&TaskStartedEvent {
            task_id: "api-t3".into(),
            project: "p".into(),
            mr_iid: None,
            pipeline_id: None,
            task_description: "d".into(),
            task_kind: "manual".into(),
            started_at: "2026-08-03T10:00:00Z".into(),
            model: "m".into(),
        }).unwrap();
        let fin = TaskFinishedEvent {
            task_id: "api-t3".into(),
            status: devnpc_core::report::event_schema::TaskStatus::Success,
            duration_secs: 5,
            total_tokens: 100,
            estimated_cost_usd: 0.01,
            mr_url: None,
            ci_url: None,
            summary: "ok".into(),
            error: None,
            finished_at: "2026-08-03T10:01:00Z".into(),
        };
        let _ = finish_task(State(state.clone()), JsonExtractor(fin)).await.unwrap();
        let row = state.storage.get_task("api-t3").unwrap().unwrap();
        assert_eq!(row.status, "success");
    }

    // 注意: import_events 的完整 multipart 测试在 Task 11 的 server::mod 测试中
    // (需要通过完整 router 验证,因为 Multipart extractor 需要 HTTP 请求上下文)
}
```

- [ ] **Step 2: 运行测试验证通过**

Run: `cargo test -p devnpc-dashboard server::api`
Expected: 3 个推送 handler 测试 PASS（import multipart 测试在 Task 11 通过完整 router 验证）

- [ ] **Step 3: 提交**

Run: `git add crates/devnpc-dashboard/src/server/api.rs ; git commit -m "feat(dashboard): 推送 API handler (start/batch/finish/import multipart)"`

---

### Task 10: 辅助 API 路由

**Files:**
- Modify: `crates/devnpc-dashboard/src/server/api.rs`

实现 7 个辅助 API handler：`list_tasks`/`get_task`/`list_task_events`/`stats_trends`/`stats_cost`/`stats_ci`/`stats_sop`。

- [ ] **Step 1: 在 api.rs 追加辅助 handler 与查询参数类型**

在 `api.rs` 的推送 API 部分之后追加：

```rust
// ============================================================
// 辅助 API (无鉴权)
// ============================================================

/// GET /api/tasks 分页查询参数
#[derive(Debug, Deserialize)]
pub struct ListTasksQuery {
    pub page: Option<usize>,
    pub size: Option<usize>,
    pub status: Option<String>,
    pub project: Option<String>,
}

/// GET /api/tasks - 任务列表 JSON
pub async fn list_tasks(
    State(state): State<AppState>,
    Query(q): Query<ListTasksQuery>,
) -> Result<Json<TaskListResponse>, DashboardError> {
    let filter = TaskFilter {
        status: q.status,
        project: q.project,
    };
    let resp = state.storage.list_tasks(q.page.unwrap_or(1), q.size.unwrap_or(20), &filter)?;
    Ok(Json(resp))
}

/// GET /api/tasks/:id - 单任务详情 JSON
pub async fn get_task(
    State(state): State<AppState>,
    Path(task_id): Path<String>,
) -> Result<Json<TaskRow>, DashboardError> {
    match state.storage.get_task(&task_id)? {
        Some(row) => Ok(Json(row)),
        None => Err(DashboardError::TaskNotFound(task_id)),
    }
}

/// GET /api/tasks/:id/events - 单任务事件流 JSON
pub async fn list_task_events(
    State(state): State<AppState>,
    Path(task_id): Path<String>,
) -> Result<Json<Vec<EventRow>>, DashboardError> {
    Ok(Json(state.storage.list_events(&task_id)?))
}

/// GET /api/stats/trends?days=7
#[derive(Debug, Deserialize)]
pub struct TrendsQuery {
    pub days: Option<u32>,
}

pub async fn stats_trends(
    State(state): State<AppState>,
    Query(q): Query<TrendsQuery>,
) -> Result<Json<crate::storage::queries::TrendsData>, DashboardError> {
    Ok(Json(state.storage.trends(q.days.unwrap_or(7))?))
}

/// GET /api/stats/cost?group_by=project|model|kind
#[derive(Debug, Deserialize)]
pub struct CostQuery {
    pub group_by: Option<String>,
}

pub async fn stats_cost(
    State(state): State<AppState>,
    Query(q): Query<CostQuery>,
) -> Result<Json<Vec<crate::storage::queries::CostBucket>>, DashboardError> {
    let group_by = q.group_by.as_deref().unwrap_or("project");
    Ok(Json(state.storage.cost_breakdown(group_by)?))
}

/// GET /api/stats/ci
pub async fn stats_ci(
    State(state): State<AppState>,
) -> Result<Json<crate::storage::queries::CiStats>, DashboardError> {
    Ok(Json(state.storage.ci_stats()?))
}

/// GET /api/stats/sop
pub async fn stats_sop(
    State(state): State<AppState>,
) -> Result<Json<Vec<crate::storage::queries::SopDeviationRow>>, DashboardError> {
    Ok(Json(state.storage.sop_stats()?))
}
```

- [ ] **Step 2: 在测试模块追加辅助 API 测试**

在 `api.rs` 测试模块追加：

```rust
    #[tokio::test]
    async fn list_tasks_api_returns_response() {
        let state = make_state();
        state.storage.start_task(&TaskStartedEvent {
            task_id: "api-list".into(),
            project: "p".into(),
            mr_iid: None,
            pipeline_id: None,
            task_description: "d".into(),
            task_kind: "manual".into(),
            started_at: "2026-08-03T10:00:00Z".into(),
            model: "m".into(),
        }).unwrap();
        let q = ListTasksQuery { page: Some(1), size: Some(10), status: None, project: None };
        let resp = list_tasks(State(state), Query(q)).await.unwrap();
        assert_eq!(resp.0.total, 1);
    }

    #[tokio::test]
    async fn get_task_api_returns_row() {
        let state = make_state();
        state.storage.start_task(&TaskStartedEvent {
            task_id: "api-get".into(),
            project: "p".into(),
            mr_iid: None,
            pipeline_id: None,
            task_description: "d".into(),
            task_kind: "manual".into(),
            started_at: "2026-08-03T10:00:00Z".into(),
            model: "m".into(),
        }).unwrap();
        let resp = get_task(State(state), Path("api-get".to_string())).await.unwrap();
        assert_eq!(resp.0.task_id, "api-get");
    }

    #[tokio::test]
    async fn get_task_api_missing_returns_404() {
        let state = make_state();
        let err = get_task(State(state), Path("nope".to_string())).await.unwrap_err();
        assert!(matches!(err, DashboardError::TaskNotFound(_)));
    }

    #[tokio::test]
    async fn list_task_events_api_returns_events() {
        let state = make_state();
        state.storage.start_task(&TaskStartedEvent {
            task_id: "api-ev".into(),
            project: "p".into(),
            mr_iid: None,
            pipeline_id: None,
            task_description: "d".into(),
            task_kind: "manual".into(),
            started_at: "2026-08-03T10:00:00Z".into(),
            model: "m".into(),
        }).unwrap();
        state.storage.insert_events("api-ev", &vec![
            devnpc_core::report::event_schema::ExecutionEvent::ToolCall {
                name: "git".into(),
                success: true,
                latency_ms: 10,
                detail: "commit".into(),
            },
        ]).unwrap();
        let resp = list_task_events(State(state), Path("api-ev".to_string())).await.unwrap();
        assert_eq!(resp.0.len(), 1);
    }

    #[tokio::test]
    async fn stats_trends_api_returns_data() {
        let state = make_state();
        let q = TrendsQuery { days: Some(7) };
        let resp = stats_trends(State(state), Query(q)).await.unwrap();
        assert_eq!(resp.0.days, 7);
    }

    #[tokio::test]
    async fn stats_cost_invalid_group_returns_400() {
        let state = make_state();
        let q = CostQuery { group_by: Some("invalid".into()) };
        let err = stats_cost(State(state), Query(q)).await.unwrap_err();
        assert!(matches!(err, DashboardError::ImportFormat(_)));
    }

    #[tokio::test]
    async fn stats_ci_api_returns() {
        let state = make_state();
        let resp = stats_ci(State(state)).await.unwrap();
        assert_eq!(resp.0.total_failed, 0);
    }

    #[tokio::test]
    async fn stats_sop_api_returns() {
        let state = make_state();
        let resp = stats_sop(State(state)).await.unwrap();
        assert!(resp.0.is_empty());
    }
```

- [ ] **Step 3: 运行测试验证通过**

Run: `cargo test -p devnpc-dashboard server::api`
Expected: 全部 api 测试 PASS

- [ ] **Step 4: 提交**

Run: `git add crates/devnpc-dashboard/src/server/api.rs ; git commit -m "feat(dashboard): 辅助 API handler (tasks 分页/详情/事件 + stats trends/cost/ci/sop)"`

---

### Task 11: SSE 端点 + 静态资源 + Router 组装 + main 启动

**Files:**
- Modify: `crates/devnpc-dashboard/src/server/mod.rs`
- Modify: `crates/devnpc-dashboard/src/main.rs`
- Create: `crates/devnpc-dashboard/tests/e2e.rs`

实现 SSE handler、静态资源 handler、`build_router` 组装、main 启动流程，以及端到端集成测试。

- [ ] **Step 1: 替换 server/mod.rs 为完整实现**

将 `crates/devnpc-dashboard/src/server/mod.rs` 替换为：

```rust
//! 路由组装与静态资源服务

use axum::extract::{Path, State};
use axum::http::{header, StatusCode};
use axum::middleware::from_fn_with_state;
use axum::response::{IntoResponse, Response, Sse};
use axum::routing::{get, post};
use axum::Router;
use futures::{Stream, StreamExt};
use rust_embed::RustEmbed;
use std::convert::Infallible;

use crate::auth::require_token;
use crate::error::DashboardError;
use crate::server::api;
use crate::state::AppState;

/// 嵌入静态资源 (编译期从 static/ 目录读取)
#[derive(RustEmbed)]
#[folder = "static/"]
struct StaticAsset;

/// 构建 HTTP 路由
pub fn build_router(state: AppState) -> Router {
    // 推送 API (token 鉴权 + 50MB body 限制)
    let protected = Router::new()
        .route("/api/events/start", post(api::start_task))
        .route("/api/events/batch", post(api::batch_events))
        .route("/api/events/finish", post(api::finish_task))
        .route("/api/events/import", post(api::import_events))
        .layer(axum::extract::DefaultBodyLimit::max(50 * 1024 * 1024))
        .layer(from_fn_with_state(state.clone(), require_token));

    // 辅助 API (无鉴权)
    let public = Router::new()
        .route("/api/tasks", get(api::list_tasks))
        .route("/api/tasks/:id", get(api::get_task))
        .route("/api/tasks/:id/events", get(api::list_task_events))
        .route("/api/stats/trends", get(api::stats_trends))
        .route("/api/stats/cost", get(api::stats_cost))
        .route("/api/stats/ci", get(api::stats_ci))
        .route("/api/stats/sop", get(api::stats_sop))
        .route("/api/realtime/stream", get(realtime_stream))
        .route("/static/*path", get(static_handler));

    Router::new()
        .merge(protected)
        .merge(public)
        .with_state(state)
}

/// GET /api/realtime/stream - SSE 实时事件推送
pub async fn realtime_stream(
    State(state): State<AppState>,
) -> Sse<impl Stream<Item = Result<axum::response::sse::Event, Infallible>>> {
    let stream = state.hub.subscribe().await.map(|ev| {
        let data = serde_json::to_string(&ev).unwrap_or_default();
        Ok(axum::response::sse::Event::default().data(data))
    });
    Sse::new(stream).keep_alive(axum::response::sse::KeepAlive::default())
}

/// GET /static/*path - 静态资源 (rust-embed)
pub async fn static_handler(Path(path): Path<String>) -> Response {
    match StaticAsset::get(&path) {
        Some(asset) => {
            let mime = mime_guess::from_path(&path).first_or_octet_stream();
            (
                [(header::CONTENT_TYPE, mime.as_ref())],
                asset.data,
            )
                .into_response()
        }
        None => (StatusCode::NOT_FOUND, "未找到资源").into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::realtime::RealtimeHub;
    use crate::storage::queries::Storage;
    use axum::body::Body;
    use axum::http::Request;
    use std::sync::Arc;
    use tower::ServiceExt;

    fn make_state() -> AppState {
        AppState {
            storage: Storage::open_in_memory().unwrap(),
            hub: Arc::new(RealtimeHub::new(100)),
            token: "secret".into(),
        }
    }

    #[tokio::test]
    async fn protected_route_without_token_returns_401() {
        let app = build_router(make_state());
        let req = Request::builder()
            .method("POST")
            .uri("/api/events/start")
            .header("content-type", "application/json")
            .body(Body::from("{}"))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn public_route_accessible_without_token() {
        let app = build_router(make_state());
        let req = Request::builder()
            .method("GET")
            .uri("/api/tasks")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn static_missing_returns_404() {
        let app = build_router(make_state());
        let req = Request::builder()
            .method("GET")
            .uri("/static/nonexistent.css")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn full_push_flow_via_router() {
        let state = make_state();
        let app = build_router(state.clone());

        // start
        let start_body = serde_json::json!({
            "task_id": "router-t1",
            "project": "proj",
            "mr_iid": null,
            "pipeline_id": null,
            "task_description": "d",
            "task_kind": "manual",
            "started_at": "2026-08-03T10:00:00Z",
            "model": "m"
        });
        let req = Request::builder()
            .method("POST")
            .uri("/api/events/start")
            .header("content-type", "application/json")
            .header("X-Devnpc-Token", "secret")
            .body(Body::from(start_body.to_string()))
            .unwrap();
        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        // batch
        let batch_body = serde_json::json!({
            "task_id": "router-t1",
            "events": [{ "type": "llm_call", "iteration": 1, "prompt_tokens": 100, "completion_tokens": 50, "latency_ms": 500 }]
        });
        let req = Request::builder()
            .method("POST")
            .uri("/api/events/batch")
            .header("content-type", "application/json")
            .header("X-Devnpc-Token", "secret")
            .body(Body::from(batch_body.to_string()))
            .unwrap();
        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        // finish
        let finish_body = serde_json::json!({
            "task_id": "router-t1",
            "status": "success",
            "duration_secs": 45,
            "total_tokens": 150,
            "estimated_cost_usd": 0.01,
            "mr_url": null,
            "ci_url": null,
            "summary": "ok",
            "error": null,
            "finished_at": "2026-08-03T10:01:00Z"
        });
        let req = Request::builder()
            .method("POST")
            .uri("/api/events/finish")
            .header("content-type", "application/json")
            .header("X-Devnpc-Token", "secret")
            .body(Body::from(finish_body.to_string()))
            .unwrap();
        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        // 查询验证
        let req = Request::builder()
            .method("GET")
            .uri("/api/tasks/router-t1")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        assert!(state.storage.get_task("router-t1").unwrap().unwrap().status == "success");
    }

    #[tokio::test]
    async fn import_via_router_multipart() {
        let state = make_state();
        let app = build_router(state.clone());
        let jsonl = serde_json::json!({
            "kind": "task_started",
            "task_id": "router-imp",
            "project": "p",
            "mr_iid": null,
            "pipeline_id": null,
            "task_description": "d",
            "task_kind": "manual",
            "started_at": "2026-08-03T10:00:00Z",
            "model": "m"
        }).to_string() + "\n" + &serde_json::json!({
            "kind": "task_finished",
            "task_id": "router-imp",
            "status": "success",
            "duration_secs": 5,
            "total_tokens": 0,
            "estimated_cost_usd": 0.0,
            "mr_url": null,
            "ci_url": null,
            "summary": "ok",
            "error": null,
            "finished_at": "2026-08-03T10:01:00Z"
        }).to_string();
        let boundary = "----testb";
        let body = format!(
            "--{}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"t.jsonl\"\r\nContent-Type: application/octet-stream\r\n\r\n{}\r\n--{}--\r\n",
            boundary, jsonl, boundary
        );
        let req = Request::builder()
            .method("POST")
            .uri("/api/events/import")
            .header("content-type", format!("multipart/form-data; boundary={}", boundary))
            .header("X-Devnpc-Token", "secret")
            .body(Body::from(body))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        assert!(state.storage.task_exists("router-imp").unwrap());
    }

    #[tokio::test]
    async fn import_finished_twice_returns_409() {
        let state = make_state();
        let app = build_router(state.clone());
        let jsonl = serde_json::json!({
            "kind": "task_started",
            "task_id": "router-imp2",
            "project": "p",
            "mr_iid": null,
            "pipeline_id": null,
            "task_description": "d",
            "task_kind": "manual",
            "started_at": "2026-08-03T10:00:00Z",
            "model": "m"
        }).to_string() + "\n" + &serde_json::json!({
            "kind": "task_finished",
            "task_id": "router-imp2",
            "status": "success",
            "duration_secs": 5,
            "total_tokens": 0,
            "estimated_cost_usd": 0.0,
            "mr_url": null,
            "ci_url": null,
            "summary": "ok",
            "error": null,
            "finished_at": "2026-08-03T10:01:00Z"
        }).to_string();
        let boundary = "----testb";
        let body = format!(
            "--{}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"t.jsonl\"\r\nContent-Type: application/octet-stream\r\n\r\n{}\r\n--{}--\r\n",
            boundary, jsonl, boundary
        );
        // 第一次
        let req = Request::builder()
            .method("POST")
            .uri("/api/events/import")
            .header("content-type", format!("multipart/form-data; boundary={}", boundary))
            .header("X-Devnpc-Token", "secret")
            .body(Body::from(body.clone()))
            .unwrap();
        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        // 第二次 -> 409
        let req = Request::builder()
            .method("POST")
            .uri("/api/events/import")
            .header("content-type", format!("multipart/form-data; boundary={}", boundary))
            .header("X-Devnpc-Token", "secret")
            .body(Body::from(body))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::CONFLICT);
    }
}
```

- [ ] **Step 2: 运行 router 测试验证通过**

Run: `cargo test -p devnpc-dashboard server::mod`
Expected: 6 个测试 PASS

- [ ] **Step 3: 填充 main.rs 完整启动流程**

将 `crates/devnpc-dashboard/src/main.rs` 替换为：

```rust
//! devnpc-dashboard CLI 入口
//!
//! 加载配置 -> 打开 SQLite -> 初始化 RealtimeHub -> 启动 axum 服务

use std::net::SocketAddr;
use std::sync::Arc;

use clap::Parser;
use tracing_subscriber::EnvFilter;

use devnpc_dashboard::realtime::RealtimeHub;
use devnpc_dashboard::server::build_router;
use devnpc_dashboard::state::AppState;
use devnpc_dashboard::storage::queries::Storage;

#[derive(Parser)]
#[command(name = "devnpc-dashboard", about = "devnpc 可观测 Dashboard 服务")]
struct Cli {
    /// 监听端口 (默认 8080)
    #[arg(long, env = "DEVNPC_DASHBOARD_PORT")]
    port: Option<u16>,

    /// 监听地址 (默认 0.0.0.0)
    #[arg(long, env = "DEVNPC_DASHBOARD_HOST")]
    host: Option<String>,

    /// SQLite 数据库路径 (默认 ./devnpc-dashboard.db)
    #[arg(long, env = "DEVNPC_DASHBOARD_DB")]
    db: Option<String>,

    /// 推送鉴权 token
    #[arg(long, env = "DEVNPC_DASHBOARD_TOKEN")]
    token: Option<String>,

    /// 实时环形缓冲容量 (默认 1000)
    #[arg(long, env = "DEVNPC_DASHBOARD_REALTIME_BUFFER")]
    realtime_buffer: Option<usize>,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 加载 .env (可选,文件不存在不报错)
    let _ = dotenvy::dotenv();

    // 初始化日志
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
        .init();

    let cli = Cli::parse();
    let port = cli.port.unwrap_or(8080);
    let host = cli.host.unwrap_or_else(|| "0.0.0.0".into());
    let db_path = cli.db.unwrap_or_else(|| "./devnpc-dashboard.db".into());
    let token = cli.token.unwrap_or_default();
    let buffer_cap = cli.realtime_buffer.unwrap_or(1000);

    // 打开 SQLite (WAL + schema 迁移)
    let storage = Storage::open(&db_path)?;
    tracing::info!(db = %db_path, "SQLite 已就绪 (WAL 模式)");

    // 初始化 RealtimeHub
    let hub = RealtimeHub::new(buffer_cap);

    // 构建共享状态
    let state = AppState {
        storage,
        hub: Arc::new(hub),
        token,
    };

    // 构建路由
    let app = build_router(state);

    // 绑定监听
    let addr: SocketAddr = format!("{}:{}", host, port).parse()?;
    let listener = tokio::net::TcpListener::bind(addr).await?;
    tracing::info!(addr = %addr, "devnpc-dashboard 服务已启动");

    axum::serve(listener, app).await?;

    Ok(())
}
```

注意：`main.rs` 通过 `devnpc_dashboard::...` 引用 lib 模块。`devnpc_dashboard` 是 `Cargo.toml` 中 `[lib] name = "devnpc_dashboard"` 定义的。

- [ ] **Step 4: 验证 bin 编译**

Run: `cargo check -p devnpc-dashboard --bin devnpc-dashboard`
Expected: 编译通过

- [ ] **Step 5: 创建端到端集成测试 tests/e2e.rs**

创建 `crates/devnpc-dashboard/tests/e2e.rs`：

```rust
//! 端到端集成测试: 完整推送 + 查询 + SSE + 导入流程

use axum::body::Body;
use axum::http::Request;
use devnpc_dashboard::realtime::RealtimeHub;
use devnpc_dashboard::server::build_router;
use devnpc_dashboard::state::AppState;
use devnpc_dashboard::storage::queries::Storage;
use std::sync::Arc;
use tower::ServiceExt;

fn make_app() -> AppState {
    AppState {
        storage: Storage::open_in_memory().unwrap(),
        hub: Arc::new(RealtimeHub::new(100)),
        token: "test-token".into(),
    }
}

#[tokio::test]
async fn e2e_full_task_lifecycle() {
    let state = make_app();
    let app = build_router(state.clone());

    // 1. start
    let start = serde_json::json!({
        "task_id": "e2e-1",
        "project": "group/proj",
        "mr_iid": 42,
        "pipeline_id": 100,
        "task_description": "修复 bug",
        "task_kind": "mr_comment",
        "started_at": "2026-08-03T10:00:00Z",
        "model": "deepseek-chat"
    });
    let req = Request::builder()
        .method("POST")
        .uri("/api/events/start")
        .header("content-type", "application/json")
        .header("X-Devnpc-Token", "test-token")
        .body(Body::from(start.to_string()))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), 200);

    // 2. batch (多次)
    for i in 1..=3 {
        let batch = serde_json::json!({
            "task_id": "e2e-1",
            "events": [{ "type": "llm_call", "iteration": i, "prompt_tokens": 100, "completion_tokens": 50, "latency_ms": 500 }]
        });
        let req = Request::builder()
            .method("POST")
            .uri("/api/events/batch")
            .header("content-type", "application/json")
            .header("X-Devnpc-Token", "test-token")
            .body(Body::from(batch.to_string()))
            .unwrap();
        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), 200);
    }

    // 3. finish
    let finish = serde_json::json!({
        "task_id": "e2e-1",
        "status": "success",
        "duration_secs": 45,
        "total_tokens": 450,
        "estimated_cost_usd": 0.05,
        "mr_url": "https://gitlab.com/mr/42",
        "ci_url": "https://gitlab.com/pipeline/100",
        "summary": "已修复",
        "error": null,
        "finished_at": "2026-08-03T10:01:00Z"
    });
    let req = Request::builder()
        .method("POST")
        .uri("/api/events/finish")
        .header("content-type", "application/json")
        .header("X-Devnpc-Token", "test-token")
        .body(Body::from(finish.to_string()))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), 200);

    // 4. 查询任务详情
    let req = Request::builder()
        .method("GET")
        .uri("/api/tasks/e2e-1")
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), 200);
    let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024).await.unwrap();
    let body: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(body["status"], "success");
    assert_eq!(body["total_tokens"], 450);

    // 5. 查询事件列表
    let req = Request::builder()
        .method("GET")
        .uri("/api/tasks/e2e-1/events")
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024).await.unwrap();
    let body: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(body.as_array().unwrap().len(), 3);

    // 6. 查询趋势统计
    let req = Request::builder()
        .method("GET")
        .uri("/api/stats/trends?days=7")
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), 200);

    // 7. 查询成本统计
    let req = Request::builder()
        .method("GET")
        .uri("/api/stats/cost?group_by=project")
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), 200);
}

#[tokio::test]
async fn e2e_auth_rejects_wrong_token() {
    let state = make_app();
    let app = build_router(state);
    let req = Request::builder()
        .method("POST")
        .uri("/api/events/start")
        .header("content-type", "application/json")
        .header("X-Devnpc-Token", "wrong")
        .body(Body::from("{}"))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), 401);
}

#[tokio::test]
async fn e2e_stats_endpoints_respond() {
    let state = make_app();
    let app = build_router(state);
    for uri in &["/api/stats/ci", "/api/stats/sop"] {
        let req = Request::builder().method("GET").uri(*uri).body(Body::empty()).unwrap();
        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), 200);
    }
}
```

- [ ] **Step 6: 运行全部测试验证通过**

Run: `cargo test -p devnpc-dashboard`
Expected: 全部测试 PASS（单元测试 + 集成测试 e2e）

- [ ] **Step 7: 运行 clippy**

Run: `cargo clippy -p devnpc-dashboard -- -D warnings`
Expected: 无 warning（如有 unused import 按提示修复）

- [ ] **Step 8: 提交**

Run: `git add crates/devnpc-dashboard ; git commit -m "feat(dashboard): SSE 端点 + 静态资源 + Router 组装 + main 启动 + E2E 集成测试"`

---

## Self-Review 检查清单

### Spec 覆盖

- [x] **spec §5.1 启动与配置**：Task 1（CLI 结构）+ Task 11（main 启动流程，加载 .env / WAL / RealtimeHub / axum）
- [x] **spec §5.2 路由表**：
  - 推送 API（token 鉴权）：Task 9 实现 start/batch/finish/import，Task 11 路由组装 + `from_fn_with_state` 中间件
  - 辅助 API（无鉴权）：Task 10 实现 tasks/tasks/:id/tasks/:id/events/stats/trends/stats/cost/stats/ci/stats/sop
  - SSE 端点 `/api/realtime/stream`：Task 11
  - 静态资源 `/static/*`：Task 11（rust-embed）
  - 页面路由（GET / 等 HTML）明确排除——阶段 4 实现
- [x] **spec §5.3 鉴权中间件**：Task 8 `require_token`（空配置 403 / 不匹配 401 / 匹配放行）
- [x] **spec §5.4 RealtimeHub**：Task 7 环形缓冲 + broadcast + subscribe（历史回放 + 实时）
- [x] **spec §5.5 存储层**：Task 2-6 覆盖全部 Storage 方法（start/insert/finish/list/get/list_events/trends/cost/ci/sop/task_exists/task_is_finished/delete/import）
- [x] **spec §3.5 SQLite Schema**：Task 2 三表 + 四索引 + WAL 模式
- [x] **spec §3.4 导入幂等**：Task 6 已 finish 跳过 / 未 finish 覆盖 / 格式错误 400 / 重复导入 409（Task 11 集成测试验证）
- [x] **spec §7.3 错误类型**：Task 1 DashboardError 七变体（Sqlite/Serde/TaskNotFound/TaskConflict/ImportFormat/Template/Io）+ IntoResponse 映射
- [x] **spec §7.2 dashboard 侧错误处理**：重复 start 409 / batch task 不存在 404 / finish 不存在 404 / finish 重复 409 / import 格式 400 / import 已 finish 409 / import 过大 413（DefaultBodyLimit）/ token 失败 401 / token 未配置 403
- [x] **spec §8.2 集成测试**：Task 11 e2e.rs 覆盖完整推送流程 + 查询 + 鉴权拒绝 + stats 端点
- [x] **并发模型**：Task 2 `Arc<Mutex<Connection>>` 串行化写，WAL 读不阻塞写

### 占位符扫描

无 "TBD"/"TODO"/"implement later"。所有代码步骤包含完整实现。`static/.gitkeep` 是预期的占位文件（LayUI 由用户后续放入）。
已移除 Task 9 中的 `import_events_via_multipart` 占位测试（原仅断言 `!jsonl.is_empty()`），import 完整测试由 Task 11 router 测试覆盖。
已移除 Task 6 `sample_jsonl` 中的死代码变量 `started`（原创建后丢弃）。

### 导入一致性

- `realtime/mod.rs`：`use futures::StreamExt;` 已加入文件顶部导入（`subscribe()` 使用 `filter_map`/`chain`）
- `server/mod.rs`：`State`/`StreamExt` 已统一到文件顶部导入（`realtime_stream` 使用 `.map()`）
- `server/api.rs`：已移除未使用导入 `StatusCode`/`IntoResponse`/`Response`/`HashMap`；测试模块已移除未使用导入 `Body`/`Request`/`ServiceExt`

### TDD 编译一致性

- Task 3 已包含 `get_task`/`list_events` 实现（从 Task 4 移入），确保 Task 3 测试模块可独立编译通过
- `finish_task_writes_sop_deviations` 测试已从 Task 3 移至 Task 5（依赖 `sop_stats` 方法）
- 每个 Task 完成后其测试模块可独立编译并运行

### 类型一致性

- `TaskRow` 字段在 Task 3 定义，Task 3(get_task)/4(list_tasks)/5(ci_stats)/10/11 复用，字段名与 schema 列一致
- `EventRow` Task 3 定义，Task 3(list_events)/10 复用
- `TaskFilter`/`TaskListResponse` Task 4 定义，Task 10 复用
- `TrendsData`/`CostBucket`/`CiStats`/`SopDeviationRow` Task 5 定义，Task 10 复用
- `RealtimeEvent` Task 7 定义，Task 11 SSE handler 复用
- `AppState` Task 1 定义，Task 8/9/10/11 复用（storage/hub/token 三字段）
- `DashboardError` Task 1 定义，Task 2-11 复用，HTTP 映射一致（404/409/400/401/403/500/413）
- `Storage::start_task`/`insert_events`/`finish_task`/`get_task`/`list_events` 签名在 Task 3 定义，`list_tasks` 在 Task 4 定义，`import_from_jsonl` 在 Task 6 定义，Task 9/10 handler 调用一致
- core 类型引用路径统一为 `devnpc_core::report::event_schema::*`
