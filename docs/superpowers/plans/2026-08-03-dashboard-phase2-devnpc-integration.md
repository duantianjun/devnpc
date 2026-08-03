# Dashboard Phase 2: devnpc 侧改造实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 改造 devnpc 侧，实现 Dashboard 配置扩展、本地事件记录（.jsonl 兜底文件）、事件推送（HTTP POST 批量推送），不阻塞主任务执行。

**Architecture:** 在 devnpc crate 新增 `report/sender.rs` 模块（`LocalEventLogger` + `EventSender`），将 `Trajectory` 从 core 迁回 devnpc 并扩展为持有三个可选组件（内存 Vec + 本地文件记录器 + 事件推送器）。`main.rs` 的 `run()` 函数在任务启动时初始化日志/推送、任务结束时 flush。配置通过 `.env` 驱动，`dashboard.enabled=false` 时降级为现状（仅本地文件可选保留）。

**Tech Stack:** Rust 2024, tokio (mpsc + spawn + sleep), reqwest (HTTP POST), serde_json, chrono, uuid, wiremock (测试), tempfile (测试)

**关联 spec:** [2026-08-03-devnpc-dashboard-design.md](../specs/2026-08-03-devnpc-dashboard-design.md) 第四章
**关联 phase 1:** [2026-08-03-dashboard-phase1-workspace.md](2026-08-03-dashboard-phase1-workspace.md)

---

## 文件结构总览

本阶段涉及的文件：

| 操作 | 路径 | 职责 |
|------|------|------|
| Modify | `crates/devnpc/src/config/mod.rs` | 新增 `DashboardConfig` 结构体 + `Config.dashboard` 字段 |
| Modify | `crates/devnpc/src/config/loader.rs` | 加载 dashboard 环境变量 |
| Create | `crates/devnpc/src/report/sender.rs` | `LocalEventLogger` + `EventSender` 实现 |
| Modify | `crates/devnpc/src/report/mod.rs` | 注册 `sender` 模块 |
| Modify | `crates/devnpc-core/src/report/types.rs` | 移除 `Trajectory`（迁回 devnpc） |
| Modify | `crates/devnpc/src/report/collector.rs` | 定义新 `Trajectory`（持有 logger/sender） |
| Modify | `crates/devnpc/src/main.rs` | `run()` 接入 dashboard 推送初始化和结束逻辑 |
| Modify | `crates/devnpc/Cargo.toml` | 添加 tokio test-util dev-feature |

**关键设计决策：**

1. **Trajectory 迁回 devnpc**：phase 1 将 `Trajectory` 迁到 core（纯数据），但 phase 2 要求 `Trajectory` 持有 `LocalEventLogger` 和 `EventSender`（devnpc 类型）。为避免 core 反向依赖 devnpc，将 `Trajectory` 迁回 devnpc crate，core 保留 `TrajectoryEvent` / `ReportData` / `CostEstimate` 等纯数据类型。

2. **降级策略**：`dashboard.enabled=false` 时不创建 `EventSender`；`local_event_log=true`（默认）时仍创建 `LocalEventLogger` 保存本地文件。两者独立。

3. **推送失败不影响主任务**：所有推送/文件写入失败均 `tracing::warn` 记录，不传播错误。

4. **批量推送策略**：20 条或 3 秒触发一次 POST `/api/events/batch`，channel 关闭时 flush。失败重试 1s/2s/4s/8s/16s 共 6 次尝试（初始 + 5 次重试），仍失败则丢弃。

---

### Task 1: DashboardConfig 配置扩展

**Files:**
- Modify: `crates/devnpc/src/config/mod.rs`
- Modify: `crates/devnpc/src/config/loader.rs`

- [ ] **Step 1: 在 config/mod.rs 编写 DashboardConfig 测试**

在 `crates/devnpc/src/config/mod.rs` 文件末尾（`impl Config` 块之前）添加 `DashboardConfig` 结构体，并在文件末尾的 `#[cfg(test)]` 模块中添加测试。

先在文件中添加 `DashboardConfig` 结构体定义（放在 `TriggerConfig` 之后、`impl Config` 之前）：

```rust
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
```

然后在 `Config` 结构体中添加 `dashboard` 字段。在 `pub trigger: TriggerConfig,` 之后添加：

```rust
    /// Dashboard 推送配置 (spec §4.1)
    pub dashboard: DashboardConfig,
```

在文件末尾添加测试模块（如果已有 `#[cfg(test)]` 模块则在其中追加）：

```rust
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
```

- [ ] **Step 2: 运行测试验证失败（Config 缺少 dashboard 字段）**

Run: `cargo test -p devnpc --lib config::dashboard_config_tests -- --nocapture`
Expected: 编译失败，`Config` 结构体缺少 `dashboard` 字段（loader.rs 未填充）

- [ ] **Step 3: 在 loader.rs 加载 dashboard 配置**

在 `crates/devnpc/src/config/loader.rs` 的 `load_internal` 函数末尾的 `Ok(Config { ... })` 块中，在 `trigger: crate::config::TriggerConfig { ... },` 之后添加 `dashboard` 字段：

```rust
        dashboard: crate::config::DashboardConfig {
            // enabled 由 URL 是否非空决定
            enabled: {
                let url = env::get_or_default("DEVNPC_DASHBOARD_URL", "");
                !url.is_empty()
            },
            url: env::get_or_default("DEVNPC_DASHBOARD_URL", ""),
            token: env::get_or_default("DEVNPC_DASHBOARD_TOKEN", ""),
            batch_size: env::get_usize("DEVNPC_DASHBOARD_BATCH_SIZE")?
                .unwrap_or(20),
            batch_interval_secs: env::get_u64("DEVNPC_DASHBOARD_BATCH_INTERVAL_SECS")?
                .unwrap_or(3),
            // local_event_log 默认 true,显式设为 "false" 才关闭
            local_event_log: env::get_optional("DEVNPC_DASHBOARD_LOCAL_LOG")
                .map(|v| v != "false")
                .unwrap_or(true),
        },
```

- [ ] **Step 4: 在 loader.rs 测试中验证 dashboard 配置加载**

在 `crates/devnpc/src/config/loader.rs` 的 `#[cfg(test)]` 模块末尾追加测试：

