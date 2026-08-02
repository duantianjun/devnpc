//! 回调适配: SOP 偏离检测 + 执行轨迹记录
//!
//! 通过 adk-rust 的 before_tool_callback 机制:
//! - 在工具调用前检查 SOP 偏离 (软约束/硬约束)
//! - 记录工具调用轨迹,供 report 模块消费

// TODO: 阶段 C 实现 - 实现 before_tool_callback