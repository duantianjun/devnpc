# Dashboard Phase 5: E2E 集成测试 + 文档 实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 为 devnpc-dashboard 编写端到端集成测试、SSE 实时推送测试、导入流程测试,为 devnpc 侧 EventSender/LocalEventLogger 编写对接 dashboard 的集成测试,并更新 `.env.example`,达成 ~450 测试覆盖目标。

**Architecture:** 测试分两层:(1) `devnpc-dashboard` 内部 E2E——在随机端口启动真实 axum dashboard 实例,用 reqwest 走完整 HTTP 流程(start/batch/finish/SSE/import)并校验响应;(2) `devnpc` 内部集成——用 wiremock 模拟 dashboard 验证 EventSender 推送策略与降级行为,用 tempfile 验证 LocalEventLogger 生成可被 dashboard 导入的 `.jsonl` 文件。所有测试遵循 TDD 节奏:先写测试 → 运行 → 通过 → 提交。

**Tech Stack:** Rust 2024 edition, axum 0.7(被测服务), reqwest 0.12(测试 HTTP 客户端 + SSE 流), wiremock 0.6(HTTP mock), tempfile 3(临时 SQLite/日志目录), tokio(异步测试运行时), devnpc-core(共享事件类型)

**关联 spec:** [2026-08-03-devnpc-dashboard-design.md](../specs/2026-08-03-devnpc-dashboard-design.md) §8.2 集成测试 / §8.4 测试覆盖目标

---

## 前置条件与 API 假设

本阶段(Phase 5)假设 Phase 1–4 已交付以下公共 API。若实际实现路径/命名有差异,执行者在调整 `use` 路径与方法名后即可运行测试,测试逻辑本身不变。

**devnpc-core(Phase 1,已完成)暴露:**
- `devnpc_core::report::event_schema::{TaskStartedEvent, ExecutionEvent, TaskFinishedEvent, SopStepStatus, CiStatus, TaskStatus, EventLogEntry, BatchEventsRequest, ImportResult}`
- `EventLogEntry` 使用 `#[serde(tag = "kind")]`,变体序列化为 `task_started` / `execution` / `task_finished`
- `ExecutionEvent` 使用 `#[serde(tag = "type")]`,变体序列化为 `llm_call` / `tool_call` / `sop_step` / `ci_status` / `team_handoff`

**devnpc(Phase 2)暴露:**
- `devnpc::config::DashboardConfig { enabled: bool, url: String, token: String, batch_size: usize, batch_interval_secs: u64, local_event_log: bool }`
- `devnpc::report::sender::EventSender`:
  - `EventSender::new(config: &DashboardConfig, started: &TaskStartedEvent) -> Self`——在后台异步推送 `POST /api/events/start`(非阻塞,不 panic)
  - `EventSender::send(&self, event: ExecutionEvent)`——写入 channel,非阻塞
  - `EventSender::finish(self, finished: TaskFinishedEvent)`——`async`,flush channel(批量推送)+ 推送 `POST /api/events/finish`,失败重试 3 次后返回
- `devnpc::report::sender::LocalEventLogger`:
  - `LocalEventLogger::new(task_id: &str, started: &TaskStartedEvent, artifact_dir: &Path) -> Self`——创建 `{task_id}.jsonl` 并写入 `task_started` 行
  - `LocalEventLogger::log_event(&self, event: &ExecutionEvent)`——追加 `execution` 行
  - `LocalEventLogger::finish(&self, finished: &TaskFinishedEvent)`——写入 `task_finished` 行并关闭文件

**devnpc-dashboard(Phase 3–4)暴露 lib(供测试启动真实服务):**
- `devnpc_dashboard::AppConfig { db_path: std::path::PathBuf, token: String, realtime_buffer: usize }`
- `devnpc_dashboard::build_app_state(config: AppConfig) -> AppState`——初始化 Storage(含 schema 迁移 + WAL)+ RealtimeHub
- `devnpc_dashboard::router(state: AppState) -> axum::Router`——构建含全部路由与鉴权中间件的 Router
- `devnpc_dashboard::storage::Storage`:`Storage::open(path: &Path) -> Result<Storage>`、`import_from_jsonl(&self, content: &str) -> Result<ImportResult>`、`get_task(&self, task_id: &str) -> Result<Option<TaskRow>>`

> 若上述函数位于子模块(如 `devnpc_dashboard::server::router`),执行者将 `use` 路径调整为实际位置即可。

---

## 文件结构总览

本阶段完成后新增/修改文件:

```
devnpc/
├── .env.example                              # 新增/更新: 添加 DEVNPC_DASHBOARD_* 变量
├── crates/devnpc-dashboard/
│   ├── Cargo.toml                            # 修改: 添加 [dev-dependencies]
│   └── tests/
│       ├── common/mod.rs                     # 新增: TestServer 辅助 + 事件构造 helper
│       ├── e2e.rs                            # 新增: 端到端流程 + SSE 测试
│       └── import.rs                         # 新增: 导入流程测试
└── crates/devnpc/
    ├── Cargo.toml                            # 修改: 添加 devnpc-dashboard dev-dep
    └── tests/
        └── dashboard_integration.rs          # 新增: EventSender + LocalEventLogger 集成测试
```

**测试分布(对应 spec §8.4 新增 ~100 测试中的集成测试部分):**
- `devnpc-dashboard/tests/e2e.rs`:3 个(冒烟 + 全生命周期 + SSE)
- `devnpc-dashboard/tests/import.rs`:4 个(成功导入 + 幂等 409 + 覆盖 running + 格式错误 400)
- `devnpc/tests/dashboard_integration.rs`:7 个(start 推送 + 数量阈值 + 时间阈值 + finish flush + 本地文件生成 + 文件可导入 + 不可达降级)

---

### Task 1: 添加 devnpc-dashboard 测试依赖

**Files:**
- Modify: `crates/devnpc-dashboard/Cargo.toml`

- [ ] **Step 1: 读取当前 devnpc-dashboard/Cargo.toml**

读取 `crates/devnpc-dashboard/Cargo.toml` 全文,确认现有 `[dependencies]`(应含 axum、tokio、rusqlite、devnpc-core 等)与是否已有 `[dev-dependencies]` 段。若已有 `[dev-dependencies]`,在其中合并下列条目;若无,在文件末尾新增 `[dev-dependencies]` 段。

- [ ] **Step 2: 添加 dev-dependencies**

在 `crates/devnpc-dashboard/Cargo.toml` 末尾添加(或合并入已有段):