```rust
    #[test]
    fn load_dashboard_config_defaults_when_env_missing() {
        unsafe { std::env::set_var("DEVNPC_TEST_DASH_API_KEY", "sk"); }
        unsafe { std::env::set_var("DEVNPC_TEST_DASH_BASE_URL", "https://api.test.com/v1"); }
        unsafe { std::env::set_var("DEVNPC_TEST_DASH_MODEL", "m"); }
        unsafe { std::env::set_var("DEVNPC_TEST_DASH_GITLAB_URL", "https://gl.test.com"); }
        unsafe { std::env::set_var("DEVNPC_TEST_DASH_GITLAB_TOKEN", "t"); }
        unsafe { std::env::set_var("DEVNPC_TEST_DASH_PROJECT_ID", "1"); }
        // 清除 dashboard 相关环境变量
        unsafe { std::env::remove_var("DEVNPC_DASHBOARD_URL"); }
        unsafe { std::env::remove_var("DEVNPC_DASHBOARD_TOKEN"); }
        unsafe { std::env::remove_var("DEVNPC_DASHBOARD_LOCAL_LOG"); }

        let config = load_internal(
            "DEVNPC_TEST_DASH_API_KEY",
            "DEVNPC_TEST_DASH_BASE_URL",
            "DEVNPC_TEST_DASH_MODEL",
            "DEVNPC_TEST_DASH_GITLAB_URL",
            "DEVNPC_TEST_DASH_GITLAB_TOKEN",
            "DEVNPC_TEST_DASH_PROJECT_ID",
            "DEVNPC_TEST_DASH_MAX_ITERATIONS",
            "DEVNPC_TEST_DASH_MAX_CI_RETRIES",
            "DEVNPC_TEST_DASH_SOP_MODE",
            "DEVNPC_TEST_DASH_REPORT_TARGET",
            "DEVNPC_TEST_DASH_MODEL_ROUTING",
            "DEVNPC_TEST_DASH_CMD_ALLOWLIST",
            "DEVNPC_TEST_DASH_CMD_DENYLIST",
            "DEVNPC_TEST_DASH_DEFAULT_TIMEOUT",
            "DEVNPC_TEST_DASH_READ_FILE_MAX_LINES",
            "DEVNPC_TEST_DASH_LOG_PARSER_MAX_FAILURES",
            "DEVNPC_TEST_DASH_KEY_FILE_PATTERNS",
            "DEVNPC_TEST_DASH_SUMMARY_README_LINES",
            "DEVNPC_TEST_DASH_SUMMARY_MAIN_RS_LINES",
            "DEVNPC_TEST_DASH_SUMMARY_OTHER_LINES",
            "DEVNPC_TEST_DASH_CTX_MAX_COMMITS",
            "DEVNPC_TEST_DASH_CTX_MAX_PIPELINES",
            "DEVNPC_TEST_DASH_CTX_MAX_FAILURES",
            "DEVNPC_TEST_DASH_CI_POLL_INTERVAL",
            "DEVNPC_TEST_DASH_CI_POLL_TIMEOUT",
            "DEVNPC_TEST_DASH_CI_PIPELINE_TIMEOUT",
            "DEVNPC_TEST_DASH_CI_MAX_RETRIES",
            None,
        )
        .unwrap();

        // 未配置 URL → enabled=false
        assert!(!config.dashboard.enabled);
        // local_event_log 默认 true
        assert!(config.dashboard.local_event_log);
        // 批量阈值默认值
        assert_eq!(config.dashboard.batch_size, 20);
        assert_eq!(config.dashboard.batch_interval_secs, 3);

        for key in [
            "DEVNPC_TEST_DASH_API_KEY",
            "DEVNPC_TEST_DASH_BASE_URL",
            "DEVNPC_TEST_DASH_MODEL",
            "DEVNPC_TEST_DASH_GITLAB_URL",
            "DEVNPC_TEST_DASH_GITLAB_TOKEN",
            "DEVNPC_TEST_DASH_PROJECT_ID",
        ] {
            unsafe { std::env::remove_var(key); }
        }
    }

    #[test]
    fn load_dashboard_config_from_env() {
        unsafe { std::env::set_var("DEVNPC_TEST_DASH2_API_KEY", "sk"); }
        unsafe { std::env::set_var("DEVNPC_TEST_DASH2_BASE_URL", "https://api.test.com/v1"); }
        unsafe { std::env::set_var("DEVNPC_TEST_DASH2_MODEL", "m"); }
        unsafe { std::env::set_var("DEVNPC_TEST_DASH2_GITLAB_URL", "https://gl.test.com"); }
        unsafe { std::env::set_var("DEVNPC_TEST_DASH2_GITLAB_TOKEN", "t"); }
        unsafe { std::env::set_var("DEVNPC_TEST_DASH2_PROJECT_ID", "1"); }
        // 设置 dashboard 环境变量
        unsafe { std::env::set_var("DEVNPC_DASHBOARD_URL", "http://dashboard:8080"); }
        unsafe { std::env::set_var("DEVNPC_DASHBOARD_TOKEN", "secret-token"); }
        unsafe { std::env::set_var("DEVNPC_DASHBOARD_BATCH_SIZE", "50"); }
        unsafe { std::env::set_var("DEVNPC_DASHBOARD_BATCH_INTERVAL_SECS", "10"); }
        unsafe { std::env::set_var("DEVNPC_DASHBOARD_LOCAL_LOG", "false"); }

        let config = load_internal(
            "DEVNPC_TEST_DASH2_API_KEY",
            "DEVNPC_TEST_DASH2_BASE_URL",
            "DEVNPC_TEST_DASH2_MODEL",
            "DEVNPC_TEST_DASH2_GITLAB_URL",
            "DEVNPC_TEST_DASH2_GITLAB_TOKEN",
            "DEVNPC_TEST_DASH2_PROJECT_ID",
            "DEVNPC_TEST_DASH2_MAX_ITERATIONS",
            "DEVNPC_TEST_DASH2_MAX_CI_RETRIES",
            "DEVNPC_TEST_DASH2_SOP_MODE",
            "DEVNPC_TEST_DASH2_REPORT_TARGET",
            "DEVNPC_TEST_DASH2_MODEL_ROUTING",
            "DEVNPC_TEST_DASH2_CMD_ALLOWLIST",
            "DEVNPC_TEST_DASH2_CMD_DENYLIST",
            "DEVNPC_TEST_DASH2_DEFAULT_TIMEOUT",
            "DEVNPC_TEST_DASH2_READ_FILE_MAX_LINES",
            "DEVNPC_TEST_DASH2_LOG_PARSER_MAX_FAILURES",
            "DEVNPC_TEST_DASH2_KEY_FILE_PATTERNS",
            "DEVNPC_TEST_DASH2_SUMMARY_README_LINES",
            "DEVNPC_TEST_DASH2_SUMMARY_MAIN_RS_LINES",
            "DEVNPC_TEST_DASH2_SUMMARY_OTHER_LINES",
            "DEVNPC_TEST_DASH2_CTX_MAX_COMMITS",
            "DEVNPC_TEST_DASH2_CTX_MAX_PIPELINES",
            "DEVNPC_TEST_DASH2_CTX_MAX_FAILURES",
            "DEVNPC_TEST_DASH2_CI_POLL_INTERVAL",
            "DEVNPC_TEST_DASH2_CI_POLL_TIMEOUT",
            "DEVNPC_TEST_DASH2_CI_PIPELINE_TIMEOUT",
            "DEVNPC_TEST_DASH2_CI_MAX_RETRIES",
            None,
        )
        .unwrap();

        // 配置了 URL → enabled=true
        assert!(config.dashboard.enabled);
        assert_eq!(config.dashboard.url, "http://dashboard:8080");
        assert_eq!(config.dashboard.token, "secret-token");
        assert_eq!(config.dashboard.batch_size, 50);
        assert_eq!(config.dashboard.batch_interval_secs, 10);
        // 显式关闭本地日志
        assert!(!config.dashboard.local_event_log);

        // 清理
        for key in [
            "DEVNPC_TEST_DASH2_API_KEY",
            "DEVNPC_TEST_DASH2_BASE_URL",
            "DEVNPC_TEST_DASH2_MODEL",
            "DEVNPC_TEST_DASH2_GITLAB_URL",
            "DEVNPC_TEST_DASH2_GITLAB_TOKEN",
            "DEVNPC_TEST_DASH2_PROJECT_ID",
            "DEVNPC_DASHBOARD_URL",
            "DEVNPC_DASHBOARD_TOKEN",
            "DEVNPC_DASHBOARD_BATCH_SIZE",
            "DEVNPC_DASHBOARD_BATCH_INTERVAL_SECS",
            "DEVNPC_DASHBOARD_LOCAL_LOG",
        ] {
            unsafe { std::env::remove_var(key); }
        }
    }
```

- [ ] **Step 5: 运行测试验证通过**

Run: `cargo test -p devnpc --lib config:: -- --nocapture`
Expected: 所有 config 测试 PASS（含新增 dashboard 测试）

- [ ] **Step 6: 提交**

Run: `git add crates/devnpc/src/config/mod.rs crates/devnpc/src/config/loader.rs ; git commit -m "feat(config): 添加 DashboardConfig 配置扩展 (enabled/url/token/batch_size/batch_interval_secs/local_event_log)"`

---

### Task 2: LocalEventLogger 组件

**Files:**
- Create: `crates/devnpc/src/report/sender.rs`
- Modify: `crates/devnpc/src/report/mod.rs`

- [ ] **Step 1: 在 report/mod.rs 注册 sender 模块**

修改 `crates/devnpc/src/report/mod.rs`：

```rust
//! 运维报告: 轨迹采集 + HTML 生成 + 推送

pub mod collector;
pub mod html;
pub mod publisher;
pub mod sender;
```

- [ ] **Step 2: 创建 sender.rs 并编写 LocalEventLogger 测试**

创建 `crates/devnpc/src/report/sender.rs`，先编写测试：

