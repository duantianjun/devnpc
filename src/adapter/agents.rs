//! 子 Agent 构建: 为 Orchestrator 提供 Code/Fix/Review Agent
//!
//! 每个子 Agent 通过 LlmAgentBuilder 构建，拥有专属的 System Prompt 和工具集。

use std::sync::Arc;

use adk_rust::agent::LlmAgentBuilder;
use adk_rust::Tool;

use crate::error::Result;

/// 构建 Code Agent - 代码读写、AST 操作、编译验证
pub fn build_code_agent(
    tools: Vec<Arc<dyn Tool>>,
    model: Arc<dyn adk_rust::Llm>,
) -> Result<adk_rust::agent::LlmAgent> {
    let builder = LlmAgentBuilder::new("code_agent")
        .instruction(
            "你是一个代码修改专家。\n\
            原则:\n\
            1. 修改前先理解上下文 (read_file / list_files / aft_outline)\n\
            2. 改完后用对应的构建工具验证编译 (如 cargo build / mvn compile)\n\
            3. 禁止修改工作目录外的文件\n\
            4. 总结修改内容",
        )
        .model(model);
    // 逐个添加工具
    let builder = tools.into_iter().fold(builder, |b, tool| b.tool(tool));
    builder.build().map_err(|e| {
        crate::error::DevnpcError::Config(format!("Code Agent 构建失败: {e}"))
    })
}

/// 构建 Fix Agent - CI 日志分析、根因定位、修复代码
pub fn build_fix_agent(
    tools: Vec<Arc<dyn Tool>>,
    model: Arc<dyn adk_rust::Llm>,
) -> Result<adk_rust::agent::LlmAgent> {
    let builder = LlmAgentBuilder::new("fix_agent")
        .instruction(
            "你是一个 CI 修复专家。\n\
            任务: 分析 CI 失败日志 → 定位根因 → 修复代码 → 验证语法\n\
            原则:\n\
            1. 先读取失败日志和相关源码\n\
            2. 定位根因后再修改\n\
            3. 修复后验证语法正确性\n\
            4. 总结修复内容",
        )
        .model(model);
    let builder = tools.into_iter().fold(builder, |b, tool| b.tool(tool));
    builder.build().map_err(|e| {
        crate::error::DevnpcError::Config(format!("Fix Agent 构建失败: {e}"))
    })
}

/// 构建 Review Agent - 代码审查、SOP 合规检查
pub fn build_review_agent(
    tools: Vec<Arc<dyn Tool>>,
    model: Arc<dyn adk_rust::Llm>,
) -> Result<adk_rust::agent::LlmAgent> {
    let builder = LlmAgentBuilder::new("review_agent")
        .instruction(
            "你是一个代码审查专家。\n\
            任务: 审查代码变更 → 检查 SOP 合规 → 输出审查报告\n\
            原则:\n\
            1. 检查代码质量、安全性、性能\n\
            2. 检查是否符合项目规范\n\
            3. 输出明确的通过/不通过结论",
        )
        .model(model);
    let builder = tools.into_iter().fold(builder, |b, tool| b.tool(tool));
    builder.build().map_err(|e| {
        crate::error::DevnpcError::Config(format!("Review Agent 构建失败: {e}"))
    })
}