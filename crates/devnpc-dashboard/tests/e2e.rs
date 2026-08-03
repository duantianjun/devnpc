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
