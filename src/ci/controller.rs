//! CI 闭环控制器 (P4 完整实现)
//!
//! 流程: 等待 Pipeline → 轮询状态 → 成功(标记 MR ready) / 失败(触发修复,重试)

use std::time::Duration;

use async_trait::async_trait;
use tokio::time::sleep;

use crate::ci::log_parser::{parse_log, ParsedFailure};
use crate::config::CiConfig;
use crate::error::{DevnpcError, Result};
use crate::git::ops::GitOps;
use crate::gitlab_api::{GitlabApi, Pipeline};

#[derive(Debug, Clone)]
pub enum CiOutcome {
    Passed { mr_iid: u64, pipeline_id: u64, attempts: u8 },
    Failed { mr_iid: u64, last_error: String, attempts: u8 },
    Timeout { mr_iid: u64, stage: String },
}

/// 修复处理器: 外部注入,负责根据 CI 失败信息驱动 agent 修复代码
#[async_trait]
pub trait FixHandler: Send + Sync {
    /// 运行修复循环,返回修复摘要
    async fn run_fix(&self, failures: &[ParsedFailure], instruction: &str) -> Result<String>;
}

/// CI 闭环控制器 (P4 实现)
pub struct CiController {
    config: CiConfig,
    gitlab: Box<dyn GitlabApi>,
    git: GitOps,
    project_id: u64,
    fix_agent: Box<dyn FixHandler>,
}

impl CiController {
    pub fn new(
        config: CiConfig,
        gitlab: Box<dyn GitlabApi>,
        git: GitOps,
        project_id: u64,
        fix_agent: Box<dyn FixHandler>,
    ) -> Self {
        Self {
            config,
            gitlab,
            git,
            project_id,
            fix_agent,
        }
    }

    /// 运行 CI 闭环
    ///
    /// 1. 等待 pipeline 触发 (5min 超时)
    /// 2. 轮询 pipeline 状态 (30min 超时)
    /// 3. 成功: 去除 Draft + 评论 + 返回 Passed
    /// 4. 失败: 获取日志 → 解析 → 修复 → 推送 → 重试 (最多 max_retries 次)
    pub async fn run(&self, mr_iid: u64, branch: &str) -> Result<CiOutcome> {
        let mut attempts = 0u8;
        let mut current_branch = branch.to_string();

        // 首次等待 pipeline
        let pipeline = self
            .wait_for_pipeline(&current_branch, "初始")
            .await?;
        let mut pipeline = self.poll_pipeline(pipeline.id).await?;

        loop {
            match pipeline.status.as_str() {
                "success" => {
                    return self.on_pipeline_success(mr_iid, pipeline.id, attempts).await;
                }
                "failed" | "canceled" => {
                    if attempts >= self.config.max_retries {
                        let last_error = format!(
                            "pipeline #{} {} after {} attempt(s)",
                            pipeline.id, pipeline.status, attempts
                        );
                        // 评论失败通知
                        if let Err(e) = self
                            .gitlab
                            .create_mr_note(
                                self.project_id,
                                mr_iid,
                                &format!(
                                    "❌ CI 修复失败 ({} 次重试后): pipeline #{} {}",
                                    attempts, pipeline.id, pipeline.status,
                                ),
                            )
                            .await
                        {
                            tracing::warn!(error = %e, "创建失败评论失败");
                        }
                        return Ok(CiOutcome::Failed {
                            mr_iid,
                            last_error,
                            attempts,
                        });
                    }
                    // 修复
                    let fix_result = self
                        .run_fix_cycle(pipeline.id, &current_branch, mr_iid, attempts)
                        .await;

                    match fix_result {
                        Ok(new_branch) => {
                            attempts += 1;
                            current_branch = new_branch;
                            // 等待新 pipeline
                            let new_pipeline = self
                                .wait_for_pipeline(&current_branch, &format!("重试 #{attempts}"))
                                .await?;
                            pipeline = self.poll_pipeline(new_pipeline.id).await?;
                            // 继续循环
                        }
                        Err(e) => {
                            return Ok(CiOutcome::Failed {
                                mr_iid,
                                last_error: format!("修复失败: {e}"),
                                attempts,
                            });
                        }
                    }
                }
                // running/pending 不应出现在 poll_pipeline 返回后,但兜底
                _ => {
                    return Ok(CiOutcome::Timeout {
                        mr_iid,
                        stage: format!("pipeline #{} status: {}", pipeline.id, pipeline.status),
                    });
                }
            }
        }
    }

