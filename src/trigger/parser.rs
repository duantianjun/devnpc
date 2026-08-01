//! 触发解析器 (P5 完整实现)
//!
//! MVP: MR 评论 + 手动触发
//! P5+: Issue 评论 + Issue 创建

use serde::Deserialize;

#[derive(Debug, Clone)]
pub enum Trigger {
    IssueTask { issue_iid: u64, task: TaskSpec },
    MrTask { mr_iid: u64, task: TaskSpec },
    Manual { task: TaskSpec },
    None,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TaskSpec {
    pub kind: TaskKind,
    pub description: String,
    pub target_issue: Option<u64>,
    #[serde(default)]
    pub acceptance_criteria: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub enum TaskKind {
    Implement,
    Fix,
    Test,
    Refactor,
    Review,
}

/// 从评论中查找 @devnpc 提及并解析任务 (P5 实现)
pub fn parse_mention(_body: &str) -> Option<TaskSpec> {
    unimplemented!("P5 将实现")
}

/// 根据关键字识别任务类型 (P5 实现)
pub fn classify_task(_text: &str) -> TaskKind {
    unimplemented!("P5 将实现")
}
