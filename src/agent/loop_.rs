//! ReAct 循环 (P3 完整实现)
//!
//! Plan-Act-Observe 循环,带 SOP 偏离检测与迭代上限。

use crate::error::Result;
use super::sop::Sop;

/// Agent 运行结果
#[derive(Debug, Clone)]
pub enum RunResult {
    /// LLM 返回无 tool_call,任务完成
    Finished { text: String, trajectory: Trajectory },
    /// 达到迭代上限
    MaxIterationsReached(Trajectory),
}

/// 执行轨迹 (供 report 模块消费)
#[derive(Debug, Clone, Default)]
pub struct Trajectory {
    pub events: Vec<TrajectoryEvent>,
}

#[derive(Debug, Clone)]
pub enum TrajectoryEvent {
    LlmCall { iteration: u32 },
    ToolCall { name: String, success: bool },
}

/// ReAct 循环执行器 (P3 完整实现)
pub struct ReactLoop {
    pub max_iterations: u32,
}

impl ReactLoop {
    pub fn new(max_iterations: u32) -> Self {
        Self { max_iterations }
    }

    /// 运行循环 (P3 实现)
    pub async fn run(&self, _sop: Option<&Sop>) -> Result<RunResult> {
        unimplemented!("P3 将实现")
    }
}