```toml
[dev-dependencies]
# HTTP mock (模拟 dashboard,devnpc 侧测试使用;dashboard 自身 E2E 不直接需要但保持统一)
wiremock = "0.6"
# 临时 SQLite / 临时目录
tempfile = "3"
# axum 服务测试工具 (oneshot 等)
tower = { version = "0.5", features = ["util"] }
# 测试用 HTTP 客户端 (发送推送请求 + 读取 SSE 流)
reqwest = { version = "0.12", default-features = false, features = ["json", "multipart", "stream", "rustls-tls"] }
# SSE 流的 StreamExt::next
futures = "0.3"
# 注: tokio (full) 与 devnpc-core 已是正式依赖,测试可直接使用,无需在此重复声明
```

- [ ] **Step 3: 验证依赖可解析**

Run: `cargo check -p devnpc-dashboard --tests`
Expected: 编译通过(可能提示 tests 目录暂无测试文件,属正常;若提示找不到 `tests/` 目录可忽略)

- [ ] **Step 4: 提交依赖变更**

Run: `git add crates/devnpc-dashboard/Cargo.toml ; git commit -m "chore(dashboard): 添加 devnpc-dashboard 测试依赖 (wiremock/tempfile/tower/reqwest/futures)"`

---

### Task 2: 添加 devnpc 测试依赖 (devnpc-dashboard dev-dep)

**Files:**
- Modify: `crates/devnpc/Cargo.toml`

devnpc 的 `tests/dashboard_integration.rs` 中"验证 .jsonl 文件可被 dashboard 导入"用例需要直接调用 `devnpc_dashboard::storage::Storage::import_from_jsonl`,因此将 devnpc-dashboard 作为 devnpc 的 dev-dependency。devnpc-dashboard 仅依赖 devnpc-core,不依赖 devnpc,故不构成循环依赖。

- [ ] **Step 1: 读取当前 devnpc/Cargo.toml 的 [dev-dependencies]**

确认现有 `[dev-dependencies]` 已含 `tokio-test`、`mockall`、`tempfile`、`wiremock`(Phase 1 已迁移)。

- [ ] **Step 2: 在 [dev-dependencies] 中追加 devnpc-dashboard**

在 `crates/devnpc/Cargo.toml` 的 `[dev-dependencies]` 段末尾追加一行:

```toml
devnpc-dashboard = { path = "../devnpc-dashboard" }
```

完整的 `[dev-dependencies]` 段应类似:

```toml
[dev-dependencies]
tokio-test = "0.4"
mockall = "0.13"
tempfile = "3"
wiremock = "0.6"
devnpc-dashboard = { path = "../devnpc-dashboard" }
```

- [ ] **Step 3: 验证 workspace 可解析**

Run: `cargo metadata --no-deps --format-version 1 > NUL`
Expected: 成功退出(无循环依赖报错)

- [ ] **Step 4: 提交依赖变更**

Run: `git add crates/devnpc/Cargo.toml ; git commit -m "chore(devnpc): 添加 devnpc-dashboard dev-dependency 用于导入集成测试"`

---

### Task 3: E2E 测试辅助 — TestServer + 冒烟测试

**Files:**
- Create: `crates/devnpc-dashboard/tests/common/mod.rs`
- Create: `crates/devnpc-dashboard/tests/e2e.rs`

- [ ] **Step 1: 创建 tests 目录与 common 模块**

Run: `mkdir crates\devnpc-dashboard\tests\common`

- [ ] **Step 2: 编写 common/mod.rs(TestServer 辅助 + 事件构造 helper)**

创建 `crates/devnpc-dashboard/tests/common/mod.rs`:

```rust
//! E2E / 导入测试公共辅助
//!
//! 在随机端口启动一个真实的 devnpc-dashboard 实例,供集成测试发送 HTTP 请求。
//! 同时提供事件构造 helper,保证各测试用例数据一致。

use devnpc_core::report::event_schema::*;
use devnpc_dashboard::{AppConfig, build_app_state, router};

/// 一个运行中的 dashboard 测试实例
pub struct TestServer {
    pub base_url: String,
    pub token: String,
    _dir: tempfile::TempDir, // 保活临时目录(内含 SQLite 文件),随 TestServer drop 清理
}

impl TestServer {
    /// 使用默认 token "test-token" 启动
    pub async fn start() -> Self {
        Self::start_with_token("test-token")
    }

    /// 使用自定义 token 启动
    pub async fn start_with_token(token: &str) -> Self {
        let dir = tempfile::tempdir().expect("创建临时目录失败");
        let db_path = dir.path().join("test-dashboard.db");
        let config = AppConfig {
            db_path,
            token: token.to_string(),
            realtime_buffer: 100,
        };
        // 初始化 Storage(含 schema 迁移 + WAL)与 RealtimeHub
        let state = build_app_state(config);
        let app = router(state);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("绑定随机端口失败");
        let addr = listener.local_addr().expect("获取端口失败");
        tokio::spawn(async move {
            // 后台运行 dashboard,测试结束随 runtime 退出
            axum::serve(listener, app).await.expect("dashboard 服务异常退出");
        });
        Self {
            base_url: format!("http://{}", addr),
            token: token.to_string(),
            _dir: dir,
        }
    }

    /// 构造一个带 10s 超时的 reqwest 客户端
    pub fn client(&self) -> reqwest::Client {
        reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(10))
            .build()
            .expect("构造 reqwest 客户端失败")
    }
}

// ============================================================
// 事件构造 helper
// ============================================================

/// 构造任务启动事件
pub fn make_started(task_id: &str) -> TaskStartedEvent {
    TaskStartedEvent {
        task_id: task_id.to_string(),
        project: "test-group/test-project".into(),
        mr_iid: Some(42),
        pipeline_id: Some(100),
        task_description: "E2E 测试任务".into(),
        task_kind: "manual".into(),
        started_at: "2026-08-03T10:00:00Z".into(),
        model: "deepseek-chat".into(),
    }
}

/// 构造任务结束事件
pub fn make_finished(task_id: &str, status: TaskStatus) -> TaskFinishedEvent {
    TaskFinishedEvent {
        task_id: task_id.to_string(),
        status,
        duration_secs: 90,
        total_tokens: 600,
        estimated_cost_usd: 0.03,
        mr_url: Some("https://gitlab.com/test-group/test-project/-/merge_requests/42".into()),
        ci_url: Some("https://gitlab.com/test-group/test-project/-/pipelines/100".into()),
        summary: "E2E 测试完成".into(),
        error: None,
        finished_at: "2026-08-03T10:01:30Z".into(),
    }
}

/// 构造 LLM 调用执行事件
pub fn make_llm_call(iteration: u32) -> ExecutionEvent {
    ExecutionEvent::LlmCall {
        iteration,
        prompt_tokens: 100 * iteration as u64,
        completion_tokens: 50 * iteration as u64,
        latency_ms: 500 * iteration as u64,
    }
}

/// 构造工具调用执行事件
pub fn make_tool_call(name: &str) -> ExecutionEvent {
    ExecutionEvent::ToolCall {
        name: name.to_string(),
        success: true,
        latency_ms: 10,
        detail: format!("{name} 调用详情"),
    }
}

/// 构造批量推送请求
pub fn make_batch(task_id: &str, events: Vec<ExecutionEvent>) -> BatchEventsRequest {
    BatchEventsRequest {
        task_id: task_id.to_string(),
        events,
    }
}

/// 构造一份完整的 .jsonl 文件内容(task_started + 2 execution + task_finished)
pub fn build_full_jsonl(task_id: &str) -> String {
    let started = EventLogEntry::TaskStarted {
        task_id: task_id.into(),
        data: make_started(task_id),
    };
    let exec1 = EventLogEntry::Execution {
        task_id: task_id.into(),
        event: make_llm_call(1),
    };
    let exec2 = EventLogEntry::Execution {
        task_id: task_id.into(),
        event: make_tool_call("read_file"),
    };
    let finished = EventLogEntry::TaskFinished {
        task_id: task_id.into(),
        data: make_finished(task_id, TaskStatus::Success),
    };
    [started, exec1, exec2, finished]
        .iter()
        .map(|e| serde_json::to_string(e).unwrap())
        .collect::<Vec<_>>()
        .join("\n")
}

/// 构造一份仅 running 的 .jsonl 文件内容(task_started + 2 execution,无 task_finished)
pub fn build_running_jsonl(task_id: &str) -> String {
    let started = EventLogEntry::TaskStarted {
        task_id: task_id.into(),
        data: make_started(task_id),
    };
    let exec1 = EventLogEntry::Execution {
        task_id: task_id.into(),
        event: make_llm_call(1),
    };
    let exec2 = EventLogEntry::Execution {
        task_id: task_id.into(),
        event: make_tool_call("read_file"),
    };
    [started, exec1, exec2]
        .iter()
        .map(|e| serde_json::to_string(e).unwrap())
        .collect::<Vec<_>>()
        .join("\n")
}
```

