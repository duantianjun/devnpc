//! 轨迹采集器
//!
//! 从执行轨迹提取事件,聚合为报告数据。

use std::sync::{Arc, Mutex};

use chrono::Utc;

use crate::ci::controller::CiOutcome;

// ============================================================
// 本地轨迹类型 (替代旧的 agent::loop_ 模块)
// ============================================================

/// 轨迹事件
#[derive(Debug, Clone)]
pub enum TrajectoryEvent {
    /// LLM 调用
    LlmCall { iteration: usize },
    /// 工具调用
    ToolCall { name: String, success: bool },
    /// SOP 偏离
    Deviation { step: String, unexpected: Vec<String> },
}

/// 轨迹 (本地定义)
#[derive(Debug, Clone, Default)]
pub struct Trajectory {
    pub events: Vec<TrajectoryEvent>,
}

impl Trajectory {
    pub fn new() -> Self {
        Self { events: Vec::new() }
    }

    /// 记录 LLM 调用
    pub fn record_llm_call(&mut self, iteration: usize) {
        self.events.push(TrajectoryEvent::LlmCall { iteration });
    }

    /// 记录工具调用
    pub fn record_tool_call(&mut self, name: &str, success: bool) {
        self.events.push(TrajectoryEvent::ToolCall {
            name: name.to_string(),
            success,
        });
    }
}

/// 报告数据 (供 HTML 生成使用)
#[derive(Debug, Clone)]
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

/// Team 协作步骤摘要 (供 HTML 渲染)
#[derive(Debug, Clone, Default)]
pub struct TeamStepSummary {
    /// 角色名 (pm/developer/tester)
    pub role: String,
    /// 输入指令
    pub instruction: String,
    /// Agent 输出
    pub output: String,
    /// 检测到的信号 (decomposed/implemented 等)
    pub signals: Vec<String>,
}

/// 轨迹摘要 (供 HTML 渲染)
#[derive(Debug, Clone, Default)]
pub struct TrajectorySummary {
    pub events: Vec<TrajectoryEventSummary>,
}

/// 轨迹事件摘要
#[derive(Debug, Clone)]
pub struct TrajectoryEventSummary {
    pub kind: String,
    pub detail: String,
    pub success: Option<bool>,
}

/// 成本估算 (基于 token 数)
#[derive(Debug, Clone)]
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
            started_at: Utc::now().to_rfc3339(),
            finished_at: Utc::now().to_rfc3339(),
            team_steps: Vec::new(),
        }
    }
}

/// 轨迹采集器
pub struct TrajectoryCollector {
    events: Arc<Mutex<Vec<String>>>,
}

