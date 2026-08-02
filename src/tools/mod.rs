//! Agent 工具集 (唯一副作用出口)
//!
//! P3 实现 8 个自建工具:
//! read_file, write_file, list_files, git_diff (自建文件/git),
//! run_command (shell), git_commit (git), create_mr_note (gitlab), finish。
//! P3.5 实现 5 个 AFT 工具:
//! aft_outline, aft_view_symbol, aft_edit_symbol, aft_search_symbols, aft_ast_replace。

pub mod aft;
pub mod file_io;
pub mod finish;
pub mod git_tool;
pub mod gitlab_tool;
pub mod shell;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::agent::message::ToolSchema;
use crate::error::{DevnpcError, Result};

/// 工具调用结果
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResult {
    pub success: bool,
    pub output: String,
}

impl ToolResult {
    pub fn ok(output: impl Into<String>) -> Self {
        Self {
            success: true,
            output: output.into(),
        }
    }
    pub fn err(output: impl Into<String>) -> Self {
        Self {
            success: false,
            output: output.into(),
        }
    }
}

/// 工具 trait
#[async_trait]
pub trait Tool: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    /// JSON Schema 描述参数 (OpenAI function calling 格式)
    fn parameters_schema(&self) -> serde_json::Value;
    async fn call(&self, arguments: &serde_json::Value) -> Result<ToolResult>;
}

/// 工具注册表
pub struct ToolRegistry {
    tools: Vec<Box<dyn Tool>>,
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self { tools: Vec::new() }
    }

    pub fn register(&mut self, tool: Box<dyn Tool>) {
        self.tools.push(tool);
    }

    /// 导出所有工具的 schema (供 LLM 知道可调用哪些工具)
    pub fn schemas(&self) -> Vec<ToolSchema> {
        self.tools
            .iter()
            .map(|t| ToolSchema {
                name: t.name().to_string(),
                description: t.description().to_string(),
                parameters: t.parameters_schema(),
            })
            .collect()
    }

    /// 按名查找工具并执行;未注册返回 Tool 错误
    pub async fn call(&self, name: &str, arguments: &serde_json::Value) -> Result<ToolResult> {
        let tool = self
            .tools
            .iter()
            .find(|t| t.name() == name)
            .ok_or_else(|| DevnpcError::Tool {
                tool: name.into(),
                msg: "工具未注册".into(),
            })?;
        tool.call(arguments).await
    }
}

impl Default for ToolRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct EchoTool;

    #[async_trait]
    impl Tool for EchoTool {
        fn name(&self) -> &str {
            "echo"
        }
        fn description(&self) -> &str {
            "echo back"
        }
        fn parameters_schema(&self) -> serde_json::Value {
            serde_json::json!({
                "type": "object",
                "properties": {"msg": {"type": "string"}},
                "required": ["msg"]
            })
        }
        async fn call(&self, args: &serde_json::Value) -> Result<ToolResult> {
            let msg = args["msg"].as_str().unwrap_or("");
            Ok(ToolResult::ok(msg.to_string()))
        }
    }

    #[tokio::test]
    async fn schemas_returns_all_registered_tools() {
        let mut reg = ToolRegistry::new();
        reg.register(Box::new(EchoTool));
        let schemas = reg.schemas();
        assert_eq!(schemas.len(), 1);
        assert_eq!(schemas[0].name, "echo");
        assert_eq!(schemas[0].parameters["type"], "object");
    }

    #[tokio::test]
    async fn call_dispatches_to_registered_tool() {
        let mut reg = ToolRegistry::new();
        reg.register(Box::new(EchoTool));
        let result = reg
            .call("echo", &serde_json::json!({"msg": "hello"}))
            .await
            .unwrap();
        assert!(result.success);
        assert_eq!(result.output, "hello");
    }

    #[tokio::test]
    async fn call_returns_error_for_unknown_tool() {
        let reg = ToolRegistry::new();
        let result = reg.call("nonexistent", &serde_json::Value::Null).await;
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), DevnpcError::Tool { .. }));
    }

    #[test]
    fn tool_result_ok_and_err_constructors() {
        let ok = ToolResult::ok("done");
        assert!(ok.success);
        assert_eq!(ok.output, "done");
        let err = ToolResult::err("fail");
        assert!(!err.success);
        assert_eq!(err.output, "fail");
    }
}
