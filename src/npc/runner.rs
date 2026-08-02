//! 单 NPC 执行器 (P5 完整实现)
//!
//! 完整流程: 创建分支 → 构建 Agent 消息 → ToolRegistry → ReactLoop → 提交+推送

use crate::agent::loop_::{ReactLoop, RunResult, Trajectory};
use crate::agent::prompt::build_initial_messages;
use crate::config::Config;
use crate::error::Result;
use crate::git::ops::GitOps;
use crate::memory::context::Context;
use crate::npc::role::Role;
use crate::tools::aft::{
    AftAstReplaceTool, AftEditSymbolTool, AftOutlineTool, AftSearchSymbolsTool, AftViewSymbolTool,
};
use crate::tools::file_io::{FileIo, ListFilesTool, ReadFileTool, WriteFileTool};
use crate::tools::finish::FinishTool;
use crate::tools::git_tool::{GitCommitTool, GitDiffTool};
use crate::tools::shell::RunCommandTool;
use crate::tools::ToolRegistry;
use crate::trigger::parser::TaskSpec;

/// NPC 执行结果
pub struct NpcResult {
    pub summary: String,
    pub trajectory: Trajectory,
    pub branch: String,
}

/// NPC 执行器
pub struct NpcRunner {
    pub role: Role,
}

impl NpcRunner {
    pub fn new(role: Role) -> Self {
        Self { role }
    }

    /// 执行任务 (P5 完整实现)
    ///
    /// 1. 创建 git 分支 (npc/<slug>)
    /// 2. 构建初始消息 (prompt::build_initial_messages)
    /// 3. 注册所有工具 (file_io, shell, git, finish, AFT)
    /// 4. 创建 ReactLoop 并运行
    /// 5. Agent 完成后: git commit + push
    /// 6. 返回 NpcResult
    pub async fn execute(
        &self,
        task_spec: &TaskSpec,
        context: &Context,
        config: &Config,
    ) -> Result<NpcResult> {
        let workspace = std::env::current_dir()?;
        let git_ops = GitOps::new(&workspace);

        // 1. 创建 git 分支 (带角色前缀)
        let branch_slug = slugify(&task_spec.description);
        let branch = format!("{}/{}-{}", config.project.branch_prefix, self.role.name.to_lowercase(), branch_slug);
        tracing::info!(branch = %branch, "创建新分支");
        git_ops.checkout_branch(&branch).await?;

        // 2. 构建初始消息 (含 workspace/branch/验收标准)
        let workspace_str = workspace.to_string_lossy();
        let messages = build_initial_messages(
            context,
            &self.role.system_prompt,
            &task_spec.description,
            &workspace_str,
            &branch,
            &task_spec.acceptance_criteria,
        );

        // 3. 创建 ToolRegistry 并注册所有工具
        let mut tools = ToolRegistry::new();
        // 自建文件工具
        tools.register(Box::new(ReadFileTool::new(FileIo::new(&workspace), config.read_file.clone())));
        tools.register(Box::new(WriteFileTool::new(FileIo::new(&workspace))));
        tools.register(Box::new(ListFilesTool::new(FileIo::new(&workspace))));
        // Git 工具
        tools.register(Box::new(GitDiffTool::new(&workspace)));
        tools.register(Box::new(GitCommitTool::new(&workspace)));
        // Shell 工具
        tools.register(Box::new(RunCommandTool::new(&workspace, config.command.clone())));
        // AFT 代码感知工具
        tools.register(Box::new(AftOutlineTool::new(FileIo::new(&workspace))));
        tools.register(Box::new(AftViewSymbolTool::new(FileIo::new(&workspace))));
        tools.register(Box::new(AftEditSymbolTool::new(FileIo::new(&workspace))));
        tools.register(Box::new(AftSearchSymbolsTool::new(FileIo::new(&workspace))));
        tools.register(Box::new(AftAstReplaceTool::new(FileIo::new(&workspace))));
        // Finish 工具
        tools.register(Box::new(FinishTool::new()));

        // 4. 创建 ReactLoop 并运行
        let llm_client = crate::agent::llm_client::LlmClient::new(config.llm.clone());
        let react = ReactLoop::new(config.limits.max_iterations, llm_client, config.project.sop_mode);
        tracing::info!("启动 ReactLoop, 最大迭代次数: {}", config.limits.max_iterations);

        let run_result = react.run(messages, &tools, None).await?;

        match run_result {
            RunResult::Finished { summary, trajectory, .. } => {
                tracing::info!(summary = %summary, "Agent 任务完成");
                // 5. Git commit + push
                git_ops.commit(&format!("devnpc: {}", summary)).await?;
                git_ops.push(&branch).await?;
                tracing::info!(branch = %branch, "代码已推送");

                Ok(NpcResult {
                    summary,
                    trajectory,
                    branch,
                })
            }
            RunResult::MaxIterationsReached(trajectory) => {
                tracing::warn!("NPC 达到迭代上限,尝试提交当前进度");
                // 尝试提交(若无变更会失败,忽略)
                if let Err(e) = git_ops.commit("devnpc: 部分完成 (迭代上限)").await {
                    tracing::warn!(error = %e, "提交当前进度失败,可能无变更");
                }
                if let Err(e) = git_ops.push(&branch).await {
                    tracing::warn!(error = %e, "推送分支失败");
                }

                Ok(NpcResult {
                    summary: "达到迭代上限,部分完成".into(),
                    trajectory,
                    branch,
                })
            }
            RunResult::SopViolation {
                step,
                unexpected_tools,
                trajectory,
            } => {
                tracing::error!(
                    step = %step,
                    tools = ?unexpected_tools,
                    "SOP 严格模式偏离,终止循环"
                );
                Ok(NpcResult {
                    summary: format!("SOP 偏离于步骤 [{step}],不允许的工具: {unexpected_tools:?}"),
                    trajectory,
                    branch,
                })
            }
        }
    }
}

/// 将文本转为 git 分支名友好的 slug
fn slugify(text: &str) -> String {
    let slug: String = text
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' || c == '.' {
                c
            } else {
                '-'
            }
        })
        .collect();
    // 合并连续短横线,去首尾短横线
    let mut result = String::with_capacity(slug.len());
    let mut prev_hyphen = false;
    for c in slug.chars() {
        if c == '-' {
            if prev_hyphen {
                continue;
            }
            prev_hyphen = true;
        } else {
            prev_hyphen = false;
        }
        result.push(c);
    }
    let result = result.trim_matches('-').to_string();
    // 限制长度,避免分支名过长
    if result.len() > 80 {
        result[..80].to_string()
    } else {
        result
    }
    .to_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugify_replaces_spaces_with_hyphens() {
        let s = slugify("修复登录 bug");
        assert_eq!(s, "修复登录-bug");
    }

    #[test]
    fn slugify_removes_leading_trailing_hyphens() {
        let s = slugify("!fix! crash!");
        assert_eq!(s, "fix-crash");
    }

    #[test]
    fn slugify_collapses_consecutive_hyphens() {
        let s = slugify("fix  crash  now");
        assert_eq!(s, "fix-crash-now");
    }

    #[test]
    fn slugify_truncates_long_strings() {
        let long = "a".repeat(200);
        let s = slugify(&long);
        assert!(s.len() <= 80);
    }

    #[test]
    fn slugify_preserves_alphanumeric_and_dot() {
        let s = slugify("feat: add login.v2");
        assert_eq!(s, "feat-add-login.v2");
    }
}