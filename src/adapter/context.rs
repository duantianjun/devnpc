//! 上下文适配: 将业务 Context 注入 adk-rust Session
//!
//! 将 memory::context::Context 中的研发记忆数据注入到 Session 中,
//! 使 LlmAgent 在执行时可以访问项目上下文。

// TODO: 阶段 C 实现 - 将业务 Context 注入 Session