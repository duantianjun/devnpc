//! GitLab Webhook 服务器
//!
//! 接收 GitLab webhook 事件,解析 @devnpc 提及并触发任务执行。
//! 替代评论轮询模式,实现实时触发。
//!
//! ## 路由
//! - `POST {path}`: 接收 GitLab webhook (Note/MergeRequest/Issue 事件)
//! - `GET /healthz`: 健康检查
//!
//! ## 安全
//! - 通过 `X-Gitlab-Token` header 校验 webhook secret (若配置了 secret)
//! - 仅处理 Note 事件中的 @devnpc 提及,忽略其他事件

use std::net::SocketAddr;
use std::sync::Arc;

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::Json;
use serde::Deserialize;
use tokio::sync::mpsc;

use crate::config::WebhookConfig;
use crate::trigger::parser::{parse_mention, TaskSpec};

/// Webhook 服务器共享状态
#[derive(Clone)]
pub struct WebhookState {
    /// webhook secret (空则不校验)
    pub secret: Arc<String>,
    /// 触发事件发送端,接收端在 main 中消费并启动任务
    pub sender: mpsc::Sender<WebhookTrigger>,
}

/// 从 webhook 解析出的触发任务
#[derive(Debug, Clone)]
pub struct WebhookTrigger {
    /// 触发来源类型 (note_mr / note_issue)
    pub source: String,
    /// 任务规格 (复用 trigger/parser 的 TaskSpec)
    pub task: TaskSpec,
    /// MR 或 Issue 的 iid (用于回复评论)
    pub target_iid: u64,
    /// 项目 ID
    pub project_id: u64,
}

/// GitLab Note 事件 payload (仅提取关键字段)
#[derive(Debug, Deserialize)]
struct NoteEvent {
    /// 评论所属对象 (MergeRequest / Issue / Commit)
    object_attributes: NoteObject,
    /// 若是 MR 评论则存在
    merge_request: Option<MrRef>,
    /// 若是 Issue 评论则存在
    issue: Option<IssueRef>,
    project_id: u64,
}

#[derive(Debug, Deserialize)]
struct NoteObject {
    noteable_type: String,
    note: String,
}

#[derive(Debug, Deserialize)]
struct MrRef {
    iid: u64,
}

#[derive(Debug, Deserialize)]
struct IssueRef {
    iid: u64,
}

/// 启动 webhook 服务器
///
/// 返回 `(server_handle, receiver)`,其中 receiver 用于消费触发事件。
/// server_handle 可用于优雅关闭。
pub async fn start_server(
    config: &WebhookConfig,
) -> Result<(tokio::task::JoinHandle<()>, mpsc::Receiver<WebhookTrigger>), std::io::Error> {
    let (sender, receiver) = mpsc::channel::<WebhookTrigger>(32);
    let state = WebhookState {
        secret: Arc::new(config.secret.clone()),
        sender,
    };

    let webhook_path = if config.path.is_empty() {
        "/webhook".to_string()
    } else {
        config.path.clone()
    };

    let app = axum::Router::new()
        .route(&webhook_path, post(handle_webhook))
        .route("/healthz", get(health_check))
        .with_state(state);

    let addr: SocketAddr = format!("{}:{}", config.host, config.port)
        .parse()
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e))?;

    let listener = tokio::net::TcpListener::bind(addr).await?;
    let actual_addr = listener.local_addr()?;

    tracing::info!(
        host = %config.host,
        port = %actual_addr.port(),
        path = %webhook_path,
        secret_configured = !config.secret.is_empty(),
        "GitLab webhook 服务器已启动"
    );

    let handle = tokio::spawn(async move {
        if let Err(e) = axum::serve(listener, app).await {
            tracing::error!(error = %e, "webhook 服务器异常退出");
        }
    });

    Ok((handle, receiver))
}

