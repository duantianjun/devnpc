//! 回调适配: SOP 偏离检测 + 执行轨迹记录
//!
//! 通过 adk-rust 的 before_tool_callback 机制:
//! - 在工具调用前检查 SOP 偏离 (软约束)
//! - 记录工具调用轨迹,供 report 模块消费

use std::sync::Arc;

use adk_rust::CallbackContext;

/// devnpc 回调处理器
///
/// 在 Agent 执行过程中插入自定义逻辑:
/// - before_tool_callback: SOP 偏离检测 + 轨迹记录
/// - after_model_callback: 响应日志记录
pub struct DevnpcCallbacks;

impl DevnpcCallbacks {
    /// 创建新的回调处理器
    pub fn new() -> Self {
        Self
    }

    /// 获取 before_tool_callback 闭包
    ///
    /// 在工具执行前调用,用于:
    /// 1. SOP 偏离检测 (软约束,仅记录警告)
    /// 2. 轨迹记录 (供 report 模块消费)
    pub fn before_tool_callback(
        &self,
    ) -> adk_rust::BeforeToolCallback {
        Box::new(|_ctx: Arc<dyn CallbackContext>| {
            Box::pin(async move {
                // TODO: 阶段 C 完成后接入 SOP 偏离检测逻辑
                // 目前仅记录工具调用,不拦截
                tracing::debug!("工具即将执行");
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
        Self::new()
    }
}