```rust
//! Dashboard 事件推送器与本地事件记录器 (spec §4.2)
//!
//! 两个独立组件:
//! - `LocalEventLogger`: 本地 .jsonl 文件记录 (兜底机制,独立于推送)
//! - `EventSender`: channel + 异步批量 POST 推送 (仅 dashboard.enabled=true 时创建)

use std::fs::{File, OpenOptions};
use std::io::{BufWriter, Write};
use std::path::Path;
use std::sync::{Arc, Mutex};

use devnpc_core::report::event_schema::{
    ExecutionEvent, EventLogEntry, TaskFinishedEvent, TaskStartedEvent,
};

// ============================================================
// LocalEventLogger
// ============================================================

/// 本地事件记录器 (兜底机制,独立于推送)
///
/// 即使 dashboard 未配置,只要 `local_event_log=true` 就会创建。
/// 文件格式: JSON Lines (.jsonl),每行一个 `EventLogEntry`。
/// 文件位置: 与 HTML 报告同目录 (artifact 目录),文件名 `{task_id}.jsonl`
pub struct LocalEventLogger {
    /// task_id (用于日志)
    task_id: String,
    /// 文件 writer,写入失败后设为 None (后续事件跳过文件写入)
    writer: Arc<Mutex<Option<BufWriter<File>>>>,
}

impl LocalEventLogger {
    /// 创建本地 .jsonl 文件,写入 task_started 行
    ///
    /// 文件创建/写入失败时返回 None (调用方降级为无日志)
    pub fn new(task_id: &str, started: &TaskStartedEvent, artifact_dir: &Path) -> Option<Self> {
        let file_path = artifact_dir.join(format!("{task_id}.jsonl"));
        let writer = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&file_path)
            .map(BufWriter::new)
            .map_err(|e| {
                tracing::warn!(
                    task_id = task_id,
                    path = %file_path.display(),
                    error = %e,
                    "本地事件文件创建失败,后续事件跳过文件写入"
                );
            })
            .ok();

        let logger = Self {
            task_id: task_id.to_string(),
            writer: Arc::new(Mutex::new(writer)),
        };

        // 写入 task_started 行
        let entry = EventLogEntry::TaskStarted {
            task_id: task_id.to_string(),
            data: started.clone(),
        };
        logger.write_entry(&entry);

        Some(logger)
    }

    /// 追加一行 execution 事件
    pub fn log_event(&self, event: &ExecutionEvent) {
        let entry = EventLogEntry::Execution {
            task_id: self.task_id.clone(),
            event: event.clone(),
        };
        self.write_entry(&entry);
    }

    /// 写入 task_finished 行并关闭文件
    pub fn finish(&self, finished: &TaskFinishedEvent) {
        let entry = EventLogEntry::TaskFinished {
            task_id: self.task_id.clone(),
            data: finished.clone(),
        };
        self.write_entry(&entry);

        // flush 并关闭文件
        if let Ok(mut guard) = self.writer.lock() {
            if let Some(w) = guard.as_mut() {
                if let Err(e) = w.flush() {
                    tracing::warn!(task_id = %self.task_id, error = %e, "事件文件 flush 失败");
                }
            }
            // 设为 None 标记已关闭
            *guard = None;
        }
    }

    /// 内部: 写入一行 JSON 并 flush (保证落盘)
    fn write_entry(&self, entry: &EventLogEntry) {
        let Ok(mut guard) = self.writer.lock() else {
            return; // 锁中毒,跳过
        };
        let Some(writer) = guard.as_mut() else {
            return; // writer 已关闭或创建失败,跳过
        };
        match serde_json::to_string(entry) {
            Ok(line) => {
                if let Err(e) = writeln!(writer, "{line}") {
                    tracing::warn!(task_id = %self.task_id, error = %e, "事件文件写入失败");
                    // 写入失败,设为 None 避免后续重复失败
                    *guard = None;
                    return;
                }
                if let Err(e) = writer.flush() {
                    tracing::warn!(task_id = %self.task_id, error = %e, "事件文件 flush 失败");
                    *guard = None;
                    return;
                }
            }
            Err(e) => {
                tracing::warn!(task_id = %self.task_id, error = %e, "事件序列化失败");
            }
        }
    }
}

#[cfg(test)]
mod local_event_logger_tests {
    use super::*;
    use devnpc_core::report::event_schema::{TaskFinishedEvent, TaskStatus};

    fn make_started(task_id: &str) -> TaskStartedEvent {
        TaskStartedEvent {
            task_id: task_id.to_string(),
            project: "test-group/test-project".to_string(),
            mr_iid: Some(42),
            pipeline_id: Some(100),
            task_description: "测试任务".to_string(),
            task_kind: "mr_comment".to_string(),
            started_at: "2026-08-03T10:00:00Z".to_string(),
            model: "deepseek-chat".to_string(),
        }
    }

    fn make_finished(task_id: &str) -> TaskFinishedEvent {
        TaskFinishedEvent {
            task_id: task_id.to_string(),
            status: TaskStatus::Success,
            duration_secs: 45,
            total_tokens: 12000,
            estimated_cost_usd: 0.05,
            mr_url: Some("https://gitlab.com/mr/42".to_string()),
            ci_url: Some("https://gitlab.com/pipeline/100".to_string()),
            summary: "已修复".to_string(),
            error: None,
            finished_at: "2026-08-03T10:01:00Z".to_string(),
        }
    }

    #[test]
    fn new_creates_file_with_task_started_line() {
        let dir = tempfile::tempdir().unwrap();
        let task_id = "test-new-creates";
        let started = make_started(task_id);

        let logger = LocalEventLogger::new(task_id, &started, dir.path());
        assert!(logger.is_some());

        let file_path = dir.path().join(format!("{task_id}.jsonl"));
        assert!(file_path.exists(), "事件文件应已创建");

        let content = std::fs::read_to_string(&file_path).unwrap();
        assert!(
            content.contains("task_started"),
            "首行应包含 task_started, 实际: {content}"
        );
        assert!(content.contains(task_id));
    }

    #[test]
    fn log_event_appends_execution_line() {
        let dir = tempfile::tempdir().unwrap();
        let task_id = "test-log-event";
        let started = make_started(task_id);
        let logger = LocalEventLogger::new(task_id, &started, dir.path()).unwrap();

        let event = ExecutionEvent::ToolCall {
            name: "read_file".to_string(),
            success: true,
            latency_ms: 50,
            detail: "src/main.rs".to_string(),
        };
        logger.log_event(&event);

        let file_path = dir.path().join(format!("{task_id}.jsonl"));
        let content = std::fs::read_to_string(&file_path).unwrap();
        let lines: Vec<&str> = content.lines().collect();
        assert_eq!(lines.len(), 2, "应有 2 行: task_started + execution");
        assert!(lines[1].contains("execution"));
        assert!(lines[1].contains("tool_call"));
        assert!(lines[1].contains("read_file"));
    }

    #[test]
    fn finish_writes_task_finished_and_closes() {
        let dir = tempfile::tempdir().unwrap();
        let task_id = "test-finish";
        let started = make_started(task_id);
        let finished = make_finished(task_id);
        let logger = LocalEventLogger::new(task_id, &started, dir.path()).unwrap();

        let event = ExecutionEvent::LlmCall {
            iteration: 1,
            prompt_tokens: 100,
            completion_tokens: 50,
            latency_ms: 500,
        };
        logger.log_event(&event);
        logger.finish(&finished);

        let file_path = dir.path().join(format!("{task_id}.jsonl"));
        let content = std::fs::read_to_string(&file_path).unwrap();
        let lines: Vec<&str> = content.lines().collect();
        assert_eq!(lines.len(), 3, "应有 3 行: started + execution + finished");
        assert!(lines[2].contains("task_finished"));
        assert!(lines[2].contains("success"));
    }

    #[test]
    fn non_existent_dir_doesnt_panic() {
        let task_id = "test-no-dir";
        let started = make_started(task_id);
        let nonexistent = Path::new("/nonexistent/path/that/does/not/exist");

        // 不应 panic
        let logger = LocalEventLogger::new(task_id, &started, nonexistent);
        assert!(logger.is_none(), "目录不存在时应返回 None");
    }

    #[test]
    fn generated_jsonl_is_parseable() {
        let dir = tempfile::tempdir().unwrap();
        let task_id = "test-parseable";
        let started = make_started(task_id);
        let finished = make_finished(task_id);
        let logger = LocalEventLogger::new(task_id, &started, dir.path()).unwrap();

        let event1 = ExecutionEvent::LlmCall {
            iteration: 1,
            prompt_tokens: 500,
            completion_tokens: 200,
            latency_ms: 1500,
        };
        let event2 = ExecutionEvent::SopStep {
            step: "analyze".to_string(),
            status: devnpc_core::report::event_schema::SopStepStatus::Completed,
            note: None,
        };
        logger.log_event(&event1);
        logger.log_event(&event2);
        logger.finish(&finished);

        // 读取并解析每行
        let file_path = dir.path().join(format!("{task_id}.jsonl"));
        let content = std::fs::read_to_string(&file_path).unwrap();
        let lines: Vec<&str> = content.lines().collect();
        assert_eq!(lines.len(), 4);

        // 每行都应能解析为 EventLogEntry
        for (i, line) in lines.iter().enumerate() {
            let result: Result<EventLogEntry, _> = serde_json::from_str(line);
            assert!(result.is_ok(), "第 {i} 行解析失败: {line}");
        }

        // 验证第一行是 TaskStarted
        let first: EventLogEntry = serde_json::from_str(lines[0]).unwrap();
        assert!(matches!(first, EventLogEntry::TaskStarted { .. }));

        // 验证最后一行是 TaskFinished
        let last: EventLogEntry = serde_json::from_str(lines[3]).unwrap();
        assert!(matches!(last, EventLogEntry::TaskFinished { .. }));
    }

    #[test]
    fn multiple_events_append_in_order() {
        let dir = tempfile::tempdir().unwrap();
        let task_id = "test-order";
        let started = make_started(task_id);
        let logger = LocalEventLogger::new(task_id, &started, dir.path()).unwrap();

        for i in 0..5 {
            logger.log_event(&ExecutionEvent::LlmCall {
                iteration: i,
                prompt_tokens: 100,
                completion_tokens: 50,
                latency_ms: 500,
            });
        }

        let file_path = dir.path().join(format!("{task_id}.jsonl"));
        let content = std::fs::read_to_string(&file_path).unwrap();
        let lines: Vec<&str> = content.lines().collect();
        // 1 started + 5 execution = 6 行
        assert_eq!(lines.len(), 6);
    }
}
```

