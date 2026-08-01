//! NPC 间通信 (P7 完整实现: GitLab 评论总线)
//!
//! 协议头: [devnpc:handoff] ... [/devnpc:handoff]

use crate::error::Result;

/// 解析 handoff 消息 (P7 实现)
pub fn parse_handoff(_body: &str) -> Result<Option<Handoff>> {
    unimplemented!("P7 将实现")
}

/// Handoff 消息
#[derive(Debug, Clone)]
pub struct Handoff {
    pub from: String,
    pub to: Vec<String>,
    pub signal: String,
}