- [ ] **Step 3: 编写 e2e.rs 冒烟测试**

创建 `crates/devnpc-dashboard/tests/e2e.rs`:

```rust
//! 端到端集成测试: 启动真实 dashboard,走完整 HTTP 流程

mod common;

use common::TestServer;

/// 冒烟测试: dashboard 能在随机端口启动并响应 GET /
#[tokio::test]
async fn dashboard_starts_and_serves_index() {
    let server = TestServer::start().await;
    let resp = server.client().get(&server.base_url).send().await.unwrap();
    assert!(
        resp.status().is_success(),
        "GET / 应返回 2xx, 实际: {}",
        resp.status()
    );
}
```

- [ ] **Step 4: 运行冒烟测试验证通过**

Run: `cargo test -p devnpc-dashboard --test e2e -- dashboard_starts_and_serves_index --nocapture`
Expected: 1 个测试 PASS。若失败并提示找不到 `devnpc_dashboard::{AppConfig, build_app_state, router}`,说明 Phase 3–4 的公共 API 路径不同——调整 `common/mod.rs` 顶部的 `use` 语句为实际路径(如 `use devnpc_dashboard::server::{build_app_state, router}` 与 `use devnpc_dashboard::AppConfig`)后重跑。

- [ ] **Step 5: 提交测试辅助与冒烟测试**

Run: `git add crates/devnpc-dashboard/tests/common/mod.rs crates/devnpc-dashboard/tests/e2e.rs ; git commit -m "test(dashboard): 新增 TestServer 辅助与 dashboard 启动冒烟测试"`

---

### Task 4: E2E 端到端全流程测试

**Files:**
- Modify: `crates/devnpc-dashboard/tests/e2e.rs`(追加测试)

- [ ] **Step 1: 在 e2e.rs 末尾追加全生命周期测试**

在 `crates/devnpc-dashboard/tests/e2e.rs` 末尾追加:

```rust
use common::{make_batch, make_finished, make_llm_call, make_started, make_tool_call};
use devnpc_core::report::event_schema::TaskStatus;

/// 端到端全流程: start → batch(×3) → finish → 校验任务详情/任务列表/趋势聚合
#[tokio::test]
async fn full_task_lifecycle_returns_complete_data() {
    let server = TestServer::start().await;
    let client = server.client();
    let token = server.token.as_str();
    let task_id = format!("e2e-full-{}", uuid::Uuid::new_v4());

    // 1. POST /api/events/start (带鉴权 header)
    let started = make_started(&task_id);
    let resp = client
        .post(format!("{}/api/events/start", server.base_url))
        .header("X-Devnpc-Token", token)
        .json(&started)
        .send()
        .await
        .unwrap();
    assert!(
        resp.status().is_success(),
        "start 应返回 2xx, 实际: {}",
        resp.status()
    );

    // 2. POST /api/events/batch 多次 (每次 2 条事件)
    for i in 1..=3u32 {
        let batch = make_batch(
            &task_id,
            vec![make_llm_call(i), make_tool_call("read_file")],
        );
        let resp = client
            .post(format!("{}/api/events/batch", server.base_url))
            .header("X-Devnpc-Token", token)
            .json(&batch)
            .send()
            .await
            .unwrap();
        assert!(
            resp.status().is_success(),
            "batch #{} 应返回 2xx, 实际: {}",
            i,
            resp.status()
        );
    }

    // 3. POST /api/events/finish
    let finished = make_finished(&task_id, TaskStatus::Success);
    let resp = client
        .post(format!("{}/api/events/finish", server.base_url))
        .header("X-Devnpc-Token", token)
        .json(&finished)
        .send()
        .await
        .unwrap();
    assert!(
        resp.status().is_success(),
        "finish 应返回 2xx, 实际: {}",
        resp.status()
    );

    // 4. 校验 GET /api/tasks/:id 返回完整数据
    let resp = client
        .get(format!("{}/api/tasks/{}", server.base_url, task_id))
        .send()
        .await
        .unwrap();
    assert!(
        resp.status().is_success(),
        "GET /api/tasks/:id 应返回 2xx, 实际: {}",
        resp.status()
    );
    let task: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(task["task_id"], task_id, "task_id 应匹配");
    assert_eq!(task["status"], "success", "状态应为 success");
    assert_eq!(task["project"], "test-group/test-project");
    assert_eq!(task["duration_secs"], 90, "duration_secs 应为 90");
    assert_eq!(task["total_tokens"], 600, "total_tokens 应为 600");
    assert_eq!(task["mr_iid"], 42, "mr_iid 应为 42");

    // 5. 校验 GET / (任务列表 SSR) 包含该任务
    let resp = client.get(&server.base_url).send().await.unwrap();
    assert!(resp.status().is_success(), "GET / 应返回 2xx");
    let html = resp.text().await.unwrap();
    assert!(
        html.contains(&task_id),
        "任务列表页 HTML 应包含 task_id"
    );

    // 6. 校验 GET /api/stats/trends?days=7 返回聚合数据 (非空 JSON)
    let resp = client
        .get(format!("{}/api/stats/trends?days=7", server.base_url))
        .send()
        .await
        .unwrap();
    assert!(
        resp.status().is_success(),
        "GET /api/stats/trends 应返回 2xx, 实际: {}",
        resp.status()
    );
    let body = resp.text().await.unwrap();
    assert!(
        body.len() > 2,
        "trends 响应不应为空, 实际: {body}"
    );
    // 应可解析为合法 JSON
    let trends: serde_json::Value = serde_json::from_str(&body).expect("trends 应为合法 JSON");
    assert!(
        trends.is_array() || trends.is_object(),
        "trends 应为 JSON 数组或对象"
    );
}
```