impl TrajectoryCollector {
    pub fn new() -> Self {
        Self {
            events: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// 获取事件列表 (用于测试和报告生成)
    pub fn events(&self) -> Vec<String> {
        self.events.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// 从 Agent Trajectory 生成报告数据
    pub fn from_trajectory(trajectory: &Trajectory) -> Self {
        let mut events = Vec::new();
        for ev in &trajectory.events {
            let desc = match ev {
                TrajectoryEvent::LlmCall { iteration } => {
                    format!("LLM 调用 iteration={iteration}")
                }
                TrajectoryEvent::ToolCall { name, success } => {
                    format!("工具调用 {name} success={success}")
                }
                TrajectoryEvent::Deviation { step, unexpected } => {
                    format!("SOP 偏离 step={step} unexpected={:?}", unexpected)
                }
            };
            events.push(desc);
        }
        Self {
            events: Arc::new(Mutex::new(events)),
        }
    }

    /// 从 Trajectory + CiOutcome 构建 ReportData
    pub fn build_report(
        trajectory: &Trajectory,
        outcome: &CiOutcome,
        task_description: &str,
        start_time: chrono::DateTime<Utc>,
        end_time: chrono::DateTime<Utc>,
    ) -> ReportData {
        let mut llm_calls = 0u32;
        let mut tool_calls = 0u32;
        let mut _tool_failures = 0u32;
        let mut trajectory_events = Vec::new();

        for ev in &trajectory.events {
            match ev {
                TrajectoryEvent::LlmCall { iteration } => {
                    llm_calls += 1;
                    trajectory_events.push(TrajectoryEventSummary {
                        kind: "llm_call".into(),
                        detail: format!("LLM 调用 (iteration #{iteration})"),
                        success: Some(true),
                    });
                }
                TrajectoryEvent::ToolCall { name, success } => {
                    tool_calls += 1;
                    if !success {
                        _tool_failures += 1;
                    }
                    trajectory_events.push(TrajectoryEventSummary {
                        kind: "tool_call".into(),
                        detail: format!("工具: {name}"),
                        success: Some(*success),
                    });
                }
                TrajectoryEvent::Deviation { step, unexpected } => {
                    trajectory_events.push(TrajectoryEventSummary {
                        kind: "deviation".into(),
                        detail: format!(
                            "SOP 偏离 — 步骤「{step}」, 意外工具: {}",
                            unexpected.join(", ")
                        ),
                        success: None,
                    });
                }
            }
        }

        // 估算 token (粗略: 假设每次 LLM call ~500 input + ~200 output)
        let input_tokens = llm_calls as u64 * 500;
        let output_tokens = llm_calls as u64 * 200;
        let estimated_cost_usd = (input_tokens as f64 * 0.000_001_5) + (output_tokens as f64 * 0.000_002_0);

        let duration_secs = (end_time - start_time).num_seconds().max(0) as u64;

        let (status, mr_iid, pipeline_id, ci_retries, mr_url, ci_url) = match outcome {
            CiOutcome::Passed {
                mr_iid,
                pipeline_id,
                attempts,
            } => (
                "passed".into(),
                Some(*mr_iid),
                Some(*pipeline_id),
                *attempts,
                None, // 由外部填充
                Some(format!("pipeline #{pipeline_id}")),
            ),
            CiOutcome::Failed {
                mr_iid,
                last_error,
                attempts,
            } => (
                format!("failed: {last_error}"),
                Some(*mr_iid),
                None,
                *attempts,
                None,
                None,
            ),
            CiOutcome::Timeout { mr_iid, stage } => (
                format!("timeout: {stage}"),
                Some(*mr_iid),
                None,
                0,
                None,
                None,
            ),
            CiOutcome::Error { mr_iid, reason } => (
                format!("error: {reason}"),
                Some(*mr_iid),
                None,
                0,
                None,
                None,
            ),
        };

        ReportData {
            status,
            duration_secs,
            token_total: input_tokens + output_tokens,
            llm_calls,
            tool_calls,
            ci_retries,
            mr_url,
            ci_url,
            summary: String::new(),
            task_description: task_description.to_string(),
            trajectory: TrajectorySummary {
                events: trajectory_events,
            },
            cost_estimate: CostEstimate {
                input_tokens,
                output_tokens,
                estimated_cost_usd,
            },
            mr_iid,
            pipeline_id,
            started_at: start_time.to_rfc3339(),
            finished_at: end_time.to_rfc3339(),
            team_steps: Vec::new(),
        }
    }
}

impl Default for TrajectoryCollector {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    #[test]
    fn build_report_from_trajectory_and_passed_outcome() {
        let mut traj = Trajectory::new();
        traj.record_llm_call(0);
        traj.record_tool_call("read_file", true);
        traj.record_tool_call("write_file", true);
        traj.record_llm_call(1);
        traj.record_tool_call("finish", true);

        let outcome = CiOutcome::Passed {
            mr_iid: 42,
            pipeline_id: 100,
            attempts: 1,
        };

        let start = Utc::now();
        let end = start + chrono::Duration::seconds(30);

        let report = TrajectoryCollector::build_report(&traj, &outcome, "修复登录 bug", start, end);

        assert_eq!(report.status, "passed");
        assert_eq!(report.llm_calls, 2);
        assert_eq!(report.tool_calls, 3);
        assert_eq!(report.duration_secs, 30);
        assert_eq!(report.mr_iid, Some(42));
        assert_eq!(report.pipeline_id, Some(100));
        assert_eq!(report.ci_retries, 1);
        assert!(report.task_description.contains("修复登录"));
    }

    #[test]
    fn build_report_from_failed_outcome() {
        let traj = Trajectory::new();
        let outcome = CiOutcome::Failed {
            mr_iid: 7,
            last_error: "编译错误".into(),
            attempts: 2,
        };
        let start = Utc::now();
        let end = start;
        let report = TrajectoryCollector::build_report(&traj, &outcome, "test", start, end);
        assert!(report.status.contains("failed"));
        assert_eq!(report.ci_retries, 2);
    }

    #[test]
    fn build_report_from_timeout_outcome() {
        let traj = Trajectory::new();
        let outcome = CiOutcome::Timeout {
            mr_iid: 1,
            stage: "build".into(),
        };
        let start = Utc::now();
        let end = start;
        let report = TrajectoryCollector::build_report(&traj, &outcome, "test", start, end);
        assert!(report.status.contains("timeout"));
    }

    #[test]
    fn from_trajectory_creates_collector_with_events() {
        let mut traj = Trajectory::new();
        traj.record_llm_call(0);
        traj.record_tool_call("read_file", true);
        let collector = TrajectoryCollector::from_trajectory(&traj);
        let events = collector.events();
        assert_eq!(events.len(), 2);
    }
}