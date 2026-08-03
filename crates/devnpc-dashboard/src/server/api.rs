//! Dashboard API handler
//!
//! 推送 API (token 鉴权): /api/events/*
//! 辅助 API (无鉴权): /api/tasks/*, /api/stats/*

use axum::extract::{Multipart, Path, Query, State};
use axum::Json as JsonExtractor;
use axum::response::Json;
use serde::Deserialize;

use devnpc_core::report::event_schema::{
    BatchEventsRequest, ImportResult, TaskFinishedEvent, TaskStartedEvent,
};

use crate::error::DashboardError;
use crate::state::AppState;
use crate::storage::queries::{EventRow, TaskFilter, TaskListResponse, TaskRow};

// ============================================================
// 推送 API
// ============================================================

/// POST /api/events/start - 创建任务记录
pub async fn start_task(
    State(state): State<AppState>,
    JsonExtractor(event): JsonExtractor<TaskStartedEvent>,
) -> Result<Json<serde_json::Value>, DashboardError> {
    state.storage.start_task(&event)?;
    state.hub.task_started(&event.task_id).await;
    Ok(Json(serde_json::json!({ "task_id": event.task_id, "status": "running" })))
}

/// POST /api/events/batch - 批量写入执行事件
pub async fn batch_events(
    State(state): State<AppState>,
    JsonExtractor(req): JsonExtractor<BatchEventsRequest>,
) -> Result<Json<serde_json::Value>, DashboardError> {
    let count = req.events.len();
    state.storage.insert_events(&req.task_id, &req.events)?;
    state.hub.push_events(&req.task_id, &req.events).await;
    Ok(Json(serde_json::json!({ "task_id": req.task_id, "received": count })))
}

/// POST /api/events/finish - 任务结束
pub async fn finish_task(
    State(state): State<AppState>,
    JsonExtractor(event): JsonExtractor<TaskFinishedEvent>,
) -> Result<Json<serde_json::Value>, DashboardError> {
    state.storage.finish_task(&event)?;
    state.hub.task_finished(&event.task_id).await;
    Ok(Json(serde_json::json!({ "task_id": event.task_id, "status": "finished" })))
}

/// POST /api/events/import - 导入本地 .jsonl 文件 (multipart)
pub async fn import_events(
    State(state): State<AppState>,
    mut multipart: Multipart,
) -> Result<Json<ImportResult>, DashboardError> {
    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| DashboardError::ImportFormat(format!("multipart 解析失败: {}", e)))?
    {
        let name = field.name().unwrap_or("").to_string();
        if name == "file" {
            let data = field
                .bytes()
                .await
                .map_err(|e| DashboardError::ImportFormat(format!("读取文件失败: {}", e)))?;
            if data.len() > 50 * 1024 * 1024 {
                return Err(DashboardError::ImportFormat("文件过大 (>50MB)".into()));
            }
            let content = String::from_utf8(data.to_vec())
                .map_err(|_| DashboardError::ImportFormat("文件非 UTF-8 编码".into()))?;
            let result = state.storage.import_from_jsonl(&content)?;
            if result.skipped {
                return Err(DashboardError::TaskConflict(format!(
                    "任务 {} 已存在，跳过导入",
                    result.task_id
                )));
            }
            return Ok(Json(result));
        }
    }
    Err(DashboardError::ImportFormat("未找到 file 字段".into()))
}

// ============================================================
// 辅助 API (无鉴权)
// ============================================================

/// GET /api/tasks 分页查询参数
#[derive(Debug, Deserialize)]
pub struct ListTasksQuery {
    pub page: Option<usize>,
    pub size: Option<usize>,
    pub status: Option<String>,
    pub project: Option<String>,
}

/// GET /api/tasks - 任务列表 JSON
pub async fn list_tasks(
    State(state): State<AppState>,
    Query(q): Query<ListTasksQuery>,
) -> Result<Json<TaskListResponse>, DashboardError> {
    let filter = TaskFilter {
        status: q.status,
        project: q.project,
    };
    let resp = state.storage.list_tasks(q.page.unwrap_or(1), q.size.unwrap_or(20), &filter)?;
    Ok(Json(resp))
}

