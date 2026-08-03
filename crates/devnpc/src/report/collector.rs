//! 轨迹采集器
//!
//! 从执行轨迹提取事件,聚合为报告数据。
//! 纯数据类型已迁移到 devnpc-core,这里通过 re-export 保持向后兼容。

// re-export core 类型 (向后兼容现有 use crate::report::collector::* 路径)
pub use devnpc_core::report::types::{
    CostEstimate, ReportData, TeamStepSummary, TrajectoryEvent,
    TrajectoryEventSummary, TrajectorySummary,
};

// ============================================================
// Trajectory (持有本地日志和推送组件, spec §4.3)
// ============================================================

use devnpc_core::report::event_schema::ExecutionEvent;

use super::sender::{EventSender, LocalEventLogger};

/// 轨迹 (本地定义,持有三个可选组件)
///
/// - `events`: 内存事件列表 (始终存在,兼容现有逻辑)
/// - `local_logger`: 本地文件记录 (`local_event_log=true` 时存在)
/// - `sender`: 实时推送 (`dashboard.enabled=true` 时存在)
pub struct Trajectory {
    /// 内存事件列表 (现状,始终存在)
    pub events: Vec<TrajectoryEvent>,
    /// 本地文件记录器 (None 时跳过文件写入)
    local_logger: Option<LocalEventLogger>,
    /// 事件推送器 (None 时跳过推送)
    sender: Option<EventSender>,
    /// task_id
    task_id: String,
}

impl Trajectory {
    /// 现状构造 (无日志无推送,兼容现有测试)
    pub fn new() -> Self {
        Self {
            events: Vec::new(),
            local_logger: None,
            sender: None,
            task_id: String::new(),
        }
    }

    /// 带本地日志和推送的构造 (spec §4.3)
    pub fn with_logging(
        task_id: String,
        local_logger: Option<LocalEventLogger>,
        sender: Option<EventSender>,
    ) -> Self {
        Self {
            events: Vec::new(),
            local_logger,
            sender,
            task_id,
        }
    }

    /// 记录 LLM 调用
    ///
    /// 同时: 推入内存 events + 转发到本地日志 + 转发到推送器
    pub fn record_llm_call(&mut self, iteration: usize) {
        self.events.push(TrajectoryEvent::LlmCall { iteration });

        if self.local_logger.is_some() || self.sender.is_some() {
            let exec_event = ExecutionEvent::LlmCall {
                iteration: iteration as u32,
                prompt_tokens: 0,
                completion_tokens: 0,
                latency_ms: 0,
            };
            if let Some(logger) = &self.local_logger {
                logger.log_event(&exec_event);
            }
            if let Some(sender) = &self.sender {
                sender.send(exec_event);
            }
        }
    }

    /// 记录工具调用
    pub fn record_tool_call(&mut self, name: &str, success: bool) {
        self.events.push(TrajectoryEvent::ToolCall {
            name: name.to_string(),
            success,
        });

        if self.local_logger.is_some() || self.sender.is_some() {
            let exec_event = ExecutionEvent::ToolCall {
                name: name.to_string(),
                success,
                latency_ms: 0,
                detail: String::new(),
            };
            if let Some(logger) = &self.local_logger {
                logger.log_event(&exec_event);
            }
            if let Some(sender) = &self.sender {
                sender.send(exec_event);
            }
        }
    }

    /// 获取 task_id
    pub fn task_id(&self) -> &str {
        &self.task_id
    }

    /// 任务结束: flush 本地日志 + 推送 TaskFinishedEvent
    ///
    /// 消费 self (任务结束后不再使用)。
    pub async fn finish(
        self,
        config: &crate::config::DashboardConfig,
        finished: &devnpc_core::report::event_schema::TaskFinishedEvent,
    ) {
        if let Some(logger) = &self.local_logger {
            logger.finish(finished);
        }
        if let Some(sender) = self.sender {
            sender.finish(config, finished.clone()).await;
        }
    }
}

