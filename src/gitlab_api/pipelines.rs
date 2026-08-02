//! Pipeline 操作高层 helper
//!
//! 在 `GitlabApi` trait 的 `get_pipelines` / `get_pipeline` (单条) /
//! `get_pipeline_jobs` / `get_job_log` 之上,提供:
//! - 失败 pipeline 过滤
//! - 批量拉取 pipeline 下所有 job 日志
//! - Pipeline 状态判断

use crate::error::Result;
use crate::gitlab_api::{GitlabApi, Job, Pipeline};

/// Job + 其日志文本
#[derive(Debug, Clone)]
pub struct JobLog {
    pub job: Job,
    pub log: String,
}

/// 从 pipeline 列表中过滤出 failed 状态,取前 `count` 条 (按 id 倒序)
pub fn filter_failed(pipelines: &[Pipeline], count: usize) -> Vec<Pipeline> {
    let mut failed: Vec<Pipeline> = pipelines
        .iter()
        .filter(|p| p.status == "failed")
        .cloned()
        .collect();
    // 按 id 倒序,最新的失败优先
    failed.sort_by_key(|p| std::cmp::Reverse(p.id));
    failed.truncate(count);
    failed
}

/// 拉取 pipeline 下所有 job 的日志
///
/// 顺序: 先取 job 列表,再并行对每个 job 取日志。
/// 单个 job 日志获取失败时,日志字段记录错误占位,不中断整体流程。
pub async fn fetch_job_logs(
    gitlab: &dyn GitlabApi,
    project_id: u64,
    pipeline_id: u64,
) -> Result<Vec<JobLog>> {
    let jobs = gitlab.get_pipeline_jobs(project_id, pipeline_id).await?;

    // 并行拉取所有 job 日志 (gitlab trait 对象是 Send + Sync,可安全共享引用)
    let tasks: Vec<_> = jobs
        .into_iter()
        .map(|job| async move {
            let log = match gitlab.get_job_log(project_id, job.id).await {
                Ok(l) => l,
                Err(e) => format!("[无法获取 job #{} 日志: {}]", job.id, e),
            };
            JobLog { job, log }
        })
        .collect();
    let result = futures::future::join_all(tasks).await;
    Ok(result)
}

/// 仅拉取 pipeline 下 failed job 的日志 (用于 CI 修复场景)
pub async fn fetch_failed_job_logs(
    gitlab: &dyn GitlabApi,
    project_id: u64,
    pipeline_id: u64,
) -> Result<Vec<JobLog>> {
    let all = fetch_job_logs(gitlab, project_id, pipeline_id).await?;
    Ok(all.into_iter().filter(|jl| jl.job.status == "failed").collect())
}

/// 判断 pipeline 是否处于终态 (success/failed/canceled)
pub fn is_terminal(pipeline: &Pipeline) -> bool {
    matches!(pipeline.status.as_str(), "success" | "failed" | "canceled")
}

