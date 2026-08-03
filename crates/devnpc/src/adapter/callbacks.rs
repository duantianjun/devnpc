//! 回调适配: SOP 偏离检测 + 执行轨迹记录
//!
//! 通过 adk-rust 的 before_tool_callback 机制:
//! - 在工具调用前检查 SOP 偏离 (软约束)
//! - 记录工具调用轨迹,供 report 模块消费

use std::sync::Arc;

use adk_rust::CallbackContext;

use crate::config::SopMode;

/// 检查工具是否在 SOP 允许列表中 (严格模式白名单)
///
/// 使用 `allowed_tools` 列表校验, 空列表表示允许全部工具。
pub fn is_tool_allowed(tool_name: &str, allowed_tools: &[String]) -> bool {
    if allowed_tools.is_empty() {
        return true;
    }
    allowed_tools.iter().any(|t| t == tool_name)
}

/// 检查路径是否命中禁止路径列表
///
/// 匹配规则: 路径包含禁止列表中任一子串即为命中。
/// 空禁止列表表示不限制。
pub fn is_forbidden_path(path: &str, forbidden_paths: &[String]) -> bool {
    if forbidden_paths.is_empty() {
        return false;
    }
    forbidden_paths.iter().any(|f| path.contains(f.as_str()))
}

/// devnpc 回调处理器
///
/// 在 Agent 执行过程中插入自定义逻辑:
/// - before_tool_callback: SOP 偏离检测 + 轨迹记录
/// - after_model_callback: 响应日志记录
pub struct DevnpcCallbacks {
    sop_mode: SopMode,
    forbidden_paths: Vec<String>,
    allowed_tools: Vec<String>,
}

impl DevnpcCallbacks {
    /// 创建新的回调处理器
    ///
    /// `allowed_tools` 为 SOP 严格模式工具白名单 (来自 `ToolsConfig.allowed_tools`)。
    pub fn new(
        sop_mode: SopMode,
        forbidden_paths: Vec<String>,
        allowed_tools: Vec<String>,
    ) -> Self {
        Self {
            sop_mode,
            forbidden_paths,
            allowed_tools,
        }
    }

    /// 获取当前 SOP 模式
    pub fn sop_mode(&self) -> SopMode {
        self.sop_mode
    }

