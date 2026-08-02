//! 模型提供商适配: 根据 Config 创建多模型 Provider
//!
//! 支持 adk-rust 的所有模型提供商:
//! - DeepSeek (默认)
//! - OpenAI
//! - Anthropic
//! - Gemini
//!   根据配置中的 provider 字段选择合适的模型客户端。

use std::sync::Arc;

use adk_rust::Llm;

use crate::config::LlmConfig;
use crate::error::DevnpcError;

/// 根据配置创建模型 Provider
///
/// 根据 `config.provider` 字段选择对应的模型客户端:
/// - `"deepseek"` → DeepSeekClient
/// - `"openai"` → OpenAIClient
/// - `"anthropic"` → AnthropicClient
/// - `"gemini"` → GeminiModel
/// - 其他 → 返回错误
pub fn create_model(config: &LlmConfig) -> Result<Arc<dyn Llm>, DevnpcError> {
    match config.provider.to_lowercase().as_str() {
        "deepseek" => create_deepseek(config),
        "openai" => create_openai(config),
        "anthropic" => create_anthropic(config),
        "gemini" => create_gemini(config),
        _ => Err(DevnpcError::Config(format!(
            "不支持的模型提供商: '{}' (可选: deepseek, openai, anthropic, gemini)",
            config.provider
        ))),
    }
}

/// 创建 DeepSeek 模型客户端
#[cfg(feature = "deepseek")]
fn create_deepseek(config: &LlmConfig) -> Result<Arc<dyn Llm>, DevnpcError> {
    use adk_rust::model::deepseek::{DeepSeekClient, DeepSeekConfig};

    let mut ds_config = DeepSeekConfig::new(&config.api_key, &config.model);
    if !config.base_url.is_empty() {
        ds_config = ds_config.with_base_url(&config.base_url);
    }
    let client = DeepSeekClient::new(ds_config)
        .map_err(|e| DevnpcError::Config(format!("DeepSeek 客户端创建失败: {e}")))?;
    Ok(Arc::new(client))
}

#[cfg(not(feature = "deepseek"))]
fn create_deepseek(_config: &LlmConfig) -> Result<Arc<dyn Llm>, DevnpcError> {
    Err(DevnpcError::Config(
        "DeepSeek 支持未编译 (需启用 deepseek feature)".to_string(),
    ))
}

/// 创建 OpenAI 模型客户端
#[cfg(feature = "openai")]
fn create_openai(config: &LlmConfig) -> Result<Arc<dyn Llm>, DevnpcError> {
    use adk_rust::model::openai::{OpenAIClient, OpenAIConfig};

    let mut oai_config = OpenAIConfig::new(&config.api_key, &config.model);
    if !config.base_url.is_empty() {
        oai_config = OpenAIConfig::compatible(&config.api_key, &config.base_url, &config.model);
    }
    let client = OpenAIClient::new(oai_config)
        .map_err(|e| DevnpcError::Config(format!("OpenAI 客户端创建失败: {e}")))?;
    Ok(Arc::new(client))
}

#[cfg(not(feature = "openai"))]
fn create_openai(_config: &LlmConfig) -> Result<Arc<dyn Llm>, DevnpcError> {
    Err(DevnpcError::Config(
        "OpenAI 支持未编译 (需启用 openai feature)".to_string(),
    ))
}

/// 创建 Anthropic 模型客户端
#[cfg(feature = "anthropic")]
fn create_anthropic(config: &LlmConfig) -> Result<Arc<dyn Llm>, DevnpcError> {
    use adk_rust::model::anthropic::{AnthropicClient, AnthropicConfig};

    let mut an_config = AnthropicConfig::new(&config.api_key, &config.model);
    if !config.base_url.is_empty() {
        an_config = AnthropicConfig::new(&config.api_key, &config.model)
            .with_base_url(&config.base_url);
    }
    let client = AnthropicClient::new(an_config)
        .map_err(|e| DevnpcError::Config(format!("Anthropic 客户端创建失败: {e}")))?;
    Ok(Arc::new(client))
}

#[cfg(not(feature = "anthropic"))]
fn create_anthropic(_config: &LlmConfig) -> Result<Arc<dyn Llm>, DevnpcError> {
    Err(DevnpcError::Config(
        "Anthropic 支持未编译 (需启用 anthropic feature)".to_string(),
    ))
}

/// 创建 Gemini 模型客户端
#[cfg(feature = "gemini")]
fn create_gemini(config: &LlmConfig) -> Result<Arc<dyn Llm>, DevnpcError> {
    use adk_rust::model::GeminiModel;

    let client = GeminiModel::new(&config.api_key, &config.model)
        .map_err(|e| DevnpcError::Config(format!("Gemini 客户端创建失败: {e}")))?;
    Ok(Arc::new(client))
}

#[cfg(not(feature = "gemini"))]
fn create_gemini(_config: &LlmConfig) -> Result<Arc<dyn Llm>, DevnpcError> {
    Err(DevnpcError::Config(
        "Gemini 支持未编译 (需启用 gemini feature)".to_string(),
    ))
}