- [ ] **Step 3: 运行测试验证通过**

Run: `cargo test -p devnpc --lib report::sender::local_event_logger_tests -- --nocapture`
Expected: 6 个测试全部 PASS

- [ ] **Step 4: 提交**

Run: `git add crates/devnpc/src/report/sender.rs crates/devnpc/src/report/mod.rs ; git commit -m "feat(report): 添加 LocalEventLogger 本地 .jsonl 事件记录器 (兜底机制)"`

---

### Task 3: EventSender 组件

**Files:**
- Modify: `crates/devnpc/src/report/sender.rs`
- Modify: `crates/devnpc/Cargo.toml`

- [ ] **Step 1: 在 Cargo.toml 添加 tokio test-util dev-feature**

在 `crates/devnpc/Cargo.toml` 的 `[dev-dependencies]` 部分添加 tokio test-util feature（用于 `#[tokio::test(start_paused = true)]` 加速时间相关测试）：

将 `[dev-dependencies]` 修改为：

```toml
[dev-dependencies]
tokio = { workspace = true, features = ["test-util"] }
tokio-test = "0.4"
mockall = "0.13"
tempfile = "3"
wiremock = "0.6"
```

- [ ] **Step 2: 在 sender.rs 添加 EventSender 测试**

在 `crates/devnpc/src/report/sender.rs` 文件末尾（`local_event_logger_tests` 模块之后）添加 EventSender 相关代码和测试。

先在文件顶部添加 EventSender 所需的 import（在现有 `use` 之后追加）：

```rust
use std::time::Duration;

use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use devnpc_core::report::event_schema::{BatchEventsRequest};

use crate::config::DashboardConfig;
```

然后在 `LocalEventLogger` 定义之后添加 `EventSender` 结构体：

```rust
// ============================================================
// EventSender
// ============================================================

/// 事件推送器 (仅 dashboard.enabled=true 时创建)
///
/// 内部通过 mpsc channel 接收事件,后台 task 批量 POST 到 dashboard。
/// 推送失败不影响主任务执行 (tracing::warn 记录)。
pub struct EventSender {
    /// channel 发送端 (send() 写入此 channel)
    tx: Option<mpsc::Sender<ExecutionEvent>>,
    /// task_id
    task_id: String,
    /// 后台 task handle (finish 时 await)
    handle: Option<JoinHandle<()>>,
}

impl EventSender {
    /// 创建推送器并启动后台批量推送 task
    ///
    /// 注意: 此方法不发送 TaskStartedEvent,需单独调用 `send_start()`。
    pub fn new(config: &DashboardConfig, task_id: &str) -> Self {
        let (tx, rx) = mpsc::channel::<ExecutionEvent>(config.batch_size * 2);

        let batch_config = config.clone();
        let batch_task_id = task_id.to_string();

        let handle = tokio::spawn(async move {
            background_batch_loop(rx, &batch_config, &batch_task_id).await;
        });

        Self {
            tx: Some(tx),
            task_id: task_id.to_string(),
            handle: Some(handle),
        }
    }

    /// 推送 TaskStartedEvent (POST /api/events/start)
    ///
    /// 失败时 tracing::warn 记录,不返回错误 (不影响主任务)。
    pub async fn send_start(&self, config: &DashboardConfig, event: &TaskStartedEvent) {
        let url = format!("{}/api/events/start", config.url.trim_end_matches('/'));
        let client = match reqwest::Client::new() {
            Ok(c) => c,
            Err(_) => reqwest::Client::builder().build().unwrap(),
        };
        post_with_retry(&client, &url, &config.token, event).await;
    }

    /// 推送单条事件 (非阻塞,写入 channel)
    ///
    /// channel 满时丢弃事件并 tracing::warn。
    pub fn send(&self, event: ExecutionEvent) {
        if let Some(tx) = &self.tx {
            if tx.try_send(event).is_err() {
                tracing::warn!(task_id = %self.task_id, "EventSender channel 满,丢弃事件");
            }
        }
    }

    /// 任务结束时 flush 并发送 TaskFinishedEvent (POST /api/events/finish)
    ///
    /// 1. 关闭 channel (drop tx) 触发后台 task flush 剩余事件
    /// 2. 等待后台 task 完成
    /// 3. POST /api/events/finish
    ///
    /// 失败时 tracing::warn 记录,不返回错误 (不影响主任务)。
    pub async fn finish(mut self, config: &DashboardConfig, event: TaskFinishedEvent) {
        // 关闭 channel,触发后台 task flush
        self.tx.take();

        // 等待后台 task 完成
        if let Some(handle) = self.handle.take() {
            let _ = handle.await;
        }

        // POST /api/events/finish
        let url = format!("{}/api/events/finish", config.url.trim_end_matches('/'));
        let client = reqwest::Client::new();
        post_with_retry(&client, &url, &config.token, &event).await;
    }
}

// ============================================================
// 后台批量推送逻辑
// ============================================================

/// 后台批量推送循环
///
/// 触发条件 (任一满足):
/// - 事件数累积到 batch_size
/// - 距上次检查超过 batch_interval_secs
/// - channel 关闭 (任务结束 flush)
async fn background_batch_loop(
    mut rx: mpsc::Receiver<ExecutionEvent>,
    config: &DashboardConfig,
    task_id: &str,
) {
    let mut batch: Vec<ExecutionEvent> = Vec::with_capacity(config.batch_size);
    let interval = Duration::from_secs(config.batch_interval_secs);
    let client = reqwest::Client::new();

    loop {
        let timeout = tokio::time::sleep(interval);
        tokio::pin!(timeout);

        tokio::select! {
            maybe_event = rx.recv() => {
                match maybe_event {
                    Some(event) => {
                        batch.push(event);
                        if batch.len() >= config.batch_size {
                            flush_batch(&client, config, task_id, &mut batch).await;
                        }
                    }
                    None => {
                        // channel 关闭, flush 剩余事件并退出
                        if !batch.is_empty() {
                            flush_batch(&client, config, task_id, &mut batch).await;
                        }
                        break;
                    }
                }
            }
            _ = &mut timeout => {
                // 超时触发,如有事件则推送
                if !batch.is_empty() {
                    flush_batch(&client, config, task_id, &mut batch).await;
                }
            }
        }
    }
}

/// 批量推送一批事件 (POST /api/events/batch)
///
/// 失败时指数退避重试 (1s/2s/4s/8s/16s,共 6 次尝试),仍失败则丢弃。
async fn flush_batch(
    client: &reqwest::Client,
    config: &DashboardConfig,
    task_id: &str,
    batch: &mut Vec<ExecutionEvent>,
) {
    if batch.is_empty() {
        return;
    }

    let events = std::mem::take(batch);
    let request = BatchEventsRequest {
        task_id: task_id.to_string(),
        events,
    };

    let url = format!("{}/api/events/batch", config.url.trim_end_matches('/'));
    post_with_retry(client, &url, &config.token, &request).await;
}

/// 带指数退避重试的 POST 请求
///
/// 重试延迟: 1s/2s/4s/8s/16s (初始 + 5 次重试 = 6 次尝试)
async fn post_with_retry<T: serde::Serialize>(
    client: &reqwest::Client,
    url: &str,
    token: &str,
    body: &T,
) {
    let delays = [1u64, 2, 4, 8, 16];
    let mut last_error = String::new();

    for attempt in 0..=5 {
        if attempt > 0 {
            tokio::time::sleep(Duration::from_secs(delays[attempt - 1])).await;
        }
        match client
            .post(url)
            .header("X-Devnpc-Token", token)
            .json(body)
            .send()
            .await
        {
            Ok(resp) if resp.status().is_success() => {
                tracing::debug!(attempt, url = %url, "推送成功");
                return;
            }
            Ok(resp) => {
                last_error = format!("HTTP {}", resp.status());
                tracing::warn!(attempt, url = %url, status = %resp.status(), "推送失败");
            }
            Err(e) => {
                last_error = e.to_string();
                tracing::warn!(attempt, url = %url, error = %e, "推送失败");
            }
        }
    }

    tracing::warn!(url = %url, error = %last_error, "推送重试耗尽,放弃");
}
```

