//! dashboard 事件协议类型
//!
//! devnpc 任务执行过程中推送到 dashboard 的事件结构。
//! 同时用于本地 .jsonl 文件存储 (兜底机制)。

use serde::{Deserialize, Serialize};

// ============================================================
// 枚举辅助类型
// ============================================================

/// SOP 步骤状态
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum SopStepStatus {
    Started,
    Completed,
    Deviated,
}

/// CI 状态
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum CiStatus {
    Running,
    Passed,
    Failed,
}

/// 任务最终状态
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    Success,
    Failed,
    CiFailed,
    Timeout,
}

// ============================================================
// 事件类型
// ============================================================

/// 任务启动事件 (任务开始时推送一次)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskStartedEvent {
    /// UUID v4,贯穿任务全生命周期
    pub task_id: String,
    /// GitLab 项目路径
    pub project: String,
    pub mr_iid: Option<u64>,
    pub pipeline_id: Option<u64>,
    pub task_description: String,
    /// issue/mr_comment/manual
    pub task_kind: String,
    /// RFC3339
    pub started_at: String,
    /// 使用的 LLM 模型名
    pub model: String,
}

/// 执行过程事件 (任务执行中持续推送)
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ExecutionEvent {
    LlmCall {
        iteration: u32,
        prompt_tokens: u64,
        completion_tokens: u64,
        latency_ms: u64,
    },
    ToolCall {
        name: String,
        success: bool,
        latency_ms: u64,
        /// 工具调用摘要(非完整参数)
        detail: String,
    },
    SopStep {
        step: String,
        status: SopStepStatus,
        note: Option<String>,
    },
    CiStatus {
        pipeline_id: u64,
        status: CiStatus,
        /// 第几次重试
        attempt: u8,
    },
    TeamHandoff {
        /// pm/developer/tester
        from_role: String,
        to_role: String,
        /// decomposed/implemented/tested
        signal: String,
    },
}

/// 任务结束事件 (任务完成时推送一次)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskFinishedEvent {
    pub task_id: String,
    pub status: TaskStatus,
    pub duration_secs: u64,
    pub total_tokens: u64,
    pub estimated_cost_usd: f64,
    pub mr_url: Option<String>,
    pub ci_url: Option<String>,
    /// LLM 生成的验收摘要
    pub summary: String,
    /// 失败原因
    pub error: Option<String>,
    pub finished_at: String,
}

// ============================================================
// JSONL 文件行类型 (本地兜底文件格式)
// ============================================================

/// .jsonl 文件每行的包装类型
///
/// 注意: `TaskStarted`/`TaskFinished` 变体使用 `#[serde(flatten)]` 将事件数据
/// 展平到行顶层,`task_id` 由展平的 `data` 提供 (这两个事件类型本身含 `task_id`)。
/// `Execution` 变体的 `task_id` 在外层提供 (因为 `ExecutionEvent` 无 `task_id` 字段)。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum EventLogEntry {
    TaskStarted {
        #[serde(flatten)]
        data: TaskStartedEvent,
    },
    Execution {
        task_id: String,
        event: ExecutionEvent,
    },
    TaskFinished {
        #[serde(flatten)]
        data: TaskFinishedEvent,
    },
}

/// 批量推送请求体
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchEventsRequest {
    pub task_id: String,
    pub events: Vec<ExecutionEvent>,
}

