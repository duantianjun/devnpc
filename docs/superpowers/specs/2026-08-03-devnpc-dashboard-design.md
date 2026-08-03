# devnpc Dashboard 设计方案

> 日期: 2026-08-03
> 状态: 待实施
> 关联: [技术分析报告](../../devnpc-技术分析报告.md) §3.3 低优先级改进 / §5 长期目标

---

## 一、背景与目标

### 1.1 背景

技术分析报告指出 devnpc 当前仅生成静态 HTML 报告，缺少实时可视化 Dashboard（报告 §3.3，标记为低优先级；§5 列为长期目标第 9 项：实时任务监控 + 历史趋势分析）。

### 1.2 目标

构建独立的 `devnpc-dashboard` 长驻服务，提供：

1. **实时监控**：devnpc 任务执行过程中事件流式推送到 dashboard，实时展示当前 SOP 步骤、工具调用、CI 状态
2. **历史分析**：任务数据持久化到 SQLite，提供趋势统计、成本分析、CI 自愈统计、SOP 偏离监控等聚合视图
3. **单次任务详情**：完整轨迹时间线回放

### 1.3 已确认的决策

| # | 决策点 | 选型 | 理由 |
|---|--------|------|------|
| 1 | 定位 | 综合型（实时+历史） | 一个面板看全貌 |
| 2 | 部署形态 | 独立长驻服务 | 适合多项目/多实例汇聚 |
| 3 | 数据通道 | 实时事件流推送 | 满足实时监控需求 |
| 4 | 持久化 | SQLite（rusqlite） | 零外部依赖，复用现有依赖 |
| 5 | 传输协议 | HTTP POST 批量推送 | 简单可靠，重试语义清晰 |
| 6 | 前端 | LayUI 2.x + 服务端渲染 | 静态文件由用户后续放入 static 目录 |
| 7 | 视图模块 | 7 个模块一次性完成 | 不分阶段 |
| 8 | devnpc 接入 | channel + 异步批量 POST | 不阻塞 Agent 主循环 |
| 9 | 鉴权 | 推送 token + 查看免鉴权 | 推送防伪造，查看开放 |
| 10 | devnpc 触发 | 配置驱动（.env） | 符合项目约定，dashboard 可选 |
| 11 | 架构方案 | B（workspace 拆分） | 共享 core crate，dashboard 精简 |
| 12 | 兜底机制 | 本地 .jsonl 事件文件 + dashboard 导入 | 推送失败时事后可导入，文件独立于 dashboard 配置 |

---

## 二、Workspace 结构

将现有单 crate 改造成 Cargo workspace，三 crate 布局：

```
devnpc/                              (workspace 根)
├── Cargo.toml                       (workspace 定义,移除 [package])
├── crates/
│   ├── devnpc-core/                 (共享 lib)
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── report/
│   │       │   ├── collector.rs     (从 src/report/ 迁移: Trajectory/ReportData/CostEstimate)
│   │       │   ├── event_schema.rs  (新增: dashboard 事件协议类型)
│   │       │   └── mod.rs
│   │       └── error.rs             (共享错误类型)
│   ├── devnpc/                      (现有 bin,精简)
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── main.rs              (现有: run/serve/config/info)
│   │       ├── adapter/ ci/ config/ git/ gitlab_api/ memory/ trigger/
│   │       └── (移除 report/,改用 devnpc-core)
│   └── devnpc-dashboard/            (新 bin)
│       ├── Cargo.toml
│       ├── src/
│       │   ├── main.rs              (CLI: --port/--host/--db/--token)
│       │   ├── server/
│       │   │   ├── mod.rs
│       │   │   ├── routes.rs        (页面路由)
│       │   │   └── api.rs           (推送 API + 辅助 API)
│       │   ├── storage/
│       │   │   ├── mod.rs
│       │   │   ├── schema.rs        (建表/迁移)
│       │   │   └── queries.rs       (CRUD + 聚合查询)
│       │   ├── realtime/
│       │   │   └── mod.rs           (内存环形缓冲 + SSE 推送)
│       │   ├── views/               (askama 模板)
│       │   │   ├── layout.html
│       │   │   ├── index.html
│       │   │   ├── task_detail.html
│       │   │   ├── realtime.html
│       │   │   ├── trends.html
│       │   │   ├── cost.html
│       │   │   ├── ci.html
│       │   │   └── sop.html
│       │   └── auth.rs              (推送 token 中间件)
│       └── static/                  (LayUI 静态文件,用户后续放入)
│           ├── layui/
│           ├── css/
│           └── js/
```

### 依赖关系

