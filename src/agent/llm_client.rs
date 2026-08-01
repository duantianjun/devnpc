//! LLM 客户端封装 (P3 完整实现,reqwest 直连 OpenAI 兼容 API)
//!
//! 设计偏离说明: P3 用 reqwest 而非 rig-core。MVP 单 provider (DeepSeek OpenAI 兼容),
//! rig-core 抽象留 P8 模型路由。LlmClient 封装 provider 细节,P8 改造仅限本文件。

use reqwest::header::{AUTHORIZATION, CONTENT_TYPE};

use crate::agent::message::{LlmResponse, Message, ToolCall, ToolSchema};
use crate::config::LlmConfig;
use crate::error::{DevnpcError, Result};

/// LLM 客户端 (OpenAI 兼容)
pub struct LlmClient {
    config: LlmConfig,
    http: reqwest::Client,
}

impl LlmClient {
    pub fn new(config: LlmConfig) -> Self {
        Self {
            config,
            http: reqwest::Client::new(),
        }
    }

    /// 调用 LLM,返回文本 + 工具调用
    pub async fn complete(
        &self,
        messages: &[Message],
        tools: &[ToolSchema],
    ) -> Result<LlmResponse> {
        let url = format!(
            "{}/chat/completions",
            self.config.base_url.trim_end_matches('/')
        );

        // OpenAI 协议要求 tools 元素为 {"type":"function","function":{...}}
        let wrapped: Vec<ToolWrapper> =
            tools.iter().map(|t| ToolWrapper::from(t)).collect();
        let body = ChatCompletionsReq {
            model: &self.config.model,
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
        let resp = client.complete(&messages, &[]).await.unwrap();
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
            .complete(&[Message::user("read main")], &[])
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
        let result = client.complete(&[Message::user("hi")], &[]).await;
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
        let result = client.complete(&[Message::user("hi")], &[]).await;
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
        let _ = client.complete(&[Message::user("hi")], &tools).await.unwrap();
        // body_partial_json 匹配器已验证请求体含 tools
    }
}
