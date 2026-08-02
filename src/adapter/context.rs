//! 上下文适配: 将业务 Context 注入 adk-rust Session
//!
//! 将 memory::context::Context 中的研发记忆数据注入到 Session 的初始状态中,
//! 使 LlmAgent 在执行时可以访问项目上下文。

use adk_rust::session::InMemorySessionService;
use adk_rust::session::SessionService;
use std::collections::HashMap;
use std::sync::Arc;

use crate::memory::context::Context;

/// 从业务 Context 构建 Session 初始状态
///
/// 将 Context 中的关键数据序列化为 HashMap,作为 CreateRequest.state 传入。
pub fn build_initial_state(ctx: &Context) -> HashMap<String, adk_rust::serde_json::Value> {
    let mut state = HashMap::new();

    // 注入仓库树摘要
    if !ctx.repo_tree.entries.is_empty() {
        state.insert(
            "repo_tree".to_string(),
            adk_rust::serde_json::json!(&ctx.repo_tree),
        );
    }

    // 注入 Issue 信息
    state.insert(
        "issue".to_string(),
        adk_rust::serde_json::json!(&ctx.issue),
    );

    // 注入 CI 失败历史
    if !ctx.ci_failures.is_empty() {
        state.insert(
            "ci_failures".to_string(),
            adk_rust::serde_json::json!(&ctx.ci_failures),
        );
    }

    // 注入项目配置
    state.insert(
        "project_config".to_string(),
        adk_rust::serde_json::json!(&ctx.project_config),
    );

    tracing::debug!(
        has_repo_tree = !ctx.repo_tree.entries.is_empty(),
        has_ci_failures = !ctx.ci_failures.is_empty(),
        "业务上下文已构建为 Session 初始状态"
    );

    state
}

/// 创建 SessionService 和 session_id
///
/// 创建 InMemorySessionService,返回 session_service 和 session_id。
/// 调用方应使用 build_initial_state 构建初始状态并传入 CreateRequest。
pub fn create_session_service() -> (Arc<dyn SessionService>, String) {
    let session_service: Arc<dyn SessionService> = Arc::new(InMemorySessionService::new());
    let session_id = format!("devnpc-{}", uuid::Uuid::new_v4());
    (session_service, session_id)
}