- `devnpc-core` → 无内部依赖（仅 axum/serde/chrono/uuid/thiserror 等基础）
- `devnpc` → `devnpc-core`
- `devnpc-dashboard` → `devnpc-core`

### 迁移要点

1. 现有 `src/report/collector.rs` 里的 `Trajectory`/`ReportData`/`TrajectoryEvent`/`CostEstimate`/`TrajectorySummary` 迁到 `devnpc-core/src/report/`
2. `src/error.rs` 的共享错误类型迁到 `devnpc-core`
3. devnpc crate 的 `use devnpc::report::collector::*` 改为 `use devnpc_core::report::collector::*`
4. 现有 350 个测试相应调整 `use` 路径，逻辑不变

---

## 三、数据模型与事件协议

### 3.1 事件流协议（devnpc → dashboard）

定义在 `devnpc-core/src/report/event_schema.rs`：

```rust
/// 任务启动事件 (任务开始时推送一次)
pub struct TaskStartedEvent {
    pub task_id: String,           // UUID v4,贯穿任务全生命周期
    pub project: String,           // GitLab 项目路径
    pub mr_iid: Option<u64>,
    pub pipeline_id: Option<u64>,
    pub task_description: String,
    pub task_kind: String,         // issue/mr_comment/manual
    pub started_at: String,        // RFC3339
    pub model: String,             // 使用的 LLM 模型名
}

/// 执行过程事件 (任务执行中持续推送)
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
        detail: String,            // 工具调用摘要(非完整参数)
    },
    SopStep {
        step: String,
        status: SopStepStatus,     // Started/Completed/Deviated
        note: Option<String>,
    },
    CiStatus {
        pipeline_id: u64,
        status: CiStatus,          // Running/Passed/Failed
        attempt: u8,               // 第几次重试
    },
    TeamHandoff {                  // Team 模式角色交接
        from_role: String,         // pm/developer/tester
        to_role: String,
        signal: String,            // decomposed/implemented/tested
    },
}

/// 任务结束事件 (任务完成时推送一次)
pub struct TaskFinishedEvent {
    pub task_id: String,
    pub status: TaskStatus,        // Success/Failed/CiFailed/Timeout
    pub duration_secs: u64,
    pub total_tokens: u64,
    pub estimated_cost_usd: f64,
    pub mr_url: Option<String>,
    pub ci_url: Option<String>,
    pub summary: String,           // LLM 生成的验收摘要
    pub error: Option<String>,     // 失败原因
    pub finished_at: String,
}
```

### 3.2 推送 API

| 方法 | 路径 | Header | Body | 说明 |
|------|------|--------|------|------|
| POST | `/api/events/start` | `X-Devnpc-Token: xxx` | `TaskStartedEvent` | 任务启动 |
| POST | `/api/events/batch` | `X-Devnpc-Token: xxx` | `{ task_id, events: [ExecutionEvent...] }` | 批量执行事件 |
| POST | `/api/events/finish` | `X-Devnpc-Token: xxx` | `TaskFinishedEvent` | 任务结束 |
| POST | `/api/events/import` | `X-Devnpc-Token: xxx` | `multipart/form-data` (上传 .jsonl 文件) | 导入本地事件文件(兜底) |

### 3.3 批量推送策略

- devnpc 侧 `Trajectory::record_*` 写入 `mpsc::channel`
- 后台 task 按以下条件之一触发 POST `/api/events/batch`：
  - 事件数累积到 **20 条**
  - 距上次推送超过 **3 秒**
  - channel 关闭（任务结束 flush）
- 推送失败时**指数退避重试**（1s/2s/4s/8s/16s，最多 5 次），仍失败则丢弃并记 tracing::warn

### 3.4 本地事件文件（兜底机制）

实时推送可能因网络/dashboard 不可达而失败，devnpc 在本地同时保存一份完整事件文件，可在事后导入 dashboard 查看。

**文件格式**：JSON Lines（.jsonl），每行一个事件，流式追加写入，进程崩溃也不丢已写入事件。

**文件位置**：与现有 HTML 报告同目录（artifact 目录），文件名 `{task_id}.jsonl`

**文件内容**（按行顺序）：
```jsonl
{"type":"task_started","task_id":"a8c6...","project":"proj-a",...}
{"type":"execution","task_id":"a8c6...","event":{"LlmCall":{"iteration":1,...}}}
{"type":"execution","task_id":"a8c6...","event":{"ToolCall":{"name":"read_file",...}}}
{"type":"execution","task_id":"a8c6...","event":{"SopStep":{"step":"analyze",...}}}
{"type":"task_finished","task_id":"a8c6...","status":"Success",...}
```