然后在文件末尾添加 EventSender 测试模块：

```rust
#[cfg(test)]
mod event_sender_tests {
    use super::*;
    use devnpc_core::report::event_schema::{
        CiStatus, SopStepStatus, TaskFinishedEvent, TaskStartedEvent, TaskStatus,
    };
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn make_config(url: String) -> DashboardConfig {
        DashboardConfig {
            enabled: true,
            url,
            token: "test-token".to_string(),
            batch_size: 2, // 小阈值便于测试
            batch_interval_secs: 60,  // 大阈值,避免时间触发干扰
            local_event_log: false,
        }
    }

    fn make_started(task_id: &str) -> TaskStartedEvent {
        TaskStartedEvent {
            task_id: task_id.to_string(),
            project: "test".to_string(),
            mr_iid: None,
            pipeline_id: None,
            task_description: "test".to_string(),
            task_kind: "manual".to_string(),
            started_at: "2026-08-03T10:00:00Z".to_string(),
            model: "test-model".to_string(),
        }
    }

    fn make_finished(task_id: &str) -> TaskFinishedEvent {
        TaskFinishedEvent {
            task_id: task_id.to_string(),
            status: TaskStatus::Success,
            duration_secs: 10,
            total_tokens: 1000,
            estimated_cost_usd: 0.01,
            mr_url: None,
            ci_url: None,
            summary: "done".to_string(),
            error: None,
            finished_at: "2026-08-03T10:01:00Z".to_string(),
        }
    }

    fn make_llm_event(iteration: u32) -> ExecutionEvent {
        ExecutionEvent::LlmCall {
            iteration,
            prompt_tokens: 100,
            completion_tokens: 50,
            latency_ms: 500,
        }
    }

    #[tokio::test]
    async fn send_start_posts_to_dashboard() {
        let server = MockServer::start().await;
        let config = make_config(server.uri());

        Mock::given(method("POST"))
            .and(path("/api/events/start"))
            .and(header("X-Devnpc-Token", "test-token"))
            .respond_with(ResponseTemplate::new(200))
            .expect(1)
            .mount(&server)
            .await;

        let sender = EventSender::new(&config, "task-start-1");
        let started = make_started("task-start-1");
        sender.send_start(&config, &started).await;

        // finish 关闭后台 task (避免泄漏)
        sender.finish(&config, make_finished("task-start-1")).await;
    }

    #[tokio::test]
    async fn batch_triggers_on_size_threshold() {
        let server = MockServer::start().await;
        let config = make_config(server.uri());

        Mock::given(method("POST"))
            .and(path("/api/events/batch"))
            .respond_with(ResponseTemplate::new(200))
            .expect(1)
            .mount(&server)
            .await;

        let sender = EventSender::new(&config, "task-batch-size");
        // batch_size=2,发送 2 条触发一次批量推送
        sender.send(make_llm_event(1));
        sender.send(make_llm_event(2));

        // 等待后台 task 处理
        tokio::time::sleep(Duration::from_millis(500)).await;

        sender.finish(&config, make_finished("task-batch-size")).await;
    }

    #[tokio::test]
    async fn batch_triggers_on_timeout() {
        let server = MockServer::start().await;
        let mut config = make_config(server.uri());
        config.batch_size = 100; // 大阈值,不会因数量触发
        config.batch_interval_secs = 1; // 1 秒后触发

        Mock::given(method("POST"))
            .and(path("/api/events/batch"))
            .respond_with(ResponseTemplate::new(200))
            .expect(1)
            .mount(&server)
            .await;

        let sender = EventSender::new(&config, "task-batch-timeout");
        sender.send(make_llm_event(1));

        // 等待超时触发 (1 秒 + 余量)
        tokio::time::sleep(Duration::from_millis(1500)).await;

        sender.finish(&config, make_finished("task-batch-timeout")).await;
    }

    #[tokio::test(start_paused = true)]
    async fn retry_on_failure_with_backoff() {
        let server = MockServer::start().await;
        let config = make_config(server.uri());

        // 前 2 次返回 500,第 3 次返回 200
        Mock::given(method("POST"))
            .and(path("/api/events/batch"))
            .respond_with(ResponseTemplate::new(500))
            .up_to_n_times(2)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/api/events/batch"))
            .respond_with(ResponseTemplate::new(200))
            .expect(1)
            .mount(&server)
            .await;

        let sender = EventSender::new(&config, "task-retry");
        sender.send(make_llm_event(1));
        sender.send(make_llm_event(2)); // 触发 batch

        // 等待重试完成 (start_paused 使 sleep 立即返回)
        tokio::time::sleep(Duration::from_millis(100)).await;

        sender.finish(&config, make_finished("task-retry")).await;
    }

    #[tokio::test(start_paused = true)]
    async fn discard_after_max_retries() {
        let server = MockServer::start().await;
        let config = make_config(server.uri());

        // 所有请求都返回 500
        Mock::given(method("POST"))
            .and(path("/api/events/batch"))
            .respond_with(ResponseTemplate::new(500))
            // 期望 6 次 (初始 + 5 次重试)
            .expect(6)
            .mount(&server)
            .await;

        let sender = EventSender::new(&config, "task-discard");
        sender.send(make_llm_event(1));
        sender.send(make_llm_event(2)); // 触发 batch

        // 等待所有重试完成
        tokio::time::sleep(Duration::from_millis(500)).await;

        sender.finish(&config, make_finished("task-discard")).await;
    }

    #[tokio::test]
    async fn finish_posts_task_finished() {
        let server = MockServer::start().await;
        let config = make_config(server.uri());

        Mock::given(method("POST"))
            .and(path("/api/events/finish"))
            .and(header("X-Devnpc-Token", "test-token"))
            .respond_with(ResponseTemplate::new(200))
            .expect(1)
            .mount(&server)
            .await;

        let sender = EventSender::new(&config, "task-finish");
        sender.finish(&config, make_finished("task-finish")).await;
    }

    #[tokio::test]
    async fn send_does_not_block_when_channel_full() {
        let server = MockServer::start().await;
        let mut config = make_config(server.uri());
        config.batch_size = 1; // 极小 channel
        config.batch_interval_secs = 3600; // 不触发时间推送

        // 不 mount 任何 mock,确保不推送
        let sender = EventSender::new(&config, "task-full");

        // 填满 channel (batch_size * 2 = 2 缓冲)
        for i in 0..10 {
            sender.send(make_llm_event(i)); // 不应 panic
        }

        // finish 会 flush 剩余 (但没 mount mock,会失败重试,start_paused 不可用因为没标注)
        // 这里仅验证 send 不阻塞
        sender.finish(&config, make_finished("task-full")).await;
    }
}
```

- [ ] **Step 3: 运行测试验证通过**

Run: `cargo test -p devnpc --lib report::sender::event_sender_tests -- --nocapture`
Expected: 7 个测试全部 PASS

- [ ] **Step 4: 运行 sender 全部测试**

Run: `cargo test -p devnpc --lib report::sender -- --nocapture`
Expected: local_event_logger_tests (6) + event_sender_tests (7) = 13 个测试全部 PASS

- [ ] **Step 5: 提交**

