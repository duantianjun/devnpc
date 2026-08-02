//! GitLab API 工具: create_mr_note

use std::sync::Arc;

use async_trait::async_trait;
use serde::Deserialize;

use crate::error::{DevnpcError, Result};
use crate::gitlab_api::GitlabApi;
use crate::tools::{Tool, ToolResult};

pub struct CreateMrNoteTool {
    client: Arc<dyn GitlabApi>,
    project_id: u64,
}

impl CreateMrNoteTool {
    pub fn new(client: Arc<dyn GitlabApi>, project_id: u64) -> Self {
        Self {
            client,
            project_id,
        }
    }
}

#[derive(Deserialize)]
struct CreateMrNoteArgs {
    mr_iid: u64,
    body: String,
}

#[async_trait]
impl Tool for CreateMrNoteTool {
    fn name(&self) -> &str {
        "create_mr_note"
    }
    fn description(&self) -> &str {
        "在指定 MR 发表评论。参数: mr_iid (MR iid), body (评论内容)。"
    }
    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "mr_iid": {"type": "integer", "description": "MR iid"},
                "body": {"type": "string", "description": "评论正文"}
            },
            "required": ["mr_iid", "body"]
        })
    }
    async fn call(&self, args: &serde_json::Value) -> Result<ToolResult> {
        let parsed: CreateMrNoteArgs = serde_json::from_value(args.clone()).map_err(|e| {
            DevnpcError::Tool {
                tool: "create_mr_note".into(),
                msg: format!("参数解析失败: {e}"),
            }
        })?;
        match self
            .client
            .create_mr_note(self.project_id, parsed.mr_iid, &parsed.body)
            .await
        {
            Ok(note) => Ok(ToolResult::ok(format!("已评论 MR !{} (note_id={})", parsed.mr_iid, note.id))),
            Err(e) => Ok(ToolResult::err(format!("评论失败: {e}"))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gitlab_api::{CreateMrReq, Issue, MergeRequest, Note, NoteAuthor, Pipeline};
    use async_trait::async_trait;

    struct MockGitlab {
        notes: std::sync::Mutex<Vec<(u64, u64, String)>>,
    }

    impl MockGitlab {
        fn new() -> Self {
            Self {
                notes: std::sync::Mutex::new(Vec::new()),
            }
        }
    }

    #[async_trait]
    impl GitlabApi for MockGitlab {
        async fn get_issue(&self, _p: u64, _i: u64) -> Result<Issue> {
            unimplemented!("mock")
        }
        async fn get_mr(&self, _p: u64, _i: u64) -> Result<MergeRequest> {
            unimplemented!("mock")
        }
        async fn create_mr(&self, _p: u64, _r: CreateMrReq) -> Result<MergeRequest> {
            unimplemented!("mock")
        }
        async fn get_pipelines(&self, _p: u64) -> Result<Vec<Pipeline>> {
            unimplemented!("mock")
        }
        async fn get_issue_notes(&self, _p: u64, _i: u64) -> Result<Vec<Note>> {
            unimplemented!("mock")
        }
        async fn get_mr_notes(&self, _p: u64, _i: u64) -> Result<Vec<Note>> {
            unimplemented!("mock")
        }
        async fn create_mr_note(&self, _p: u64, mr_iid: u64, body: &str) -> Result<Note> {
            self.notes.lock().unwrap().push((mr_iid, 0, body.to_string()));
            Ok(Note {
                id: 999,
                body: body.to_string(),
                author: NoteAuthor {
                    id: 1,
                    username: "devnpc".into(),
                    name: "devnpc".into(),
                },
                created_at: "2026-08-01T00:00:00Z".into(),
            })
        }
        async fn create_issue_note(&self, _p: u64, _i: u64, body: &str) -> Result<Note> {
            self.notes.lock().unwrap().push((_i, 0, body.to_string()));
            Ok(Note {
                id: 999,
                body: body.to_string(),
                author: NoteAuthor {
                    id: 1,
                    username: "devnpc".into(),
                    name: "devnpc".into(),
                },
                created_at: "2026-08-01T00:00:00Z".into(),
            })
        }
        async fn get_related_mrs(&self, _p: u64, _i: u64) -> Result<Vec<MergeRequest>> {
            unimplemented!("mock")
        }
        async fn get_recent_pipelines(&self, _p: u64, _c: usize) -> Result<Vec<Pipeline>> {
            unimplemented!("mock")
        }
        async fn update_mr(&self, _p: u64, _i: u64, _t: &str, _d: bool) -> Result<MergeRequest> {
            unimplemented!("mock")
        }
        async fn get_pipeline_jobs(&self, _p: u64, _pi: u64) -> Result<Vec<crate::gitlab_api::Job>> {
            unimplemented!("mock")
        }
        async fn get_job_log(&self, _p: u64, _j: u64) -> Result<String> {
            unimplemented!("mock")
        }
    }

    #[tokio::test]
    async fn create_mr_note_calls_api_and_returns_success() {
        let mock = Arc::new(MockGitlab::new());
        let tool = CreateMrNoteTool::new(mock.clone(), 1);
        let result = tool
            .call(&serde_json::json!({"mr_iid": 7, "body": "CI 通过"}))
            .await
            .unwrap();
        assert!(result.success);
        assert!(result.output.contains("MR !7"));
        // 验证 mock 收到调用
        let notes = mock.notes.lock().unwrap();
        assert_eq!(notes.len(), 1);
        assert_eq!(notes[0].0, 7);
        assert_eq!(notes[0].2, "CI 通过");
    }

    #[tokio::test]
    async fn create_mr_note_rejects_missing_args() {
        let mock = Arc::new(MockGitlab::new());
        let tool = CreateMrNoteTool::new(mock, 1);
        let result = tool
            .call(&serde_json::json!({"mr_iid": 7}))
            .await;
        assert!(result.is_err());
    }
}