**写入时机**：`EventSender` 内部同时做两件事（并行，互不影响）：
1. 写入 mpsc channel → 后台批量 POST 推送（实时）
2. 追加写入本地 `.jsonl` 文件（兜底）

**导入接口**（`POST /api/events/import`）：
- 接收 multipart 文件上传（.jsonl）
- 逐行解析 JSON，按顺序写入 SQLite
- **幂等处理**：基于 task_id 判断
  - 若 task_id 已存在且状态为 running（仅 start 无 finish）：覆盖该任务及其事件
  - 若 task_id 已存在且已 finish：跳过，返回 409 Conflict + 提示"任务已存在"
  - 若 task_id 不存在：正常导入
- 返回：`{ imported: true, task_id: "xxx", events_count: N }` 或冲突信息

**导入触发方式**：
- 任务列表页（`/`）顶部增加"导入事件文件"按钮
- 点击弹出文件选择框，选择 .jsonl 文件上传
- 上传成功后刷新任务列表

### 3.5 SQLite Schema

```sql
-- 任务主表
CREATE TABLE tasks (
    task_id TEXT PRIMARY KEY,
    project TEXT NOT NULL,
    mr_iid INTEGER,
    pipeline_id INTEGER,
    task_description TEXT NOT NULL,
    task_kind TEXT NOT NULL,
    model TEXT NOT NULL,
    status TEXT NOT NULL,          -- running/success/failed/ci_failed/timeout
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

-- 执行事件表 (按 task_id 关联)
CREATE TABLE events (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    task_id TEXT NOT NULL,
    seq INTEGER NOT NULL,          -- 事件序号(同 task 内递增)
    event_type TEXT NOT NULL,      -- llm_call/tool_call/sop_step/ci_status/team_handoff
    payload TEXT NOT NULL,         -- JSON
    created_at TEXT NOT NULL,
    FOREIGN KEY (task_id) REFERENCES tasks(task_id)
);

-- SOP 偏离记录表 (从 sop_step 事件中偏离的单独索引,便于 SOP 视图查询)
CREATE TABLE sop_deviations (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    task_id TEXT NOT NULL,
    step TEXT NOT NULL,
    note TEXT,
    created_at TEXT NOT NULL,
    FOREIGN KEY (task_id) REFERENCES tasks(task_id)
);

-- 索引
CREATE INDEX idx_events_task_id ON events(task_id);
CREATE INDEX idx_tasks_status ON tasks(status);
CREATE INDEX idx_tasks_project ON tasks(project);
CREATE INDEX idx_tasks_started_at ON tasks(started_at);
```

**WAL 模式**：dashboard 启动时执行 `PRAGMA journal_mode=WAL;` 保证读写并发。

---

## 四、devnpc 侧改造

### 4.1 配置扩展

`config/env.rs` 和 `Config` 结构新增字段：

```rust
pub struct DashboardConfig {
    pub enabled: bool,             // 默认 false,未配置 URL 时不推送
    pub url: String,               // DEVNPC_DASHBOARD_URL
    pub token: String,             // DEVNPC_DASHBOARD_TOKEN
    pub batch_size: usize,         // 默认 20
    pub batch_interval_secs: u64,  // 默认 3
    pub local_event_log: bool,     // 默认 true,即使 dashboard 未启用也保存本地事件文件
}
```

`.env` 示例：
```
DEVNPC_DASHBOARD_URL=http://dashboard-host:8080
DEVNPC_DASHBOARD_TOKEN=your-secret-token
# local_event_log 默认 true,无需配置;设为 false 可关闭本地事件文件
DEVNPC_DASHBOARD_LOCAL_LOG=false
```

**降级策略**：`enabled=false`（未配置 URL）时，不创建 `EventSender`，不推送事件，`Trajectory::record_*` 行为退化为现状（只写内存 Vec），不影响现有测试。

**本地文件独立于 dashboard 推送**：即使 `enabled=false`，只要 `local_event_log=true`（默认），devnpc 仍会保存本地 `.jsonl` 事件文件作为任务记录，方便事后导入 dashboard。文件写入由独立的 `LocalEventLogger` 组件负责，不依赖 `EventSender`。

### 4.2 事件推送器与本地事件记录器

新增 `src/report/sender.rs`（位于 devnpc crate），包含两个独立组件：

