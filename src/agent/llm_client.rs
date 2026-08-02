//! LLM 客户端封装 (P3 完整实现,reqwest 直连 OpenAI 兼容 API)
//!
//! 设计偏离说明: P3 用 reqwest 而非 rig-core。MVP 单 provider (DeepSeek OpenAI 兼容),
//! rig-core 抽象留 P8 模型路由。LlmClient 封装 provider 细节,P8 改造仅限本文件。
//!
//! P8: 新增 ModelRouter 按任务类型选择模型

use reqwest::header::{AUTHORIZATION, CONTENT_TYPE};

use crate::agent::message::{LlmResponse, Message, ToolCall, ToolSchema};
use crate::config::LlmConfig;
use crate::config::ModelRoutingConfig;
use crate::error::{DevnpcError, Result};

/// LLM 客户端 (OpenAI 兼容, P8 集成 ModelRouter)
pub struct LlmClient {
    config: LlmConfig,
    http: reqwest::Client,
    model_router: Option<ModelRouter>,
}

impl LlmClient {
    pub fn new(config: LlmConfig) -> Self {
        Self {
            config,
            http: reqwest::Client::new(),
            model_router: None,
        }
    }

    /// 设置模型路由器 (P8)
    pub fn with_model_router(mut self, router: ModelRouter) -> Self {
        self.model_router = Some(router);
        self
    }

    /// 调用 LLM,返回文本 + 工具调用
    ///
    /// 如果设置了 ModelRouter 且提供了 task_kind,会根据任务类型选择模型;
    /// 否则使用默认模型。
    pub async fn complete(
        &self,
        messages: &[Message],
        tools: &[ToolSchema],
        task_kind: Option<&TaskKind>,
    ) -> Result<LlmResponse> {
        let url = format!(
            "{}/chat/completions",
            self.config.base_url.trim_end_matches('/')
        );

        // 根据任务类型选择模型
        let model = match (task_kind, &self.model_router) {
            (Some(kind), Some(router)) => {
                router.select_model(kind).unwrap_or(&self.config.model)
            }
            _ => &self.config.model,
        };

        // OpenAI 协议要求 tools 元素为 {"type":"function","function":{...}}
        let wrapped: Vec<ToolWrapper> =
            tools.iter().map(ToolWrapper::from).collect();
        let body = ChatCompletionsReq {
            model,
            messages,
            tools: if wrapped.is_empty() { None } else { Some(&wrapped) },
            tool_choice: if tools.is_empty() { None } else { Some("auto") },
        };
        let body_json = serde_json::to_value(&body)
            .map_err(|e| DevnpcError::Llm(format!("序列化请求失败: {e}")))?;

        let resp = self
            .http
            .post(&url)
            .header(CONTENT_TYPE, "application/json")
            .header(AUTHORIZATION, format!("Bearer {}", self.config.api_key))
            .json(&body_json)
            .send()
            .await?;

        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(DevnpcError::Llm(format!(
                "LLM API 错误: {} {}",
                status.as_u16(),
                text
            )));
        }

        let resp_json: ChatCompletionsResp = resp.json().await?;
        let choice = resp_json
            .choices
            .into_iter()
            .next()
            .ok_or_else(|| DevnpcError::Llm("LLM 返回 choices 为空".into()))?;

        let text = choice.message.content.unwrap_or_default();
        let tool_calls = choice
            .message
            .tool_calls
            .unwrap_or_default()
            .into_iter()
            .map(|raw| {
                let args: serde_json::Value = serde_json::from_str(&raw.function.arguments)
                    .unwrap_or(serde_json::Value::Null);
                ToolCall {
                    id: raw.id,
                    name: raw.function.name,
                    arguments: args,
                }
            })
            .collect();

        Ok(LlmResponse { text, tool_calls })
    }
}

// === 请求/响应序列化结构 ===

#[derive(serde::Serialize)]
struct ChatCompletionsReq<'a> {
    model: &'a str,
    messages: &'a [Message],
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<&'a [ToolWrapper<'a>]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_choice: Option<&'a str>,
}

/// OpenAI tools 数组元素包装: {"type":"function","function":{...}}
#[derive(serde::Serialize)]
struct ToolWrapper<'a> {
    #[serde(rename = "type")]
    kind: &'a str,
    function: ToolWrapperFunction<'a>,
}

#[derive(serde::Serialize)]
struct ToolWrapperFunction<'a> {
    name: &'a str,
    description: &'a str,
    parameters: &'a serde_json::Value,
}

impl<'a> From<&'a ToolSchema> for ToolWrapper<'a> {
    fn from(t: &'a ToolSchema) -> Self {
        Self {
            kind: "function",
            function: ToolWrapperFunction {
                name: &t.name,
                description: &t.description,
                parameters: &t.parameters,
            },
        }
    }
}

#[derive(serde::Deserialize)]
struct ChatCompletionsResp {
    choices: Vec<ChatChoice>,
}

#[derive(serde::Deserialize)]
struct ChatChoice {
    message: ChatMessage,
}

#[derive(serde::Deserialize)]
struct ChatMessage {
    content: Option<String>,
    tool_calls: Option<Vec<RawToolCall>>,
}

#[derive(serde::Deserialize)]
struct RawToolCall {
    id: String,
    function: RawToolCallFunction,
}

