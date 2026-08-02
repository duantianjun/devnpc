//! 工具适配: 将 devnpc 业务工具包装为 adk-rust FunctionTool
//!
//! 保留 src/tools/ 下各工具的实现逻辑,通过本模块适配为框架可用的 FunctionTool。

// TODO: 阶段 B 实现 - 将现有工具包装为 FunctionTool