```rust
/// 本地事件记录器 (兜底机制,独立于推送)
/// 即使 dashboard 未配置,只要 local_event_log=true 就会创建
pub struct LocalEventLogger {
    task_id: String,
    writer: Arc<Mutex<Option<BufWriter<File>>>>,
}

impl LocalEventLogger {
    /// 创建本地 .jsonl 文件 (在 artifact 目录),写入 task_started 行
    pub fn new(task_id: &str, started: &TaskStartedEvent, artifact_dir: &Path) -> Self { ... }

    /// 追加一行 execution 事件
    pub fn log_event(&self, event: &ExecutionEvent) { ... }

    /// 写入 task_finished 行并关闭文件
    pub fn finish(&self, finished: &TaskFinishedEvent) { ... }
}

/// 事件推送器 (仅 dashboard.enabled=true 时创建)
pub struct EventSender {
    tx: mpsc::Sender<ExecutionEvent>,
    task_id: String,
    handle: Option<JoinHandle<()>>,
}

impl EventSender {
    /// 创建推送器并启动后台 task
    pub fn new(config: &DashboardConfig, task_id: &str) -> Self { ... }

    /// 推送单条事件 (非阻塞,写入 channel)
    pub fn send(&self, event: ExecutionEvent) { ... }

    /// 任务结束时 flush 并发送 TaskFinishedEvent
    pub async fn finish(self, event: TaskFinishedEvent) { ... }
}
```

**Trajectory 持有三个可选组件**：
- `events: Vec<TrajectoryEvent>` — 内存事件列表（现状，始终存在）
- `local_logger: Option<LocalEventLogger>` — 本地文件记录（`local_event_log=true` 时存在）
- `sender: Option<EventSender>` — 实时推送（`enabled=true` 时存在）

**record_* 方法的行为**：
```rust
pub fn record_llm_call(&mut self, iteration: usize) {
    let event = ExecutionEvent::LlmCall { ... };
    self.events.push(TrajectoryEvent::LlmCall { iteration });
    if let Some(logger) = &self.local_logger { logger.log_event(&event); }
    if let Some(sender) = &self.sender { sender.send(event); }
}
```

**本地文件写入要点**：
- 文件创建时机：`LocalEventLogger::new()` 时立即创建，先写入 `task_started` 行
- 每次 `log_event()` 追加一行，带 `BufWriter` 但每次 `flush()` 保证落盘（事件量小，性能无影响）
- `finish()` 写入 `task_finished` 行后关闭文件
- 文件写入失败：tracing::warn 记录，不影响推送和主任务（文件只是兜底）

### 4.3 Trajectory 改造

```rust
pub struct Trajectory {
    pub events: Vec<TrajectoryEvent>,
    local_logger: Option<LocalEventLogger>,  // 本地文件记录
    sender: Option<EventSender>,             // 实时推送
    task_id: String,
}

impl Trajectory {
    /// 现状构造 (无日志无推送,兼容现有测试)
    pub fn new() -> Self { ... }

    /// 带本地日志和推送的构造
    pub fn with_logging(
        task_id: String,
        local_logger: Option<LocalEventLogger>,
        sender: Option<EventSender>,
    ) -> Self { ... }

    // record_* 方法见 4.2 末尾示例
}
```

### 4.4 主流程接入

`main.rs` 的 `run()` 改造：

1. **任务启动时**：生成 `task_id = Uuid::new_v4()`
   - 若 `config.dashboard.local_event_log`：创建 `LocalEventLogger`，写入 `task_started` 行
   - 若 `config.dashboard.enabled`：创建 `EventSender`，POST `/api/events/start`
2. **执行过程中**：`Trajectory::with_logging()` 构造，所有 `record_*` 自动写本地文件 + 推送
3. **任务结束时**：`build_report()` 后
   - `local_logger.finish()` 写入 `task_finished` 行
   - `sender.finish()` POST `/api/events/finish`

### 4.5 影响面

| 模块 | 改动 |
|------|------|
| `Cargo.toml` | devnpc crate 依赖 `devnpc-core`，移除已迁出的 report 模块 |
| `src/report/mod.rs` | 移除 collector.rs，新增 sender.rs，re-export core 类型 |
| `src/report/collector.rs` | 迁移到 core，devnpc 侧删除 |
| `src/main.rs` | `run()` 增加 dashboard 推送初始化和结束逻辑 |
| `src/config/` | 新增 `DashboardConfig` |
| 现有测试 | `use devnpc::report::collector::*` → `use devnpc_core::report::collector::*` |

---

## 五、Dashboard 服务端

### 5.1 启动与配置

`devnpc-dashboard/src/main.rs`：

```rust
#[derive(Parser)]
struct Cli {
    #[arg(long)]
    port: Option<u16>,                    // 默认 8080
    #[arg(long)]
    host: Option<String>,                 // 默认 0.0.0.0
    #[arg(long)]
    db: Option<String>,                   // 默认 ./devnpc-dashboard.db
    #[arg(long)]
    token: Option<String>,                // DEVNPC_DASHBOARD_TOKEN
    #[arg(long)]
    realtime_buffer: Option<usize>,       // 默认 1000
}
```

