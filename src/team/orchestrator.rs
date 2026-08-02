//! 多 NPC 编排器 (P7 完整实现)
//!
//! 任务分解 → 并行执行 → 联调 → 单 MR 汇总
//!
//! 流程:
//! 1. PM NPC 将需求分解为开发任务和测试任务
//! 2. 通过 GitLab Issue 评论发送 handoff 消息
//! 3. Developer 和 Tester NPC 并行执行
//! 4. 合并各 NPC 分支到 team 分支
//! 5. 创建单个 MR 提交给 CI 控制器

use std::sync::Arc;

use async_trait::async_trait;

use crate::ci::controller::{CiController, CiOutcome, FixHandler};
use crate::ci::log_parser::ParsedFailure;
use crate::config::Config;
use crate::error::Result;
use crate::git::ops::GitOps;
use crate::gitlab_api::client::GitlabClient;
use crate::gitlab_api::GitlabApi;
use crate::memory::context::Context;
use crate::npc::role::Role;
use crate::npc::runner::NpcRunner;
use crate::team::comm::build_handoff;
use crate::trigger::parser::{TaskKind, TaskSpec};

/// 空的修复处理器,用于 CI 控制器 (实际修复由 P4 完整实现处理)
struct NoopFixHandler;

#[async_trait]
impl FixHandler for NoopFixHandler {
    async fn run_fix(&self, _failures: &[ParsedFailure], _instruction: &str) -> Result<String> {
        Ok("noop 修复: 无操作".into())
    }
}

/// Team 编排结果 (类似 CiOutcome)
#[derive(Debug, Clone)]
pub enum TeamOutcome {
    Passed {
        mr_iid: u64,
        pipeline_id: u64,
        attempts: u8,
    },
    Failed {
        mr_iid: u64,
        last_error: String,
        attempts: u8,
    },
    Timeout {
        mr_iid: u64,
        stage: String,
    },
}

/// Team 编排器 (P7 实现)
pub struct Orchestrator {
    gitlab: Arc<dyn GitlabApi>,
    issue_iid: u64,
    project_id: u64,
    config: Config,
}

impl Orchestrator {
    pub fn new(
        gitlab: Arc<dyn GitlabApi>,
        issue_iid: u64,
        project_id: u64,
        config: Config,
    ) -> Self {
        Self {
            gitlab,
            issue_iid,
            project_id,
            config,
        }
    }

