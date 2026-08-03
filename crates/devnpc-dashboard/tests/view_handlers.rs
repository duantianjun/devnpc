//! 视图层 handler 集成测试
//!
//! 验证 7 个页面 handler 在空数据库情况下均能正确返回 HTML,
//! 以及静态资源路由和 404 行为。模板通过 askama 编译期渲染,
//! 此处主要验证 handler 路由可达性与 HTML 骨架内容。

use axum::body::Body;
use axum::http::{Request, StatusCode};
use std::sync::Arc;
use tower::ServiceExt;

use devnpc_dashboard::realtime::RealtimeHub;
use devnpc_dashboard::server::build_router;
use devnpc_dashboard::state::AppState;
use devnpc_dashboard::storage::queries::Storage;

/// 构造测试用 AppState (内存 SQLite,无需临时文件)
fn make_state() -> AppState {
    AppState {
        storage: Storage::open_in_memory().unwrap(),
        hub: Arc::new(RealtimeHub::new(100)),
        token: "test-token".to_string(),
    }
}

/// 读取响应 body 为 String (限制 1MB,HTML 页面远小于此)
async fn body_to_string(resp: axum::response::Response) -> String {
    let bytes = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    String::from_utf8(bytes.to_vec()).unwrap()
}

#[tokio::test]
async fn index_page_returns_html_with_title() {
    let app = build_router(make_state());
    let resp = app
        .oneshot(Request::builder().uri("/").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let html = body_to_string(resp).await;
    assert!(html.contains("任务列表"), "应包含页面标题");
    assert!(html.contains("layui-layout-admin"), "应包含 LayUI 布局类");
    assert!(html.contains("导入事件文件"), "应包含导入按钮");
}

#[tokio::test]
async fn realtime_page_returns_html() {
    let app = build_router(make_state());
    let resp = app
        .oneshot(Request::builder().uri("/realtime").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let html = body_to_string(resp).await;
    assert!(html.contains("EventSource"), "应包含 SSE EventSource 代码");
}

#[tokio::test]
async fn trends_page_returns_html_with_charts() {
    let app = build_router(make_state());
    let resp = app
        .oneshot(Request::builder().uri("/trends").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let html = body_to_string(resp).await;
    assert!(html.contains("chart-success"), "应包含成功率图表容器");
    assert!(html.contains("echarts.init"), "应包含 ECharts 初始化代码");
}

#[tokio::test]
async fn cost_page_returns_html() {
    let app = build_router(make_state());
    let resp = app
        .oneshot(Request::builder().uri("/cost").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let html = body_to_string(resp).await;
    assert!(html.contains("chart-pie"), "应包含饼图容器");
    assert!(html.contains("group_by"), "应包含分组维度查询参数");
}

#[tokio::test]
async fn ci_page_returns_html() {
    let app = build_router(make_state());
    let resp = app
        .oneshot(Request::builder().uri("/ci").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let html = body_to_string(resp).await;
    assert!(html.contains("stat-total-failed"), "应包含总失败统计卡片");
    assert!(html.contains("chart-retry"), "应包含重试分布图表");
}

#[tokio::test]
async fn sop_page_returns_html() {
    let app = build_router(make_state());
    let resp = app
        .oneshot(Request::builder().uri("/sop").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let html = body_to_string(resp).await;
    assert!(html.contains("chart-sop"), "应包含 SOP 图表");
    assert!(html.contains("deviation-table"), "应包含偏离事件列表表格");
}

#[tokio::test]
async fn task_detail_page_returns_404_for_unknown_task() {
    let app = build_router(make_state());
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/tasks/nonexistent-uuid")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn static_handler_serves_dashboard_css() {
    let app = build_router(make_state());
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/static/css/dashboard.css")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let ct = resp.headers().get("content-type").unwrap();
    assert_eq!(ct, "text/css");
}

#[tokio::test]
async fn static_handler_returns_404_for_missing_asset() {
    let app = build_router(make_state());
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/static/nonexistent/file.xyz")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}
