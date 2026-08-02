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

/// 检查工具是否在 SOP 允许列表中 (严格模式白名单)
pub fn is_tool_allowed(tool_name: &str) -> bool {
    DEFAULT_ALLOWED_TOOLS.contains(&tool_name)
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
}

impl DevnpcCallbacks {
    /// 创建新的回调处理器
    pub fn new(sop_mode: SopMode, forbidden_paths: Vec<String>) -> Self {
        Self {
            sop_mode,
            forbidden_paths,
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
        Box::new(move |ctx: Arc<dyn CallbackContext>| {
            let fp = forbidden_paths.clone();
            Box::pin(async move {
                // SOP 偏离检测
                if let Some(tool_name) = ctx.tool_name() {
                    match sop_mode {
                        SopMode::Strict => {
                            if !is_tool_allowed(tool_name) {
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
        Self::new(SopMode::Soft, Vec::new())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_tool_allowed_default_whitelist() {
        // 白名单内工具
        assert!(is_tool_allowed("read_file"));
        assert!(is_tool_allowed("write_file"));
        assert!(is_tool_allowed("run_command"));
        assert!(is_tool_allowed("aft_outline"));
        assert!(is_tool_allowed("aft_view_symbol"));
        assert!(is_tool_allowed("aft_edit_symbol"));
        assert!(is_tool_allowed("aft_search_symbols"));
        assert!(is_tool_allowed("aft_ast_replace"));
    }

    #[test]
    fn test_is_tool_allowed_rejects_unknown() {
        // 不在白名单
        assert!(!is_tool_allowed("rm_rf"));
        assert!(!is_tool_allowed("shell_inject"));
        assert!(!is_tool_allowed(""));
        assert!(!is_tool_allowed("unknown_tool"));
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
        let cb = DevnpcCallbacks::new(SopMode::Strict, vec!["/secret".into()]);
        assert!(matches!(cb.sop_mode(), SopMode::Strict));
        assert_eq!(cb.forbidden_paths(), &["/secret".to_string()]);
    }

    #[test]
    fn test_callbacks_new_soft_mode() {
        let cb = DevnpcCallbacks::new(SopMode::Soft, vec![]);
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
        let cb = DevnpcCallbacks::new(SopMode::Strict, vec!["/secret".into()]);
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

        let cb = DevnpcCallbacks::new(SopMode::Soft, vec!["/secret".into()]);
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

        let cb = DevnpcCallbacks::new(SopMode::Strict, vec![]);
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

        let cb = DevnpcCallbacks::new(SopMode::Soft, vec!["/secret".into()]);
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

        let cb = DevnpcCallbacks::new(SopMode::Strict, vec!["/secret".into()]);
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