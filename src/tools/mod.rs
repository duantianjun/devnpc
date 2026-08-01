//! Agent 工具集 (唯一副作用出口)
//!
//! P3 完整实现 13 个工具:
//! view_symbol, edit_symbol, ast_replace, outline, search_symbols (AFT),
//! read_file, write_file, list_files, git_diff (自建文件工具),
//! run_command (shell), git_commit (git), create_mr_note (gitlab), finish。

pub mod file_io;
pub mod git_tool;
pub mod gitlab_tool;
pub mod shell;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::error::Result;

/// 工具调用请求
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub name: String,
    pub arguments: serde_json::Value,
}

/// 工具调用结果
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResult {
    pub success: bool,
    pub output: String,
}

/// 工具 trait (P3 完整实现)
#[async_trait]
pub trait Tool: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    async fn call(&self, arguments: &serde_json::Value) -> Result<ToolResult>;
}

/// 工具注册表 (P3 完整实现)
pub struct ToolRegistry {
    #[allow(dead_code)]
    tools: Vec<Box<dyn Tool>>,
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self { tools: Vec::new() }
    }

    pub fn register(&mut self, tool: Box<dyn Tool>) {
        self.tools.push(tool);
    }
}

impl Default for ToolRegistry {
    fn default() -> Self {
        Self::new()
    }
}
