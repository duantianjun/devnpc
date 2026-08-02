//! Issue 操作高层 helper
//!
//! 组合 `GitlabApi` trait 上的 `get_issue` / `get_related_mrs` / `get_issue_notes`
//! 为更高层的领域操作,供 `Context::build()` 等场景复用。

use crate::error::Result;
use crate::gitlab_api::{GitlabApi, Issue, MergeRequest, Note};

/// Issue 聚合上下文: Issue 本体 + 关联 MR + 评论
#[derive(Debug, Clone)]
pub struct IssueContext {
    pub issue: Issue,
    pub related_mrs: Vec<MergeRequest>,
    pub notes: Vec<Note>,
}

/// 并行拉取 Issue 完整上下文 (issue + 相关 MR + 评论)
///
/// 单次 `tokio::try_join!` 并发 3 个请求,任一失败立即返回错误。
pub async fn fetch_issue_context(
    gitlab: &dyn GitlabApi,
    project_id: u64,
    issue_iid: u64,
) -> Result<IssueContext> {
    let (issue, related_mrs, notes) = tokio::try_join!(
        gitlab.get_issue(project_id, issue_iid),
        gitlab.get_related_mrs(project_id, issue_iid),
        gitlab.get_issue_notes(project_id, issue_iid),
    )?;
    Ok(IssueContext {
        issue,
        related_mrs,
        notes,
    })
}

/// 判断 Issue 是否处于可处理状态 (opened)
pub fn is_open(issue: &Issue) -> bool {
    issue.state == "opened"
}