- [ ] **Step 2: 运行全流程测试**

Run: `cargo test -p devnpc-dashboard --test e2e -- full_task_lifecycle_returns_complete_data --nocapture`
Expected: PASS。若 `task["status"]` 断言失败,检查 Phase 3 的 `TaskRow.status` 字段在 finish 后是否落库为小写 `"success"`(spec §3.5 约定 status 文本为 `running/success/failed/ci_failed/timeout`);若 `task["mr_iid"]` 类型不匹配(如返回 `42` vs `"42"`),将断言改为 `task["mr_iid"].as_u64() == Some(42)`。

- [ ] **Step 3: 提交全流程测试**

Run: `git add crates/devnpc-dashboard/tests/e2e.rs ; git commit -m "test(dashboard): 新增端到端全生命周期测试 (start/batch/finish + 详情/列表/趋势校验)"`

---

### Task 5: SSE 实时推送测试

**Files:**
- Modify: `crates/devnpc-dashboard/tests/e2e.rs`(追加测试)

- [ ] **Step 1: 在 e2e.rs 末尾追加 SSE 测试**

在 `crates/devnpc-dashboard/tests/e2e.rs` 末尾追加:

```rust
use futures::StreamExt;

/// SSE 实时推送: 订阅 → 推送事件 → 校验 stream 收到事件
#[tokio::test]
async fn sse_stream_receives_pushed_events() {
    let server = TestServer::start().await;
    let client = server.client();
    let token = server.token.as_str();
    let task_id = format!("e2e-sse-{}", uuid::Uuid::new_v4());

    // 1. 先启动任务 (使后续 batch 事件进入 RealtimeHub)
    let started = make_started(&task_id);
    let resp = client
        .post(format!("{}/api/events/start", server.base_url))
        .header("X-Devnpc-Token", token)
        .json(&started)
        .send()
        .await
        .unwrap();
    assert!(resp.status().is_success(), "start 应成功");

    // 2. 订阅 SSE 流 (连接建立即代表服务端已注册订阅者)
    let sse_resp = client
        .get(format!("{}/api/realtime/stream", server.base_url))
        .send()
        .await
        .unwrap();
    assert!(sse_resp.status().is_success(), "SSE 端点应返回 2xx");
    let content_type = sse_resp
        .headers()
        .get("content-type")
        .expect("缺少 content-type")
        .to_str()
        .unwrap()
        .to_string();
    assert!(
        content_type.contains("text/event-stream"),
        "content-type 应为 text/event-stream, 实际: {content_type}"
    );

    // 等待一小段时间确保服务端订阅注册完成后再推送
    tokio::time::sleep(std::time::Duration::from_millis(150)).await;

    // 3. 推送一批执行事件 (应被 RealtimeHub 广播到 SSE 订阅者)
    let batch = make_batch(
        &task_id,
        vec![make_llm_call(1), make_tool_call("read_file")],
    );
    let resp = client
        .post(format!("{}/api/events/batch", server.base_url))
        .header("X-Devnpc-Token", token)
        .json(&batch)
        .send()
        .await
        .unwrap();
    assert!(resp.status().is_success(), "batch 应成功");

    // 4. 从 SSE 流读取,校验收到 data: 行 (3s 内)
    let mut stream = sse_resp.bytes_stream();
    let mut received = String::new();
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(3);
    loop {
        match tokio::time::timeout_at(deadline, stream.next()).await {
            Ok(Some(chunk)) => {
                received.push_str(&String::from_utf8_lossy(&chunk));
                if received.contains("data:") {
                    break;
                }
            }
            _ => break,
        }
    }
    assert!(
        received.contains("data:"),
        "SSE 流应在 3s 内收到 data: 行, 实际收到: {received}"
    );
}
```

- [ ] **Step 2: 运行 SSE 测试**

Run: `cargo test -p devnpc-dashboard --test e2e -- sse_stream_receives_pushed_events --nocapture`
Expected: PASS。若失败提示 `bytes_stream` 未找到,确认 `crates/devnpc-dashboard/Cargo.toml` 的 reqwest dev-dep 含 `"stream"` feature(见 Task 1)。若 SSE 流未收到事件,确认 Phase 4 的 `/api/events/batch` handler 在写入 Storage 后调用了 `RealtimeHub::push_events` 广播。

- [ ] **Step 3: 提交 SSE 测试**

Run: `git add crates/devnpc-dashboard/tests/e2e.rs ; git commit -m "test(dashboard): 新增 SSE 实时推送测试 (订阅→推送→校验 stream 收到 data 行)"`

---

### Task 6: 导入流程测试

**Files:**
- Create: `crates/devnpc-dashboard/tests/import.rs`

- [ ] **Step 1: 编写 import.rs(4 个用例)**

创建 `crates/devnpc-dashboard/tests/import.rs`:

