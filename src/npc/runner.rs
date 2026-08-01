//! 单 NPC 执行器 (P3-P5 完整实现)

use crate::error::Result;
use crate::memory::context::Context;
use super::role::Role;

/// NPC 执行器
pub struct NpcRunner {
    pub role: Role,
}

impl NpcRunner {
    pub fn new(role: Role) -> Self {
        Self { role }
    }

    /// 执行任务 (P3+ 实现)
    pub async fn execute(&self, _context: &Context) -> Result<()> {
        unimplemented!("P3+ 将实现")
    }
}
