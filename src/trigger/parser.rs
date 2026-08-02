//! 触发解析器
//!
//! 支持三种触发源: MR 评论 + Issue 评论 + 手动 --task 参数。

use regex::Regex;
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

/// 从评论中查找 @devnpc 提及并解析任务
pub fn parse_mention(body: &str) -> Option<TaskSpec> {
    use std::sync::OnceLock;
    static MENTION_RE: OnceLock<Regex> = OnceLock::new();
    static ISSUE_RE: OnceLock<Regex> = OnceLock::new();
    // 查找 @devnpc 提及
    let re = MENTION_RE.get_or_init(|| {
        Regex::new(r"@devnpc\s*(.*)").expect("静态 Regex 编译失败 (MENTION_RE)")
    });
    let caps = re.captures(body)?;
    let text = caps.get(1)?.as_str().trim();
    if text.is_empty() {
        return None;
    }

    let kind = classify_task(text);

    // 检测目标 Issue (#42) 引用
    let issue_re = ISSUE_RE.get_or_init(|| {
        Regex::new(r"#(\d+)").expect("静态 Regex 编译失败 (ISSUE_RE)")
    });
    let target_issue = issue_re
        .captures(text)
        .and_then(|c| c[1].parse().ok());

    Some(TaskSpec {
        kind,
        description: text.to_string(),
        target_issue,
        acceptance_criteria: Vec::new(),
    })
}

/// 根据关键字识别任务类型
pub fn classify_task(text: &str) -> TaskKind {
    let lower = text.to_lowercase();
    if lower.contains("修复") || lower.contains("fix") || lower.contains("bug") {
        TaskKind::Fix
    } else if lower.contains("测试") || lower.contains("test") {
        TaskKind::Test
    } else if lower.contains("重构") || lower.contains("refactor") {
        TaskKind::Refactor
    } else if lower.contains("review") {
        TaskKind::Review
    } else {
        TaskKind::Implement
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_mention_extracts_text_after_mention() {
        let body = "请 @devnpc 修复这个 bug #42";
        let spec = parse_mention(body).unwrap();
        assert_eq!(spec.description, "修复这个 bug #42");
        assert_eq!(spec.kind, TaskKind::Fix);
        assert_eq!(spec.target_issue, Some(42));
    }

    #[test]
    fn parse_mention_returns_none_when_no_mention() {
        let body = "这是一个普通评论";
        assert!(parse_mention(body).is_none());
    }

    #[test]
    fn parse_mention_returns_none_when_empty_after_mention() {
        let body = "@devnpc  ";
        assert!(parse_mention(body).is_none());
    }

    #[test]
    fn parse_mention_detects_issue_reference() {
        let body = "@devnpc 实现用户登录功能 #123";
        let spec = parse_mention(body).unwrap();
        assert_eq!(spec.target_issue, Some(123));
    }

    #[test]
    fn parse_mention_no_issue_reference() {
        let body = "@devnpc 优化代码结构";
        let spec = parse_mention(body).unwrap();
        assert!(spec.target_issue.is_none());
    }

    #[test]
    fn classify_task_fix_by_chinese_keyword() {
        assert_eq!(classify_task("修复登录 bug"), TaskKind::Fix);
    }

    #[test]
    fn classify_task_fix_by_english_keyword() {
        assert_eq!(classify_task("fix the crash"), TaskKind::Fix);
    }

    #[test]
    fn classify_task_test_by_chinese_keyword() {
        assert_eq!(classify_task("测试用户模块"), TaskKind::Test);
    }

    #[test]
    fn classify_task_test_by_english_keyword() {
        assert_eq!(classify_task("add unit tests"), TaskKind::Test);
    }

    #[test]
    fn classify_task_refactor_by_chinese_keyword() {
        assert_eq!(classify_task("重构支付模块"), TaskKind::Refactor);
    }

    #[test]
    fn classify_task_refactor_by_english_keyword() {
        assert_eq!(classify_task("refactor the handler"), TaskKind::Refactor);
    }

    #[test]
    fn classify_task_review() {
        assert_eq!(classify_task("review the MR"), TaskKind::Review);
    }

    #[test]
    fn classify_task_defaults_to_implement() {
        assert_eq!(
            classify_task("随便写点代码"),
            TaskKind::Implement
        );
    }

    #[test]
    fn classify_task_kind_display() {
        // 验证 Fix 关键字匹配"bug"
        assert_eq!(classify_task("这个 bug 需要修复"), TaskKind::Fix);
    }
}