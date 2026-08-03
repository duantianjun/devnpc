//! MR 操作高层 helper
//!
//! 组合 `GitlabApi` trait 上的 `get_mr` / `get_mr_notes` 为高层领域操作,
//! 并提供 MR 选择/筛选工具函数。

use crate::error::Result;
use crate::gitlab_api::{GitlabApi, MergeRequest, Note};

/// MR 聚合上下文: MR 本体 + 评论列表
#[derive(Debug, Clone)]
pub struct MrContext {
    pub mr: MergeRequest,
    pub notes: Vec<Note>,
}

/// 并行拉取 MR 完整上下文 (MR + 评论)
pub async fn fetch_mr_context(
    gitlab: &dyn GitlabApi,
    project_id: u64,
    mr_iid: u64,
) -> Result<MrContext> {
    let (mr, notes) = tokio::try_join!(
        gitlab.get_mr(project_id, mr_iid),
        gitlab.get_mr_notes(project_id, mr_iid),
    )?;
    Ok(MrContext { mr, notes })
}

/// 从一组 MR 中找最新的 opened 状态 MR (按 iid 降序)
pub fn find_latest_open_mr(mrs: &[MergeRequest]) -> Option<&MergeRequest> {
    mrs.iter()
        .filter(|m| m.state == "opened")
        .max_by_key(|m| m.iid)
}

/// 过滤掉 Draft (WIP) 状态的 MR,返回非草稿 MR 的引用
pub fn filter_ready(mrs: &[MergeRequest]) -> Vec<&MergeRequest> {
    mrs.iter().filter(|m| !m.draft).collect()
}

/// 判断 MR 是否处于可合并状态 (opened 且非 Draft)
pub fn is_mergeable(mr: &MergeRequest) -> bool {
    mr.state == "opened" && !mr.draft
}

/// 移除 MR 标题的 "Draft: " 前缀,返回干净标题
pub fn clean_draft_title(title: &str) -> String {
    title.trim_start_matches("Draft: ").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gitlab_api::{CreateMrReq, Issue, Job, NoteAuthor, Pipeline, RepoTreeEntry};
    use async_trait::async_trait;

    fn make_mr(iid: u64, state: &str, draft: bool) -> MergeRequest {
        MergeRequest {
            iid,
            title: if draft {
                format!("Draft: MR !{iid}")
            } else {
                format!("MR !{iid}")
            },
            description: None,
            state: state.into(),
            source_branch: "feat".into(),
            target_branch: "main".into(),
            web_url: format!("https://gl.test/mrs/{iid}"),
            draft,
        }
    }

    fn make_note(id: u64) -> Note {
        Note {
            id,
            body: format!("note {id}"),
            author: NoteAuthor {
                id: 1,
                username: "alice".into(),
                name: "Alice".into(),
            },
            created_at: "2026-08-01T00:00:00Z".into(),
        }
    }

    struct MrMock {
        mr: MergeRequest,
        notes: Vec<Note>,
    }

    #[async_trait]
    impl GitlabApi for MrMock {
        async fn get_issue(&self, _p: u64, _i: u64) -> Result<Issue> {
            unreachable!()
        }
        async fn get_mr(&self, _p: u64, _i: u64) -> Result<MergeRequest> {
            Ok(self.mr.clone())
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
            Ok(self.notes.clone())
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

    #[tokio::test]
    async fn fetch_mr_context_aggregates_mr_and_notes() {
        let mock = MrMock {
            mr: make_mr(7, "opened", false),
            notes: vec![make_note(1), make_note(2)],
        };
        let ctx = fetch_mr_context(&mock, 1, 7).await.unwrap();
        assert_eq!(ctx.mr.iid, 7);
        assert_eq!(ctx.notes.len(), 2);
    }

    #[test]
    fn find_latest_open_mr_returns_highest_iid_open() {
        let mrs = vec![
            make_mr(1, "merged", false),
            make_mr(3, "opened", false),
            make_mr(5, "opened", true),
        ];
        let latest = find_latest_open_mr(&mrs).unwrap();
        assert_eq!(latest.iid, 5);
    }

    #[test]
    fn find_latest_open_mr_returns_none_when_all_closed() {
        let mrs = vec![make_mr(1, "merged", false), make_mr(2, "closed", false)];
        assert!(find_latest_open_mr(&mrs).is_none());
    }

    #[test]
    fn find_latest_open_mr_returns_none_for_empty() {
        assert!(find_latest_open_mr(&[]).is_none());
    }

    #[test]
    fn filter_ready_excludes_drafts() {
        let mrs = vec![
            make_mr(1, "opened", false),
            make_mr(2, "opened", true),
            make_mr(3, "opened", false),
        ];
        let ready = filter_ready(&mrs);
        assert_eq!(ready.len(), 2);
        assert_eq!(ready[0].iid, 1);
        assert_eq!(ready[1].iid, 3);
    }

    #[test]
    fn is_mergeable_requires_opened_and_not_draft() {
        assert!(is_mergeable(&make_mr(1, "opened", false)));
        assert!(!is_mergeable(&make_mr(1, "opened", true)));
        assert!(!is_mergeable(&make_mr(1, "merged", false)));
    }

    #[test]
    fn clean_draft_title_strips_prefix() {
        assert_eq!(clean_draft_title("Draft: feat: login"), "feat: login");
        assert_eq!(clean_draft_title("feat: login"), "feat: login");
        // trim_start_matches 移除所有前导 "Draft: " 前缀
        // (与 ci/controller.rs 既有行为一致)
        assert_eq!(clean_draft_title("Draft: Draft: x"), "x");
    }
}