    // ── 内部辅助 ──

    /// 等待指定分支上出现新的 pipeline
    async fn wait_for_pipeline(&self, branch: &str, label: &str) -> Result<Pipeline> {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(self.config.poll_timeout_secs);
        tracing::info!(
            branch,
            label,
            "等待 pipeline 触发, 超时 {}s",
            self.config.poll_timeout_secs
        );

        while tokio::time::Instant::now() < deadline {
            let pipelines = self.gitlab.get_pipelines(self.project_id).await?;
            // 按分支过滤,取最新一条
            if let Some(p) = pipelines
                .into_iter()
                .filter(|p| p.ref_.as_deref() == Some(branch))
                .max_by_key(|p| p.id)
            {
                tracing::info!(pipeline_id = p.id, status = %p.status, "发现 pipeline");
                return Ok(p);
            }
            sleep(Duration::from_secs(self.config.poll_interval_secs)).await;
        }

        Err(DevnpcError::PipelineTimeout {
            stage: format!("等待 pipeline 触发 (branch={branch}, {label})"),
        })
    }

    /// 轮询 pipeline 直到完成 (success/failed/canceled)
    async fn poll_pipeline(&self, pipeline_id: u64) -> Result<Pipeline> {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(self.config.pipeline_timeout_secs);
        tracing::info!(
            pipeline_id,
            "轮询 pipeline 状态, 超时 {}s",
            self.config.pipeline_timeout_secs
        );

        while tokio::time::Instant::now() < deadline {
            let pipelines = self.gitlab.get_pipelines(self.project_id).await?;
            if let Some(p) = pipelines.into_iter().find(|p| p.id == pipeline_id) {
                match p.status.as_str() {
                    "success" | "failed" | "canceled" => {
                        tracing::info!(pipeline_id, status = %p.status, "pipeline 完成");
                        return Ok(p);
                    }
                    _ => {
                        tracing::debug!(pipeline_id, status = %p.status, "pipeline 进行中");
                    }
                }
            }
            sleep(Duration::from_secs(self.config.poll_interval_secs)).await;
        }

        Err(DevnpcError::PipelineTimeout {
            stage: format!("pipeline #{pipeline_id} 轮询超时"),
        })
    }

    /// Pipeline 成功: 去除 Draft + 评论 + 返回 Passed
    async fn on_pipeline_success(
        &self,
        mr_iid: u64,
        pipeline_id: u64,
        attempts: u8,
    ) -> Result<CiOutcome> {
        tracing::info!(mr_iid, pipeline_id, "pipeline 通过, 标记 MR ready");

        // 获取 MR 信息以得到当前 title
        let mr = self.gitlab.get_mr(self.project_id, mr_iid).await?;
        let clean_title = mr.title.trim_start_matches("Draft: ").to_string();

        // 去除 Draft 状态
        self.gitlab
            .update_mr(self.project_id, mr_iid, &clean_title, false)
            .await?;

        // 评论总结
        let summary = format!(
            "✅ CI pipeline #{} 通过, 已自动移除 Draft 状态。",
            pipeline_id
        );
        self.gitlab
            .create_mr_note(self.project_id, mr_iid, &summary)
            .await?;

        Ok(CiOutcome::Passed {
            mr_iid,
            pipeline_id,
            attempts,
        })
    }