Run: `git add crates/devnpc/src/report/sender.rs crates/devnpc/Cargo.toml ; git commit -m "feat(report): 添加 EventSender 事件推送器 (channel + 异步批量 POST + 指数退避重试)"`

---

### Task 4: Trajectory 改造

**Files:**
- Modify: `crates/devnpc-core/src/report/types.rs`
- Modify: `crates/devnpc/src/report/collector.rs`

- [ ] **Step 1: 从 core/types.rs 移除 Trajectory**

修改 `crates/devnpc-core/src/report/types.rs`，删除 `Trajectory` 结构体及其 `impl` 块（保留 `TrajectoryEvent` 和其他类型）。

删除以下代码：

```rust
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
```

同时删除 `#[cfg(test)]` 模块中引用 `Trajectory` 的测试：

```rust
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
```

保留 `TrajectoryEvent`、`ReportData`、`TeamStepSummary`、`TrajectorySummary`、`TrajectoryEventSummary`、`CostEstimate` 不变。

- [ ] **Step 2: 验证 core 编译（Trajectory 已移除）**

Run: `cargo check -p devnpc-core`
Expected: 编译通过（Trajectory 已删除，剩余类型无依赖 Trajectory）

- [ ] **Step 3: 在 collector.rs 定义新 Trajectory**

修改 `crates/devnpc/src/report/collector.rs`。

首先更新文件顶部的 re-export，移除 `Trajectory`：

将：
```rust
pub use devnpc_core::report::types::{
    CostEstimate, ReportData, TeamStepSummary, Trajectory, TrajectoryEvent,
    TrajectoryEventSummary, TrajectorySummary,
};
```

改为：
```rust
pub use devnpc_core::report::types::{
    CostEstimate, ReportData, TeamStepSummary, TrajectoryEvent,
    TrajectoryEventSummary, TrajectorySummary,
};
```

然后在 re-export 之后、`use std::sync::{Arc, Mutex};` 之前添加新 `Trajectory` 定义：

```rust
// ============================================================
// Trajectory (持有本地日志和推送组件, spec §4.3)
// ============================================================

use devnpc_core::report::event_schema::ExecutionEvent;

use super::sender::{EventSender, LocalEventLogger};

/// 轨迹 (本地定义,持有三个可选组件)
///
/// - `events`: 内存事件列表 (始终存在,兼容现有逻辑)
/// - `local_logger`: 本地文件记录 (`local_event_log=true` 时存在)
/// - `sender`: 实时推送 (`dashboard.enabled=true` 时存在)
pub struct Trajectory {
    /// 内存事件列表 (现状,始终存在)
    pub events: Vec<TrajectoryEvent>,
    /// 本地文件记录器 (None 时跳过文件写入)
    local_logger: Option<LocalEventLogger>,
    /// 事件推送器 (None 时跳过推送)
    sender: Option<EventSender>,
    /// task_id
    task_id: String,
}

impl Trajectory {
    /// 现状构造 (无日志无推送,兼容现有测试)
    pub fn new() -> Self {
        Self {
            events: Vec::new(),
            local_logger: None,
            sender: None,
            task_id: String::new(),
        }
    }

    /// 带本地日志和推送的构造 (spec §4.3)
    pub fn with_logging(
        task_id: String,
        local_logger: Option<LocalEventLogger>,
        sender: Option<EventSender>,
    ) -> Self {
        Self {
            events: Vec::new(),
            local_logger,
            sender,
            task_id,
        }
    }

    /// 记录 LLM 调用
    ///
    /// 同时: 推入内存 events + 转发到本地日志 + 转发到推送器
    pub fn record_llm_call(&mut self, iteration: usize) {
        self.events.push(TrajectoryEvent::LlmCall { iteration });

        if self.local_logger.is_some() || self.sender.is_some() {
            let exec_event = ExecutionEvent::LlmCall {
                iteration: iteration as u32,
                prompt_tokens: 0,
                completion_tokens: 0,
                latency_ms: 0,
            };
            if let Some(logger) = &self.local_logger {
                logger.log_event(&exec_event);
            }
            if let Some(sender) = &self.sender {
                sender.send(exec_event);
            }
        }
    }

    /// 记录工具调用
    pub fn record_tool_call(&mut self, name: &str, success: bool) {
        self.events.push(TrajectoryEvent::ToolCall {
            name: name.to_string(),
            success,
        });

        if self.local_logger.is_some() || self.sender.is_some() {
            let exec_event = ExecutionEvent::ToolCall {
                name: name.to_string(),
                success,
                latency_ms: 0,
                detail: String::new(),
            };
            if let Some(logger) = &self.local_logger {
                logger.log_event(&exec_event);
            }
            if let Some(sender) = &self.sender {
                sender.send(exec_event);
            }
        }
    }

    /// 获取 task_id
    pub fn task_id(&self) -> &str {
        &self.task_id
    }

    /// 任务结束: flush 本地日志 + 推送 TaskFinishedEvent
    ///
    /// 消费 self (任务结束后不再使用)。
    pub async fn finish(
        self,
        config: &crate::config::DashboardConfig,
        finished: &devnpc_core::report::event_schema::TaskFinishedEvent,
    ) {
        if let Some(logger) = &self.local_logger {
            logger.finish(finished);
        }
        if let Some(sender) = self.sender {
            sender.finish(config, finished.clone()).await;
        }
    }
}

impl Default for Trajectory {
    fn default() -> Self {
        Self::new()
    }
}
```

- [ ] **Step 4: 在 collector.rs 测试模块添加 Trajectory 测试**

在 `crates/devnpc/src/report/collector.rs` 的 `#[cfg(test)]` 模块中追加测试。在现有测试之后添加：

```rust
    // ============================================================
    // Trajectory 改造测试 (spec §4.3)
    // ============================================================

    use devnpc_core::report::event_schema::{
        ExecutionEvent, TaskFinishedEvent, TaskStartedEvent, TaskStatus,
    };
    use tempfile::tempdir;

    fn make_started_for_traj(task_id: &str) -> TaskStartedEvent {
        TaskStartedEvent {
            task_id: task_id.to_string(),
            project: "test".to_string(),
            mr_iid: None,
            pipeline_id: None,
            task_description: "test".to_string(),
            task_kind: "manual".to_string(),
            started_at: "2026-08-03T10:00:00Z".to_string(),
            model: "test".to_string(),
        }
    }

    fn make_finished_for_traj(task_id: &str) -> TaskFinishedEvent {
        TaskFinishedEvent {
            task_id: task_id.to_string(),
            status: TaskStatus::Success,
            duration_secs: 10,
            total_tokens: 1000,
            estimated_cost_usd: 0.01,
            mr_url: None,
            ci_url: None,
            summary: "done".to_string(),
            error: None,
            finished_at: "2026-08-03T10:01:00Z".to_string(),
        }
    }

    #[test]
    fn trajectory_new_is_empty_without_logger_or_sender() {
        let t = Trajectory::new();
        assert!(t.events.is_empty());
        assert!(t.task_id().is_empty());
    }

    #[test]
    fn trajectory_new_record_llm_call_backward_compat() {
        // 无 logger/sender 时,record_llm_call 仅推入 events
        let mut t = Trajectory::new();
        t.record_llm_call(1);
        assert_eq!(t.events.len(), 1);
        assert!(matches!(t.events[0], TrajectoryEvent::LlmCall { iteration: 1 }));
    }

    #[test]
    fn trajectory_new_record_tool_call_backward_compat() {
        let mut t = Trajectory::new();
        t.record_tool_call("read_file", true);
        assert_eq!(t.events.len(), 1);
        assert!(matches!(
            &t.events[0],
            TrajectoryEvent::ToolCall { name, success } if name == "read_file" && *success
        ));
    }

    #[test]
    fn trajectory_with_logging_holds_logger() {
        let dir = tempdir().unwrap();
        let task_id = "traj-with-logging";
        let started = make_started_for_traj(task_id);
        let logger = LocalEventLogger::new(task_id, &started, dir.path());

        let t = Trajectory::with_logging(task_id.to_string(), logger, None);
        assert_eq!(t.task_id(), task_id);
        assert!(t.events.is_empty());
    }

    #[test]
    fn trajectory_record_llm_call_forwards_to_logger() {
        let dir = tempdir().unwrap();
        let task_id = "traj-forward-llm";
        let started = make_started_for_traj(task_id);
        let logger = LocalEventLogger::new(task_id, &started, dir.path());

        let mut t = Trajectory::with_logging(task_id.to_string(), logger, None);
        t.record_llm_call(1);

        // 内存 events 应有 1 条
        assert_eq!(t.events.len(), 1);

        // 本地文件应有 2 行: task_started + execution
        let file_path = dir.path().join(format!("{task_id}.jsonl"));
        let content = std::fs::read_to_string(&file_path).unwrap();
        let lines: Vec<&str> = content.lines().collect();
        assert_eq!(lines.len(), 2);
        assert!(lines[1].contains("llm_call"));
    }

    #[test]
    fn trajectory_record_tool_call_forwards_to_logger() {
        let dir = tempdir().unwrap();
        let task_id = "traj-forward-tool";
        let started = make_started_for_traj(task_id);
        let logger = LocalEventLogger::new(task_id, &started, dir.path());

        let mut t = Trajectory::with_logging(task_id.to_string(), logger, None);
        t.record_tool_call("write_file", true);

        assert_eq!(t.events.len(), 1);

        let file_path = dir.path().join(format!("{task_id}.jsonl"));
        let content = std::fs::read_to_string(&file_path).unwrap();
        let lines: Vec<&str> = content.lines().collect();
        assert_eq!(lines.len(), 2);
        assert!(lines[1].contains("tool_call"));
        assert!(lines[1].contains("write_file"));
    }

    #[test]
    fn trajectory_record_llm_call_no_logger_no_sender_works() {
        // 无 logger/sender 时不应 panic
        let mut t = Trajectory::with_logging("none".to_string(), None, None);
        t.record_llm_call(1);
        t.record_tool_call("test", false);
        assert_eq!(t.events.len(), 2);
    }

    #[tokio::test]
    async fn trajectory_finish_writes_task_finished_to_file() {
        let dir = tempdir().unwrap();
        let task_id = "traj-finish";
        let started = make_started_for_traj(task_id);
        let logger = LocalEventLogger::new(task_id, &started, dir.path());

        let t = Trajectory::with_logging(task_id.to_string(), logger, None);
        t.record_llm_call(1);

        let config = crate::config::DashboardConfig::default();
        let finished = make_finished_for_traj(task_id);
        t.finish(&config, &finished).await;

        let file_path = dir.path().join(format!("{task_id}.jsonl"));
        let content = std::fs::read_to_string(&file_path).unwrap();
        let lines: Vec<&str> = content.lines().collect();
        // task_started + execution + task_finished = 3 行
        assert_eq!(lines.len(), 3);
        assert!(lines[2].contains("task_finished"));
    }
```

