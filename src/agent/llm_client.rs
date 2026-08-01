//! LLM 客户端封装 (P3 完整实现,基于 rig-core)

use crate::config::LlmConfig;
use crate::error::Result;

/// LLM 客户端 (P3 实现)
pub struct LlmClient {
    #[allow(dead_code)]
    config: LlmConfig,
}

impl LlmClient {
    pub fn new(config: LlmConfig) -> Self {
        Self { config }
    }

    /// 调用 LLM (P3 实现)
    pub async fn complete(&self, _messages: &[String]) -> Result<String> {
        unimplemented!("P3 将实现")
    }
}
