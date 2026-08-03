//! 路由组装与静态资源服务

pub mod api;
pub mod views;

use axum::extract::{Path, State};
use axum::http::{header, StatusCode};
use axum::middleware::from_fn_with_state;
use axum::response::{Html, IntoResponse, Response, Sse};
use axum::routing::{get, post};
use axum::Router;
use futures::{Stream, StreamExt};
use rust_embed::RustEmbed;
use std::convert::Infallible;

use askama::Template;
use crate::auth::require_token;
use crate::error::DashboardError;
use crate::server::views::{
    IndexTemplate, RealtimeTemplate, TaskDetailTemplate, TrendsTemplate,
};
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
        // === 页面路由 (Phase 4) ===
        .route("/", get(index_page))
        .route("/tasks/:id", get(task_detail_page))
        .route("/realtime", get(realtime_page))
        .route("/trends", get(trends_page))
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

// ============================================================
// 页面 handler (Phase 4: askama 服务端渲染)
// ============================================================

/// GET / - 任务列表页
pub async fn index_page() -> Result<Html<String>, DashboardError> {
    let tmpl = IndexTemplate {
        active_nav: "tasks".to_string(),
    };
    let html = tmpl.render()?;
    Ok(Html(html))
}

/// GET /tasks/:id - 任务详情页
pub async fn task_detail_page(
    State(state): State<AppState>,
    Path(task_id): Path<String>,
) -> Result<Html<String>, DashboardError> {
    let task = state
        .storage
        .get_task(&task_id)?
        .ok_or_else(|| DashboardError::TaskNotFound(task_id.clone()))?;
    let tmpl = TaskDetailTemplate {
        active_nav: "tasks".to_string(),
        task,
    };
    let html = tmpl.render()?;
    Ok(Html(html))
}

/// GET /realtime - 实时监控页
pub async fn realtime_page() -> Result<Html<String>, DashboardError> {
    let tmpl = RealtimeTemplate {
        active_nav: "realtime".to_string(),
    };
    let html = tmpl.render()?;
    Ok(Html(html))
}

/// GET /trends - 趋势统计页
pub async fn trends_page() -> Result<Html<String>, DashboardError> {
    let tmpl = TrendsTemplate {
        active_nav: "trends".to_string(),
    };
    let html = tmpl.render()?;
    Ok(Html(html))
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
    async fn static_serves_dashboard_css_with_text_css_mime() {
        // dashboard.css 由 Phase 4 Task 1 创建,验证 rust-embed 嵌入生效
        let app = build_router(make_state());
        let req = Request::builder()
            .method("GET")
            .uri("/static/css/dashboard.css")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let ct = resp.headers().get("content-type").unwrap();
        assert_eq!(ct, "text/css");
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