/// GET /api/tasks/:id - 单任务详情 JSON
pub async fn get_task(
    State(state): State<AppState>,
    Path(task_id): Path<String>,
) -> Result<Json<TaskRow>, DashboardError> {
    match state.storage.get_task(&task_id)? {
        Some(row) => Ok(Json(row)),
        None => Err(DashboardError::TaskNotFound(task_id)),
    }
}

/// GET /api/tasks/:id/events - 单任务事件流 JSON
pub async fn list_task_events(
    State(state): State<AppState>,
    Path(task_id): Path<String>,
) -> Result<Json<Vec<EventRow>>, DashboardError> {
    Ok(Json(state.storage.list_events(&task_id)?))
}

/// GET /api/stats/trends?days=7
#[derive(Debug, Deserialize)]
pub struct TrendsQuery {
    pub days: Option<u32>,
}

pub async fn stats_trends(
    State(state): State<AppState>,
    Query(q): Query<TrendsQuery>,
) -> Result<Json<crate::storage::queries::TrendsData>, DashboardError> {
    Ok(Json(state.storage.trends(q.days.unwrap_or(7))?))
}

/// GET /api/stats/cost?group_by=project|model|kind
#[derive(Debug, Deserialize)]
pub struct CostQuery {
    pub group_by: Option<String>,
}

pub async fn stats_cost(
    State(state): State<AppState>,
    Query(q): Query<CostQuery>,
) -> Result<Json<Vec<crate::storage::queries::CostBucket>>, DashboardError> {
    let group_by = q.group_by.as_deref().unwrap_or("project");
    Ok(Json(state.storage.cost_breakdown(group_by)?))
}

/// GET /api/stats/ci
pub async fn stats_ci(
    State(state): State<AppState>,
) -> Result<Json<crate::storage::queries::CiStats>, DashboardError> {
    Ok(Json(state.storage.ci_stats()?))
}