/// POST /webhook — 接收 GitLab webhook
async fn handle_webhook(
    State(state): State<WebhookState>,
    headers: HeaderMap,
    Json(payload): Json<serde_json::Value>,
) -> Result<StatusCode, (StatusCode, String)> {
    // 1. 校验 X-Gitlab-Token (若配置了 secret)
    if !state.secret.is_empty() {
        let token = headers
            .get("X-Gitlab-Token")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        if token != state.secret.as_str() {
            tracing::warn!(
                "webhook secret 校验失败 (X-Gitlab-Token 不匹配)"
            );
            return Err((StatusCode::UNAUTHORIZED, "Invalid webhook token".into()));
        }
    }

    // 2. 解析事件类型
    let object_kind = payload
        .get("object_kind")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    match object_kind {
        "note" => handle_note_event(state, payload).await,
        // 其他事件类型 (push/merge_request/pipeline) 暂不处理
        kind => {
            tracing::debug!(object_kind = %kind, "webhook 事件类型暂不处理,忽略");
            Ok(StatusCode::OK)
        }
    }
}

/// 处理 Note 事件 (评论)
async fn handle_note_event(
    state: WebhookState,
    payload: serde_json::Value,
) -> Result<StatusCode, (StatusCode, String)> {
    let event: NoteEvent = serde_json::from_value(payload)
        .map_err(|e| {
            tracing::warn!(error = %e, "Note 事件反序列化失败");
            (StatusCode::BAD_REQUEST, format!("Invalid payload: {e}"))
        })?;

    // 仅处理 MR 和 Issue 评论
    let (source, target_iid) = match event.object_attributes.noteable_type.as_str() {
        "MergeRequest" => {
            let iid = event
                .merge_request
                .as_ref()
                .map(|mr| mr.iid)
                .unwrap_or(0);
            ("note_mr", iid)
        }
        "Issue" => {
            let iid = event.issue.as_ref().map(|i| i.iid).unwrap_or(0);
            ("note_issue", iid)
        }
        other => {
            tracing::debug!(noteable_type = %other, "非 MR/Issue 评论,忽略");
            return Ok(StatusCode::OK);
        }
    };

    // 解析 @devnpc 提及
    let body = &event.object_attributes.note;
    let task = match parse_mention(body) {
        Some(t) => t,
        None => {
            // 不含 @devnpc 提及,正常返回
            return Ok(StatusCode::OK);
        }
    };

    let trigger = WebhookTrigger {
        source: source.to_string(),
        task,
        target_iid,
        project_id: event.project_id,
    };

    tracing::info!(
        source = %trigger.source,
        target_iid = trigger.target_iid,
        project_id = trigger.project_id,
        task_kind = ?trigger.task.kind,
        "webhook 收到 @devnpc 提及,已触发任务"
    );

    // 发送到 channel (非阻塞,满了返回错误)
    match state.sender.try_send(trigger) {
        Ok(()) => Ok(StatusCode::OK),
        Err(mpsc::error::TrySendError::Full(_)) => {
            tracing::warn!("webhook 触发队列已满,拒绝新任务");
            Err((StatusCode::TOO_MANY_REQUESTS, "Trigger queue full".into()))
        }
        Err(mpsc::error::TrySendError::Closed(_)) => {
            tracing::error!("webhook 触发队列已关闭");
            Err((StatusCode::INTERNAL_SERVER_ERROR, "Trigger queue closed".into()))
        }
    }
}

