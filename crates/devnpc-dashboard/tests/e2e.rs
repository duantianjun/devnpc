//! 端到端集成测试: 完整推送 + 查询 + SSE + 导入流程

mod common;

use axum::body::Body;
use axum::http::Request;
use devnpc_dashboard::realtime::RealtimeHub;
use devnpc_dashboard::server::build_router;
use devnpc_dashboard::state::AppState;
use devnpc_dashboard::storage::queries::Storage;
use std::sync::Arc;
use tower::ServiceExt;

use common::TestServer;

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

// ============================================================
// Phase 5: 真实 HTTP server (随机端口) 端到端测试
// ============================================================

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
    assert_eq!(
        task["mr_iid"].as_u64(),
        Some(42),
        "mr_iid 应为 42, 实际: {:?}",
        task["mr_iid"]
    );

    // 5. 校验 GET /api/tasks (任务列表 JSON) 包含该任务
    //    (GET / 页面通过 AJAX 异步加载 /api/tasks,HTML 本身不含 task_id)
    let resp = client
        .get(format!("{}/api/tasks", server.base_url))
        .send()
        .await
        .unwrap();
    assert!(resp.status().is_success(), "GET /api/tasks 应返回 2xx");
    let list: serde_json::Value = resp.json().await.unwrap();
    let tasks = list["tasks"]
        .as_array()
        .expect("任务列表响应应含 tasks 数组");
    let found = tasks
        .iter()
        .any(|t| t["task_id"].as_str() == Some(task_id.as_str()));
    assert!(found, "任务列表应包含刚创建的 task_id");

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
    while let Ok(Some(Ok(chunk))) = tokio::time::timeout_at(deadline, stream.next()).await {
        received.push_str(&String::from_utf8_lossy(&chunk));
        if received.contains("data:") {
            break;
        }
    }
    assert!(
        received.contains("data:"),
        "SSE 流应在 3s 内收到 data: 行, 实际收到: {received}"
    );
}