环境变量：`DEVNPC_DASHBOARD_PORT` / `DEVNPC_DASHBOARD_HOST` / `DEVNPC_DASHBOARD_DB` / `DEVNPC_DASHBOARD_TOKEN` / `DEVNPC_DASHBOARD_REALTIME_BUFFER`

启动流程：
1. 加载 `.env`
2. 打开/创建 SQLite，执行 schema 迁移，开启 WAL
3. 初始化 `RealtimeHub`
4. 构建 axum Router，绑定监听地址

### 5.2 路由总表

| 方法 | 路径 | 鉴权 | 说明 |
|------|------|------|------|
| POST | `/api/events/start` | token | 创建任务记录，状态=running，加入实时缓冲 |
| POST | `/api/events/batch` | token | 批量写入 events 表，更新实时缓冲，SSE 广播 |
| POST | `/api/events/finish` | token | 更新任务状态/汇总字段，写入 sop_deviations |
| POST | `/api/events/import` | token | 导入本地 .jsonl 事件文件（multipart 上传，幂等处理） |
| GET | `/` | 无 | 任务列表（分页，最近 100 条） |
| GET | `/tasks/:id` | 无 | 任务详情（时间线 + 事件列表） |
| GET | `/realtime` | 无 | 实时监控（SSE 推送当前 running 任务事件） |
| GET | `/trends` | 无 | 趋势统计（7/30/90 天图表） |
| GET | `/cost` | 无 | 成本分析（按项目/模型/任务类型拆分） |
| GET | `/ci` | 无 | CI 自愈统计 |
| GET | `/sop` | 无 | SOP 偏离监控 |
| GET | `/static/*` | 无 | LayUI 静态资源（rust-embed 嵌入） |
| GET | `/api/realtime/stream` | 无 | SSE 端点，推送实时事件到浏览器 |
| GET | `/api/tasks` | 无 | 任务列表 JSON（支持分页/过滤） |
| GET | `/api/tasks/:id` | 无 | 单任务详情 JSON |
| GET | `/api/tasks/:id/events` | 无 | 单任务事件流 JSON |
| GET | `/api/stats/trends?days=7` | 无 | 趋势聚合数据 JSON |
| GET | `/api/stats/cost?group_by=project\|model\|kind` | 无 | 成本聚合 JSON |
| GET | `/api/stats/ci` | 无 | CI 自愈统计 JSON |
| GET | `/api/stats/sop` | 无 | SOP 偏离统计 JSON |

### 5.3 鉴权中间件

```rust
async fn require_token(State(state): State<AppState>, req: Request, next: Next) -> Response {
    let expected = &state.config.token;
    if expected.is_empty() {
        return (StatusCode::FORBIDDEN, "DEVNPC_DASHBOARD_TOKEN 未配置").into_response();
    }
    match req.headers().get("X-Devnpc-Token").and_then(|v| v.to_str().ok()) {
        Some(t) if t == expected => next.run(req).await,
        _ => (StatusCode::UNAUTHORIZED, "无效的推送 token").into_response(),
    }
}
```

### 5.4 RealtimeHub

```rust
pub struct RealtimeHub {
    buffer: Arc<RwLock<RingBuffer<RealtimeEvent>>>,       // 环形缓冲
    subscribers: Arc<RwLock<Vec<broadcast::Sender<RealtimeEvent>>>>,  // SSE 订阅者
    running_tasks: Arc<RwLock<HashSet<String>>>,          // 当前 running 任务
}

impl RealtimeHub {
    pub async fn push_events(&self, task_id: &str, events: &[ExecutionEvent]) { ... }
    pub async fn subscribe(&self) -> impl Stream<Item = RealtimeEvent> { ... }
}
```

### 5.5 存储层

```rust
pub struct Storage {
    conn: Arc<Mutex<rusqlite::Connection>>,
}

impl Storage {
    pub fn start_task(&self, e: &TaskStartedEvent) -> Result<()>;
    pub fn insert_events(&self, task_id: &str, events: &[ExecutionEvent]) -> Result<()>;
    pub fn finish_task(&self, e: &TaskFinishedEvent) -> Result<()>;
    pub fn list_tasks(&self, page: usize, size: usize, filter: &TaskFilter) -> Result<Vec<TaskRow>>;
    pub fn get_task(&self, task_id: &str) -> Result<Option<TaskRow>>;
    pub fn list_events(&self, task_id: &str) -> Result<Vec<EventRow>>;
    pub fn trends(&self, days: u32) -> Result<TrendsData>;
    pub fn cost_breakdown(&self, group_by: &str) -> Result<Vec<CostBucket>>;
    pub fn ci_stats(&self) -> Result<CiStats>;
    pub fn sop_stats(&self) -> Result<Vec<SopDeviationRow>>;

    // 导入相关
    pub fn task_exists(&self, task_id: &str) -> Result<bool>;
    pub fn task_is_finished(&self, task_id: &str) -> Result<bool>;
    pub fn delete_task(&self, task_id: &str) -> Result<()>;  // 覆盖导入时先删除
    pub fn import_from_jsonl(&self, content: &str) -> Result<ImportResult>;
}

pub struct ImportResult {
    pub task_id: String,
    pub events_count: usize,
    pub skipped: bool,        // true=因已 finish 而跳过
}
```

