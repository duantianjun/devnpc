//! ReAct 循环 (P3 完整实现)
//!
//! LLM ↔ Tool 反复迭代,带 SOP 偏离检测 (软约束) 与迭代上限。
//! LLM 调 finish 工具即终止并返回 Finished。
//!
//! P8 增强: 并行工具执行。

use crate::agent::llm_client::LlmClient;
use crate::agent::message::Message;
use crate::agent::sop::{DeviationReport, Sop};
use crate::config::SopMode;
use crate::error::Result;
use crate::tools::ToolRegistry;

/// Agent 运行结果
#[derive(Debug, Clone)]
pub enum RunResult {
    /// LLM 调 finish,任务完成
    Finished {
        text: String,
        summary: String,
        trajectory: Trajectory,
    },
    /// 达到迭代上限
    MaxIterationsReached(Trajectory),
    /// SOP 严格模式偏离,循环终止
    SopViolation {
        step: String,
        unexpected_tools: Vec<String>,
        trajectory: Trajectory,
    },
}

/// 执行轨迹 (供 report 模块消费)
#[derive(Debug, Clone, Default)]
pub struct Trajectory {
    pub events: Vec<TrajectoryEvent>,
}

#[derive(Debug, Clone)]
pub enum TrajectoryEvent {
    LlmCall { iteration: u32 },
    ToolCall { name: String, success: bool },
    Deviation { step: String, unexpected: Vec<String> },
}

impl Trajectory {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn record_llm_call(&mut self, iteration: u32) {
        self.events.push(TrajectoryEvent::LlmCall { iteration });
    }
    pub fn record_tool_call(&mut self, name: &str, success: bool) {
        self.events.push(TrajectoryEvent::ToolCall {
            name: name.to_string(),
            success,
        });
    }
    pub fn record_deviation(&mut self, step: &str, unexpected: Vec<String>) {
        self.events.push(TrajectoryEvent::Deviation {
            step: step.to_string(),
            unexpected,
        });
    }
}

/// ReAct 循环执行器
pub struct ReactLoop {
    pub max_iterations: u32,
    pub llm: LlmClient,
    pub sop_mode: SopMode,
}

impl ReactLoop {
    pub fn new(max_iterations: u32, llm: LlmClient, sop_mode: SopMode) -> Self {
        Self {
            max_iterations,
            llm,
            sop_mode,
        }
    }