/// 导入结果
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportResult {
    pub task_id: String,
    pub events_count: usize,
    /// true=因已 finish 而跳过
    pub skipped: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn task_started_event_serializes() {
        let e = TaskStartedEvent {
            task_id: "abc-123".into(),
            project: "my-group/my-project".into(),
            mr_iid: Some(42),
            pipeline_id: Some(100),
            task_description: "修复 bug".into(),
            task_kind: "mr_comment".into(),
            started_at: "2026-08-03T10:00:00Z".into(),
            model: "deepseek-chat".into(),
        };
        let json = serde_json::to_string(&e).unwrap();
        assert!(json.contains("abc-123"));
        assert!(json.contains("deepseek-chat"));
        // round-trip
        let e2: TaskStartedEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(e2.task_id, "abc-123");
    }

    #[test]
    fn execution_event_llm_call_serializes() {
        let e = ExecutionEvent::LlmCall {
            iteration: 1,
            prompt_tokens: 500,
            completion_tokens: 200,
            latency_ms: 1500,
        };
        let json = serde_json::to_string(&e).unwrap();
        assert!(json.contains("llm_call"));
        let e2: ExecutionEvent = serde_json::from_str(&json).unwrap();
        match e2 {
            ExecutionEvent::LlmCall { iteration, .. } => assert_eq!(iteration, 1),
            _ => panic!("应为 LlmCall"),
        }
    }

    #[test]
    fn execution_event_tool_call_serializes() {
        let e = ExecutionEvent::ToolCall {
            name: "read_file".into(),
            success: true,
            latency_ms: 50,
            detail: "src/main.rs".into(),
        };
        let json = serde_json::to_string(&e).unwrap();
        assert!(json.contains("tool_call"));
        assert!(json.contains("read_file"));
    }

    #[test]
    fn execution_event_sop_step_serializes() {
        let e = ExecutionEvent::SopStep {
            step: "analyze".into(),
            status: SopStepStatus::Completed,
            note: None,
        };
        let json = serde_json::to_string(&e).unwrap();
        assert!(json.contains("sop_step"));
        assert!(json.contains("completed"));
    }

    #[test]
    fn execution_event_ci_status_serializes() {
        let e = ExecutionEvent::CiStatus {
            pipeline_id: 100,
            status: CiStatus::Failed,
            attempt: 2,
        };
        let json = serde_json::to_string(&e).unwrap();
        assert!(json.contains("ci_status"));
        assert!(json.contains("failed"));
    }

    #[test]
    fn execution_event_team_handoff_serializes() {
        let e = ExecutionEvent::TeamHandoff {
            from_role: "pm".into(),
            to_role: "developer".into(),
            signal: "decomposed".into(),
        };
        let json = serde_json::to_string(&e).unwrap();
        assert!(json.contains("team_handoff"));
        assert!(json.contains("decomposed"));
    }

    #[test]
    fn task_finished_event_serializes() {
        let e = TaskFinishedEvent {
            task_id: "abc-123".into(),
            status: TaskStatus::Success,
            duration_secs: 45,
            total_tokens: 12000,
            estimated_cost_usd: 0.05,
            mr_url: Some("https://gitlab.com/mr/42".into()),
            ci_url: Some("https://gitlab.com/pipeline/100".into()),
            summary: "已修复 bug".into(),
            error: None,
            finished_at: "2026-08-03T10:01:00Z".into(),
        };
        let json = serde_json::to_string(&e).unwrap();
        assert!(json.contains("success"));
        let e2: TaskFinishedEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(e2.status, TaskStatus::Success);
    }

    #[test]
    fn event_log_entry_task_started_serializes() {
        let entry = EventLogEntry::TaskStarted {
            data: TaskStartedEvent {
                task_id: "abc-123".into(),
                project: "proj".into(),
                mr_iid: None,
                pipeline_id: None,
                task_description: "test".into(),
                task_kind: "manual".into(),
                started_at: "2026-08-03T10:00:00Z".into(),
                model: "gpt-4".into(),
            },
        };
        let json = serde_json::to_string(&entry).unwrap();
        assert!(json.contains("task_started"));
        let entry2: EventLogEntry = serde_json::from_str(&json).unwrap();
        match entry2 {
            EventLogEntry::TaskStarted { data, .. } => assert_eq!(data.task_id, "abc-123"),
            _ => panic!("应为 TaskStarted"),
        }
    }

    #[test]
    fn event_log_entry_execution_serializes() {
        let entry = EventLogEntry::Execution {
            task_id: "abc-123".into(),
            event: ExecutionEvent::LlmCall {
                iteration: 1,
                prompt_tokens: 100,
                completion_tokens: 50,
                latency_ms: 500,
            },
        };
        let json = serde_json::to_string(&entry).unwrap();
        assert!(json.contains("execution"));
        assert!(json.contains("llm_call"));
    }

    #[test]
    fn event_log_entry_task_finished_serializes() {
        let entry = EventLogEntry::TaskFinished {
            data: TaskFinishedEvent {
                task_id: "abc-123".into(),
                status: TaskStatus::Failed,
                duration_secs: 100,
                total_tokens: 5000,
                estimated_cost_usd: 0.02,
                mr_url: None,
                ci_url: None,
                summary: "失败".into(),
                error: Some("CI 超时".into()),
                finished_at: "2026-08-03T10:02:00Z".into(),
            },
        };
        let json = serde_json::to_string(&entry).unwrap();
        assert!(json.contains("task_finished"));
        assert!(json.contains("failed"));
    }

    #[test]
    fn batch_events_request_serializes() {
        let req = BatchEventsRequest {
            task_id: "abc-123".into(),
            events: vec![
                ExecutionEvent::LlmCall {
                    iteration: 1,
                    prompt_tokens: 100,
                    completion_tokens: 50,
                    latency_ms: 500,
                },
                ExecutionEvent::ToolCall {
                    name: "read_file".into(),
                    success: true,
                    latency_ms: 10,
                    detail: "test.rs".into(),
                },
            ],
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains("abc-123"));
        assert!(json.contains("llm_call"));
        assert!(json.contains("tool_call"));
    }

    #[test]
    fn import_result_serializes() {
        let r = ImportResult {
            task_id: "abc-123".into(),
            events_count: 10,
            skipped: false,
        };
        let json = serde_json::to_string(&r).unwrap();
        assert!(json.contains("10"));
        assert!(json.contains("false"));
    }

    #[test]
    fn jsonl_line_round_trip() {
        // 模拟 .jsonl 文件中的一行
        let entry = EventLogEntry::Execution {
            task_id: "t1".into(),
            event: ExecutionEvent::ToolCall {
                name: "git_commit".into(),
                success: true,
                latency_ms: 200,
                detail: "commit message".into(),
            },
        };
        let line = serde_json::to_string(&entry).unwrap();
        // 模拟从文件读取一行并解析
        let parsed: EventLogEntry = serde_json::from_str(&line).unwrap();
        match parsed {
            EventLogEntry::Execution { task_id, event } => {
                assert_eq!(task_id, "t1");
                match event {
                    ExecutionEvent::ToolCall { name, success, .. } => {
                        assert_eq!(name, "git_commit");
                        assert!(success);
                    }
                    _ => panic!("应为 ToolCall"),
                }
            }
            _ => panic!("应为 Execution"),
        }
    }
}
