//! SOP 偏离检测 (方案 C 核心)
//!
//! 软约束 (soft): 偏离只记录,下轮提示 LLM;strict 模式直接终止循环。

use serde::Deserialize;

use super::loop_::{Trajectory, TrajectoryEvent};
use crate::config::SopMode;

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
    /// 无偏离
    None,
    /// 软约束偏离 (只警告,不阻断)
    Soft {
        step: String,
        unexpected_tools: Vec<String>,
    },
    /// 严格约束偏离 (终止循环)
    Strict {
        step: String,
        unexpected_tools: Vec<String>,
    },
}

impl Sop {
    /// 估算当前步骤
    ///
    /// 规则: 找到第一个"尚未调用其 expected_tools 中任一工具"的步骤。
    /// 若所有步骤都已触及,返回最后一步 (收尾阶段)。
    /// trajectory 为空时返回第一步。
    pub fn estimate_current_step(&self, trajectory: &Trajectory) -> &SopStep {
        let called_tools: std::collections::HashSet<&str> = trajectory
            .events
            .iter()
            .filter_map(|e| match e {
                TrajectoryEvent::ToolCall { name, .. } => Some(name.as_str()),
                _ => None,
            })
            .collect();

        for step in &self.steps {
            let touched = step
                .expected_tools
                .iter()
                .any(|t| called_tools.contains(t.as_str()));
            if !touched {
                return step;
            }
        }
        // 所有步骤都已触及,返回最后一步
        self.steps.last().expect("SOP 至少一步")
    }

    /// 检查本轮 tool_calls 是否偏离当前步
    ///
    /// `mode` 决定返回 `Soft` 还是 `Strict` 偏离报告。
    pub fn check_deviation(
        &self,
        tool_calls: &[String],
        trajectory: &Trajectory,
        mode: SopMode,
    ) -> DeviationReport {
        let current = self.estimate_current_step(trajectory);
        let unexpected: Vec<String> = tool_calls
            .iter()
            .filter(|tc| !current.expected_tools.contains(tc))
            .cloned()
            .collect();
        if unexpected.is_empty() {
            return DeviationReport::None;
        }
        match mode {
            SopMode::Strict => DeviationReport::Strict {
                step: current.name.clone(),
                unexpected_tools: unexpected,
            },
            SopMode::Soft => DeviationReport::Soft {
                step: current.name.clone(),
                unexpected_tools: unexpected,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::SopMode;

    fn make_sop() -> Sop {
        Sop {
            name: "bugfix".into(),
            description: "".into(),
            steps: vec![
                SopStep {
                    name: "复现".into(),
                    expected_tools: vec!["run_command".into(), "read_file".into()],
                    hint: "".into(),
                },
                SopStep {
                    name: "修复".into(),
                    expected_tools: vec!["write_file".into()],
                    hint: "".into(),
                },
                SopStep {
                    name: "完成".into(),
                    expected_tools: vec!["finish".into()],
                    hint: "".into(),
                },
            ],
        }
    }

    fn traj_with_tools(tools: &[&str]) -> Trajectory {
        let mut t = Trajectory::default();
        for tool in tools {
            t.events.push(TrajectoryEvent::ToolCall {
                name: (*tool).into(),
                success: true,
            });
        }
        t
    }

    #[test]
    fn estimate_current_step_returns_first_when_trajectory_empty() {
        let sop = make_sop();
        let traj = Trajectory::default();
        let step = sop.estimate_current_step(&traj);
        assert_eq!(step.name, "复现");
    }

    #[test]
    fn estimate_current_step_advances_when_step_tools_called() {
        let sop = make_sop();
        let traj = traj_with_tools(&["run_command"]);
        let step = sop.estimate_current_step(&traj);
        // run_command 触及"复现"步,应推进到"修复"
        assert_eq!(step.name, "修复");
    }

    #[test]
    fn estimate_current_step_returns_last_when_all_touched() {
        let sop = make_sop();
        let traj = traj_with_tools(&["run_command", "write_file", "finish"]);
        let step = sop.estimate_current_step(&traj);
        assert_eq!(step.name, "完成");
    }

    #[test]
    fn check_deviation_none_when_tools_in_expected() {
        let sop = make_sop();
        let traj = Trajectory::default();
        let report = sop.check_deviation(&["run_command".into()], &traj, SopMode::Soft);
        assert!(matches!(report, DeviationReport::None));
    }

    #[test]
    fn check_deviation_soft_when_unexpected_tool() {
        let sop = make_sop();
        let traj = Trajectory::default();
        let report = sop.check_deviation(&["write_file".into()], &traj, SopMode::Soft);
        match report {
            DeviationReport::Soft {
                step,
                unexpected_tools,
            } => {
                assert_eq!(step, "复现");
                assert_eq!(unexpected_tools, vec!["write_file".to_string()]);
            }
            _ => panic!("expected Soft"),
        }
    }

    #[test]
    fn check_deviation_strict_when_unexpected_tool() {
        let sop = make_sop();
        let traj = Trajectory::default();
        let report = sop.check_deviation(&["write_file".into()], &traj, SopMode::Strict);
        match report {
            DeviationReport::Strict {
                step,
                unexpected_tools,
            } => {
                assert_eq!(step, "复现");
                assert_eq!(unexpected_tools, vec!["write_file".to_string()]);
            }
            _ => panic!("expected Strict"),
        }
    }
}