```rust
//! 导入流程测试: 构造本地 .jsonl → POST /api/events/import → 校验结果

mod common;

use common::{build_full_jsonl, build_running_jsonl, TestServer};
use devnpc_core::report::event_schema::ImportResult;

/// 构造 multipart 上传并调用导入接口,返回响应
async fn upload(
    server: &TestServer,
    content: &str,
    filename: &str,
) -> reqwest::Response {
    let part = reqwest::multipart::Part::text(content.to_string())
        .file_name(filename.to_string())
        .mime_str("application/x-ndjson")
        .unwrap();
    let form = reqwest::multipart::Form::new().part("file", part);
    server
        .client()
        .post(format!("{}/api/events/import", server.base_url))
        .header("X-Devnpc-Token", &server.token)
        .multipart(form)
        .send()
        .await
        .unwrap()
}

/// 成功导入完整 .jsonl: 返回 200,任务可查询
#[tokio::test]
async fn import_jsonl_creates_task() {
    let server = TestServer::start().await;
    let client = server.client();
    let task_id = format!("imp-{}", uuid::Uuid::new_v4());
    let jsonl = build_full_jsonl(&task_id);

    let resp = upload(&server, &jsonl, &format!("{task_id}.jsonl")).await;
    assert!(
        resp.status().is_success(),
        "导入应返回 2xx, 实际: {}",
        resp.status()
    );
    let result: ImportResult = resp.json().await.unwrap();
    assert_eq!(result.task_id, task_id, "返回的 task_id 应匹配");
    assert_eq!(result.events_count, 2, "应导入 2 条 execution 事件");
    assert!(!result.skipped, "首次导入不应跳过");

    // 校验 GET /api/tasks/:id 返回导入数据
    let resp = client
        .get(format!("{}/api/tasks/{}", server.base_url, task_id))
        .send()
        .await
        .unwrap();
    assert!(resp.status().is_success(), "导入后任务应可查询");
    let task: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(task["task_id"], task_id);
    assert_eq!(task["status"], "success", "导入的 finished 任务状态应为 success");
}

/// 重复导入已 finish 的文件: 第二次返回 409 Conflict
#[tokio::test]
async fn import_duplicate_finished_returns_409() {
    let server = TestServer::start().await;
    let task_id = format!("dup-{}", uuid::Uuid::new_v4());
    let jsonl = build_full_jsonl(&task_id);

    // 第一次导入成功
    let resp = upload(&server, &jsonl, &format!("{task_id}.jsonl")).await;
    assert!(resp.status().is_success(), "首次导入应成功");

    // 第二次导入同一文件 (task 已 finish) → 409
    let resp = upload(&server, &jsonl, &format!("{task_id}.jsonl")).await;
    assert_eq!(
        resp.status(),
        409,
        "重复导入已 finish 任务应返回 409, 实际: {}",
        resp.status()
    );
}

/// 覆盖导入 running 任务: 再次上传返回 200 (先删后写)
#[tokio::test]
async fn import_overwrites_running_task() {
    let server = TestServer::start().await;
    let client = server.client();
    let task_id = format!("owr-{}", uuid::Uuid::new_v4());
    let running = build_running_jsonl(&task_id); // 仅 task_started + execution,无 finish

    // 第一次导入 running 任务
    let resp = upload(&server, &running, &format!("{task_id}.jsonl")).await;
    assert!(resp.status().is_success(), "首次导入 running 应成功");
    // 校验任务处于 running
    let task: serde_json::Value = client
        .get(format!("{}/api/tasks/{}", server.base_url, task_id))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(task["status"], "running", "未 finish 的导入任务应为 running");

    // 第二次导入同一 running 文件 → 覆盖,返回 200
    let resp = upload(&server, &running, &format!("{task_id}.jsonl")).await;
    assert_eq!(
        resp.status(),
        200,
        "覆盖 running 任务应返回 200, 实际: {}",
        resp.status()
    );
}

/// 上传格式错误文件: 返回 400 Bad Request
#[tokio::test]
async fn import_malformed_returns_400() {
    let server = TestServer::start().await;
    let bad = "this is not json\n{also not valid json";
    let resp = upload(&server, bad, "bad.jsonl").await;
    assert_eq!(
        resp.status(),
        400,
        "格式错误文件应返回 400, 实际: {}",
        resp.status()
    );
}
```

> **说明:** 上述用例假设导入接口的 multipart 字段名为 `file`。若 Phase 4 实现使用了其他字段名(如 `events`),将 `Form::new().part("file", ...)` 中的 `"file"` 改为实际字段名。

- [ ] **Step 2: 运行导入测试**

Run: `cargo test -p devnpc-dashboard --test import --nocapture`
Expected: 4 个测试全部 PASS。若 `import_overwrites_running_task` 中 `task["status"]` 不是 `"running"`,确认 Phase 3 导入逻辑对无 `task_finished` 行的文件将任务状态置为 `running`。

- [ ] **Step 3: 提交导入测试**

Run: `git add crates/devnpc-dashboard/tests/import.rs ; git commit -m "test(dashboard): 新增导入流程测试 (成功导入/幂等409/覆盖running/格式错误400)"`

---

### Task 7: devnpc 侧集成测试 — EventSender 推送 start/batch/finish

**Files:**
- Create: `crates/devnpc/tests/dashboard_integration.rs`

- [ ] **Step 1: 创建测试文件骨架与 helper**

创建 `crates/devnpc/tests/dashboard_integration.rs`:

```rust
//! devnpc 侧 EventSender + LocalEventLogger 集成测试
//!
//! 用 wiremock 模拟 dashboard,验证推送策略与降级行为;
//! 用 tempfile 验证本地 .jsonl 文件生成,并调用 dashboard Storage 验证可导入。

use std::time::Duration;

use devnpc::config::DashboardConfig;
use devnpc::report::sender::{EventSender, LocalEventLogger};
use devnpc_core::report::event_schema::{
    ExecutionEvent, TaskFinishedEvent, TaskStartedEvent, TaskStatus,
};

// ============================================================
// helper
// ============================================================

/// 构造默认 DashboardConfig (enabled=true, 指向给定 url)
fn config_with(url: String) -> DashboardConfig {
    DashboardConfig {
        enabled: true,
        url,
        token: "test-token".into(),
        batch_size: 20,
        batch_interval_secs: 3,
        local_event_log: true,
    }
}

fn make_started(task_id: &str) -> TaskStartedEvent {
    TaskStartedEvent {
        task_id: task_id.to_string(),
        project: "test-group/test-project".into(),
        mr_iid: Some(42),
        pipeline_id: Some(100),
        task_description: "集成测试任务".into(),
        task_kind: "manual".into(),
        started_at: "2026-08-03T10:00:00Z".into(),
        model: "deepseek-chat".into(),
    }
}

fn make_llm_call(iteration: u32) -> ExecutionEvent {
    ExecutionEvent::LlmCall {
        iteration,
        prompt_tokens: 100 * iteration as u64,
        completion_tokens: 50 * iteration as u64,
        latency_ms: 500 * iteration as u64,
    }
}

fn make_tool_call(name: &str) -> ExecutionEvent {
    ExecutionEvent::ToolCall {
        name: name.to_string(),
        success: true,
        latency_ms: 10,
        detail: format!("{name} 详情"),
    }
}

fn make_finished(task_id: &str, status: TaskStatus) -> TaskFinishedEvent {
    TaskFinishedEvent {
        task_id: task_id.to_string(),
        status,
        duration_secs: 90,
        total_tokens: 600,
        estimated_cost_usd: 0.03,
        mr_url: None,
        ci_url: None,
        summary: "集成测试完成".into(),
        error: None,
        finished_at: "2026-08-03T10:01:30Z".into(),
    }
}

/// 在 wiremock 上挂一个对指定路径返回 200 的 POST mock
async fn mount_ok(mock: &wiremock::MockServer, path: &str) {
    wiremock::Mock::given(wiremock::matchers::method("POST"))
        .and(wiremock::matchers::path(path))
        .respond_with(wiremock::ResponseTemplate::new(200))
        .mount(mock)
        .await;
}

/// 轮询校验 mock 收到至少 expected 个匹配 path 前缀的请求 (5s 超时)
async fn wait_for_request(mock: &wiremock::MockServer, path: &str, expected: usize) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        let requests = mock.received_requests().await.unwrap_or_default();
        let count = requests
            .iter()
            .filter(|r| r.url.starts_with(path))
            .count();
        if count >= expected {
            return;
        }
        if tokio::time::Instant::now() >= deadline {
            panic!("期望 {expected} 个 {path} 请求, 实际 {count}");
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}
```