/// GET /healthz — 健康检查
async fn health_check() -> impl IntoResponse {
    (StatusCode::OK, "ok")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::WebhookConfig;
    use tokio::sync::mpsc;

    fn make_state(secret: &str) -> (WebhookState, mpsc::Receiver<WebhookTrigger>) {
        let (sender, receiver) = mpsc::channel(32);
        (
            WebhookState {
                secret: Arc::new(secret.to_string()),
                sender,
            },
            receiver,
        )
    }

    fn make_note_payload(
        noteable_type: &str,
        body: &str,
        mr_iid: Option<u64>,
        issue_iid: Option<u64>,
        project_id: u64,
    ) -> serde_json::Value {
        let mut payload = serde_json::json!({
            "object_kind": "note",
            "object_attributes": {
                "noteable_type": noteable_type,
                "note": body,
            },
            "project_id": project_id,
        });
        if let Some(iid) = mr_iid {
            payload["merge_request"] = serde_json::json!({ "iid": iid });
        }
        if let Some(iid) = issue_iid {
            payload["issue"] = serde_json::json!({ "iid": iid });
        }
        payload
    }

    #[tokio::test]
    async fn handle_note_mr_with_mention_triggers_task() {
        let (state, mut rx) = make_state("");
        let payload = make_note_payload(
            "MergeRequest",
            "@devnpc 修复登录 bug #42",
            Some(15),
            None,
            100,
        );

        let result = handle_note_event(state, payload).await;
        assert!(result.is_ok(), "应成功处理: {:?}", result.err());
        assert_eq!(result.unwrap(), StatusCode::OK);

        let trigger = rx.recv().await.expect("应收到触发事件");
        assert_eq!(trigger.source, "note_mr");
        assert_eq!(trigger.target_iid, 15);
        assert_eq!(trigger.project_id, 100);
        assert_eq!(trigger.task.target_issue, Some(42));
    }

    #[tokio::test]
    async fn handle_note_issue_with_mention_triggers_task() {
        let (state, mut rx) = make_state("");
        let payload = make_note_payload(
            "Issue",
            "@devnpc 实现用户注册功能",
            None,
            Some(8),
            200,
        );

        let result = handle_note_event(state, payload).await;
        assert!(result.is_ok());
        let trigger = rx.recv().await.expect("应收到触发事件");
        assert_eq!(trigger.source, "note_issue");
        assert_eq!(trigger.target_iid, 8);
        assert_eq!(trigger.project_id, 200);
    }

    #[tokio::test]
    async fn handle_note_without_mention_returns_ok_no_trigger() {
        let (state, mut rx) = make_state("");
        let payload = make_note_payload(
            "MergeRequest",
            "看起来不错,approve",
            Some(1),
            None,
            1,
        );

        let result = handle_note_event(state, payload).await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), StatusCode::OK);
        assert!(rx.try_recv().is_err(), "不应产生触发事件");
    }

    #[tokio::test]
    async fn handle_note_commit_type_ignored() {
        let (state, mut rx) = make_state("");
        // Commit 评论不处理
        let payload = make_note_payload("Commit", "@devnpc test", None, None, 1);

        let result = handle_note_event(state, payload).await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), StatusCode::OK);
        assert!(rx.try_recv().is_err(), "Commit 评论不应触发");
    }

    #[tokio::test]
    async fn handle_note_invalid_payload_returns_bad_request() {
        let (state, _) = make_state("");
        // 缺少 object_attributes 字段
        let payload = serde_json::json!({
            "object_kind": "note",
            "project_id": 1,
        });

        let result = handle_note_event(state, payload).await;
        assert!(result.is_err());
        let (status, _) = result.unwrap_err();
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    #[test]
    fn webhook_config_default() {
        let config = WebhookConfig::default();
        assert!(!config.enabled);
        assert_eq!(config.port, 0);
        assert!(config.host.is_empty());
        assert!(config.secret.is_empty());
        assert!(config.path.is_empty());
    }

    #[test]
    fn webhook_trigger_carries_task_spec() {
        let task = TaskSpec {
            kind: crate::trigger::parser::TaskKind::Fix,
            description: "修复 bug".into(),
            target_issue: Some(42),
            acceptance_criteria: vec![],
        };
        let trigger = WebhookTrigger {
            source: "note_mr".into(),
            task,
            target_iid: 15,
            project_id: 100,
        };
        assert_eq!(trigger.source, "note_mr");
        assert_eq!(trigger.target_iid, 15);
        assert_eq!(trigger.project_id, 100);
        assert_eq!(trigger.task.target_issue, Some(42));
    }
}
