//! 模型提供商适配: 根据 Config 创建多模型 Provider
//!
//! 支持 adk-rust 的所有模型提供商:
//! - Gemini (默认, minimal feature 自带)
//! - OpenAI
//! - Anthropic
//! - DeepSeek
//! 根据配置中的 provider 字段选择合适的模型客户端。

// TODO: 阶段 C 实现 - 根据 Config 创建多模型 Provider