/// GET /api/stats/sop
pub async fn stats_sop(
    State(state): State<AppState>,
) -> Result<Json<Vec<crate::storage::queries::SopDeviationRow>>, DashboardError> {
    Ok(Json(state.storage.sop_stats()?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::realtime::RealtimeHub;
    use crate::storage::queries::Storage;
    use std::sync::Arc;

    fn make_state() -> AppState {
        AppState {
            storage: Storage::open_in_memory().unwrap(),
            hub: Arc::new(RealtimeHub::new(100)),
            token: "secret".into(),
        }
    }

    #[tokio::test]
    async fn start_task_creates_running() {
        let state = make_state();
        let event = TaskStartedEvent {
            task_id: "api-t1".into(),
            project: "proj".into(),
            mr_iid: Some(1),
            pipeline_id: Some(2),
            task_description: "desc".into(),
            task_kind: "manual".into(),
            started_at: "2026-08-03T10:00:00Z".into(),
            model: "m".into(),
        };
        let _resp = start_task(State(state.clone()), JsonExtractor(event)).await.unwrap();
        assert!(state.storage.task_exists("api-t1").unwrap());
    }

    #[tokio::test]
    async fn batch_events_inserts_and_broadcasts() {
        let state = make_state();
        state.storage.start_task(&TaskStartedEvent {
            task_id: "api-t2".into(),
            project: "p".into(),
            mr_iid: None,
            pipeline_id: None,
            task_description: "d".into(),
            task_kind: "manual".into(),
            started_at: "2026-08-03T10:00:00Z".into(),
            model: "m".into(),
        }).unwrap();
        let req = BatchEventsRequest {
            task_id: "api-t2".into(),
            events: vec![devnpc_core::report::event_schema::ExecutionEvent::LlmCall {
                iteration: 1,
                prompt_tokens: 10,
                completion_tokens: 5,
                latency_ms: 100,
            }],
        };
        let _resp = batch_events(State(state.clone()), JsonExtractor(req)).await.unwrap();
        let events = state.storage.list_events("api-t2").unwrap();
        assert_eq!(events.len(), 1);
    }

    #[tokio::test]
    async fn finish_task_updates_status() {
        let state = make_state();
        state.storage.start_task(&TaskStartedEvent {
            task_id: "api-t3".into(),
            project: "p".into(),
            mr_iid: None,
            pipeline_id: None,
            task_description: "d".into(),
            task_kind: "manual".into(),
            started_at: "2026-08-03T10:00:00Z".into(),
            model: "m".into(),
        }).unwrap();
        let fin = TaskFinishedEvent {
            task_id: "api-t3".into(),
            status: devnpc_core::report::event_schema::TaskStatus::Success,
            duration_secs: 5,
            total_tokens: 100,
            estimated_cost_usd: 0.01,
            mr_url: None,
            ci_url: None,
            summary: "ok".into(),
            error: None,
            finished_at: "2026-08-03T10:01:00Z".into(),
        };
        let _ = finish_task(State(state.clone()), JsonExtractor(fin)).await.unwrap();
        let row = state.storage.get_task("api-t3").unwrap().unwrap();
        assert_eq!(row.status, "success");
    }

    // 注意: import_events 的完整 multipart 测试在 Task 11 的 server::mod 测试中
    // (需要通过完整 router 验证,因为 Multipart extractor 需要 HTTP 请求上下文)

    #[tokio::test]
    async fn list_tasks_api_returns_response() {
        let state = make_state();
        state.storage.start_task(&TaskStartedEvent {
            task_id: "api-list".into(),
            project: "p".into(),
            mr_iid: None,
            pipeline_id: None,
            task_description: "d".into(),
            task_kind: "manual".into(),
            started_at: "2026-08-03T10:00:00Z".into(),
            model: "m".into(),
        }).unwrap();
        let q = ListTasksQuery { page: Some(1), size: Some(10), status: None, project: None };
        let resp = list_tasks(State(state), Query(q)).await.unwrap();
        assert_eq!(resp.0.total, 1);
    }

    #[tokio::test]
    async fn get_task_api_returns_row() {
        let state = make_state();
        state.storage.start_task(&TaskStartedEvent {
            task_id: "api-get".into(),
            project: "p".into(),
            mr_iid: None,
            pipeline_id: None,
            task_description: "d".into(),
            task_kind: "manual".into(),
            started_at: "2026-08-03T10:00:00Z".into(),
            model: "m".into(),
        }).unwrap();
        let resp = get_task(State(state), Path("api-get".to_string())).await.unwrap();
        assert_eq!(resp.0.task_id, "api-get");
    }

    #[tokio::test]
    async fn get_task_api_missing_returns_404() {
        let state = make_state();
        let err = get_task(State(state), Path("nope".to_string())).await.unwrap_err();
        assert!(matches!(err, DashboardError::TaskNotFound(_)));
    }

    #[tokio::test]
    async fn list_task_events_api_returns_events() {
        let state = make_state();
        state.storage.start_task(&TaskStartedEvent {
            task_id: "api-ev".into(),
            project: "p".into(),
            mr_iid: None,
            pipeline_id: None,
            task_description: "d".into(),
            task_kind: "manual".into(),
            started_at: "2026-08-03T10:00:00Z".into(),
            model: "m".into(),
        }).unwrap();
        state.storage.insert_events("api-ev", &[
            devnpc_core::report::event_schema::ExecutionEvent::ToolCall {
                name: "git".into(),
                success: true,
                latency_ms: 10,
                detail: "commit".into(),
            },
        ]).unwrap();
        let resp = list_task_events(State(state), Path("api-ev".to_string())).await.unwrap();
        assert_eq!(resp.0.len(), 1);
    }

    #[tokio::test]
    async fn stats_trends_api_returns_data() {
        let state = make_state();
        let q = TrendsQuery { days: Some(7) };
        let resp = stats_trends(State(state), Query(q)).await.unwrap();
        assert_eq!(resp.0.days, 7);
    }

    #[tokio::test]
    async fn stats_cost_invalid_group_returns_400() {
        let state = make_state();
        let q = CostQuery { group_by: Some("invalid".into()) };
        let err = stats_cost(State(state), Query(q)).await.unwrap_err();
        assert!(matches!(err, DashboardError::ImportFormat(_)));
    }

    #[tokio::test]
    async fn stats_ci_api_returns() {
        let state = make_state();
        let resp = stats_ci(State(state)).await.unwrap();
        assert_eq!(resp.0.total_failed, 0);
    }

    #[tokio::test]
    async fn stats_sop_api_returns() {
        let state = make_state();
        let resp = stats_sop(State(state)).await.unwrap();
        assert!(resp.0.is_empty());
    }
}
