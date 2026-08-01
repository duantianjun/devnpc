//! Finish 工具: LLM 调用表示任务完成
//!
//! ReactLoop 检测到 tool name == "finish" 即终止循环并返回 Finished。
//! 本工具仅返回成功 + summary,不做副作用。

use async_trait::async_trait;

use crate::error::Result;
use crate::tools::{Tool, ToolResult};

pub struct FinishTool;

impl FinishTool {
    pub fn new() -> Self {
        Self
    }
}

impl Default for FinishTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for FinishTool {
    fn name(&self) -> &str {
        "finish"
    }
    fn description(&self) -> &str {
        "标记任务完成。当所有工作做完后调用,参数 summary 为验收摘要。"
    }
    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "summary": {"type": "string", "description": "任务完成摘要"}
            },
            "required": ["summary"]
        })
    }
    async fn call(&self, args: &serde_json::Value) -> Result<ToolResult> {
        let summary = args["summary"].as_str().unwrap_or("");
        Ok(ToolResult::ok(format!("FINISH:{summary}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn finish_returns_success_with_summary() {
        let tool = FinishTool::new();
        let result = tool
            .call(&serde_json::json!({"summary": "已修复登录 bug"}))
            .await
            .unwrap();
        assert!(result.success);
        assert!(result.output.contains("已修复登录 bug"));
    }

    #[tokio::test]
    async fn finish_handles_missing_summary() {
        let tool = FinishTool::new();
        let result = tool
            .call(&serde_json::json!({}))
            .await
            .unwrap();
        assert!(result.success);
    }

    #[test]
    fn finish_default_constructs() {
        let _tool = FinishTool;
    }
}