impl Default for Trajectory {
    fn default() -> Self {
        Self::new()
    }
}

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

    // ============================================================
    // Trajectory 改造测试 (spec §4.3)
    // ============================================================

    use devnpc_core::report::event_schema::{
        TaskFinishedEvent, TaskStartedEvent, TaskStatus,
    };
    use tempfile::tempdir;

    fn make_started_for_traj(task_id: &str) -> TaskStartedEvent {
        TaskStartedEvent {
            task_id: task_id.to_string(),
            project: "test".to_string(),
            mr_iid: None,
            pipeline_id: None,
            task_description: "test".to_string(),
            task_kind: "manual".to_string(),
            started_at: "2026-08-03T10:00:00Z".to_string(),
            model: "test".to_string(),
        }
    }

    fn make_finished_for_traj(task_id: &str) -> TaskFinishedEvent {
        TaskFinishedEvent {
            task_id: task_id.to_string(),
            status: TaskStatus::Success,
            duration_secs: 10,
            total_tokens: 1000,
            estimated_cost_usd: 0.01,
            mr_url: None,
            ci_url: None,
            summary: "done".to_string(),
            error: None,
            finished_at: "2026-08-03T10:01:00Z".to_string(),
        }
    }

    #[test]
    fn trajectory_new_is_empty_without_logger_or_sender() {
        let t = Trajectory::new();
        assert!(t.events.is_empty());
        assert!(t.task_id().is_empty());
    }

    #[test]
    fn trajectory_new_record_llm_call_backward_compat() {
        // 无 logger/sender 时,record_llm_call 仅推入 events
        let mut t = Trajectory::new();
        t.record_llm_call(1);
        assert_eq!(t.events.len(), 1);
        assert!(matches!(t.events[0], TrajectoryEvent::LlmCall { iteration: 1 }));
    }

    #[test]
    fn trajectory_new_record_tool_call_backward_compat() {
        let mut t = Trajectory::new();
        t.record_tool_call("read_file", true);
        assert_eq!(t.events.len(), 1);
        assert!(matches!(
            &t.events[0],
            TrajectoryEvent::ToolCall { name, success } if name == "read_file" && *success
        ));
    }

    #[test]
    fn trajectory_with_logging_holds_logger() {
        let dir = tempdir().unwrap();
        let task_id = "traj-with-logging";
        let started = make_started_for_traj(task_id);
        let logger = LocalEventLogger::new(task_id, &started, dir.path());

        let t = Trajectory::with_logging(task_id.to_string(), logger, None);
        assert_eq!(t.task_id(), task_id);
        assert!(t.events.is_empty());
    }

    #[test]
    fn trajectory_record_llm_call_forwards_to_logger() {
        let dir = tempdir().unwrap();
        let task_id = "traj-forward-llm";
        let started = make_started_for_traj(task_id);
        let logger = LocalEventLogger::new(task_id, &started, dir.path());

        let mut t = Trajectory::with_logging(task_id.to_string(), logger, None);
        t.record_llm_call(1);

        // 内存 events 应有 1 条
        assert_eq!(t.events.len(), 1);

        // 本地文件应有 2 行: task_started + execution
        let file_path = dir.path().join(format!("{task_id}.jsonl"));
        let content = std::fs::read_to_string(&file_path).unwrap();
        let lines: Vec<&str> = content.lines().collect();
        assert_eq!(lines.len(), 2);
        assert!(lines[1].contains("llm_call"));
    }

    #[test]
    fn trajectory_record_tool_call_forwards_to_logger() {
        let dir = tempdir().unwrap();
        let task_id = "traj-forward-tool";
        let started = make_started_for_traj(task_id);
        let logger = LocalEventLogger::new(task_id, &started, dir.path());

        let mut t = Trajectory::with_logging(task_id.to_string(), logger, None);
        t.record_tool_call("write_file", true);

        assert_eq!(t.events.len(), 1);

        let file_path = dir.path().join(format!("{task_id}.jsonl"));
        let content = std::fs::read_to_string(&file_path).unwrap();
        let lines: Vec<&str> = content.lines().collect();
        assert_eq!(lines.len(), 2);
        assert!(lines[1].contains("tool_call"));
        assert!(lines[1].contains("write_file"));
    }

    #[test]
    fn trajectory_record_llm_call_no_logger_no_sender_works() {
        // 无 logger/sender 时不应 panic
        let mut t = Trajectory::with_logging("none".to_string(), None, None);
        t.record_llm_call(1);
        t.record_tool_call("test", false);
        assert_eq!(t.events.len(), 2);
    }

    #[tokio::test]
    async fn trajectory_finish_writes_task_finished_to_file() {
        let dir = tempdir().unwrap();
        let task_id = "traj-finish";
        let started = make_started_for_traj(task_id);
        let logger = LocalEventLogger::new(task_id, &started, dir.path());

        let mut t = Trajectory::with_logging(task_id.to_string(), logger, None);
        t.record_llm_call(1);

        let config = crate::config::DashboardConfig::default();
        let finished = make_finished_for_traj(task_id);
        t.finish(&config, &finished).await;

        let file_path = dir.path().join(format!("{task_id}.jsonl"));
        let content = std::fs::read_to_string(&file_path).unwrap();
        let lines: Vec<&str> = content.lines().collect();
        // task_started + execution + task_finished = 3 行
        assert_eq!(lines.len(), 3);
        assert!(lines[2].contains("task_finished"));
    }
}