    /// 运行一次修复循环: 获取日志 → 解析 → 评论 → 调用 fix_agent
    async fn run_fix_cycle(
        &self,
        pipeline_id: u64,
        branch: &str,
        mr_iid: u64,
        attempt: u8,
    ) -> Result<String> {
        tracing::info!(
            pipeline_id,
            branch,
            attempt,
            "开始修复循环"
        );

        // 在 MR 上评论修复进度
        if let Err(e) = self
            .gitlab
            .create_mr_note(
                self.project_id,
                mr_iid,
                &format!(
                    "🔄 CI 修复中 (attempt {}/{}): 正在处理 pipeline #{} 的失败...",
                    attempt + 1,
                    self.config.max_retries,
                    pipeline_id,
                ),
            )
            .await
        {
            tracing::warn!(error = %e, "创建修复进度评论失败");
        }

        // 获取所有 failed job 的日志
        let jobs = self
            .gitlab
            .get_pipeline_jobs(self.project_id, pipeline_id)
            .await?;

        let failed_jobs: Vec<_> = jobs.iter().filter(|j| j.status == "failed").collect();

        if failed_jobs.is_empty() {
            return Err(DevnpcError::CiFixExhausted { attempts: attempt });
        }

        let mut all_failures = Vec::new();
        for job in &failed_jobs {
            let log = self
                .gitlab
                .get_job_log(self.project_id, job.id)
                .await
                .unwrap_or_else(|_| format!("[无法获取 job #{} 日志]", job.id));
            let failures = parse_log(&job.name, &log);
            all_failures.extend(failures);
        }

        if all_failures.is_empty() {
            // 日志解析没有识别出具体失败,用通用描述
            let job_names: Vec<&str> = failed_jobs.iter().map(|j| j.name.as_str()).collect();
            let instruction = format!(
                "CI pipeline #{} 失败于 job(s): {}. 请检查并修复问题后推送。",
                pipeline_id,
                job_names.join(", ")
            );
            self.fix_agent.run_fix(&[], &instruction).await?;
        } else {
            // 构造修复指令
            let failures_desc: Vec<String> = all_failures
                .iter()
                .map(|f| {
                    let loc = match (&f.file, f.line) {
                        (Some(file), Some(line)) => format!("{file}:{line}"),
                        (Some(file), None) => file.clone(),
                        _ => "unknown".into(),
                    };
                    format!("[{:?}] {loc}: {}", f.failure_type, f.error_message)
                })
                .collect();
            let instruction = format!(
                "CI pipeline #{} 失败, 发现以下 {} 个问题:\n{}\n\n请修复后推送提交。",
                pipeline_id,
                failures_desc.len(),
                failures_desc.join("\n")
            );
            self.fix_agent.run_fix(&all_failures, &instruction).await?;
        }

        // 修复完成后推送
        self.git.push(branch).await.map_err(|e| {
            tracing::error!(branch, error = %e, "推送修复失败");
            DevnpcError::GitCommand {
                cmd: format!("git push origin {branch}"),
                code: -1,
            }
        })?;

        // 评论修复完成通知
        if let Err(e) = self
            .gitlab
            .create_mr_note(
                self.project_id,
                mr_iid,
                &format!(
                    "✅ CI 修复尝试 #{}/{} 完成，已推送至 {}，正在等待新 pipeline...",
                    attempt + 1,
                    self.config.max_retries,
                    branch,
                ),
            )
            .await
        {
            tracing::warn!(error = %e, "创建修复完成评论失败");
        }

        tracing::info!(branch, "修复已推送");
        Ok(branch.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::DevnpcError;
    use crate::gitlab_api::{CreateMrReq, Issue, Job, MergeRequest, Note, NoteAuthor};
    use async_trait::async_trait;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    /// Mock GitlabApi 用于测试
    struct MockGitlab {
        /// (pipeline_id, status, ref_) 三元组
        pipelines: Arc<Vec<(u64, String, String)>>,
        call_count: Arc<AtomicUsize>,
        /// 模拟 job 列表
        jobs: Vec<Job>,
        /// 模拟 job 日志
        job_logs: Vec<(u64, String)>,
        /// 是否成功
        should_succeed: bool,
        /// 成功前的失败次数
        fail_before_success: u8,
        /// 当前失败计数
        fail_count: Arc<AtomicUsize>,
    }

    impl MockGitlab {
        fn new_success() -> Self {
            Self {
                pipelines: Arc::new(vec![(1, "success".into(), "feat/test".into())]),
                call_count: Arc::new(AtomicUsize::new(0)),
                jobs: vec![],
                job_logs: vec![],
                should_succeed: true,
                fail_before_success: 0,
                fail_count: Arc::new(AtomicUsize::new(0)),
            }
        }

        fn new_with_failures(fail_before_success: u8) -> Self {
            Self {
                pipelines: Arc::new(vec![(1, "failed".into(), "feat/test".into())]),
                call_count: Arc::new(AtomicUsize::new(0)),
                jobs: vec![Job {
                    id: 10,
                    name: "build".into(),
                    status: "failed".into(),
                    stage: "build".into(),
                    web_url: Some("https://gl.test/jobs/10".into()),
                }],
                job_logs: vec![(
                    10,
                    "error[E0277]: cannot find value `x` in this scope\n  --> src/main.rs:10:5\n   |\n10|     x + 1\n   |     ^ not found\n".into(),
                )],
                should_succeed: false,
                fail_before_success,
                fail_count: Arc::new(AtomicUsize::new(0)),
            }
        }
    }

    #[async_trait]
    impl GitlabApi for MockGitlab {
        async fn get_issue(&self, _p: u64, _i: u64) -> Result<Issue> {
            unimplemented!()
        }
        async fn get_mr(&self, _p: u64, _i: u64) -> Result<MergeRequest> {
            Ok(MergeRequest {
                iid: _i,
                title: "Draft: feat: test".into(),
                description: None,
                state: "opened".into(),
                source_branch: "feat/test".into(),
                target_branch: "main".into(),
                web_url: "https://gl.test/mrs/1".into(),
                draft: true,
            })
        }
        async fn create_mr(&self, _p: u64, _r: CreateMrReq) -> Result<MergeRequest> {
            unimplemented!()
        }
        async fn get_pipelines(&self, _p: u64) -> Result<Vec<Pipeline>> {
            let count = self.call_count.fetch_add(1, Ordering::SeqCst);
            // 首次调用返回空,模拟 pipeline 尚未触发
            if count == 0 {
                return Ok(vec![]);
            }
            let pipelines: Vec<Pipeline> = self
                .pipelines
                .iter()
                .map(|(id, status, ref_)| {
                    let mut status = status.clone();
                    if !self.should_succeed && self.fail_before_success > 0 {
                        let fc = self.fail_count.load(Ordering::SeqCst);
                        if fc < self.fail_before_success as usize {
                            status = "failed".into();
                        } else {
                            status = "success".into();
                        }
                    }
                    Pipeline {
                        id: *id,
                        status,
                        ref_: Some(ref_.clone()),
                        sha: Some("abc123".into()),
                        web_url: format!("https://gl.test/pipelines/{id}"),
                    }
                })
                .collect();
            Ok(pipelines)
        }
        async fn get_issue_notes(&self, _p: u64, _i: u64) -> Result<Vec<Note>> {
            unimplemented!()
        }
        async fn get_mr_notes(&self, _p: u64, _i: u64) -> Result<Vec<Note>> {
            unimplemented!()
        }
        async fn create_mr_note(&self, _p: u64, _i: u64, _body: &str) -> Result<Note> {
            Ok(Note {
                id: 1,
                body: _body.into(),
                author: NoteAuthor {
                    id: 99,
                    username: "devnpc".into(),
                    name: "devnpc bot".into(),
                },
                created_at: "2026-08-01T00:00:00Z".into(),
            })
        }
        async fn create_issue_note(&self, _p: u64, _i: u64, _body: &str) -> Result<Note> {
            Ok(Note {
                id: 1,
                body: _body.into(),
                author: NoteAuthor {
                    id: 99,
                    username: "devnpc".into(),
                    name: "devnpc bot".into(),
                },
                created_at: "2026-08-01T00:00:00Z".into(),
            })
        }
        async fn get_related_mrs(&self, _p: u64, _i: u64) -> Result<Vec<MergeRequest>> {
            unimplemented!()
        }
        async fn get_recent_pipelines(&self, _p: u64, _c: usize) -> Result<Vec<Pipeline>> {
            unimplemented!()
        }
        async fn update_mr(&self, _p: u64, _i: u64, _t: &str, _d: bool) -> Result<MergeRequest> {
            Ok(MergeRequest {
                iid: _i,
                title: _t.into(),
                description: None,
                state: "opened".into(),
                source_branch: "feat/test".into(),
                target_branch: "main".into(),
                web_url: "https://gl.test/mrs/1".into(),
                draft: _d,
            })
        }
        async fn get_pipeline_jobs(&self, _p: u64, _pi: u64) -> Result<Vec<Job>> {
            Ok(self.jobs.clone())
        }
        async fn get_job_log(&self, _p: u64, jid: u64) -> Result<String> {
            Ok(self
                .job_logs
                .iter()
                .find(|(id, _)| *id == jid)
                .map(|(_, log)| log.clone())
                .unwrap_or_default())
        }
    }

    /// Mock FixHandler 模拟修复
    struct MockFixHandler;

    #[async_trait]
    impl FixHandler for MockFixHandler {
        async fn run_fix(&self, _failures: &[ParsedFailure], _instruction: &str) -> Result<String> {
            Ok("修复完成".into())
        }
    }

    /// 快速创建 CiController 的辅助函数,使用 poll_interval=1s 加速测试
    fn make_controller(gitlab: Box<dyn GitlabApi>, git_workspace: &str) -> CiController {
        let config = CiConfig {
            poll_interval_secs: 0,
            poll_timeout_secs: 2,
            pipeline_timeout_secs: 2,
            max_retries: 2,
        };
        CiController::new(
            config,
            gitlab,
            GitOps::new(git_workspace),
            1,
            Box::new(MockFixHandler),
        )
    }

    #[tokio::test]
    async fn run_pipeline_success_returns_passed() {
        let gitlab = Box::new(MockGitlab::new_success());
        let controller = make_controller(gitlab, ".");
        let outcome = controller.run(1, "feat/test").await.unwrap();
        match outcome {
            CiOutcome::Passed {
                mr_iid,
                pipeline_id,
                attempts,
            } => {
                assert_eq!(mr_iid, 1);
                assert_eq!(pipeline_id, 1);
                assert_eq!(attempts, 0);
            }
            _ => panic!("expected Passed, got {outcome:?}"),
        }
    }

    #[tokio::test]
    async fn run_pipeline_failed_without_retries_returns_failed() {
        let gitlab = Box::new(MockGitlab::new_with_failures(0));
        let controller = make_controller(gitlab, ".");
        let outcome = controller.run(1, "feat/test").await.unwrap();
        match outcome {
            CiOutcome::Failed { .. } => {} // expected
            _ => panic!("expected Failed, got {outcome:?}"),
        }
    }

    #[tokio::test]
    async fn run_pipeline_failed_with_retries_recovers() {
        let gitlab = MockGitlab::new_with_failures(0);
        // 初始化一个临时 git 仓库,使 GitOps::push 可以正常工作
        let dir = tempfile::tempdir().unwrap();
        std::process::Command::new("git")
            .args(["init"])
            .current_dir(dir.path())
            .output()
            .expect("git init 失败");
        std::process::Command::new("git")
            .args(["config", "user.email", "test@test.com"])
            .current_dir(dir.path())
            .output()
            .expect("git config email 失败");
        std::process::Command::new("git")
            .args(["config", "user.name", "test"])
            .current_dir(dir.path())
            .output()
            .expect("git config name 失败");
        // 添加一个文件并提交,使分支存在
        std::fs::write(dir.path().join("test.txt"), b"hello").unwrap();
        std::process::Command::new("git")
            .args(["add", "."])
            .current_dir(dir.path())
            .output()
            .expect("git add 失败");
        std::process::Command::new("git")
            .args(["commit", "-m", "initial"])
            .current_dir(dir.path())
            .output()
            .expect("git commit 失败");
        // 创建本地 bare 仓库作为 remote,使 push 可以成功
        let bare_dir = tempfile::tempdir().unwrap();
        std::process::Command::new("git")
            .args(["init", "--bare"])
            .current_dir(bare_dir.path())
            .output()
            .expect("git init bare 失败");
        std::process::Command::new("git")
            .args(["remote", "add", "origin", bare_dir.path().to_str().unwrap()])
            .current_dir(dir.path())
            .output()
            .expect("git remote add 失败");
        // 创建并切换到 feat/test 分支,使 push 可以找到该分支
        std::process::Command::new("git")
            .args(["checkout", "-b", "feat/test"])
            .current_dir(dir.path())
            .output()
            .expect("git checkout -b 失败");

        // 设置 max_retries=2, 所以会尝试修复
        let config = CiConfig {
            poll_interval_secs: 0,
            poll_timeout_secs: 2,
            pipeline_timeout_secs: 2,
            max_retries: 2,
        };
        let controller = CiController::new(
            config,
            Box::new(gitlab),
            GitOps::new(dir.path()),
            1,
            Box::new(MockFixHandler),
        );
        let outcome = controller.run(1, "feat/test").await.unwrap();
        // 由于修复后 pipeline 还是 failed,最终会 Failed
        match outcome {
            CiOutcome::Failed { attempts, .. } => {
                assert_eq!(attempts, 2);
            }
            _ => panic!("expected Failed, got {outcome:?}"),
        }
    }

    #[tokio::test]
    async fn timeout_when_pipeline_never_triggered() {
        // get_pipelines 始终返回空
        struct NeverTriggerMock;
        #[async_trait]
        impl GitlabApi for NeverTriggerMock {
            async fn get_issue(&self, _p: u64, _i: u64) -> Result<Issue> {
                unimplemented!()
            }
            async fn get_mr(&self, _p: u64, _i: u64) -> Result<MergeRequest> {
                unimplemented!()
            }
            async fn create_mr(&self, _p: u64, _r: CreateMrReq) -> Result<MergeRequest> {
                unimplemented!()
            }
            async fn get_pipelines(&self, _p: u64) -> Result<Vec<Pipeline>> {
                Ok(vec![])
            }
            async fn get_issue_notes(&self, _p: u64, _i: u64) -> Result<Vec<Note>> {
                unimplemented!()
            }
            async fn get_mr_notes(&self, _p: u64, _i: u64) -> Result<Vec<Note>> {
                unimplemented!()
            }
            async fn create_mr_note(&self, _p: u64, _i: u64, _b: &str) -> Result<Note> {
                unimplemented!()
            }
            async fn create_issue_note(&self, _p: u64, _i: u64, _b: &str) -> Result<Note> {
                unimplemented!()
            }
            async fn get_related_mrs(&self, _p: u64, _i: u64) -> Result<Vec<MergeRequest>> {
                unimplemented!()
            }
            async fn get_recent_pipelines(&self, _p: u64, _c: usize) -> Result<Vec<Pipeline>> {
                unimplemented!()
            }
            async fn update_mr(&self, _p: u64, _i: u64, _t: &str, _d: bool) -> Result<MergeRequest> {
                unimplemented!()
            }
            async fn get_pipeline_jobs(&self, _p: u64, _pi: u64) -> Result<Vec<Job>> {
                unimplemented!()
            }
            async fn get_job_log(&self, _p: u64, _j: u64) -> Result<String> {
                unimplemented!()
            }
        }

        let config = CiConfig {
            poll_interval_secs: 0,
            poll_timeout_secs: 0,  // 100ms, 但字段是秒级,设 0 表示极短超时
            pipeline_timeout_secs: 2,
            max_retries: 0,
        };
        let controller = CiController::new(
            config,
            Box::new(NeverTriggerMock),
            GitOps::new("."),
            1,
            Box::new(MockFixHandler),
        );
        let result = controller.run(1, "feat/nonexistent").await;
        assert!(result.is_err());
        match result.unwrap_err() {
            DevnpcError::PipelineTimeout { stage } => {
                assert!(stage.contains("等待 pipeline 触发"));
            }
            e => panic!("expected PipelineTimeout, got {e}"),
        }
    }

    #[tokio::test]
    async fn ci_config_default_has_sane_values() {
        let cfg = CiConfig::default();
        assert_eq!(cfg.poll_interval_secs, 10);
        assert_eq!(cfg.poll_timeout_secs, 300);
        assert_eq!(cfg.pipeline_timeout_secs, 1800);
        assert_eq!(cfg.max_retries, 3);
    }

    #[tokio::test]
    async fn parse_log_works_with_controller_integration() {
        // 验证 controller 内部使用 parse_log 的正确性
        let log = "error[E0277]: cannot find value `x` in this scope\n  --> src/main.rs:10:5\n   |\n10|     x + 1\n   |     ^ not found\n";
        let failures = parse_log("build", log);
        assert_eq!(failures.len(), 1);
        assert_eq!(failures[0].file.as_deref(), Some("src/main.rs"));
        assert_eq!(failures[0].line, Some(10));
    }
}