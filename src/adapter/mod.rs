//! adk-rust 框架适配层
//!
//! 将 devnpc 业务逻辑与 adk-rust 框架桥接:
//! - tools.rs: 业务工具 → FunctionTool 包装
//! - callbacks.rs: SOP 检测 + 轨迹记录
//! - context.rs: 业务上下文 → Session 注入
//! - provider.rs: 多模型提供商配置

pub mod callbacks;
pub mod context;
pub mod provider;
pub mod tools;