并发模型：`Arc<Mutex<Connection>>` 串行化写。WAL 模式下读不阻塞写。dashboard 是低频服务，单连接串行写完全够用。

### 5.6 视图层

使用 askama 编译期模板，`layout.html` 作为公共布局（侧边栏导航 + 顶部 + LayUI 引入）。每个页面模板继承 layout，通过 axum handler 渲染。

---

## 六、前端实现

### 6.1 技术栈

- **框架**：LayUI 2.x（用户后续放入 static 目录）
- **图表**：ECharts（与 LayUI 兼容）
- **模板**：askama 服务端渲染 HTML 骨架 + LayUI 组件
- **交互**：页面初始 SSR 渲染，后续动态更新走 AJAX
- **实时**：原生 `EventSource` API 订阅 `/api/realtime/stream`

### 6.2 页面结构

公共 layout：顶栏 + 侧边栏导航（任务列表/实时监控/趋势统计/成本分析/CI 自愈/SOP 偏离）+ 主内容区。

### 6.3 七个视图模块

#### 1. 任务列表 `/`
- `layui-table` + AJAX 数据源 `/api/tasks`
- 列：状态/项目/任务/耗时/Token/成本/时间
- 运行中任务行 5 秒自动刷新
- 状态用 LayUI badge：成功=绿/失败=红/运行=蓝/超时=橙
- 点击行跳转 `/tasks/:id`
- 顶部工具栏增加"导入事件文件"按钮：点击弹出 `layui-upload` 文件选择框，上传 .jsonl 到 `/api/events/import`，成功后刷新列表并提示"导入成功 N 条事件"

#### 2. 任务详情 `/tasks/:id`
- 任务元信息卡片（状态/耗时/Token/成本/项目/MR/CI）
- `layui-timeline` 展示执行时间线（LLM 调用/工具调用/SOP 步骤/CI 状态/Team 交接）
- 不同事件类型用不同图标和颜色
- Team 模式下按 `TeamHandoff` 分组展示
- 验收摘要 + 错误信息

#### 3. 实时监控 `/realtime`
- SSE 订阅 `/api/realtime/stream`
- 每个 running 任务一个 `layui-collapse` 折叠面板
- 事件实时追加到日志区
- 任务完成时面板边框变色，3 秒后自动收起

#### 4. 趋势统计 `/trends`
- 4 个 ECharts 图表：成功率/平均耗时/Token 消耗/成本
- 时间范围切换：7天/30天/90天
- 数据源 `/api/stats/trends?days=N`

#### 5. 成本分析 `/cost`
- ECharts 饼图（成本占比）+ 明细表格
- 分组维度切换：项目/模型/任务类型
- 数据源 `/api/stats/cost?group_by=xxx`

#### 6. CI 自愈统计 `/ci`
- 概览卡片：总失败/自动修复/成功率/平均重试
- ECharts 柱状图（重试分布）
- 失败任务列表

#### 7. SOP 偏离监控 `/sop`
- ECharts 柱状图（按步骤统计偏离频率）
- 偏离事件列表（时间/任务/步骤/说明）

### 6.4 实时刷新策略

| 页面 | 刷新方式 | 频率 |
|------|----------|------|
| 任务列表 | AJAX `table.reload` | 5 秒（仅当有 running 任务时） |
| 实时监控 | SSE `EventSource` | 实时推送 |
| 任务详情 | 不刷新 | - |
| 趋势/成本/CI/SOP | 不刷新 | 用户手动刷新 |

### 6.5 错误处理

- AJAX 请求失败：LayUI `layer.msg` 提示 + 3 秒后自动重试
- SSE 断连：`EventSource` 自动重连（浏览器原生），重连后补看缓冲区历史事件
- API 返回 4xx/5xx：页面顶部红色 banner 提示

---

## 七、错误处理

### 7.1 devnpc 侧

