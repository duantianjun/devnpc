//! Orchestrator Agent: 任务拆解、分发、结果汇总
//!
//! 将子 Agent 调用封装为 FunctionTool，通过 Orchestrator Agent 统一调度。
//! 子 Agent 不直接相互调用，通过 Orchestrator 传递中间结果，保持解耦。

use std::sync::Arc;

use adk_rust::agent::LlmAgent;
use adk_rust::runner::Runner;
use adk_rust::session::{CreateRequest, InMemorySessionService, SessionService};
use adk_rust::{Content, SessionId, UserId, Llm};
use futures::StreamExt;

use crate::adapter::memory::MemoryStore;
use crate::error::Result;

/// Orchestrator: 负责任务编排
pub struct Orchestrator {
    /// 主 Agent (Orchestrator 自身)
    pub agent: Arc<LlmAgent>,
    /// 子 Agent
    pub code_agent: Option<Arc<LlmAgent>>,
    pub fix_agent: Option<Arc<LlmAgent>>,
    pub review_agent: Option<Arc<LlmAgent>>,
    /// 简单模型 (小模型，用于阅读/搜索)
    pub simple_model: Option<Arc<dyn Llm>>,
    /// 复杂模型 (大模型，用于改码/修复/推理)
    pub complex_model: Option<Arc<dyn Llm>>,
    /// 长期记忆存储器
    pub memory_store: Option<MemoryStore>,
}

impl Orchestrator {
    pub fn new(
        agent: Arc<LlmAgent>,
        code_agent: Option<Arc<LlmAgent>>,
        fix_agent: Option<Arc<LlmAgent>>,
        review_agent: Option<Arc<LlmAgent>>,
        simple_model: Option<Arc<dyn Llm>>,
        complex_model: Option<Arc<dyn Llm>>,
        memory_store: Option<MemoryStore>,
    ) -> Self {
        Self {
            agent,
            code_agent,
            fix_agent,
            review_agent,
            simple_model,
            complex_model,
            memory_store,
        }
    }

    /// 运行主 Agent 执行任务
    pub async fn run(
        &self,
        user_input: &str,
        session_service: Arc<dyn SessionService>,
        session_id: &str,
        initial_state: std::collections::HashMap<String, adk_rust::serde_json::Value>,
    ) -> Result<String> {
        let session_id_typed = SessionId::try_from(session_id).map_err(|e| {
            crate::error::DevnpcError::Config(format!("SessionId 创建失败: {e}"))
        })?;

        session_service
            .create(CreateRequest {
                app_name: "devnpc".to_string(),
                user_id: "devnpc".to_string(),
                session_id: Some(session_id_typed.to_string()),
                state: initial_state,
            })
            .await
            .map_err(|e| crate::error::DevnpcError::Config(format!("会话创建失败: {e}")))?;

        let runner = Runner::builder()
            .app_name("devnpc")
            .agent(self.agent.clone())
            .session_service(session_service)
            .build()
            .map_err(|e| crate::error::DevnpcError::Config(format!("Runner 构建失败: {e}")))?;

        let content = Content::new("user").with_text(user_input);
        let user_id = UserId::new("devnpc").map_err(|e| {
            crate::error::DevnpcError::Config(format!("UserId 创建失败: {e}"))
        })?;

        let mut stream = runner
            .run(user_id, session_id_typed, content)
            .await
            .map_err(|e| crate::error::DevnpcError::Config(format!("Agent 执行失败: {e}")))?;

        let mut final_text = String::new();
        while let Some(event_result) = stream.next().await {
            if let Ok(event) = event_result
                && event.is_final_response()
                && let Some(content) = &event.llm_response.content
            {
                for part in &content.parts {
                    if let Some(text) = part.text() {
                        final_text.push_str(text);
                    }
                }
            }
        }

        Ok(final_text)
    }

    /// 运行 Fix Agent 执行 CI 修复
    pub async fn run_fix_agent(
        &self,
        instruction: &str,
    ) -> Result<String> {
        let fix_agent = self.fix_agent.as_ref().ok_or_else(|| {
            crate::error::DevnpcError::Config("Fix Agent 未配置".to_string())
        })?;

        let session_service: Arc<dyn SessionService> = Arc::new(InMemorySessionService::new());
        let session_id = format!("fix-{}", uuid::Uuid::new_v4());
        let session_id_typed = SessionId::try_from(session_id.as_str()).map_err(|e| {
            crate::error::DevnpcError::Config(format!("SessionId 创建失败: {e}"))
        })?;

        session_service
            .create(CreateRequest {
                app_name: "devnpc-fix".to_string(),
                user_id: "devnpc".to_string(),
                session_id: Some(session_id_typed.to_string()),
                state: std::collections::HashMap::new(),
            })
            .await
            .map_err(|e| crate::error::DevnpcError::Config(format!("会话创建失败: {e}")))?;

        let runner = Runner::builder()
            .app_name("devnpc-fix")
            .agent(fix_agent.clone())
            .session_service(session_service)
            .build()
            .map_err(|e| crate::error::DevnpcError::Config(format!("Fix Runner 构建失败: {e}")))?;

        let content = Content::new("user").with_text(instruction);
        let user_id = UserId::new("devnpc").map_err(|e| {
            crate::error::DevnpcError::Config(format!("UserId 创建失败: {e}"))
        })?;

        let mut stream = runner
            .run(user_id, session_id_typed, content)
            .await
            .map_err(|e| crate::error::DevnpcError::Config(format!("Fix Agent 执行失败: {e}")))?;

        let mut result = String::new();
        while let Some(event_result) = stream.next().await {
            if let Ok(event) = event_result
                && event.is_final_response()
                && let Some(content) = &event.llm_response.content
            {
                for part in &content.parts {
                    if let Some(text) = part.text() {
                        result.push_str(text);
                    }
                }
            }
        }

        Ok(result)
    }

    /// 运行主 Agent 执行任务 (带记忆注入)
    pub async fn run_with_memory(
        &self,
        user_input: &str,
        session_service: Arc<dyn SessionService>,
        session_id: &str,
        initial_state: std::collections::HashMap<String, adk_rust::serde_json::Value>,
    ) -> Result<String> {
        // 检索相关记忆并注入
        if let Some(ref store) = self.memory_store
            && let Ok(history) = store.retrieve_relevant(user_input)
            && !history.is_empty()
        {
            tracing::info!(count = history.len(), "注入历史记忆到 Agent 上下文");
            let enriched_input = format!(
                "{}\n\n## 历史相关记忆\n{}",
                user_input,
                history.join("\n---\n")
            );
            return self.run(&enriched_input, session_service, session_id, initial_state).await;
        }
        self.run(user_input, session_service, session_id, initial_state).await
    }
}