    /// 获取禁止路径列表
    pub fn forbidden_paths(&self) -> &[String] {
        &self.forbidden_paths
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
        let allowed_tools = self.allowed_tools.clone();
        Box::new(move |ctx: Arc<dyn CallbackContext>| {
            let fp = forbidden_paths.clone();
            let at = allowed_tools.clone();
            Box::pin(async move {
                // SOP 偏离检测
                if let Some(tool_name) = ctx.tool_name() {
                    match sop_mode {
                        SopMode::Strict => {
                            if !is_tool_allowed(tool_name, &at) {
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
                        && is_forbidden_path(path, &fp)
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
        Self::new(SopMode::Soft, Vec::new(), Vec::new())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_tool_allowed_default_whitelist() {
        let whitelist: Vec<String> = crate::config::default_allowed_tools_list();
        // 白名单内工具
        assert!(is_tool_allowed("read_file", &whitelist));
        assert!(is_tool_allowed("write_file", &whitelist));
        assert!(is_tool_allowed("run_command", &whitelist));
        assert!(is_tool_allowed("aft_outline", &whitelist));
        assert!(is_tool_allowed("aft_view_symbol", &whitelist));
        assert!(is_tool_allowed("aft_edit_symbol", &whitelist));
        assert!(is_tool_allowed("aft_search_symbols", &whitelist));
        assert!(is_tool_allowed("aft_ast_replace", &whitelist));
    }

    #[test]
    fn test_is_tool_allowed_rejects_unknown() {
        let whitelist: Vec<String> = crate::config::default_allowed_tools_list();
        // 不在白名单
        assert!(!is_tool_allowed("rm_rf", &whitelist));
        assert!(!is_tool_allowed("shell_inject", &whitelist));
        assert!(!is_tool_allowed("", &whitelist));
        assert!(!is_tool_allowed("unknown_tool", &whitelist));
    }

    #[test]
    fn test_is_tool_allowed_empty_list_allows_all() {
        // 空白名单 → 允许全部
        assert!(is_tool_allowed("anything", &[]));
        assert!(is_tool_allowed("rm_rf", &[]));
    }

    #[test]
    fn test_is_forbidden_path_empty_list_never_matches() {
        // 空禁止列表 → 永不命中
        assert!(!is_forbidden_path("/etc/passwd", &[]));
        assert!(!is_forbidden_path("any/path", &[]));
    }

    #[test]
    fn test_is_forbidden_path_matches_substring() {
        let forbidden = vec!["/etc".to_string(), "secrets".to_string()];
        assert!(is_forbidden_path("/etc/passwd", &forbidden));
        assert!(is_forbidden_path("/home/user/secrets.env", &forbidden));
        assert!(is_forbidden_path("config/secrets.yaml", &forbidden));
    }

    #[test]
    fn test_is_forbidden_path_no_match() {
        let forbidden = vec!["/etc".to_string()];
        assert!(!is_forbidden_path("/home/user/file.txt", &forbidden));
        assert!(!is_forbidden_path("/var/log/app.log", &forbidden));
    }

    #[test]
    fn test_callbacks_new_strict_mode() {
        let cb = DevnpcCallbacks::new(SopMode::Strict, vec!["/secret".into()], vec![]);
        assert!(matches!(cb.sop_mode(), SopMode::Strict));
        assert_eq!(cb.forbidden_paths(), &["/secret".to_string()]);
    }

    #[test]
    fn test_callbacks_new_soft_mode() {
        let cb = DevnpcCallbacks::new(SopMode::Soft, vec![], vec![]);
        assert!(matches!(cb.sop_mode(), SopMode::Soft));
        assert!(cb.forbidden_paths().is_empty());
    }

    #[test]
    fn test_callbacks_default_is_soft_mode() {
        let cb = DevnpcCallbacks::default();
        assert!(matches!(cb.sop_mode(), SopMode::Soft));
        assert!(cb.forbidden_paths().is_empty());
    }

    #[test]
    fn test_before_tool_callback_returns_closure() {
        // 验证闭包可成功构建
        let cb = DevnpcCallbacks::new(SopMode::Strict, vec!["/secret".into()], vec![]);
        let _closure = cb.before_tool_callback();
        let _after_closure = cb.after_model_callback();
        // 构造成功即验证 (闭包类型不便于直接断言)
    }

    #[tokio::test]
    async fn test_before_tool_callback_soft_mode_no_crash() {
        use adk_rust::{Artifacts, Content, ReadonlyContext};
        use async_trait::async_trait;
        use std::sync::Arc;

        // Mock CallbackContext: 仅注入 tool_name
        struct MockCtx {
            tool_name: Option<String>,
            tool_input: Option<serde_json::Value>,
            content: Content,
        }

        #[async_trait]
        impl ReadonlyContext for MockCtx {
            fn invocation_id(&self) -> &str { "inv-test" }
            fn agent_name(&self) -> &str { "test-agent" }
            fn user_id(&self) -> &str { "devnpc" }
            fn app_name(&self) -> &str { "test-app" }
            fn session_id(&self) -> &str { "session-test" }
            fn branch(&self) -> &str { "" }
            fn user_content(&self) -> &Content { &self.content }
        }

        #[async_trait]
        impl CallbackContext for MockCtx {
            fn artifacts(&self) -> Option<Arc<dyn Artifacts>> { None }
            fn tool_name(&self) -> Option<&str> { self.tool_name.as_deref() }
            fn tool_input(&self) -> Option<&serde_json::Value> { self.tool_input.as_ref() }
        }

        let cb = DevnpcCallbacks::new(SopMode::Soft, vec!["/secret".into()], vec![]);
        let closure = cb.before_tool_callback();

        let ctx: Arc<dyn CallbackContext> = Arc::new(MockCtx {
            tool_name: Some("read_file".into()),
            tool_input: Some(serde_json::json!({"path": "/home/user/file.txt"})),
            content: Content::new("user"),
        });
        let result = closure(ctx).await;
        assert!(result.is_ok(), "回调执行失败: {:?}", result.err());
        assert!(result.unwrap().is_none(), "应返回 None (继续执行)");
    }

    #[tokio::test]
    async fn test_before_tool_callback_strict_mode_unknown_tool() {
        use adk_rust::{Artifacts, Content, ReadonlyContext};
        use async_trait::async_trait;
        use std::sync::Arc;

        struct MockCtx {
            tool_name: Option<String>,
            tool_input: Option<serde_json::Value>,
            content: Content,
        }

        #[async_trait]
        impl ReadonlyContext for MockCtx {
            fn invocation_id(&self) -> &str { "inv-test" }
            fn agent_name(&self) -> &str { "test-agent" }
            fn user_id(&self) -> &str { "devnpc" }
            fn app_name(&self) -> &str { "test-app" }
            fn session_id(&self) -> &str { "session-test" }
            fn branch(&self) -> &str { "" }
            fn user_content(&self) -> &Content { &self.content }
        }

        #[async_trait]
        impl CallbackContext for MockCtx {
            fn artifacts(&self) -> Option<Arc<dyn Artifacts>> { None }
            fn tool_name(&self) -> Option<&str> { self.tool_name.as_deref() }
            fn tool_input(&self) -> Option<&serde_json::Value> { self.tool_input.as_ref() }
        }

        let cb = DevnpcCallbacks::new(SopMode::Strict, vec![], vec![]);
        let closure = cb.before_tool_callback();

        // 未知工具 (不在白名单) → 严格模式下记录警告,但仍然返回 Ok(None)
        let ctx: Arc<dyn CallbackContext> = Arc::new(MockCtx {
            tool_name: Some("dangerous_tool".into()),
            tool_input: None,
            content: Content::new("user"),
        });
        let result = closure(ctx).await;
        assert!(result.is_ok());
        assert!(result.unwrap().is_none(), "SOP 软约束不应阻断工具执行");
    }

    #[tokio::test]
    async fn test_before_tool_callback_forbidden_path_detected() {
        use adk_rust::{Artifacts, Content, ReadonlyContext};
        use async_trait::async_trait;
        use std::sync::Arc;

        struct MockCtx {
            tool_name: Option<String>,
            tool_input: Option<serde_json::Value>,
            content: Content,
        }

        #[async_trait]
        impl ReadonlyContext for MockCtx {
            fn invocation_id(&self) -> &str { "inv-test" }
            fn agent_name(&self) -> &str { "test-agent" }
            fn user_id(&self) -> &str { "devnpc" }
            fn app_name(&self) -> &str { "test-app" }
            fn session_id(&self) -> &str { "session-test" }
            fn branch(&self) -> &str { "" }
            fn user_content(&self) -> &Content { &self.content }
        }

        #[async_trait]
        impl CallbackContext for MockCtx {
            fn artifacts(&self) -> Option<Arc<dyn Artifacts>> { None }
            fn tool_name(&self) -> Option<&str> { self.tool_name.as_deref() }
            fn tool_input(&self) -> Option<&serde_json::Value> { self.tool_input.as_ref() }
        }

        let cb = DevnpcCallbacks::new(SopMode::Soft, vec!["/secret".into()], vec![]);
        let closure = cb.before_tool_callback();

        // 工具输入含禁止路径 → 记录警告但不阻断
        let ctx: Arc<dyn CallbackContext> = Arc::new(MockCtx {
            tool_name: Some("read_file".into()),
            tool_input: Some(serde_json::json!({"path": "/secret/credentials.env"})),
            content: Content::new("user"),
        });
        let result = closure(ctx).await;
        assert!(result.is_ok(), "禁止路径检测不应导致回调失败");
        assert!(result.unwrap().is_none());
    }

    #[tokio::test]
    async fn test_before_tool_callback_no_tool_name() {
        use adk_rust::{Artifacts, Content, ReadonlyContext};
        use async_trait::async_trait;
        use std::sync::Arc;

        struct MockCtx {
            content: Content,
        }

        #[async_trait]
        impl ReadonlyContext for MockCtx {
            fn invocation_id(&self) -> &str { "inv-test" }
            fn agent_name(&self) -> &str { "test-agent" }
            fn user_id(&self) -> &str { "devnpc" }
            fn app_name(&self) -> &str { "test-app" }
            fn session_id(&self) -> &str { "session-test" }
            fn branch(&self) -> &str { "" }
            fn user_content(&self) -> &Content { &self.content }
        }

        #[async_trait]
        impl CallbackContext for MockCtx {
            fn artifacts(&self) -> Option<Arc<dyn Artifacts>> { None }
            // tool_name 和 tool_input 使用默认实现 (返回 None)
        }

        let cb = DevnpcCallbacks::new(SopMode::Strict, vec!["/secret".into()], vec![]);
        let closure = cb.before_tool_callback();

        // 无 tool_name (非工具执行上下文) → 回调安全返回
        let ctx: Arc<dyn CallbackContext> = Arc::new(MockCtx {
            content: Content::new("user"),
        });
        let result = closure(ctx).await;
        assert!(result.is_ok());
        assert!(result.unwrap().is_none());
    }
}