    /// 运行循环
    ///
    /// `initial_messages`: 首轮消息 (System + User,由 prompt::build_initial_messages 构造)
    /// `tools`: 工具注册表
    /// `sop`: 可选 SOP (soft 模式只记录偏离)
    pub async fn run(
        &self,
        initial_messages: Vec<Message>,
        tools: &ToolRegistry,
        sop: Option<&Sop>,
    ) -> Result<RunResult> {
        let mut messages = initial_messages;
        let mut trajectory = Trajectory::new();

        for iteration in 0..self.max_iterations {
            let response = self.llm.complete(&messages, &tools.schemas(), None).await?;
            trajectory.record_llm_call(iteration);

            let tool_calls = response.tool_calls;

            // 无 tool_call 且无 finish: LLM 直接给文本,视为完成
            if tool_calls.is_empty() {
                return Ok(RunResult::Finished {
                    text: response.text.clone(),
                    summary: response.text,
                    trajectory,
                });
            }

            // SOP 偏离检测
            if let Some(sop) = sop {
                let tool_names: Vec<String> =
                    tool_calls.iter().map(|tc| tc.name.clone()).collect();
                match sop.check_deviation(&tool_names, &trajectory, self.sop_mode) {
                    DeviationReport::Soft {
                        step,
                        unexpected_tools,
                    } => {
                        trajectory.record_deviation(&step, unexpected_tools);
                    }
                    DeviationReport::Strict {
                        step,
                        unexpected_tools,
                    } => {
                        trajectory.record_deviation(&step, unexpected_tools.clone());
                        return Ok(RunResult::SopViolation {
                            step,
                            unexpected_tools,
                            trajectory,
                        });
                    }
                    DeviationReport::None => {}
                }
            }

            // 追加 assistant 消息 (含 tool_calls)
            messages.push(Message::assistant(&response.text, &tool_calls));

            // 执行工具并喂回结果 (并行执行)
            let mut finish_summary: Option<String> = None;
            let tool_futures: Vec<_> = tool_calls
                .iter()
                .map(|tc| {
                    let tc_name = tc.name.clone();
                    let tc_id = tc.id.clone();
                    let tc_args = tc.arguments.clone();
                    async move {
                        let result = tools.call(&tc_name, &tc_args).await;
                        (tc_id, tc_name, result)
                    }
                })
                .collect();
            let tool_results = futures::future::join_all(tool_futures).await;

            for (tc_id, tc_name, result) in tool_results {
                let output = match &result {
                    Ok(r) => {
                        trajectory.record_tool_call(&tc_name, r.success);
                        r.output.clone()
                    }
                    Err(e) => {
                        trajectory.record_tool_call(&tc_name, false);
                        format!("错误: {e}")
                    }
                };

                // finish 工具: 提取 summary
                if tc_name == "finish" {
                    finish_summary = Some(
                        // 从原始参数中提取 summary
                        tool_calls
                            .iter()
                            .find(|tc| tc.id == tc_id)
                            .and_then(|tc| tc.arguments["summary"].as_str())
                            .unwrap_or(&output)
                            .to_string(),
                    );
                }

                messages.push(Message::tool(&tc_id, output));
            }

            // 若调了 finish,终止
            if let Some(summary) = finish_summary {
                return Ok(RunResult::Finished {
                    text: response.text,
                    summary,
                    trajectory,
                });
            }
        }

        Ok(RunResult::MaxIterationsReached(trajectory))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::SopMode;
    use crate::tools::{Tool, ToolResult, ToolRegistry};
    use async_trait::async_trait;
    use wiremock::matchers::method;
    use wiremock::{Mock, MockServer, ResponseTemplate};

    /// 简单 echo 工具
    struct EchoTool;

    #[async_trait]
    impl Tool for EchoTool {
        fn name(&self) -> &str {
            "echo"
        }
        fn description(&self) -> &str {
            "echo"
        }
        fn parameters_schema(&self) -> serde_json::Value {
            serde_json::json!({"type": "object"})
        }
        async fn call(&self, _args: &serde_json::Value) -> Result<ToolResult> {
            Ok(ToolResult::ok("echoed"))
        }
    }

    /// finish 工具 (用真实 FinishTool)
    use crate::tools::finish::FinishTool;

    fn llm_for(server: &MockServer) -> LlmClient {
        LlmClient::new(crate::config::LlmConfig {
            api_key: "test".into(),
            base_url: server.uri(),
            model: "test".into(),
        })
    }

    #[tokio::test]
    async fn loop_finishes_when_llm_calls_finish() {
        let server = MockServer::start().await;
        // 第一次响应: 调 echo
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "choices": [{"message": {"content": "thinking", "tool_calls": [
                    {"id": "c1", "type": "function", "function": {"name": "echo", "arguments": "{}"}}
                ]}}]
            })))
            .up_to_n_times(1)
            .mount(&server)
            .await;
        // 第二次响应: 调 finish
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "choices": [{"message": {"content": "done", "tool_calls": [
                    {"id": "c2", "type": "function", "function": {"name": "finish", "arguments": "{\"summary\":\"已修复\"}"}}
                ]}}]
            })))
            .mount(&server)
            .await;

        let llm = llm_for(&server);
        let react = ReactLoop::new(10, llm, SopMode::Soft);

        let mut tools = ToolRegistry::new();
        tools.register(Box::new(EchoTool));
        tools.register(Box::new(FinishTool::new()));

        let result = react
            .run(vec![Message::user("hi")], &tools, None)
            .await
            .unwrap();
        match result {
            RunResult::Finished {
                summary, trajectory, ..
            } => {
                assert_eq!(summary, "已修复");
                // 2 次 LLM 调用 + 2 次工具调用
                let llm_calls = trajectory
                    .events
                    .iter()
                    .filter(|e| matches!(e, TrajectoryEvent::LlmCall { .. }))
                    .count();
                assert_eq!(llm_calls, 2);
                let tool_calls = trajectory
                    .events
                    .iter()
                    .filter(|e| matches!(e, TrajectoryEvent::ToolCall { .. }))
                    .count();
                assert_eq!(tool_calls, 2);
            }
            _ => panic!("expected Finished"),
        }
    }

    #[tokio::test]
    async fn loop_finishes_when_llm_returns_text_only() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "choices": [{"message": {"content": "无需工具,任务完成", "tool_calls": null}}]
            })))
            .mount(&server)
            .await;

        let llm = llm_for(&server);
        let react = ReactLoop::new(10, llm, SopMode::Soft);
        let tools = ToolRegistry::new();

        let result = react
            .run(vec![Message::user("hi")], &tools, None)
            .await
            .unwrap();
        match result {
            RunResult::Finished { text, .. } => {
                assert_eq!(text, "无需工具,任务完成");
            }
            _ => panic!("expected Finished"),
        }
    }

    #[tokio::test]
    async fn loop_returns_max_iterations_when_never_finishes() {
        let server = MockServer::start().await;
        // 每次都调 echo,永不 finish
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "choices": [{"message": {"content": null, "tool_calls": [
                    {"id": "c1", "type": "function", "function": {"name": "echo", "arguments": "{}"}}
                ]}}]
            })))
            .mount(&server)
            .await;

        let llm = llm_for(&server);
        let react = ReactLoop::new(3, llm, SopMode::Soft); // 上限 3

        let mut tools = ToolRegistry::new();
        tools.register(Box::new(EchoTool));

        let result = react
            .run(vec![Message::user("hi")], &tools, None)
            .await
            .unwrap();
        assert!(matches!(result, RunResult::MaxIterationsReached(_)));
    }

    #[tokio::test]
    async fn loop_records_sop_deviation_in_soft_mode() {
        let server = MockServer::start().await;
        // 调 echo (SOP 第一步只允许 read_file,echo 会偏离)
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "choices": [{"message": {"content": null, "tool_calls": [
                    {"id": "c1", "type": "function", "function": {"name": "echo", "arguments": "{}"}}
                ]}}]
            })))
            .up_to_n_times(1)
            .mount(&server)
            .await;
        // 第二次 finish
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "choices": [{"message": {"content": "ok", "tool_calls": [
                    {"id": "c2", "type": "function", "function": {"name": "finish", "arguments": "{\"summary\":\"done\"}"}}
                ]}}]
            })))
            .mount(&server)
            .await;

        let llm = llm_for(&server);
        let react = ReactLoop::new(10, llm, SopMode::Soft);

        let mut tools = ToolRegistry::new();
        tools.register(Box::new(EchoTool));
        tools.register(Box::new(FinishTool::new()));

        // SOP: 第一步只允许 read_file,echo 会偏离
        let sop = Sop {
            name: "test".into(),
            description: "".into(),
            steps: vec![crate::agent::sop::SopStep {
                name: "只读".into(),
                expected_tools: vec!["read_file".into()],
                hint: "".into(),
            }],
        };

        let result = react
            .run(vec![Message::user("hi")], &tools, Some(&sop))
            .await
            .unwrap();
        if let RunResult::Finished { trajectory, .. } = result {
            let deviations = trajectory
                .events
                .iter()
                .filter(|e| matches!(e, TrajectoryEvent::Deviation { .. }))
                .count();
            assert!(deviations >= 1, "应记录至少 1 次偏离");
        } else {
            panic!("expected Finished");
        }
    }
}