/// 判断 pipeline 是否成功
pub fn is_success(pipeline: &Pipeline) -> bool {
    pipeline.status == "success"
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gitlab_api::{
        CreateMrReq, Issue, MergeRequest, Note, RepoTreeEntry,
    };
    use async_trait::async_trait;
    use std::sync::Mutex;

    fn make_pipeline(id: u64, status: &str) -> Pipeline {
        Pipeline {
            id,
            status: status.into(),
            ref_: Some("main".into()),
            sha: Some("abc".into()),
            web_url: format!("https://gl.test/p/{id}"),
        }
    }

    fn make_job(id: u64, name: &str, status: &str) -> Job {
        Job {
            id,
            name: name.into(),
            status: status.into(),
            stage: "test".into(),
            web_url: Some(format!("https://gl.test/jobs/{id}")),
        }
    }

    /// 可注入 job + 日志的 Mock
    struct PipelineMock {
        jobs: Vec<Job>,
        logs: std::collections::HashMap<u64, String>,
        // 记录 get_job_log 调用次数 (验证并行/顺序)
        log_calls: Mutex<u32>,
    }

    #[async_trait]
    impl GitlabApi for PipelineMock {
        async fn get_issue(&self, _p: u64, _i: u64) -> Result<Issue> {
            unreachable!()
        }
        async fn get_mr(&self, _p: u64, _i: u64) -> Result<MergeRequest> {
            unreachable!()
        }
        async fn create_mr(&self, _p: u64, _r: CreateMrReq) -> Result<MergeRequest> {
            unreachable!()
        }
        async fn get_pipelines(&self, _p: u64) -> Result<Vec<Pipeline>> {
            Ok(vec![])
        }
        async fn get_issue_notes(&self, _p: u64, _i: u64) -> Result<Vec<Note>> {
            Ok(vec![])
        }
        async fn get_mr_notes(&self, _p: u64, _i: u64) -> Result<Vec<Note>> {
            Ok(vec![])
        }
        async fn create_mr_note(&self, _p: u64, _i: u64, _b: &str) -> Result<Note> {
            unreachable!()
        }
        async fn create_issue_note(&self, _p: u64, _i: u64, _b: &str) -> Result<Note> {
            unreachable!()
        }
        async fn get_related_mrs(&self, _p: u64, _i: u64) -> Result<Vec<MergeRequest>> {
            Ok(vec![])
        }
        async fn get_recent_pipelines(&self, _p: u64, _c: usize) -> Result<Vec<Pipeline>> {
            Ok(vec![])
        }
        async fn update_mr(&self, _p: u64, _i: u64, _t: &str, _d: bool) -> Result<MergeRequest> {
            unreachable!()
        }
        async fn get_pipeline_jobs(&self, _p: u64, _pi: u64) -> Result<Vec<Job>> {
            Ok(self.jobs.clone())
        }
        async fn get_job_log(&self, _p: u64, jid: u64) -> Result<String> {
            *self.log_calls.lock().unwrap() += 1;
            Ok(self.logs.get(&jid).cloned().unwrap_or_default())
        }
        async fn get_pipeline(&self, _p: u64, _pi: u64) -> Result<Pipeline> {
            Ok(make_pipeline(1, "success"))
        }
        async fn get_file(&self, _p: u64, _fp: &str, _r: &str) -> Result<String> {
            Ok(String::new())
        }
        async fn list_tree(&self, _p: u64, _path: &str, _r: &str) -> Result<Vec<RepoTreeEntry>> {
            Ok(vec![])
        }
    }

    /// 日志总是失败的 Mock (验证错误占位)
    struct FailLogMock;
    #[async_trait]
    impl GitlabApi for FailLogMock {
        async fn get_issue(&self, _p: u64, _i: u64) -> Result<Issue> {
            unreachable!()
        }
        async fn get_mr(&self, _p: u64, _i: u64) -> Result<MergeRequest> {
            unreachable!()
        }
        async fn create_mr(&self, _p: u64, _r: CreateMrReq) -> Result<MergeRequest> {
            unreachable!()
        }
        async fn get_pipelines(&self, _p: u64) -> Result<Vec<Pipeline>> {
            Ok(vec![])
        }
        async fn get_issue_notes(&self, _p: u64, _i: u64) -> Result<Vec<Note>> {
            Ok(vec![])
        }
        async fn get_mr_notes(&self, _p: u64, _i: u64) -> Result<Vec<Note>> {
            Ok(vec![])
        }
        async fn create_mr_note(&self, _p: u64, _i: u64, _b: &str) -> Result<Note> {
            unreachable!()
        }
        async fn create_issue_note(&self, _p: u64, _i: u64, _b: &str) -> Result<Note> {
            unreachable!()
        }
        async fn get_related_mrs(&self, _p: u64, _i: u64) -> Result<Vec<MergeRequest>> {
            Ok(vec![])
        }
        async fn get_recent_pipelines(&self, _p: u64, _c: usize) -> Result<Vec<Pipeline>> {
            Ok(vec![])
        }
        async fn update_mr(&self, _p: u64, _i: u64, _t: &str, _d: bool) -> Result<MergeRequest> {
            unreachable!()
        }
        async fn get_pipeline_jobs(&self, _p: u64, _pi: u64) -> Result<Vec<Job>> {
            Ok(vec![make_job(1, "build", "failed")])
        }
        async fn get_job_log(&self, _p: u64, _j: u64) -> Result<String> {
            Err(crate::error::DevnpcError::GitlabNotFound {
                resource: "log".into(),
            })
        }
        async fn get_pipeline(&self, _p: u64, _pi: u64) -> Result<Pipeline> {
            Ok(make_pipeline(1, "success"))
        }
        async fn get_file(&self, _p: u64, _fp: &str, _r: &str) -> Result<String> {
            Ok(String::new())
        }
        async fn list_tree(&self, _p: u64, _path: &str, _r: &str) -> Result<Vec<RepoTreeEntry>> {
            Ok(vec![])
        }
    }

    #[test]
    fn filter_failed_returns_newest_failed() {
        let pipelines = vec![
            make_pipeline(1, "success"),
            make_pipeline(3, "failed"),
            make_pipeline(2, "failed"),
            make_pipeline(4, "running"),
        ];
        let failed = filter_failed(&pipelines, 10);
        assert_eq!(failed.len(), 2);
        // 倒序: 3 在 2 之前
        assert_eq!(failed[0].id, 3);
        assert_eq!(failed[1].id, 2);
    }

    #[test]
    fn filter_failed_respects_count_limit() {
        let pipelines: Vec<Pipeline> = (1..=5).map(|i| make_pipeline(i, "failed")).collect();
        let failed = filter_failed(&pipelines, 2);
        assert_eq!(failed.len(), 2);
        // 最新的 2 条
        assert_eq!(failed[0].id, 5);
        assert_eq!(failed[1].id, 4);
    }

    #[test]
    fn filter_failed_empty_when_no_failures() {
        let pipelines = vec![make_pipeline(1, "success"), make_pipeline(2, "running")];
        assert!(filter_failed(&pipelines, 10).is_empty());
    }

    #[tokio::test]
    async fn fetch_job_logs_collects_all_jobs() {
        let mut logs = std::collections::HashMap::new();
        logs.insert(1, "build log".into());
        logs.insert(2, "test log".into());
        let mock = PipelineMock {
            jobs: vec![make_job(1, "build", "success"), make_job(2, "test", "failed")],
            logs,
            log_calls: Mutex::new(0),
        };
        let result = fetch_job_logs(&mock, 1, 100).await.unwrap();
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].job.name, "build");
        assert_eq!(result[0].log, "build log");
        assert_eq!(result[1].job.name, "test");
        assert_eq!(result[1].log, "test log");
        // 每个 job 调用一次 get_job_log
        assert_eq!(*mock.log_calls.lock().unwrap(), 2);
    }

    #[tokio::test]
    async fn fetch_job_logs_handles_log_error_gracefully() {
        let result = fetch_job_logs(&FailLogMock, 1, 100).await.unwrap();
        assert_eq!(result.len(), 1);
        // 日志获取失败,记录占位
        assert!(result[0].log.contains("[无法获取 job #1 日志"));
    }

    #[tokio::test]
    async fn fetch_failed_job_logs_returns_only_failed() {
        let mut logs = std::collections::HashMap::new();
        logs.insert(1, "ok".into());
        logs.insert(2, "fail".into());
        let mock = PipelineMock {
            jobs: vec![
                make_job(1, "build", "success"),
                make_job(2, "test", "failed"),
            ],
            logs,
            log_calls: Mutex::new(0),
        };
        let result = fetch_failed_job_logs(&mock, 1, 100).await.unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].job.status, "failed");
        assert_eq!(result[0].log, "fail");
    }

    #[test]
    fn is_terminal_recognizes_finished_states() {
        assert!(is_terminal(&make_pipeline(1, "success")));
        assert!(is_terminal(&make_pipeline(1, "failed")));
        assert!(is_terminal(&make_pipeline(1, "canceled")));
        assert!(!is_terminal(&make_pipeline(1, "running")));
        assert!(!is_terminal(&make_pipeline(1, "pending")));
    }

    #[test]
    fn is_success_only_matches_success() {
        assert!(is_success(&make_pipeline(1, "success")));
        assert!(!is_success(&make_pipeline(1, "failed")));
    }
}
