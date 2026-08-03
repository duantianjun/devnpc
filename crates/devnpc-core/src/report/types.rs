//! 报告数据类型 (从 devnpc report/collector.rs 迁移)
//!
//! 纯数据结构,无 devnpc 业务依赖。

use serde::{Deserialize, Serialize};

// ============================================================
// 轨迹类型
// ============================================================

/// 轨迹事件
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TrajectoryEvent {
    /// LLM 调用
    LlmCall { iteration: usize },
    /// 工具调用
    ToolCall { name: String, success: bool },
    /// SOP 偏离
    Deviation { step: String, unexpected: Vec<String> },
}

// ============================================================
// 报告数据
// ============================================================

/// 报告数据 (供 HTML 生成使用)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReportData {
    pub status: String,
    pub duration_secs: u64,
    pub token_total: u64,
    pub llm_calls: u32,
    pub tool_calls: u32,
    pub ci_retries: u8,
    pub mr_url: Option<String>,
    pub ci_url: Option<String>,
    pub summary: String,
    pub task_description: String,
    pub trajectory: TrajectorySummary,
    pub cost_estimate: CostEstimate,
    pub mr_iid: Option<u64>,
    pub pipeline_id: Option<u64>,
    pub started_at: String,
    pub finished_at: String,
    /// Team 协作流程步骤 (仅在 Team 编排模式下填充)
    pub team_steps: Vec<TeamStepSummary>,
}

/// Team 协作步骤摘要
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TeamStepSummary {
    pub role: String,
    pub instruction: String,
    pub output: String,
    pub signals: Vec<String>,
}

/// 轨迹摘要
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TrajectorySummary {
    pub events: Vec<TrajectoryEventSummary>,
}

/// 轨迹事件摘要
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrajectoryEventSummary {
    pub kind: String,
    pub detail: String,
    pub success: Option<bool>,
}

/// 成本估算
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CostEstimate {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub estimated_cost_usd: f64,
}

impl Default for CostEstimate {
    fn default() -> Self {
        Self {
            input_tokens: 0,
            output_tokens: 0,
            estimated_cost_usd: 0.0,
        }
    }
}

impl Default for ReportData {
    fn default() -> Self {
        Self {
            status: "unknown".into(),
            duration_secs: 0,
            token_total: 0,
            llm_calls: 0,
            tool_calls: 0,
            ci_retries: 0,
            mr_url: None,
            ci_url: None,
            summary: String::new(),
            task_description: String::new(),
            trajectory: TrajectorySummary::default(),
            cost_estimate: CostEstimate::default(),
            mr_iid: None,
            pipeline_id: None,
            started_at: chrono::Utc::now().to_rfc3339(),
            finished_at: chrono::Utc::now().to_rfc3339(),
            team_steps: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn report_data_default_has_unknown_status() {
        let d = ReportData::default();
        assert_eq!(d.status, "unknown");
    }

    #[test]
    fn cost_estimate_default_is_zero() {
        let c = CostEstimate::default();
        assert_eq!(c.input_tokens, 0);
        assert_eq!(c.output_tokens, 0);
        assert_eq!(c.estimated_cost_usd, 0.0);
    }

    #[test]
    fn trajectory_event_serializes_to_json() {
        let event = TrajectoryEvent::LlmCall { iteration: 5 };
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("LlmCall"));
        assert!(json.contains("5"));
    }

    #[test]
    fn report_data_serializes_to_json() {
        let data = ReportData::default();
        let json = serde_json::to_string(&data).unwrap();
        assert!(json.contains("unknown"));
    }
}
