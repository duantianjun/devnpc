//! 轨迹采集器 (P4 完整实现)
//!
//! 通过 tracing 事件订阅,不侵入业务逻辑。

use std::sync::{Arc, Mutex};

use crate::agent::loop_::Trajectory;

/// 轨迹采集器
pub struct TrajectoryCollector {
    #[allow(dead_code)]
    events: Arc<Mutex<Vec<String>>>,
}

impl TrajectoryCollector {
    pub fn new() -> Self {
        Self {
            events: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// 从 Agent Trajectory 生成报告数据 (P4 实现)
    pub fn from_trajectory(_trajectory: &Trajectory) -> Self {
        Self::new()
    }
}

impl Default for TrajectoryCollector {
    fn default() -> Self {
        Self::new()
    }
}
