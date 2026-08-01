//! CI 闭环控制器 (P4 完整实现)

use std::time::Duration;

use serde::Deserialize;

use crate::error::Result;

#[derive(Debug, Clone, Deserialize)]
pub struct CiConfig {
    pub poll_interval: Duration,
    pub poll_timeout: Duration,
    pub pipeline_timeout: Duration,
    pub max_retries: u8,
}

impl Default for CiConfig {
    fn default() -> Self {
        Self {
            poll_interval: Duration::from_secs(10),
            poll_timeout: Duration::from_secs(300),       // 5 min
            pipeline_timeout: Duration::from_secs(1800),  // 30 min
            max_retries: 3,
        }
    }
}

#[derive(Debug, Clone)]
pub enum CiOutcome {
    Passed { mr_iid: u64, pipeline_id: u64, attempts: u8 },
    Failed { mr_iid: u64, last_error: String, attempts: u8 },
    Timeout { mr_iid: u64, stage: String },
}

/// CI 闭环控制器 (P4 实现)
pub struct CiController {
    #[allow(dead_code)]
    config: CiConfig,
}

impl CiController {
    pub fn new(config: CiConfig) -> Self {
        Self { config }
    }

    /// 运行 CI 闭环 (P4 实现)
    pub async fn run(&self, _mr_iid: u64) -> Result<CiOutcome> {
        unimplemented!("P4 将实现")
    }
}
