//! E2E / 导入测试公共辅助
//!
//! 在随机端口启动一个真实的 devnpc-dashboard 实例,供集成测试发送 HTTP 请求。
//! 同时提供事件构造 helper,保证各测试用例数据一致。
//!
//! 注意: 本模块被多个测试 crate (e2e / import) 独立编译,某些 helper 在某个 crate 中
//! 可能未被使用。此处整体允许 dead_code,避免在 clippy -D warnings 下报错。

#![allow(dead_code)]

use std::sync::Arc;

use devnpc_core::report::event_schema::{
    BatchEventsRequest, EventLogEntry, ExecutionEvent, TaskFinishedEvent, TaskStartedEvent,
    TaskStatus,
};
use devnpc_dashboard::realtime::RealtimeHub;
use devnpc_dashboard::server::build_router;
use devnpc_dashboard::state::AppState;
use devnpc_dashboard::storage::queries::Storage;

/// 一个运行中的 dashboard 测试实例
pub struct TestServer {
    pub base_url: String,
    pub token: String,
    // 保活临时目录(内含 SQLite 文件),随 TestServer drop 清理
    _dir: tempfile::TempDir,
}

impl TestServer {
    /// 使用默认 token "test-token" 启动
    pub async fn start() -> Self {
        Self::start_with_token("test-token").await
    }

    /// 使用自定义 token 启动
    pub async fn start_with_token(token: &str) -> Self {
        let dir = tempfile::tempdir().expect("创建临时目录失败");
        let db_path = dir.path().join("test-dashboard.db");
        // Storage::open 接收 &str;Windows 下 SQLite 接受反斜杠路径
        let db_path_str = db_path.to_str().expect("临时 DB 路径含非 UTF-8 字符");
        let storage = Storage::open(db_path_str).expect("打开 dashboard Storage 失败");
        let hub = Arc::new(RealtimeHub::new(100));
        let state = AppState {
            storage,
            hub,
            token: token.to_string(),
        };
        let app = build_router(state);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("绑定随机端口失败");
        let addr = listener.local_addr().expect("获取端口失败");
        tokio::spawn(async move {
            // 后台运行 dashboard,测试结束随 runtime 退出
            axum::serve(listener, app)
                .await
                .expect("dashboard 服务异常退出");
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