- [ ] **Step 2: 追加 start 推送测试**

在 `crates/devnpc/tests/dashboard_integration.rs` 末尾追加:

```rust
/// EventSender::new 应在后台推送 POST /api/events/start
#[tokio::test]
async fn event_sender_posts_task_started() {
    let mock = wiremock::MockServer::start().await;
    mount_ok(&mock, "/api/events/start").await;

    let started = make_started("t-start-1");
    let _sender = EventSender::new(&config_with(mock.uri()), &started);

    // 等待后台推送完成
    wait_for_request(&mock, "/api/events/start", 1).await;
}
```

- [ ] **Step 3: 追加批量推送数量阈值测试**

在文件末尾追加:

```rust
/// 累积到 batch_size 阈值时触发批量推送 (不受时间阈值影响)
#[tokio::test]
async fn batch_push_triggers_on_count_threshold() {
    let mock = wiremock::MockServer::start().await;
    mount_ok(&mock, "/api/events/start").await;
    mount_ok(&mock, "/api/events/batch").await;

    // batch_size=3, batch_interval_secs=60 (大间隔,只靠数量触发)
    let config = DashboardConfig {
        batch_size: 3,
        batch_interval_secs: 60,
        ..config_with(mock.uri())
    };
    let started = make_started("t-cnt-1");
    let sender = EventSender::new(&config, &started);

    // 推 3 条,达到数量阈值 → 触发一次 batch
    sender.send(make_llm_call(1));
    sender.send(make_tool_call("read_file"));
    sender.send(make_llm_call(2));

    wait_for_request(&mock, "/api/events/batch", 1).await;
}
```

- [ ] **Step 4: 追加批量推送时间阈值测试**

在文件末尾追加:

```rust
/// 距上次推送超过 batch_interval_secs 时触发批量推送 (未达数量阈值)
#[tokio::test]
async fn batch_push_triggers_on_time_threshold() {
    let mock = wiremock::MockServer::start().await;
    mount_ok(&mock, "/api/events/start").await;
    mount_ok(&mock, "/api/events/batch").await;

    // batch_size=100 (大数量,不靠数量触发), batch_interval_secs=1
    let config = DashboardConfig {
        batch_size: 100,
        batch_interval_secs: 1,
        ..config_with(mock.uri())
    };
    let started = make_started("t-time-1");
    let sender = EventSender::new(&config, &started);

    // 只推 1 条 (未达数量阈值),等待时间阈值触发 (~1s)
    sender.send(make_llm_call(1));

    wait_for_request(&mock, "/api/events/batch", 1).await;
}
```

- [ ] **Step 5: 追加 finish flush 测试**

在文件末尾追加:

```rust
/// finish 应 flush 残留事件 (batch) 并推送 POST /api/events/finish
#[tokio::test]
async fn finish_posts_task_finished_and_flushes() {
    let mock = wiremock::MockServer::start().await;
    mount_ok(&mock, "/api/events/start").await;
    mount_ok(&mock, "/api/events/batch").await;
    mount_ok(&mock, "/api/events/finish").await;

    let started = make_started("t-fin-1");
    let sender = EventSender::new(&config_with(mock.uri()), &started);

    // 推 1 条 (未达 batch_size=20,留在 channel)
    sender.send(make_llm_call(1));

    // finish: 先 flush channel (1 条 → batch),再 POST finish
    let finished = make_finished("t-fin-1", TaskStatus::Success);
    sender.finish(finished).await;

    wait_for_request(&mock, "/api/events/batch", 1).await;
    wait_for_request(&mock, "/api/events/finish", 1).await;
}
```

- [ ] **Step 6: 运行 EventSender 测试**

Run: `cargo test -p devnpc --test dashboard_integration --nocapture`
Expected: 4 个测试全部 PASS。若提示找不到 `devnpc::report::sender::{EventSender, LocalEventLogger}` 或 `devnpc::config::DashboardConfig`,确认 Phase 2 的模块导出路径并调整 `use` 语句。若 `EventSender::new` 签名为 `(config, task_id)` 而非 `(config, &TaskStartedEvent)`,将调用改为传 `task_id` 并改用 Phase 2 实际的 start 推送方式。

- [ ] **Step 7: 提交 EventSender 集成测试**

Run: `git add crates/devnpc/tests/dashboard_integration.rs ; git commit -m "test(devnpc): 新增 EventSender 集成测试 (start 推送/数量阈值/时间阈值/finish flush)"`

---

### Task 8: devnpc 侧集成测试 — LocalEventLogger 文件生成与可导入

**Files:**
- Modify: `crates/devnpc/tests/dashboard_integration.rs`(追加测试)

- [ ] **Step 1: 追加 LocalEventLogger 文件生成测试**

在 `crates/devnpc/tests/dashboard_integration.rs` 末尾追加:

```rust
/// LocalEventLogger 应在 artifact_dir 下生成 {task_id}.jsonl,
/// 按顺序写入 task_started / execution / task_finished 行
#[test]
fn local_logger_writes_jsonl_file() {
    let dir = tempfile::tempdir().unwrap();
    let task_id = "log-1";

    let started = make_started(task_id);
    let logger = LocalEventLogger::new(task_id, &started, dir.path());

    logger.log_event(&make_llm_call(1));
    logger.log_event(&make_tool_call("read_file"));

    let finished = make_finished(task_id, TaskStatus::Success);
    logger.finish(&finished);

    // 校验文件存在且为 4 行
    let file_path = dir.path().join(format!("{task_id}.jsonl"));
    let content = std::fs::read_to_string(&file_path).expect("jsonl 文件应存在");
    let lines: Vec<&str> = content.lines().collect();
    assert_eq!(lines.len(), 4, "应有 4 行 (started + 2 exec + finished)");

    // 第一行 kind=task_started
    let first: serde_json::Value = serde_json::from_str(lines[0]).unwrap();
    assert_eq!(first["kind"], "task_started", "首行应为 task_started");
    assert_eq!(first["task_id"], task_id);

    // 中间两行 kind=execution
    let mid: serde_json::Value = serde_json::from_str(lines[1]).unwrap();
    assert_eq!(mid["kind"], "execution");
    assert!(mid["event"]["type"].is_string(), "execution 行应含 event.type");

    // 末行 kind=task_finished
    let last: serde_json::Value = serde_json::from_str(lines[3]).unwrap();
    assert_eq!(last["kind"], "task_finished");
    assert_eq!(last["status"], "success");
}
```

- [ ] **Step 2: 追加"文件可被 dashboard 导入"测试**

在文件末尾追加:

```rust
/// LocalEventLogger 生成的 .jsonl 文件应可被 dashboard Storage 导入
#[test]
fn local_logger_file_importable_by_dashboard() {
    use devnpc_dashboard::storage::Storage;

    // 1. 用 LocalEventLogger 生成 .jsonl
    let dir = tempfile::tempdir().unwrap();
    let task_id = format!("imp-{}", uuid::Uuid::new_v4());
    let started = make_started(&task_id);
    let logger = LocalEventLogger::new(&task_id, &started, dir.path());
    logger.log_event(&make_llm_call(1));
    logger.log_event(&make_tool_call("read_file"));
    logger.finish(&make_finished(&task_id, TaskStatus::Success));

    let file_path = dir.path().join(format!("{task_id}.jsonl"));
    let content = std::fs::read_to_string(&file_path).unwrap();
    assert!(!content.is_empty(), "jsonl 文件不应为空");

    // 2. 用 dashboard 的 Storage 直接导入,验证文件可被解析写入
    let db_dir = tempfile::tempdir().unwrap();
    let db_path = db_dir.path().join("import.db");
    let storage = Storage::open(&db_path).expect("打开 dashboard Storage 失败");
    let result = storage
        .import_from_jsonl(&content)
        .expect("dashboard 导入 .jsonl 失败");
    assert_eq!(result.task_id, task_id, "导入返回的 task_id 应匹配");
    assert_eq!(result.events_count, 2, "应导入 2 条 execution 事件");
    assert!(!result.skipped, "首次导入不应跳过");
}
```

> **说明:** 本用例通过 `devnpc-dashboard` 的 `Storage::import_from_jsonl` 直接验证 devnpc 生成的文件可被 dashboard 导入逻辑接受(HTTP multipart 路径已在 `devnpc-dashboard/tests/import.rs` 覆盖)。若 Phase 3 的 Storage 构造函数命名为 `Storage::new` 或位于其他路径,调整 `use` 与调用即可。

- [ ] **Step 3: 运行 LocalEventLogger 测试**

Run: `cargo test -p devnpc --test dashboard_integration -- local_logger --nocapture`
Expected: 2 个测试 PASS。若 `Storage::open` 未找到,确认 Phase 3 的 Storage 构造函数公开导出(`pub fn open` 或 `pub fn new`)并调整调用。

- [ ] **Step 4: 提交 LocalEventLogger 测试**

Run: `git add crates/devnpc/tests/dashboard_integration.rs ; git commit -m "test(devnpc): 新增 LocalEventLogger 测试 (jsonl 文件生成 + 可被 dashboard 导入)"`

---

### Task 9: devnpc 侧集成测试 — dashboard 不可达降级

**Files:**
- Modify: `crates/devnpc/tests/dashboard_integration.rs`(追加测试)

- [ ] **Step 1: 追加降级测试**

在 `crates/devnpc/tests/dashboard_integration.rs` 末尾追加:

```rust
/// dashboard 持续返回 500 (不可达/故障) 时,EventSender 不应 panic,
/// finish 应在重试后返回,不永久阻塞主流程
#[tokio::test]
async fn event_sender_degrades_when_dashboard_unreachable() {
    // wiremock 对所有 POST 返回 500,模拟 dashboard 持续故障
    let mock = wiremock::MockServer::start().await;
    wiremock::Mock::given(wiremock::matchers::method("POST"))
        .respond_with(wiremock::ResponseTemplate::new(500))
        .mount(&mock)
        .await;

    let config = DashboardConfig {
        enabled: true,
        url: mock.uri(),
        token: "test-token".into(),
        batch_size: 20,
        batch_interval_secs: 3,
        local_event_log: false,
    };
    let started = make_started("deg-1");

    // 创建 sender 不应 panic (start POST 在后台失败重试,不阻塞)
    let sender = EventSender::new(&config, &started);

    // 不推送任何事件 → finish 时无需 flush batch,仅 POST finish (重试 3 次后返回)
    let finished = make_finished("deg-1", TaskStatus::Failed);
    let result = tokio::time::timeout(
        Duration::from_secs(30),
        sender.finish(finished),
    )
    .await;
    assert!(
        result.is_ok(),
        "finish 应在重试后返回,不应永久阻塞"
    );
}
```

> **说明:** 本用例不调用 `sender.send()`,避免 finish 时触发 batch 的 5 次指数退避(1+2+4+8+16=31s)导致超时。`local_event_log=false` 关闭本地文件写入,聚焦降级路径。start POST 的后台重试在测试结束时随 runtime 退出被取消,不影响断言。

- [ ] **Step 2: 运行降级测试**

Run: `cargo test -p devnpc --test dashboard_integration -- event_sender_degrades_when_dashboard_unreachable --nocapture`
Expected: PASS(通常 < 10s)。若超时失败,说明 Phase 2 的 `finish` 重试未在 30s 内放弃——检查重试次数与退避策略是否符合 spec §4.1(finish 重试 3 次)。

- [ ] **Step 3: 运行 devnpc 全部集成测试确认无回归**

Run: `cargo test -p devnpc --test dashboard_integration --nocapture`
Expected: 7 个测试全部 PASS(start/数量阈值/时间阈值/finish flush/文件生成/可导入/降级)。

- [ ] **Step 4: 提交降级测试**

