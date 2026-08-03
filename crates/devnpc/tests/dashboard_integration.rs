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
        // wiremock 的 Request.url 是 Url 类型,用 path() 取路径部分匹配
        let count = requests
            .iter()
            .filter(|r| r.url.path().starts_with(path))
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

// ============================================================
// EventSender 测试
// ============================================================

/// EventSender::send_start 应推送 POST /api/events/start
#[tokio::test]
async fn event_sender_posts_task_started() {
    let mock = wiremock::MockServer::start().await;
    mount_ok(&mock, "/api/events/start").await;

    let config = config_with(mock.uri());
    let started = make_started("t-start-1");
    let sender = EventSender::new(&config, "t-start-1");
    // 实际 API: send_start 是独立的异步方法
    sender.send_start(&config, &started).await;

    // 等待后台推送完成
    wait_for_request(&mock, "/api/events/start", 1).await;
}

/// 累积到 batch_size 阈值时触发批量推送 (不受时间阈值影响)
#[tokio::test]
async fn batch_push_triggers_on_count_threshold() {
    let mock = wiremock::MockServer::start().await;
    mount_ok(&mock, "/api/events/batch").await;

    // batch_size=3, batch_interval_secs=60 (大间隔,只靠数量触发)
    let config = DashboardConfig {
        batch_size: 3,
        batch_interval_secs: 60,
        ..config_with(mock.uri())
    };
    let sender = EventSender::new(&config, "t-cnt-1");

    // 推 3 条,达到数量阈值 → 触发一次 batch
    sender.send(make_llm_call(1));
    sender.send(make_tool_call("read_file"));
    sender.send(make_llm_call(2));

    wait_for_request(&mock, "/api/events/batch", 1).await;
}

/// 距上次推送超过 batch_interval_secs 时触发批量推送 (未达数量阈值)
#[tokio::test]
async fn batch_push_triggers_on_time_threshold() {
    let mock = wiremock::MockServer::start().await;
    mount_ok(&mock, "/api/events/batch").await;

    // batch_size=100 (大数量,不靠数量触发), batch_interval_secs=1
    let config = DashboardConfig {
        batch_size: 100,
        batch_interval_secs: 1,
        ..config_with(mock.uri())
    };
    let sender = EventSender::new(&config, "t-time-1");

    // 只推 1 条 (未达数量阈值),等待时间阈值触发 (~1s)
    sender.send(make_llm_call(1));

    wait_for_request(&mock, "/api/events/batch", 1).await;
}

/// finish 应 flush 残留事件 (batch) 并推送 POST /api/events/finish
#[tokio::test]
async fn finish_posts_task_finished_and_flushes() {
    let mock = wiremock::MockServer::start().await;
    mount_ok(&mock, "/api/events/batch").await;
    mount_ok(&mock, "/api/events/finish").await;

    let config = config_with(mock.uri());
    let sender = EventSender::new(&config, "t-fin-1");

    // 推 1 条 (未达 batch_size=20,留在 channel)
    sender.send(make_llm_call(1));

    // finish: 先 flush channel (1 条 → batch),再 POST finish
    // 实际 API: finish(self, config, event)
    let finished = make_finished("t-fin-1", TaskStatus::Success);
    sender.finish(&config, finished).await;

    wait_for_request(&mock, "/api/events/batch", 1).await;
    wait_for_request(&mock, "/api/events/finish", 1).await;
}

// ============================================================
// LocalEventLogger 测试
// ============================================================

/// LocalEventLogger 应在 artifact_dir 下生成 {task_id}.jsonl,
/// 按顺序写入 task_started / execution / task_finished 行
#[test]
fn local_logger_writes_jsonl_file() {
    let dir = tempfile::tempdir().unwrap();
    let task_id = "log-1";

    let started = make_started(task_id);
    // 实际 API: LocalEventLogger::new 返回 Option<Self>
    let logger = LocalEventLogger::new(task_id, &started, dir.path()).expect("logger 创建失败");

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

/// LocalEventLogger 生成的 .jsonl 文件应可被 dashboard Storage 导入
#[test]
fn local_logger_file_importable_by_dashboard() {
    use devnpc_dashboard::storage::queries::Storage;

    // 1. 用 LocalEventLogger 生成 .jsonl
    let dir = tempfile::tempdir().unwrap();
    let task_id = format!("imp-{}", uuid::Uuid::new_v4());
    let started = make_started(&task_id);
    let logger = LocalEventLogger::new(&task_id, &started, dir.path()).expect("logger 创建失败");
    logger.log_event(&make_llm_call(1));
    logger.log_event(&make_tool_call("read_file"));
    logger.finish(&make_finished(&task_id, TaskStatus::Success));

    let file_path = dir.path().join(format!("{task_id}.jsonl"));
    let content = std::fs::read_to_string(&file_path).unwrap();
    assert!(!content.is_empty(), "jsonl 文件不应为空");

    // 2. 用 dashboard 的 Storage 直接导入,验证文件可被解析写入
    let db_dir = tempfile::tempdir().unwrap();
    let db_path = db_dir.path().join("import.db");
    // Storage::open 接收 &str
    let db_path_str = db_path.to_str().expect("临时 DB 路径含非 UTF-8 字符");
    let storage = Storage::open(db_path_str).expect("打开 dashboard Storage 失败");
    let result = storage
        .import_from_jsonl(&content)
        .expect("dashboard 导入 .jsonl 失败");
    assert_eq!(result.task_id, task_id, "导入返回的 task_id 应匹配");
    assert_eq!(result.events_count, 2, "应导入 2 条 execution 事件");
    assert!(!result.skipped, "首次导入不应跳过");
}
