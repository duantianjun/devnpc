//! adk-rust 框架适配层
//!
//! 将 devnpc 业务逻辑与 adk-rust 框架桥接:
//! - file_io.rs: 带路径安全检查的文件 I/O
//! - tools.rs: 业务工具 → FunctionTool 包装
//! - callbacks.rs: SOP 检测 + 轨迹记录
//! - context.rs: 业务上下文 → Session 注入
//! - provider.rs: 多模型提供商配置

pub mod agents;
pub mod callbacks;
pub mod context;
pub mod file_io;
pub mod mcp_gateway;
pub mod memory;
pub mod orchestrator;
pub mod provider;
pub mod tools;