Run: `git add crates/devnpc/tests/dashboard_integration.rs ; git commit -m "test(devnpc): 新增 dashboard 不可达降级测试 (不 panic/不永久阻塞)"`

---

### Task 10: 文档更新 — .env.example

**Files:**
- Create or Modify: `.env.example`(项目根目录)

> 依据用户约束:本阶段文档工作**仅更新 `.env.example`**,不创建 README.md 或其他说明文档。

- [ ] **Step 1: 读取(或确认)根目录 .env.example**

读取 `d:\workspace\trae_work\devnpc\.env.example`。若文件不存在,本步骤创建之;若已存在(含其他 DEVNPC_* 变量),在文件末尾追加分隔注释与下方的 dashboard 段。

- [ ] **Step 2: 写入/追加 dashboard 配置段**

在 `.env.example` 末尾追加(若新建文件则只含此段):

```env
# ============================================================
# devnpc Dashboard 配置 (可选)
# ============================================================
# 配置 DEVNPC_DASHBOARD_URL 后,devnpc 任务执行时将事件实时推送到 dashboard;
# 未配置 URL 时推送关闭,devnpc 行为与现状完全一致 (向后兼容)。

# Dashboard 服务地址 (配置后启用实时推送)
# DEVNPC_DASHBOARD_URL=http://localhost:8080

# 推送鉴权 token (需与 dashboard 启动时的 --token / DEVNPC_DASHBOARD_TOKEN 一致)
# DEVNPC_DASHBOARD_TOKEN=your-secret-token

# 批量推送事件数阈值 (累积到该数量触发一次 POST /api/events/batch, 默认 20)
# DEVNPC_DASHBOARD_BATCH_SIZE=20

# 批量推送时间阈值 (距上次推送超过该秒数触发一次, 默认 3)
# DEVNPC_DASHBOARD_BATCH_INTERVAL_SECS=3

# 是否保存本地 .jsonl 事件文件 (兜底机制, 默认 true;即使 dashboard 未启用也保存)
# 设为 false 可关闭本地事件文件
# DEVNPC_DASHBOARD_LOCAL_LOG=true
```

- [ ] **Step 3: 校验 .env.example 可被现有 load_env_file 解析**

确认每行均为 `KEY=VALUE` 或 `# 注释` 格式,无空行中的非法字符。现有 `src/config/env.rs::load_env_file_from` 支持 `#` 注释与引号,上述格式兼容。

- [ ] **Step 4: 提交文档变更**

Run: `git add .env.example ; git commit -m "docs: 更新 .env.example 添加 DEVNPC_DASHBOARD_* 配置变量"`

---

### Task 11: 全量测试验证与 clippy

**Files:**
- 无修改(仅运行验证)

- [ ] **Step 1: 运行 devnpc-core 全部测试**

Run: `cargo test -p devnpc-core`
Expected: ~20 个测试全部 PASS(Phase 1 已交付)

- [ ] **Step 2: 运行 devnpc 全部测试**

Run: `cargo test -p devnpc`
Expected: ~380 个测试全部 PASS。其中本阶段新增的 `dashboard_integration` 7 个集成用例(属 spec §8.4 devnpc "新增 ~40" 的一部分)与现有 `integration_e2e` 12 个用例均应 PASS。

- [ ] **Step 3: 运行 devnpc-dashboard 全部测试**

Run: `cargo test -p devnpc-dashboard`
Expected: ~50 个测试全部 PASS。其中本阶段新增 e2e 3 个 + import 4 个(共 7 个集成用例,属 spec §8.4 devnpc-dashboard "新增 ~50" 的一部分)均应 PASS。

- [ ] **Step 4: 运行 workspace 全量测试统计总数**

Run: `cargo test --workspace --no-fail-fast 2>&1 | Select-String "test result"`
Expected: 各 crate 测试结果行汇总后约 ~450 个测试通过(对应 spec §8.4 目标)。若个别用例因 Phase 2–4 API 命名差异失败,按各 Task 中"调整 use 路径/方法名"的说明修正后重跑。

- [ ] **Step 5: 运行 clippy 确保无警告**

Run: `cargo clippy --workspace --all-targets -- -D warnings`
Expected: 无 warning。若测试代码有 unused import 等 warning,清理后重跑。

- [ ] **Step 6: 提交验证记录(可选,如有 clippy 修复)**

若 Step 5 产生代码修复:

Run: `git add -A ; git commit -m "test: 修复 clippy 警告,Phase 5 集成测试全量通过"`

---

## Self-Review 检查清单

- [ ] **spec §8.2 端到端流程测试**:`e2e.rs::full_task_lifecycle_returns_complete_data` 覆盖 start→batch(×3)→finish→校验 GET /api/tasks/:id / GET / / GET /api/stats/trends
- [ ] **spec §8.2 SSE 实时推送测试**:`e2e.rs::sse_stream_receives_pushed_events` 覆盖 订阅→推送→校验 stream 收到 data 行 + content-type 校验
- [ ] **spec §8.2 导入流程测试**:`import.rs` 4 个用例覆盖 成功导入+校验 / 幂等 409 / 覆盖 running 200 / 格式错误 400
- [ ] **spec §8.2 devnpc 侧集成**:`dashboard_integration.rs` 7 个用例覆盖 TaskStartedEvent 推送 / 批量数量阈值 / 批量时间阈值 / TaskFinishedEvent 推送 / .jsonl 生成 / .jsonl 可导入 / 不可达降级
- [ ] **spec §8.4 测试覆盖目标**:devnpc +7、devnpc-dashboard +7(集成),配合 Phase 1–4 单元测试达成 ~450 总数(Task 11 Step 4 验证)
- [ ] **文档**:`.env.example` 已添加 DEVNPC_DASHBOARD_URL/TOKEN/BATCH_SIZE/BATCH_INTERVAL_SECS/LOCAL_LOG(Task 10);未创建 README.md(符合用户约束)
- [ ] **无占位符**:所有 Step 包含完整可运行代码或确切命令;无 "TODO/TBD/类似 Task N"
- [ ] **类型一致**:`EventLogEntry` 的 `kind` 标签与 `ExecutionEvent` 的 `type` 标签贯穿 common helper 与所有测试断言,与 Phase 1 的 `event_schema.rs` 定义一致;`DashboardConfig` 字段名与 spec §4.1 一致
- [ ] **向后兼容**:本阶段仅新增测试文件与 .env.example,不修改 Phase 1–4 的生产代码(除两处 Cargo.toml dev-dep 追加)
- [ ] **降级可靠**:降级测试(Task 9)验证 dashboard 故障时 EventSender 不 panic、不永久阻塞,符合 spec §7.1 核心原则