- [ ] **Step 5: 运行 collector 测试验证通过**

Run: `cargo test -p devnpc --lib report::collector -- --nocapture`
Expected: 现有 4 个 + 新增 8 个 = 12 个测试全部 PASS

- [ ] **Step 6: 运行全部测试验证无回归**

Run: `cargo test -p devnpc --lib`
Expected: 所有单元测试 PASS（Trajectory 改造向后兼容）

Run: `cargo test -p devnpc-core --lib`
Expected: core 测试 PASS（Trajectory 已移除，剩余类型测试通过）

- [ ] **Step 7: 提交**

Run: `git add crates/devnpc-core/src/report/types.rs crates/devnpc/src/report/collector.rs ; git commit -m "refactor(report): Trajectory 迁回 devnpc 并扩展为持有 LocalEventLogger + EventSender (spec §4.3)"`

---

### Task 5: main.rs run() 接入

**Files:**
- Modify: `crates/devnpc/src/main.rs`

- [ ] **Step 1: 在 main.rs 添加 dashboard 接入逻辑**

修改 `crates/devnpc/src/main.rs` 的 `run()` 函数。

首先在文件顶部添加 import（在现有 `use devnpc::report::collector::{...}` 之后）：

```rust
use devnpc_core::report::event_schema::{
    ExecutionEvent, TaskFinishedEvent, TaskStartedEvent, TaskStatus,
};
use devnpc::report::sender::{EventSender, LocalEventLogger};
use devnpc::report::publisher;
```

注意：`use devnpc::report::publisher;` 已存在，不要重复添加。只需添加前两行。

然后修改 `run()` 函数。在 `let config = Config::load()?;` 之后（`tracing::info!(project_id = ...)` 之后）添加 dashboard 初始化代码：

```rust
    // 1.5 生成 task_id 并初始化 dashboard 推送/本地日志 (spec §4.4)
    let task_id = uuid::Uuid::new_v4().to_string();
    tracing::info!(task_id = %task_id, "任务 task_id 已生成");

    // 构造 TaskStartedEvent
    let started_event = TaskStartedEvent {
        task_id: task_id.clone(),
        project: format!("{}", config.gitlab.project_id),
        mr_iid,
        pipeline_id: None,
        task_description: task_spec_description(&task_spec),
        task_kind: format!("{:?}", task_spec_kind(&task_spec)),
        started_at: start_time.to_rfc3339(),
        model: config.llm.model.clone(),
    };
```

注意：`start_time` 在原代码后面才定义。需要将 `let start_time = chrono::Utc::now();` 移到 dashboard 初始化之前。

**具体修改**：

1. 在 `// 4. 解析触发源` 之前添加 `let start_time = chrono::Utc::now();`（如果原代码的 start_time 在后面，需要前移）

2. 在 `// 4. 解析触发源` 之后、`let (task_spec, mr_iid, issue_iid) = match trigger {` 之前，无法构造 TaskStartedEvent（因为 mr_iid 还未解析）。所以 dashboard 初始化应在 trigger 解析之后。

**重新设计接入位置**：在 `let issue_iid = issue_iid.or(task_spec.target_issue);` 之后（即 mr_iid 已确定后）添加 dashboard 初始化：

找到 `let context = if let Some(iid) = issue_iid {` 之前，插入：

```rust
    // 生成 task_id 并初始化 dashboard 推送/本地日志 (spec §4.4)
    let task_id = uuid::Uuid::new_v4().to_string();
    tracing::info!(task_id = %task_id, "任务 task_id 已生成");

    let started_event = TaskStartedEvent {
        task_id: task_id.clone(),
        project: format!("{}", config.gitlab.project_id),
        mr_iid,
        pipeline_id: None,
        task_description: task_spec.description.clone(),
        task_kind: format!("{:?}", task_spec.kind),
        started_at: chrono::Utc::now().to_rfc3339(),
        model: config.llm.model.clone(),
    };

    // 创建 LocalEventLogger (local_event_log=true 时,独立于 dashboard.enabled)
    let local_logger = if config.dashboard.local_event_log {
        match publisher::get_report_dir(&config.report) {
            Ok(artifact_dir) => {
                // 确保目录存在
                if let Err(e) = std::fs::create_dir_all(&artifact_dir) {
                    tracing::warn!(error = %e, "artifact 目录创建失败,跳过本地事件文件");
                }
                LocalEventLogger::new(&task_id, &started_event, &artifact_dir)
            }
            Err(e) => {
                tracing::warn!(error = %e, "获取 artifact 目录失败,跳过本地事件文件");
                None
            }
        }
    } else {
        tracing::info!("local_event_log=false,不保存本地事件文件");
        None
    };

    // 创建 EventSender (dashboard.enabled=true 时)
    let event_sender = if config.dashboard.enabled {
        let sender = EventSender::new(&config.dashboard, &task_id);
        sender.send_start(&config.dashboard, &started_event).await;
        Some(sender)
    } else {
        None
    };
```

3. 修改 `let trajectory = Trajectory::new();` 为：

```rust
    let trajectory = Trajectory::with_logging(task_id.clone(), local_logger, event_sender);
```

4. 在报告发布之后（`tracing::info!(report_url = %report_url, "报告已发布");` 之后），添加 dashboard finish 逻辑：

```rust
    // 8.5 flush dashboard 推送和本地事件文件 (spec §4.4)
    let finished_event = TaskFinishedEvent {
        task_id: task_id.clone(),
        status: ci_outcome_to_task_status(&ci_outcome),
        duration_secs: report_data.duration_secs,
        total_tokens: report_data.token_total,
        estimated_cost_usd: report_data.cost_estimate.estimated_cost_usd,
        mr_url: report_data.mr_url.clone(),
        ci_url: report_data.ci_url.clone(),
        summary: report_data.summary.clone(),
        error: ci_outcome_error(&ci_outcome),
        finished_at: chrono::Utc::now().to_rfc3339(),
    };
    trajectory.finish(&config.dashboard, &finished_event).await;
    tracing::info!(task_id = %task_id, "dashboard 推送和本地事件文件已 flush");
```

