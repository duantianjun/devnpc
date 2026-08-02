//! 回调适配: SOP 偏离检测 + 执行轨迹记录
//!
//! 通过 adk-rust 的 before_tool_callback 机制:
//! - 在工具调用前检查 SOP 偏离 (软约束)
//! - 记录工具调用轨迹,供 report 模块消费

use std::sync::Arc;

use adk_rust::CallbackContext;

use crate::config::SopMode;

/// 默认允许的工具列表 (SOP 严格模式白名单)
const DEFAULT_ALLOWED_TOOLS: &[&str] = &[
    "read_file",
    "write_file",
    "edit_file",
    "delete_file",
    "list_files",
    "search_files",
    "grep_files",
    "run_command",
    "aft_outline",
    "aft_view_symbol",
    "aft_edit_symbol",
    "aft_search_symbols",
    "aft_ast_replace",
];

/// devnpc 回调处理器
///
/// 在 Agent 执行过程中插入自定义逻辑:
/// - before_tool_callback: SOP 偏离检测 + 轨迹记录
/// - after_model_callback: 响应日志记录
pub struct DevnpcCallbacks {
    sop_mode: SopMode,
    forbidden_paths: Vec<String>,
}

impl DevnpcCallbacks {
    /// 创建新的回调处理器
    pub fn new(sop_mode: SopMode, forbidden_paths: Vec<String>) -> Self {
        Self {
            sop_mode,
            forbidden_paths,
        }
    }

    /// 获取 before_tool_callback 闭包
    ///
    /// 在工具执行前调用,用于:
    /// 1. SOP 偏离检测 (软约束,仅记录警告)
    /// 2. 轨迹记录 (供 report 模块消费)
    pub fn before_tool_callback(
        &self,
    ) -> adk_rust::BeforeToolCallback {
        let sop_mode = self.sop_mode;
        let forbidden_paths = self.forbidden_paths.clone();
        Box::new(move |ctx: Arc<dyn CallbackContext>| {
            let fp = forbidden_paths.clone();
            Box::pin(async move {
                // SOP 偏离检测
                if let Some(tool_name) = ctx.tool_name() {
                    match sop_mode {
                        SopMode::Strict => {
                            if !DEFAULT_ALLOWED_TOOLS.contains(&tool_name) {
                                tracing::warn!(
                                    tool = %tool_name,
                                    "SOP 偏离警告: 使用未授权工具 (严格模式)"
                                );
                            }
                        }
                        SopMode::Soft => {
                            tracing::debug!(
                                tool = %tool_name,
                                "SOP 软约束: 工具调用已记录"
                            );
                        }
                    }

                    // 检查工具输入中是否包含禁止路径
                    if !fp.is_empty()
                        && let Some(input) = ctx.tool_input()
                        && let Some(path) = input.get("path").and_then(|v| v.as_str())
                        && fp.iter().any(|f| path.contains(f.as_str()))
                    {
                        tracing::warn!(
                            tool = %tool_name,
                            path = %path,
                            "SOP 偏离警告: 操作禁止路径"
                        );
                    }
                }
                Ok(None) // None = 继续执行
            })
        })
    }

    /// 获取 after_model_callback 闭包
    ///
    /// 在模型响应后调用,用于记录响应日志
    pub fn after_model_callback(
        &self,
    ) -> adk_rust::AfterModelCallback {
        Box::new(|_ctx: Arc<dyn CallbackContext>, response: adk_rust::LlmResponse| {
            Box::pin(async move {
                tracing::debug!(
                    finish_reason = ?response.finish_reason,
                    has_content = response.content.is_some(),
                    "模型响应完成"
                );
                Ok(None) // None = 保持原始响应
            })
        })
    }
}

impl Default for DevnpcCallbacks {
    fn default() -> Self {
        Self::new(SopMode::Soft, Vec::new())
    }
}