| 场景 | 处理 |
|------|------|
| `EventSender::start_task` 失败（dashboard 不可达） | tracing::warn 记录，**不中断任务执行**，降级为现状（只生成 HTML 报告） |
| `EventSender::send` channel 满 | 丢弃事件，tracing::warn，记录丢弃计数 |
| 后台批量 POST 失败 | 指数退避重试（1s/2s/4s/8s/16s，5 次），仍失败则丢弃该批，tracing::warn |
| `EventSender::finish` 失败 | 重试 3 次，失败则 tracing::warn，**不影响 HTML 报告发布** |
| `LocalEventLogger` 文件创建/写入失败 | tracing::warn 记录，后续事件跳过文件写入，**不影响推送和主任务** |
| `LocalEventLogger::finish` 失败 | tracing::warn 记录，**不影响 HTML 报告发布和推送** |
| `Config::load` 时 dashboard 配置非法 | tracing::warn 并降级为 `enabled=false`（推送关闭），`local_event_log` 保持默认 true |

**核心原则**：dashboard 推送和本地文件记录都是"尽力而为"，绝不能让它们失败影响 devnpc 主任务执行。本地 .jsonl 文件作为兜底，即使推送全部失败，事后仍可导入。

### 7.2 dashboard 侧

| 场景 | 处理 |
|------|------|
| SQLite 打开失败 | 启动失败，进程退出，错误信息打印到 stderr |
| Schema 迁移失败 | 启动失败，进程退出 |
| POST `/api/events/start` 重复 task_id | 返回 409 Conflict，已有记录不变 |
| POST `/api/events/batch` task_id 不存在 | 返回 404，丢弃事件 |
| POST `/api/events/finish` task_id 不存在 | 返回 404 |
| POST `/api/events/finish` task_id 已 finish | 返回 409 Conflict |
| POST `/api/events/import` 文件非 .jsonl 或格式错误 | 返回 400 Bad Request + 错误行号 |
| POST `/api/events/import` task_id 已 finish | 返回 409 Conflict + "任务已存在，跳过导入" |
| POST `/api/events/import` task_id 存在但未 finish | 覆盖导入（先删除再写入），返回 200 |
| POST `/api/events/import` 文件过大（>50MB） | 返回 413 Payload Too Large |
| Token 校验失败 | 返回 401 |
| SQLite 写入失败（磁盘满等） | 返回 500，记录 tracing::error |
| SSE 订阅者 channel 满（消费慢） | 丢弃该订阅者的旧事件，保持连接 |
| 模板渲染失败 | 返回 500 "页面渲染失败" |

### 7.3 错误类型

`devnpc-core/src/error.rs` 扩展：

```rust
#[derive(Debug, thiserror::Error)]
pub enum DevnpcError {
    // 现有变体...

    #[error("Dashboard 推送失败: {0}")]
    DashboardPush(String),

    #[error("Dashboard 配置错误: {0}")]
    DashboardConfig(String),
}
```

`devnpc-dashboard/src/error.rs`：

```rust
#[derive(Debug, thiserror::Error)]
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
```

---

## 八、测试策略

### 8.1 单元测试

**devnpc-core**（约 20 个测试）：
- `event_schema.rs`：事件序列化/反序列化 round-trip 测试
- `collector.rs`：现有 `Trajectory`/`ReportData` 测试迁移
- `error.rs`：错误类型 Display 一致性

**devnpc crate**（约 40 个新增测试）：
- `EventSender` 测试：
  - channel 满时丢弃行为
  - 后台 task 批量触发条件（数量阈值/时间阈值/flush）
  - 重试逻辑（mock HTTP server）
  - 降级行为（dashboard.enabled=false 时不推送）
- `LocalEventLogger` 测试：
  - 文件创建并写入 task_started 行
  - log_event 追加 execution 行
  - finish 写入 task_finished 行并关闭
  - 文件写入失败时降级（不 panic）
  - 生成的 .jsonl 文件可被 dashboard 导入解析
- `Trajectory::with_logging` 测试：record 方法同时写内存 + 本地文件 + 推送
- 现有 350 个测试：`use` 路径调整后全部通过

**devnpc-dashboard crate**（约 50 个测试）：
- `storage/queries.rs`：CRUD 全覆盖、聚合查询、重复 task_id 处理、分页/过滤
- `storage/import.rs`：JSONL 解析、幂等导入、覆盖导入、格式错误处理、大文件拒绝
- `realtime/`：环形缓冲容量限制、订阅/取消订阅、broadcast 推送
- `auth.rs`：token 校验中间件
- `server/routes.rs` + `api.rs`：axum::test 服务端集成测试（含 import 端点）

### 8.2 集成测试

