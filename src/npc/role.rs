//! Role 定义 (P6 完整实现: 从 YAML 加载)

use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct Role {
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub system_prompt: String,
    #[serde(default = "default_max_iterations")]
    pub max_iterations: u32,
    pub default_sop: Option<String>,
    pub tools: Vec<String>,
}

fn default_max_iterations() -> u32 {
    20
}
