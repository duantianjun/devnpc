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

/// 创建简单任务模型 (小模型，用于阅读/搜索)
pub fn create_simple_model(config: &crate::config::Config) -> Result<Arc<dyn adk_rust::Llm>, crate::error::DevnpcError> {
    if config.model_routing.simple_model.is_empty() {
        // 回退到主模型
        return create_model(&config.llm);
    }
    // 使用简单模型配置
    let simple_config = crate::config::LlmConfig {
        model: config.model_routing.simple_model.clone(),
        ..config.llm.clone()
    };
    create_model(&simple_config)
}

/// 创建复杂任务模型 (大模型，用于改码/修复/推理)
pub fn create_complex_model(config: &crate::config::Config) -> Result<Arc<dyn adk_rust::Llm>, crate::error::DevnpcError> {
    if config.model_routing.complex_model.is_empty() {
        // 回退到主模型
        return create_model(&config.llm);
    }
    let complex_config = crate::config::LlmConfig {
        model: config.model_routing.complex_model.clone(),
        ..config.llm.clone()
    };
    create_model(&complex_config)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{
        CiConfig, CommandConfig, ContextConfig, GitlabConfig, LlmConfig, Limits, LogParserConfig,
        McpConfig, MemoryConfig, ModelRoutingConfig, ProjectConfig, ReadFileConfig, ReportConfig,
        SummaryConfig,
    };

    /// 构造测试用 Config (DeepSeek provider, 假 API key)
    fn make_test_config(simple: &str, complex: &str) -> crate::config::Config {
        crate::config::Config {
            llm: LlmConfig {
                api_key: "test-key".into(),
                base_url: "".into(),
                model: "deepseek-chat".into(),
                provider: "deepseek".into(),
            },
            gitlab: GitlabConfig {
                url: "".into(),
                token: "".into(),
                project_id: 1,
            },
            limits: Limits::default(),
            project: ProjectConfig::default(),
            model_routing: ModelRoutingConfig {
                simple_model: simple.into(),
                complex_model: complex.into(),
            },
            report: ReportConfig::default(),
            command: CommandConfig::default(),
            read_file: ReadFileConfig::default(),
            log_parser: LogParserConfig::default(),
            summary: SummaryConfig::default(),
            context: ContextConfig::default(),
            ci: CiConfig::default(),
            mcp: McpConfig::default(),
            memory: MemoryConfig::default(),
            npc_config: crate::config::NpcConfigSection::default(),
        }
    }

    #[test]
    fn test_create_model_unsupported_provider_returns_error() {
        let cfg = LlmConfig {
            api_key: "k".into(),
            base_url: "".into(),
            model: "m".into(),
            provider: "unsupported_provider".into(),
        };
        let result = create_model(&cfg);
        match result {
            Err(DevnpcError::Config(msg)) => assert!(msg.contains("不支持的模型提供商")),
            Err(_) => panic!("expected Config error"),
            Ok(_) => panic!("expected error for unsupported provider"),
        }
    }

    #[test]
    fn test_create_model_deepseek_ok() {
        let cfg = LlmConfig {
            api_key: "test-key".into(),
            base_url: "".into(),
            model: "deepseek-chat".into(),
            provider: "deepseek".into(),
        };
        let result = create_model(&cfg);
        assert!(result.is_ok(), "DeepSeek 构建应成功: {:?}", result.err());
    }

    #[test]
    fn test_create_model_openai_ok() {
        let cfg = LlmConfig {
            api_key: "test-key".into(),
            base_url: "".into(),
            model: "gpt-4".into(),
            provider: "openai".into(),
        };
        let result = create_model(&cfg);
        assert!(result.is_ok(), "OpenAI 构建应成功: {:?}", result.err());
    }

    #[test]
    fn test_create_simple_model_fallback_when_empty() {
        // simple_model 为空时,应回退到主模型 (deepseek-chat)
        let config = make_test_config("", "");
        let result = create_simple_model(&config);
        assert!(result.is_ok(), "回退到主模型应成功: {:?}", result.err());
    }

    #[test]
    fn test_create_complex_model_fallback_when_empty() {
        // complex_model 为空时,应回退到主模型 (deepseek-chat)
        let config = make_test_config("", "");
        let result = create_complex_model(&config);
        assert!(result.is_ok(), "回退到主模型应成功: {:?}", result.err());
    }

    #[test]
    fn test_create_simple_model_uses_configured_name() {
        // simple_model 配置了 "deepseek-coder",应使用该模型名构造客户端
        let config = make_test_config("deepseek-coder", "");
        let result = create_simple_model(&config);
        assert!(result.is_ok(), "simple_model 应构造成功: {:?}", result.err());
        // 模型名不影响 Arc<dyn Llm> 实例的可观察行为,只能验证 Ok
    }

    #[test]
    fn test_create_complex_model_uses_configured_name() {
        // complex_model 配置了 "deepseek-reasoner",应使用该模型名构造客户端
        let config = make_test_config("", "deepseek-reasoner");
        let result = create_complex_model(&config);
        assert!(result.is_ok(), "complex_model 应构造成功: {:?}", result.err());
    }

    #[test]
    fn test_create_simple_model_unsupported_provider_in_routing() {
        // simple_model 配置了不支持的 provider 不会触发,因为 routing 只换 model 名,
        // provider 仍来自主 LlmConfig
        let config = make_test_config("gpt-4-mini", "");
        // provider 还是 deepseek,只是 model 名变成 gpt-4-mini,DeepSeek 客户端会接受任意 model 名
        let result = create_simple_model(&config);
        assert!(result.is_ok(), "应使用 deepseek provider 构造成功: {:?}", result.err());
    }
}