**端到端流程测试**（`devnpc-dashboard/tests/e2e.rs`）：
1. 启动 dashboard（绑定随机端口）
2. POST `/api/events/start`
3. POST `/api/events/batch`（多次）
4. POST `/api/events/finish`
5. 验证 `GET /api/tasks/:id` 返回完整数据
6. 验证 `GET /`（任务列表）包含该任务
7. 验证 `GET /api/stats/trends` 包含聚合数据

**SSE 实时推送测试**：
1. 订阅 SSE
2. 推送事件
3. 验证 stream 收到事件

**导入流程测试**（`devnpc-dashboard/tests/import.rs`）：
1. 构造一个本地 .jsonl 文件（task_started + 多个 execution + task_finished）
2. POST `/api/events/import` 上传
3. 验证 `GET /api/tasks/:id` 返回导入的数据
4. 再次上传同一文件，验证幂等跳过（409 Conflict）
5. 上传格式错误的文件，验证 400 Bad Request

### 8.3 测试工具

- HTTP mock：`wiremock`（devnpc 已有 dev-dependency）
- 临时 SQLite：`tempfile`（devnpc 已有 dev-dependency）
- axum 测试：`axum::body::Body` + `tower::ServiceExt::oneshot`

### 8.4 测试覆盖目标

| crate | 现有测试 | 新增测试 | 总计 |
|-------|----------|----------|------|
| devnpc-core | ~10（从 devnpc 迁移） | ~10 | ~20 |
| devnpc | ~340 | ~40 | ~380 |
| devnpc-dashboard | 0 | ~50 | ~50 |
| **合计** | ~350 | ~100 | **~450** |

---

## 九、新增依赖

### devnpc-core
```
serde = { version = "1", features = ["derive"] }
serde_json = "1"
chrono = { version = "0.4", features = ["serde"] }
uuid = { version = "1", features = ["v4"] }
thiserror = "2"
```

### devnpc crate（新增）
```
devnpc-core = { path = "../devnpc-core" }
reqwest = { version = "0.12", default-features = false, features = ["json", "rustls-tls"] }  # 已有
tokio = { version = "1", features = ["sync", "rt", "macros"] }  # 已有
```

### devnpc-dashboard（新增 crate）
```
devnpc-core = { path = "../devnpc-core" }
axum = { version = "0.7", features = ["multipart"] }  # multipart 用于文件导入
tokio = { version = "1", features = ["full"] }
rusqlite = { version = "0.32", features = ["bundled"] }  # 已有
serde = { version = "1", features = ["derive"] }
serde_json = "1"
chrono = { version = "0.4", features = ["serde"] }
uuid = { version = "1", features = ["v4"] }
thiserror = "2"
clap = { version = "4", features = ["derive", "env"] }
askama = "0.12"
rust-embed = "8"
tower = "0.5"                   # 已有
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter", "json"] }
dotenvy = "0.15"                # .env 加载

[dev-dependencies]
tokio-test = "0.4"
wiremock = "0.6"
tempfile = "3"
tower = { version = "0.5", features = ["util"] }
```

---

## 十、成功标准

1. **功能完整**：7 个视图模块全部可用，实时监控 SSE 推送正常
2. **向后兼容**：未配置 `DEVNPC_DASHBOARD_URL` 时，devnpc 任务执行行为与现状完全一致
3. **本地兜底**：即使 dashboard 不可达，devnpc 仍生成本地 .jsonl 事件文件，可通过 dashboard 导入页面事后查看
4. **测试通过**：~450 个测试全部通过（含 100 个新增）
5. **零外部依赖**：dashboard 单二进制部署，仅需 SQLite 文件存储
6. **降级可靠**：dashboard 推送失败和本地文件写入失败均不影响 devnpc 主任务执行
7. **CI 友好**：devnpc 侧零 CI 配置改动（.env 驱动）

---

## 十一、实施顺序建议

1. **Workspace 拆分**：建立三 crate 结构，迁移 report/error 到 core，现有测试通过
2. **事件协议**：在 core 定义 event_schema.rs 类型
3. **devnpc 侧改造**：DashboardConfig + LocalEventLogger + EventSender + Trajectory 改造 + main.rs 接入
4. **dashboard 存储**：SQLite schema + queries CRUD + 聚合查询 + JSONL 导入解析
5. **dashboard 服务端**：axum 路由 + 推送 API + 导入 API + 鉴权 + RealtimeHub
6. **dashboard 视图**：askama 模板 + 7 个页面 handler + 静态资源嵌入
7. **前端交互**：LayUI 集成 + AJAX + SSE 订阅 + 图表渲染 + 导入按钮
8. **测试补全**：单元测试 + 集成测试 + E2E + 导入测试
9. **文档与示例**：.env.example 更新 + dashboard 启动说明 + .jsonl 格式说明
