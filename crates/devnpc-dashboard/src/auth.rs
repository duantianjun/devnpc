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
    use axum::middleware::from_fn_with_state;
    use axum::routing::get;
    use axum::Router;
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