#[derive(serde::Deserialize)]
struct RawToolCallFunction {
    name: String,
    /// OpenAI 协议: arguments 是 JSON 字符串
    arguments: String,
}

// === 模型路由 (P8) ===

/// 任务类型
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TaskKind {
    Fix,
    Test,
    Implement,
    Refactor,
    Unknown,
}

impl TaskKind {
    /// 从任务描述字符串推断任务类型
    pub fn from_goal(goal: &str) -> Self {
        let lower = goal.to_lowercase();
        if lower.contains("fix") || lower.contains("bug") || lower.contains("修复") || lower.contains("hotfix") {
            Self::Fix
        } else if lower.contains("test") || lower.contains("测试") || lower.contains("unit") || lower.contains("集成") {
            Self::Test
        } else if lower.contains("refactor") || lower.contains("重构") || lower.contains("优化") {
            Self::Refactor
        } else if lower.contains("implement") || lower.contains("实现") || lower.contains("feature") || lower.contains("add") || lower.contains("新增") {
            Self::Implement
        } else {
            Self::Unknown
        }
    }
}

/// 模型路由器 (P8)
///
/// 根据任务类型选择适合的模型:
/// - 简单任务 (Fix, Test) → 使用便宜模型
/// - 复杂任务 (Implement, Refactor, Unknown) → 使用强大模型
#[derive(Debug, Clone)]
pub struct ModelRouter {
    simple_model: String,
    complex_model: String,
}

impl ModelRouter {
    pub fn new(config: &ModelRoutingConfig) -> Self {
        Self {
            simple_model: config.simple_model.clone(),
            complex_model: config.complex_model.clone(),
        }
    }

    /// 根据任务类型选择模型名
    ///
    /// 如果路由配置为空 (simple_model/complex_model 未设置),
    /// 则返回 None,表示使用默认模型。
    pub fn select_model(&self, kind: &TaskKind) -> Option<&str> {
        if self.simple_model.is_empty() && self.complex_model.is_empty() {
            return None;
        }
        match kind {
            TaskKind::Fix | TaskKind::Test => {
                if self.simple_model.is_empty() {
                    Some(self.complex_model.as_str())
                } else {
                    Some(self.simple_model.as_str())
                }
            }
            TaskKind::Implement | TaskKind::Refactor | TaskKind::Unknown => {
                Some(self.complex_model.as_str())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::LlmConfig;
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn client_for(server: &MockServer) -> LlmClient {
        LlmClient::new(LlmConfig {
            api_key: "test-key".into(),
            base_url: server.uri(),
            model: "test-model".into(),
        })
    }

    #[tokio::test]
    async fn complete_returns_text_when_no_tool_calls() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .and(header("authorization", "Bearer test-key"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "choices": [
                    {
                        "message": {
                            "content": "任务已完成",
                            "tool_calls": null
                        }
                    }
                ]
            })))
            .mount(&server)
            .await;

        let client = client_for(&server);
        let messages = vec![Message::user("hi")];
        let resp = client.complete(&messages, &[], None).await.unwrap();
        assert_eq!(resp.text, "任务已完成");
        assert!(resp.tool_calls.is_empty());
    }

    #[tokio::test]
    async fn complete_parses_tool_calls_with_string_arguments() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "choices": [
                    {
                        "message": {
                            "content": null,
                            "tool_calls": [
                                {
                                    "id": "call_abc",
                                    "type": "function",
                                    "function": {
                                        "name": "read_file",
                                        "arguments": "{\"path\":\"src/main.rs\"}"
                                    }
                                }
                            ]
                        }
                    }
                ]
            })))
            .mount(&server)
            .await;

        let client = client_for(&server);
        let resp = client
            .complete(&[Message::user("read main")], &[], None)
            .await
            .unwrap();
        assert_eq!(resp.tool_calls.len(), 1);
        assert_eq!(resp.tool_calls[0].id, "call_abc");
        assert_eq!(resp.tool_calls[0].name, "read_file");
        assert_eq!(resp.tool_calls[0].arguments["path"], "src/main.rs");
    }

    #[tokio::test]
    async fn complete_returns_error_on_non_2xx() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(ResponseTemplate::new(401).set_body_string("unauthorized"))
            .mount(&server)
            .await;

        let client = client_for(&server);
        let result = client.complete(&[Message::user("hi")], &[], None).await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(matches!(err, DevnpcError::Llm(_)));
        assert!(err.to_string().contains("401"));
    }

    #[tokio::test]
    async fn complete_returns_error_when_choices_empty() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "choices": []
            })))
            .mount(&server)
            .await;

        let client = client_for(&server);
        let result = client.complete(&[Message::user("hi")], &[], None).await;
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), DevnpcError::Llm(_)));
    }

    #[tokio::test]
    async fn complete_sends_tools_in_request_body() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .and(wiremock::matchers::body_partial_json(serde_json::json!({
                "tools": [{"type": "function", "function": {"name": "read_file"}}]
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "choices": [{"message": {"content": "ok", "tool_calls": null}}]
            })))
            .mount(&server)
            .await;

        let client = client_for(&server);
        let tools = vec![ToolSchema {
            name: "read_file".into(),
            description: "read".into(),
            parameters: serde_json::json!({"type": "object"}),
        }];
        let _ = client.complete(&[Message::user("hi")], &tools, None).await.unwrap();
        // body_partial_json 匹配器已验证请求体含 tools
    }
}
