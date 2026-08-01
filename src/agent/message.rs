//! OpenAI 兼容 Chat Completions 消息类型
//!
//! 供 llm_client (序列化请求/反序列化响应) 与 loop_ (累积对话) 共用。

use serde::{Deserialize, Serialize};

/// 工具调用请求 (LLM 返回)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    /// OpenAI 返回的调用 id (喂回 tool 结果时关联)
    pub id: String,
    pub name: String,
    /// 工具参数 (已解析的 JSON Value)
    pub arguments: serde_json::Value,
}

/// 工具的 JSON Schema (告诉 LLM 可调用的工具)
#[derive(Debug, Clone, Serialize)]
pub struct ToolSchema {
    pub name: String,
    pub description: String,
    /// JSON Schema 对象,描述参数
    pub parameters: serde_json::Value,
}

/// LLM 单次响应
#[derive(Debug, Clone)]
pub struct LlmResponse {
    /// LLM 文本输出 (可能为空,当只返回 tool_calls 时)
    pub text: String,
    /// LLM 请求调用的工具 (空表示任务完成)
    pub tool_calls: Vec<ToolCall>,
}

/// 对话消息 (OpenAI Chat Completions 格式)
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "role", rename_all = "lowercase")]
pub enum Message {
    System {
        content: String,
    },
    User {
        content: String,
    },
    Assistant {
        /// 文本内容 (可能为空字符串)
        content: String,
        /// 本轮请求的工具调用 (序列化为 OpenAI tool_calls 格式)
        #[serde(skip_serializing_if = "Vec::is_empty")]
        tool_calls: Vec<AssistantToolCall>,
    },
    /// 工具执行结果喂回 LLM
    Tool {
        /// 关联的 tool_call id
        tool_call_id: String,
        content: String,
    },
}

/// OpenAI tool_calls 数组元素格式
#[derive(Debug, Clone, Serialize)]
pub struct AssistantToolCall {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub function: AssistantToolCallFunction,
}

#[derive(Debug, Clone, Serialize)]
pub struct AssistantToolCallFunction {
    pub name: String,
    /// OpenAI 要求 arguments 为 JSON 字符串
    pub arguments: String,
}

impl Message {
    pub fn system(content: impl Into<String>) -> Self {
        Self::System {
            content: content.into(),
        }
    }
    pub fn user(content: impl Into<String>) -> Self {
        Self::User {
            content: content.into(),
        }
    }
    pub fn assistant(text: impl Into<String>, tool_calls: &[ToolCall]) -> Self {
        let tc: Vec<AssistantToolCall> = tool_calls
            .iter()
            .map(|tc| AssistantToolCall {
                id: tc.id.clone(),
                kind: "function".into(),
                function: AssistantToolCallFunction {
                    name: tc.name.clone(),
                    arguments: tc.arguments.to_string(),
                },
            })
            .collect();
        Self::Assistant {
            content: text.into(),
            tool_calls: tc,
        }
    }
    pub fn tool(tool_call_id: impl Into<String>, content: impl Into<String>) -> Self {
        Self::Tool {
            tool_call_id: tool_call_id.into(),
            content: content.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_message_serializes_with_role_system() {
        let msg = Message::system("hello");
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains("\"role\":\"system\""));
        assert!(json.contains("\"content\":\"hello\""));
    }

    #[test]
    fn assistant_message_without_tool_calls_skips_field() {
        let msg = Message::assistant("hi", &[]);
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains("\"role\":\"assistant\""));
        // tool_calls 为空时应被 skip
        assert!(!json.contains("tool_calls"));
    }

    #[test]
    fn assistant_message_with_tool_calls_serializes_function_array() {
        let tc = ToolCall {
            id: "call_1".into(),
            name: "read_file".into(),
            arguments: serde_json::json!({"path": "src/main.rs"}),
        };
        let msg = Message::assistant("", &[tc]);
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains("\"tool_calls\""));
        assert!(json.contains("\"id\":\"call_1\""));
        assert!(json.contains("\"name\":\"read_file\""));
        // arguments 必须是字符串
        assert!(json.contains("\"arguments\":\"{\\\"path\\\":\\\"src/main.rs\\\"}\""));
    }

    #[test]
    fn tool_message_serializes_tool_call_id() {
        let msg = Message::tool("call_1", "fn main() {}");
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains("\"role\":\"tool\""));
        assert!(json.contains("\"tool_call_id\":\"call_1\""));
    }
}
