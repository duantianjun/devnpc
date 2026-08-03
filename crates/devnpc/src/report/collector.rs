//! 轨迹采集器
//!
//! 从执行轨迹提取事件,聚合为报告数据。
//! 纯数据类型已迁移到 devnpc-core,这里通过 re-export 保持向后兼容。

// re-export core 类型 (向后兼容现有 use crate::report::collector::* 路径)
pub use devnpc_core::report::types::{
    CostEstimate, ReportData, TeamStepSummary, Trajectory, TrajectoryEvent,
    TrajectoryEventSummary, TrajectorySummary,
};

use std::sync::{Arc, Mutex};

use chrono::Utc;

use crate::ci::controller::CiOutcome;

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
    ///
    /// `cost_config` 用于覆盖默认的 token 估算值与费率;
    /// 传入 `None` 时回退到硬编码 (500 input / 200 output per call)。
    pub fn build_report(
        trajectory: &Trajectory,
        outcome: &CiOutcome,
        task_description: &str,
        start_time: chrono::DateTime<Utc>,
        end_time: chrono::DateTime<Utc>,
        cost_config: Option<&crate::config::CostConfig>,
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

        // 估算 token: 优先使用 cost_config, 否则回退到硬编码 (500 input + 200 output per call)
        let (input_tokens, output_tokens, estimated_cost_usd) = match cost_config {
            Some(cfg) => {
                let in_tok = llm_calls as u64 * cfg.est_input_tokens_per_call;
                let out_tok = llm_calls as u64 * cfg.est_output_tokens_per_call;
                let cost = crate::adapter::orchestrator::UsageStats::estimate_cost_with_rates(
                    in_tok as i64,
                    out_tok as i64,
                    cfg.input_rate,
                    cfg.output_rate,
                );
                (in_tok, out_tok, cost)
            }
            None => {
                // 默认回退: 500 input + 200 output per call
                let in_tok = llm_calls as u64 * 500;
                let out_tok = llm_calls as u64 * 200;
                let cost = crate::adapter::orchestrator::UsageStats::estimate_cost(
                    in_tok as i64,
                    out_tok as i64,
                );
                (in_tok, out_tok, cost)
            }
        };

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

        let report = TrajectoryCollector::build_report(&traj, &outcome, "修复登录 bug", start, end, None);

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
        let report = TrajectoryCollector::build_report(&traj, &outcome, "test", start, end, None);
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
        let report = TrajectoryCollector::build_report(&traj, &outcome, "test", start, end, None);
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