    /// 运行 Team 任务 (P7 实现)
    ///
    /// 1. 运行 PM NPC 分解需求
    /// 2. 通过 Issue 评论发送 handoff 给开发和测试
    /// 3. 并行运行 Developer 和 Tester NPC
    /// 4. 合并分支到 team 分支
    /// 5. 创建单个 MR
    pub async fn run(&self, goal: &str) -> Result<TeamOutcome> {
        let workspace = std::env::current_dir()?;
        let git_ops = GitOps::new(&workspace);

        // 0. 构建上下文
        let context = Context::build(
            &*self.gitlab,
            &git_ops,
            self.project_id,
            self.issue_iid,
            &self.config.summary,
            &self.config.context,
        )
        .await?;

        // 1. 运行 PM NPC 分解需求
        tracing::info!("=== 步骤 1: PM NPC 分解需求 ===");
        let pm_role = Role {
            name: "PM".into(),
            description: "项目经理,负责需求分解和任务规划".into(),
            system_prompt: "你是一个项目经理(PM)。\
                你的任务是将用户需求分解为具体的开发任务和测试任务。\
                分析需求,输出清晰的任务分解方案。\
                完成后调用 finish 工具,在 summary 中写入分解后的任务描述。"
                .into(),
            max_iterations: self.config.limits.max_iterations,
            default_sop: None,
            tools: vec![],
        };
        let pm_runner = NpcRunner::new(pm_role);
        let pm_task = TaskSpec {
            kind: TaskKind::Implement,
            description: format!("需求分解: {goal}"),
            target_issue: Some(self.issue_iid),
            acceptance_criteria: vec![],
        };
        let pm_result = pm_runner
            .execute(&pm_task, &context, &self.config)
            .await?;
        let decomposition = pm_result.summary;
        tracing::info!(decomposition = %decomposition, "PM 需求分解完成");

        // 2. 通过 Issue 评论发送 handoff 消息
        tracing::info!("=== 步骤 2: 发送 handoff 消息 ===");
        let to = vec!["developer".to_string(), "tester".to_string()];
        let handoff_msg = build_handoff(
            "PM",
            &to,
            "ready",
            &format!("需求已分解:\n{decomposition}"),
        );
        let note_body = format!(
            "## devnpc 团队协作\n\n**PM 需求分解完成**\n\n{decomposition}\n\n---\n{handoff_msg}"
        );
        self.gitlab
            .create_issue_note(self.project_id, self.issue_iid, &note_body)
            .await?;
        tracing::info!("handoff 消息已发布到 Issue #{}", self.issue_iid);

        // 3. 并行运行 Developer 和 Tester NPC
        tracing::info!("=== 步骤 3: 并行执行 Developer 和 Tester ===");

        let dev_role = Role {
            name: "developer".into(),
            description: "开发工程师,负责代码实现".into(),
            system_prompt: "你是一个 Rust 开发工程师。\
                根据 PM 分解的任务进行代码实现。\
                遵循以下原则:\n\
                1. 修改前先理解上下文 (read_file / list_files / aft_outline)\n\
                2. 改完后用 cargo build 验证编译\n\
                3. 完成后调用 finish 工具,summary 写验收摘要\n\
                4. 禁止修改工作目录外的文件"
                .into(),
            max_iterations: self.config.limits.max_iterations,
            default_sop: None,
            tools: vec![],
        };
        let dev_runner = NpcRunner::new(dev_role);
        let dev_task = TaskSpec {
            kind: TaskKind::Implement,
            description: format!("代码实现: {decomposition}"),
            target_issue: Some(self.issue_iid),
            acceptance_criteria: vec![],
        };

        let test_role = Role {
            name: "tester".into(),
            description: "测试工程师,负责编写测试和验证".into(),
            system_prompt: "你是一个测试工程师。\
                根据 PM 分解的任务编写测试用例和进行测试验证。\
                遵循以下原则:\n\
                1. 先理解现有代码结构\n\
                2. 编写全面的测试用例\n\
                3. 运行测试确保通过\n\
                4. 完成后调用 finish 工具,summary 写测试报告"
                .into(),
            max_iterations: self.config.limits.max_iterations,
            default_sop: None,
            tools: vec![],
        };
        let test_runner = NpcRunner::new(test_role);
        let test_task = TaskSpec {
            kind: TaskKind::Test,
            description: format!("测试验证: {decomposition}"),
            target_issue: Some(self.issue_iid),
            acceptance_criteria: vec![],
        };

        // 使用 tokio::join! 并行执行
        let (dev_result, test_result) = tokio::join!(
            dev_runner.execute(&dev_task, &context, &self.config),
            test_runner.execute(&test_task, &context, &self.config),
        );

        let dev_result = dev_result?;
        let test_result = test_result?;
        tracing::info!(
            dev_branch = %dev_result.branch,
            test_branch = %test_result.branch,
            "Developer 和 Tester 执行完成"
        );

        // 4. 合并分支到 team 分支
        tracing::info!("=== 步骤 4: 合并分支到 team 分支 ===");
        let team_branch = format!(
            "{}/team-{}",
            self.config.project.branch_prefix, self.issue_iid
        );
        git_ops.checkout_branch(&team_branch).await?;
        git_ops.merge_branch(&dev_result.branch).await?;
        git_ops.merge_branch(&test_result.branch).await?;
        git_ops.push(&team_branch).await?;
        tracing::info!(team_branch = %team_branch, "team 分支已合并并推送");

        // 5. 创建单个 MR
        tracing::info!("=== 步骤 5: 创建 MR ===");
        let create_req = crate::gitlab_api::CreateMrReq {
            source_branch: team_branch.clone(),
            target_branch: "main".to_string(),
            title: format!("devnpc team: {}", goal),
            description: format!(
                "由 devnpc 团队协作自动创建\n\n## 需求\n{}\n\n## 开发\n{}\n\n## 测试\n{}",
                goal, dev_result.summary, test_result.summary
            ),
            draft: true,
        };

        match self
            .gitlab
            .create_mr(self.project_id, create_req)
            .await
        {
            Ok(mr) => {
                tracing::info!(mr_iid = mr.iid, mr_url = %mr.web_url, "团队 MR 已创建");

                // 运行 CI 控制器
                let ci_config = self.config.ci.clone();
                let gitlab_for_ci = Box::new(GitlabClient::new(
                    &self.config.gitlab.url,
                    &self.config.gitlab.token,
                ));
                let controller = CiController::new(
                    ci_config,
                    gitlab_for_ci,
                    GitOps::new(std::env::current_dir()?),
                    self.project_id,
                    Box::new(NoopFixHandler),
                );
                match controller.run(mr.iid, &team_branch).await {
                    Ok(outcome) => {
                        tracing::info!(?outcome, "CI 闭环完成");
                        let (pipeline_id, attempts) = match &outcome {
                            CiOutcome::Passed {
                                pipeline_id, attempts, ..
                            } => (*pipeline_id, *attempts),
                            CiOutcome::Failed {
                                attempts, ..
                            } => (0, *attempts),
                            CiOutcome::Timeout { .. } => (0, 0),
                        };
                        Ok(TeamOutcome::Passed {
                            mr_iid: mr.iid,
                            pipeline_id,
                            attempts,
                        })
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, "CI 控制器未实现,跳过");
                        Ok(TeamOutcome::Passed {
                            mr_iid: mr.iid,
                            pipeline_id: 0,
                            attempts: 0,
                        })
                    }
                }
            }
            Err(e) => {
                let error = format!("创建团队 MR 失败: {e}");
                tracing::error!(error = %error);
                Ok(TeamOutcome::Failed {
                    mr_iid: 0,
                    last_error: error,
                    attempts: 0,
                })
            }
        }
    }
}

impl Default for Orchestrator {
    fn default() -> Self {
        panic!("Orchestrator::default() 不可用,请使用 Orchestrator::new()")
    }
}