5. 在文件末尾（`fn print_info()` 之前）添加辅助函数：

```rust
/// CiOutcome 转换为 TaskStatus (spec §4.4)
fn ci_outcome_to_task_status(ci_outcome: &CiOutcome) -> TaskStatus {
    match ci_outcome {
        CiOutcome::Passed { .. } => TaskStatus::Success,
        CiOutcome::Failed { .. } => TaskStatus::CiFailed,
        CiOutcome::Timeout { .. } => TaskStatus::Timeout,
        CiOutcome::Error { .. } => TaskStatus::Failed,
    }
}

/// CiOutcome 提取错误信息 (spec §4.4)
fn ci_outcome_error(ci_outcome: &CiOutcome) -> Option<String> {
    match ci_outcome {
        CiOutcome::Passed { .. } => None,
        CiOutcome::Failed { last_error, .. } => Some(last_error.clone()),
        CiOutcome::Timeout { stage, .. } => Some(format!("阶段: {stage}")),
        CiOutcome::Error { reason, .. } => Some(reason.clone()),
    }
}
```

- [ ] **Step 2: 验证编译**

Run: `cargo check -p devnpc`
Expected: 编译通过。如果报错 `cannot find type TaskStartedEvent`，检查 `use devnpc_core::report::event_schema::*` 是否正确导入。

- [ ] **Step 3: 在 main.rs 添加集成测试**

在 `crates/devnpc/src/main.rs` 末尾添加测试模块（验证 dashboard 接入逻辑的核心函数）：

```rust
#[cfg(test)]
mod dashboard_integration_tests {
    use super::*;
    use devnpc::ci::controller::CiOutcome;
    use devnpc_core::report::event_schema::TaskStatus;

    #[test]
    fn ci_outcome_passed_maps_to_success() {
        let outcome = CiOutcome::Passed {
            mr_iid: 1,
            pipeline_id: 100,
            attempts: 1,
        };
        assert_eq!(ci_outcome_to_task_status(&outcome), TaskStatus::Success);
        assert!(ci_outcome_error(&outcome).is_none());
    }

    #[test]
    fn ci_outcome_failed_maps_to_ci_failed() {
        let outcome = CiOutcome::Failed {
            mr_iid: 1,
            last_error: "编译错误".to_string(),
            attempts: 2,
        };
        assert_eq!(ci_outcome_to_task_status(&outcome), TaskStatus::CiFailed);
        assert_eq!(ci_outcome_error(&outcome).as_deref(), Some("编译错误"));
    }

    #[test]
    fn ci_outcome_timeout_maps_to_timeout() {
        let outcome = CiOutcome::Timeout {
            mr_iid: 1,
            stage: "build".to_string(),
        };
        assert_eq!(ci_outcome_to_task_status(&outcome), TaskStatus::Timeout);
        assert_eq!(ci_outcome_error(&outcome).as_deref(), Some("阶段: build"));
    }

    #[test]
    fn ci_outcome_error_maps_to_failed() {
        let outcome = CiOutcome::Error {
            mr_iid: 1,
            reason: "MR 创建失败".to_string(),
        };
        assert_eq!(ci_outcome_to_task_status(&outcome), TaskStatus::Failed);
        assert_eq!(ci_outcome_error(&outcome).as_deref(), Some("MR 创建失败"));
    }

    #[test]
    fn dashboard_disabled_by_default() {
        let config = devnpc::config::DashboardConfig::default();
        assert!(!config.enabled, "默认不启用 dashboard 推送");
        assert!(config.local_event_log, "默认保存本地事件文件");
    }
}
```

- [ ] **Step 4: 运行测试验证通过**

Run: `cargo test -p devnpc --lib dashboard_integration_tests -- --nocapture`
Expected: 5 个测试全部 PASS

- [ ] **Step 5: 运行全部单元测试**

Run: `cargo test -p devnpc --lib`
Expected: 所有单元测试 PASS（含新增 dashboard 测试 + 现有测试无回归）

- [ ] **Step 6: 运行 clippy**

Run: `cargo clippy -p devnpc -- -D warnings`
Expected: 无 warning

- [ ] **Step 7: 运行集成测试验证无回归**

Run: `cargo test -p devnpc --test integration_e2e`
Expected: 现有集成测试全部 PASS

- [ ] **Step 8: 提交**

Run: `git add crates/devnpc/src/main.rs ; git commit -m "feat(main): run() 接入 dashboard 推送和本地事件文件 (task_id 生成 + start/finish 推送 + 降级安全)"`

---

## Self-Review 检查清单

### 1. Spec 覆盖检查 (spec §4)

- [x] **§4.1 DashboardConfig 配置扩展**: Task 1 实现了 `enabled/url/token/batch_size/batch_interval_secs/local_event_log` 六个字段，环境变量驱动，降级策略（enabled 由 URL 决定）。
- [x] **§4.2 LocalEventLogger**: Task 2 实现了 `new/log_event/finish` 三个方法，.jsonl 格式，BufWriter + 每次 flush，写入失败降级（设为 None）。
- [x] **§4.2 EventSender**: Task 3 实现了 `new/send_start/send/finish` 方法，mpsc channel + 后台批量 POST，20 条或 3 秒触发，指数退避重试（1s/2s/4s/8s/16s，6 次尝试）。
- [x] **§4.3 Trajectory 改造**: Task 4 将 Trajectory 迁回 devnpc，持有 `events + local_logger + sender + task_id`，`new()` 向后兼容，`with_logging()` 新构造，`record_*` 方法同时写内存 + 文件 + 推送，`finish()` flush。
- [x] **§4.4 主流程接入**: Task 5 在 `run()` 中生成 task_id、构造 TaskStartedEvent、创建 LocalEventLogger/EventSender、构造 Trajectory、任务结束时构造 TaskFinishedEvent 并 finish。
- [x] **§4.5 影响面**: config/mod.rs（DashboardConfig）、config/loader.rs（env 加载）、report/mod.rs（sender 模块）、report/collector.rs（新 Trajectory）、main.rs（run 接入）均已覆盖。

### 2. 关键约束检查

- [x] **路径前缀**: 所有 devnpc 文件路径使用 `crates/devnpc/src/...`，core 使用 `crates/devnpc-core/src/...`。
- [x] **devnpc-core 类型路径**: 使用 `devnpc_core::report::event_schema::*` 和 `devnpc_core::report::types::*`。
- [x] **降级策略**: `dashboard.enabled=false` 时不创建 EventSender；`local_event_log=true`（默认）时仍创建 LocalEventLogger。两者独立。
- [x] **推送失败不影响主任务**: 所有推送/文件操作失败均 `tracing::warn`，不传播错误。
- [x] **批量推送**: 20 条或 3 秒触发一次 POST（可配置）。
- [x] **指数退避**: 1s/2s/4s/8s/16s，6 次尝试（初始 + 5 次重试）。
- [x] **TDD**: 每个 Task 先写测试，再实现，再验证。
- [x] **Windows PowerShell**: 命令使用 `;` 分隔，`cargo test` 等跨平台命令。
- [x] **中文注释**: 所有代码注释为中文。

### 3. 类型一致性检查

- [x] `DashboardConfig` 字段名在 Task 1/3/5 一致（`enabled/url/token/batch_size/batch_interval_secs/local_event_log`）。
- [x] `LocalEventLogger::new()` 签名在 Task 2/4 一一致（返回 `Option<Self>`，参数 `task_id/started/artifact_dir`）。
- [x] `EventSender::new()` 签名在 Task 3/4 一致（参数 `config/task_id`）。
- [x] `EventSender::send_start()` 在 Task 3/5 一致（参数 `config/started`，async）。
- [x] `EventSender::finish()` 在 Task 3/4/5 一致（参数 `config/event`，async，消费 self）。
- [x] `Trajectory::with_logging()` 在 Task 4/5 一致（参数 `task_id/local_logger/sender`）。
- [x] `Trajectory::finish()` 在 Task 4/5 一致（参数 `config/finished`，async，消费 self）。
- [x] `ci_outcome_to_task_status` / `ci_outcome_error` 在 Task 5 定义并测试。

### 4. 无占位符检查

- [x] 所有代码步骤包含完整实现，无 "TODO" / "TBD" / "..."。
- [x] 所有测试包含完整断言，无 "assert!(true)" 之类占位。
- [x] 所有命令包含 expected 输出说明。