/// 从 Issue 描述/标题中提取关联 MR iid (匹配 `!数字` 或 `MR !数字`)
pub fn extract_mr_iids(text: &str) -> Vec<u64> {
    let mut result = Vec::new();
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'!' {
            let mut j = i + 1;
            while j < bytes.len() && bytes[j].is_ascii_digit() {
                j += 1;
            }
            if j > i + 1
                && let Ok(s) = std::str::from_utf8(&bytes[i + 1..j])
                    && let Ok(n) = s.parse::<u64>()
                        && !result.contains(&n) {
                            result.push(n);
                        }
            i = j;
        } else {
            i += 1;
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gitlab_api::{CreateMrReq, Job, NoteAuthor, Pipeline, RepoTreeEntry};
    use async_trait::async_trait;

    /// 测试用 Mock: 可注入预设返回值
    struct IssueMock {
        issue: Issue,
        related_mrs: Vec<MergeRequest>,
        notes: Vec<Note>,
    }

    #[async_trait]
    impl GitlabApi for IssueMock {
        async fn get_issue(&self, _p: u64, _i: u64) -> Result<Issue> {
            Ok(self.issue.clone())
        }
        async fn get_mr(&self, _p: u64, _i: u64) -> Result<MergeRequest> {
            Err(crate::error::DevnpcError::GitlabNotFound {
                resource: "mock".into(),
            })
        }
        async fn create_mr(&self, _p: u64, _r: CreateMrReq) -> Result<MergeRequest> {
            unreachable!()
        }
        async fn get_pipelines(&self, _p: u64) -> Result<Vec<Pipeline>> {
            Ok(vec![])
        }
        async fn get_issue_notes(&self, _p: u64, _i: u64) -> Result<Vec<Note>> {
            Ok(self.notes.clone())
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
            Ok(self.related_mrs.clone())
        }
        async fn get_recent_pipelines(&self, _p: u64, _c: usize) -> Result<Vec<Pipeline>> {
            Ok(vec![])
        }
        async fn update_mr(&self, _p: u64, _i: u64, _t: &str, _d: bool) -> Result<MergeRequest> {
            unreachable!()
        }
        async fn get_pipeline_jobs(&self, _p: u64, _pi: u64) -> Result<Vec<Job>> {
            Ok(vec![])
        }
        async fn get_job_log(&self, _p: u64, _j: u64) -> Result<String> {
            Ok(String::new())
        }
        async fn get_pipeline(&self, _p: u64, _pi: u64) -> Result<Pipeline> {
            Ok(Pipeline {
                id: 1,
                status: "success".into(),
                ref_: Some("main".into()),
                sha: None,
                web_url: String::new(),
            })
        }
        async fn get_file(&self, _p: u64, _fp: &str, _r: &str) -> Result<String> {
            Ok(String::new())
        }
        async fn list_tree(&self, _p: u64, _path: &str, _r: &str) -> Result<Vec<RepoTreeEntry>> {
            Ok(vec![])
        }
    }

    fn make_issue(iid: u64, state: &str) -> Issue {
        Issue {
            iid,
            title: format!("Issue #{iid}"),
            description: Some(format!("see !{iid}")),
            state: state.into(),
            web_url: format!("https://gl.test/issues/{iid}"),
        }
    }

    fn make_note(id: u64, body: &str) -> Note {
        Note {
            id,
            body: body.into(),
            author: NoteAuthor {
                id: 1,
                username: "alice".into(),
                name: "Alice".into(),
            },
            created_at: "2026-08-01T00:00:00Z".into(),
        }
    }

    fn make_mr(iid: u64) -> MergeRequest {
        MergeRequest {
            iid,
            title: format!("MR !{iid}"),
            description: None,
            state: "opened".into(),
            source_branch: "feat".into(),
            target_branch: "main".into(),
            web_url: format!("https://gl.test/mrs/{iid}"),
            draft: false,
        }
    }

    #[tokio::test]
    async fn fetch_issue_context_aggregates_three_calls() {
        let mock = IssueMock {
            issue: make_issue(42, "opened"),
            related_mrs: vec![make_mr(7), make_mr(8)],
            notes: vec![make_note(1, "评论 1")],
        };
        let ctx = fetch_issue_context(&mock, 1, 42).await.unwrap();
        assert_eq!(ctx.issue.iid, 42);
        assert_eq!(ctx.related_mrs.len(), 2);
        assert_eq!(ctx.related_mrs[0].iid, 7);
        assert_eq!(ctx.notes.len(), 1);
        assert_eq!(ctx.notes[0].body, "评论 1");
    }

    #[tokio::test]
    async fn fetch_issue_context_propagates_error() {
        // 用一个总是返回 NotFound 的 mock 验证错误传播
        struct ErrMock;
        #[async_trait]
        impl GitlabApi for ErrMock {
            async fn get_issue(&self, _p: u64, _i: u64) -> Result<Issue> {
                Err(crate::error::DevnpcError::GitlabNotFound {
                    resource: "issue".into(),
                })
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
                Ok(vec![])
            }
            async fn get_job_log(&self, _p: u64, _j: u64) -> Result<String> {
                Ok(String::new())
            }
            async fn get_pipeline(&self, _p: u64, _pi: u64) -> Result<Pipeline> {
                Ok(Pipeline {
                    id: 1,
                    status: "success".into(),
                    ref_: None,
                    sha: None,
                    web_url: String::new(),
                })
            }
            async fn get_file(&self, _p: u64, _fp: &str, _r: &str) -> Result<String> {
                Ok(String::new())
            }
            async fn list_tree(&self, _p: u64, _path: &str, _r: &str) -> Result<Vec<RepoTreeEntry>> {
                Ok(vec![])
            }
        }
        let result = fetch_issue_context(&ErrMock, 1, 999).await;
        assert!(result.is_err());
    }

    #[test]
    fn is_open_returns_true_for_opened() {
        assert!(is_open(&make_issue(1, "opened")));
        assert!(!is_open(&make_issue(1, "closed")));
    }

    #[test]
    fn extract_mr_iids_finds_bang_refs() {
        let iids = extract_mr_iids("关联 !7 和 !8,重复 !7");
        assert_eq!(iids, vec![7, 8]);
    }

    #[test]
    fn extract_mr_iids_handles_empty_and_no_match() {
        assert!(extract_mr_iids("").is_empty());
        assert!(extract_mr_iids("no refs here").is_empty());
    }

    #[test]
    fn extract_mr_iids_does_not_match_lone_bang() {
        // "!" 后无数字不应 panic
        assert!(extract_mr_iids("! end").is_empty());
    }
}
