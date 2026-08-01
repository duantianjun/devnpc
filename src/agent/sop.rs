//! SOP 偏离检测 (方案 C 核心)
//!
//! 软约束: 偏离只警告;strict 模式: 偏离即阻断。

use serde::Deserialize;

use super::loop_::Trajectory;

/// SOP 定义
#[derive(Debug, Clone, Deserialize)]
pub struct Sop {
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub steps: Vec<SopStep>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SopStep {
    pub name: String,
    pub expected_tools: Vec<String>,
    #[serde(default)]
    pub hint: String,
}

/// 偏离报告
#[derive(Debug, Clone)]
pub enum DeviationReport {
    None,
    Soft {
        step: String,
        unexpected_tools: Vec<String>,
    },
}

impl Sop {
    /// 估算当前步骤 (P3 完整实现)
    pub fn estimate_current_step(&self, _trajectory: &Trajectory) -> &SopStep {
        &self.steps[0]
    }

    /// 检查偏离 (P3 完整实现)
    pub fn check_deviation(
        &self,
        _tool_calls: &[String],
        _trajectory: &Trajectory,
    ) -> DeviationReport {
        DeviationReport::None
    }
}
