//! 提示词模板: 把 Context + 任务渲染成初始消息
//!
//! System: 角色 + 项目规范 + 工具使用指引
//! User: 研发记忆 (仓库结构/关键文件/Issue/PR/CI) + 任务描述

use crate::agent::message::Message;
use crate::memory::context::Context;

/// 构建初始消息 (System + User)
///
/// `role_prompt`: 角色 system prompt (来自 Role,P6 引入;P3 由调用方传入)
/// `task_description`: 任务描述 (来自 trigger 解析)
pub fn build_initial_messages(
    context: &Context,
    role_prompt: &str,
    task_description: &str,
) -> Vec<Message> {
    let system = build_system_prompt(role_prompt, &context.project_config);
    let user = build_user_prompt(context, task_description);
    vec![Message::system(system), Message::user(user)]
}

fn build_system_prompt(role_prompt: &str, project: &crate::config::ProjectConfig) -> String {
    let mut parts = Vec::new();
    parts.push(role_prompt.to_string());
    parts.push(
        "你是 devnpc,基于 GitLab 的研发流程 AI 智能体。遵循项目规范,优先用最小改动解决问题。"
            .to_string(),
    );
    if !project.guidelines_markdown.is_empty() {
        parts.push(format!("\n# 项目规范\n{}", project.guidelines_markdown));
    }
    if !project.forbidden_paths.is_empty() {
        parts.push(format!(
            "\n# 禁止修改的路径\n{}",
            project.forbidden_paths.join("\n")
        ));
    }
    if !project.required_checks.is_empty() {
        parts.push(format!(
            "\n# 提交前必须通过的检查\n{}",
            project.required_checks.join("\n")
        ));
    }
    parts.push(
        "\n# 工作规则\n1. 修改前先用 read_file/list_files 理解上下文\n2. 改完用 run_command 验证\n3. 完成后调 finish 工具,summary 写验收摘要\n4. 禁止访问工作目录外文件"
            .to_string(),
    );
    parts.join("\n\n")
}

fn build_user_prompt(context: &Context, task_description: &str) -> String {
    let mut sections = Vec::new();

    // 仓库结构
    let tree: Vec<String> = context
        .repo_tree
        .entries
        .iter()
        .map(|e| {
            let kind = if e.kind == crate::memory::context::TreeKind::Dir {
                "/"
            } else {
                ""
            };
            format!("{}{}", e.path, kind)
        })
        .collect();
    sections.push(format!("## 仓库结构\n{}", tree.join("\n")));

    // 关键文件摘要
    if !context.key_files.is_empty() {
        let mut files = Vec::new();
        for kf in &context.key_files {
            files.push(format!("### {}\n{}", kf.path, kf.summary));
        }
        sections.push(format!("## 关键文件摘要\n{}", files.join("\n\n")));
    }

    // 目标 Issue
    sections.push(format!(
        "## 目标 Issue #{}\n**标题**: {}\n**描述**: {}\n**状态**: {}",
        context.issue.iid,
        context.issue.title,
        context.issue.description.as_deref().unwrap_or("(无)"),
        context.issue.state
    ));

    // 相关 PR
    if !context.related_prs.is_empty() {
        let prs: Vec<String> = context
            .related_prs
            .iter()
            .map(|mr| format!("!{} {} [{}]", mr.iid, mr.title, mr.state))
            .collect();
        sections.push(format!("## 相关 PR 历史\n{}", prs.join("\n")));
    }

    // Issue 评论
    if !context.issue_notes.is_empty() {
        let notes: Vec<String> = context
            .issue_notes
            .iter()
            .map(|n| format!("- {}: {}", n.author.username, n.body))
            .collect();
        sections.push(format!("## Issue 评论\n{}", notes.join("\n")));
    }

    // 最近提交
    if !context.recent_commits.is_empty() {
        sections.push(format!(
            "## 最近提交\n{}",
            context.recent_commits.join("\n")
        ));
    }

    // CI 失败
    if !context.ci_failures.is_empty() {
        let failures: Vec<String> = context
            .ci_failures
            .iter()
            .map(|f| format!("- pipeline #{}: {}", f.pipeline_id, f.root_cause))
            .collect();
        sections.push(format!("## 已知 CI 失败\n{}", failures.join("\n")));
    }

    // 任务
    sections.push(format!("# 任务\n{}", task_description));

    sections.join("\n\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ProjectConfig;
    use crate::gitlab_api::{Issue, Note, NoteAuthor};
    use crate::memory::context::{KeyFile, RepoTree, TreeEntry, TreeKind};

    fn make_context() -> Context {
        Context {
            repo_tree: RepoTree {
                entries: vec![
                    TreeEntry {
                        path: "src".into(),
                        kind: TreeKind::Dir,
                        size: None,
                    },
                    TreeEntry {
                        path: "src/main.rs".into(),
                        kind: TreeKind::File,
                        size: None,
                    },
                ],
            },
            key_files: vec![KeyFile {
                path: "src/main.rs".into(),
                summary: "fn main() {}".into(),
            }],
            issue: Issue {
                iid: 42,
                title: "登录 bug".into(),
                description: Some("无法登录".into()),
                state: "opened".into(),
                web_url: "https://gl.test/42".into(),
            },
            related_prs: vec![],
            issue_notes: vec![Note {
                id: 1,
                body: "@devnpc 修复".into(),
                author: NoteAuthor {
                    id: 10,
                    username: "alice".into(),
                    name: "Alice".into(),
                },
                created_at: "2026-08-01T10:00:00Z".into(),
            }],
            recent_commits: vec!["abc123 init".into()],
            ci_failures: vec![],
            project_config: ProjectConfig::default(),
        }
    }

    #[test]
    fn build_initial_messages_returns_system_then_user() {
        let ctx = make_context();
        let msgs = build_initial_messages(&ctx, "你是开发 NPC", "修复登录 bug");
        assert_eq!(msgs.len(), 2);
        assert!(matches!(msgs[0], Message::System { .. }));
        assert!(matches!(msgs[1], Message::User { .. }));
    }

    #[test]
    fn system_prompt_includes_role_and_guidelines() {
        let ctx = make_context();
        let mut project = ctx.project_config.clone();
        project.guidelines_markdown = "## 编码约定\n- 禁止 unwrap".into();
        let ctx_with_guidelines = Context {
            project_config: project,
            ..ctx
        };
        let msgs = build_initial_messages(&ctx_with_guidelines, "你是开发 NPC", "任务");
        if let Message::System { content } = &msgs[0] {
            assert!(content.contains("你是开发 NPC"));
            assert!(content.contains("禁止 unwrap"));
        } else {
            panic!("expected System message");
        }
    }

    #[test]
    fn user_prompt_includes_issue_and_task() {
        let ctx = make_context();
        let msgs = build_initial_messages(&ctx, "role", "修复登录 bug");
        if let Message::User { content } = &msgs[1] {
            assert!(content.contains("登录 bug"));
            assert!(content.contains("无法登录"));
            assert!(content.contains("修复登录 bug"));
            assert!(content.contains("src/main.rs"));
        } else {
            panic!("expected User message");
        }
    }

    #[test]
    fn user_prompt_includes_repo_tree_with_dir_marker() {
        let ctx = make_context();
        let msgs = build_initial_messages(&ctx, "role", "task");
        if let Message::User { content } = &msgs[1] {
            // src 是目录,应带 /
            assert!(content.contains("src/"));
        } else {
            panic!("expected User message");
        }
    }
}
