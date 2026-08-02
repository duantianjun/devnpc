//! 端到端集成测试: trigger 解析 → Agent 构建 → Team 编排 → 报告生成
//!
//! 使用 MockLlm 替代真实 LLM 调用,验证完整链路的正确性。

use std::sync::Arc;

use adk_rust::agent::LlmAgentBuilder;
use adk_rust::{Content, FinishReason, Llm, LlmRequest, LlmResponse, LlmResponseStream};
use async_trait::async_trait;
use futures::stream;

use devnpc::adapter::memory::MemoryStore;
use devnpc::adapter::orchestrator::{Orchestrator, TeamResult};
use devnpc::ci::controller::CiOutcome;
use devnpc::config::npc_config::{HandoffRule, Team, TeamNpc};
use devnpc::config::MemoryConfig;

// ============================================================
// MockLlm: 确定性测试替身,返回预设响应
// ============================================================

/// 可配置的 Mock LLM,按顺序返回预设响应
struct MockLlm {
    name: String,
    /// 预设响应队列 (FIFO),每次调用弹出一个
    responses: std::sync::Mutex<std::collections::VecDeque<String>>,
}

impl MockLlm {
    fn new(name: &str, responses: Vec<String>) -> Self {
        Self {
            name: name.to_string(),
            responses: std::sync::Mutex::new(responses.into_iter().collect()),
        }
    }
}

#[async_trait]
impl Llm for MockLlm {
    fn name(&self) -> &str {
        &self.name
    }

    async fn generate_content(
        &self,
        _req: LlmRequest,
        _stream: bool,
    ) -> adk_rust::Result<LlmResponseStream> {
        let mut queue = self.responses.lock().unwrap();
        let text = queue.pop_front().unwrap_or_else(|| "默认响应".to_string());
        drop(queue);

        let response = LlmResponse {
            content: Some(Content::new("model").with_text(&text)),
            usage_metadata: Some(adk_rust::UsageMetadata {
                prompt_token_count: 10,
                candidates_token_count: 20,
                total_token_count: 30,
                ..Default::default()
            }),
            finish_reason: Some(FinishReason::Stop),
            citation_metadata: None,
            partial: false,
            turn_complete: true,
            interrupted: false,
            error_code: None,
            error_message: None,
            provider_metadata: None,
            interaction_id: None,
        };

        let s = stream::iter(vec![Ok(response)]);
        Ok(Box::pin(s))
    }
}

// ============================================================
// 辅助函数
// ============================================================

/// 构建一个带预设响应的 MockLlm
fn make_mock_llm(responses: Vec<&str>) -> Arc<dyn Llm> {
    Arc::new(MockLlm::new(
        "mock-model",
        responses.into_iter().map(String::from).collect(),
    ))
}

/// 构建一个简单的 Agent (用于测试 Orchestrator)
fn make_test_agent(llm: Arc<dyn Llm>, instruction: &str) -> Arc<adk_rust::agent::LlmAgent> {
    Arc::new(
        LlmAgentBuilder::new("test-agent")
            .instruction(instruction.to_string())
            .model(llm)
            .build()
            .expect("Agent 构建失败"),
    )
}

/// 构建测试用 Team (PM→Developer→Tester)
fn make_test_team() -> Team {
    Team {
        name: "test-team".into(),
        description: "测试团队".into(),
        npcs: vec![
            TeamNpc {
                role: "pm".into(),
                sop: Some("requirement-decompose".into()),
            },
            TeamNpc {
                role: "developer".into(),
                sop: Some("feature".into()),
            },
            TeamNpc {
                role: "tester".into(),
                sop: Some("test-gen".into()),
            },
        ],
        handoff: vec![
            HandoffRule {
                from: "pm".into(),
                to: vec!["developer".into(), "tester".into()],
                trigger: "pm 发出 \"decomposed\" 信号".into(),
            },
            HandoffRule {
                from: "developer".into(),
                to: vec!["tester".into()],
                trigger: "developer 发出 \"implemented\" 信号".into(),
            },
        ],
        merge: Some(devnpc::config::npc_config::MergeStrategy {
            strategy: "single-mr".into(),
        }),
    }
}

// ============================================================
// 测试用例
// ============================================================

#[tokio::test]
async fn test_trigger_parser_classify_task_correctly() {
    use devnpc::trigger::parser::{classify_task, TaskKind};

    assert_eq!(classify_task("修复登录 bug"), TaskKind::Fix);
    assert_eq!(classify_task("fix the crash"), TaskKind::Fix);
    assert_eq!(classify_task("实现用户注册"), TaskKind::Implement);
    assert_eq!(classify_task("add new feature"), TaskKind::Implement);
    assert_eq!(classify_task("重构认证模块"), TaskKind::Refactor);
    assert_eq!(classify_task("refactor utils"), TaskKind::Refactor);
    assert_eq!(classify_task("review this PR"), TaskKind::Review);
    assert_eq!(classify_task("测试登录流程"), TaskKind::Test);
    assert_eq!(classify_task("write unit test"), TaskKind::Test);
}

#[tokio::test]
async fn test_trigger_parser_parses_mention_with_issue_ref() {
    use devnpc::trigger::parser::parse_mention;

    let spec = parse_mention("@devnpc 修复 #42 的登录 bug").expect("解析失败");
    assert_eq!(spec.description, "修复 #42 的登录 bug");
    assert_eq!(spec.target_issue, Some(42));

    let spec = parse_mention("@devnpc 实现用户管理功能").expect("解析失败");
    assert_eq!(spec.description, "实现用户管理功能");
    assert!(spec.target_issue.is_none());

    // 无效提及 (空任务)
    assert!(parse_mention("@devnpc").is_none());
    assert!(parse_mention("没有提及").is_none());
}

#[tokio::test]
async fn test_orchestrator_team_execution_with_signals() {
    // PM 发出 decomposed 信号 → developer 发出 implemented 信号 → tester 执行
    let pm_llm = make_mock_llm(vec![
        "需求已分解为任务1和任务2\n[SIGNAL:decomposed]",
    ]);
    let dev_llm = make_mock_llm(vec![
        "功能已实现,代码已提交\n[SIGNAL:implemented]",
    ]);
    let tester_llm = make_mock_llm(vec![
        "测试通过,所有用例验证成功",
    ]);

    let pm_agent = make_test_agent(pm_llm, "你是 PM,分解需求后输出 [SIGNAL:decomposed]");
    let dev_agent = make_test_agent(dev_llm, "你是开发者,完成后输出 [SIGNAL:implemented]");
    let tester_agent = make_test_agent(tester_llm, "你是测试,验证功能");

    let main_agent = make_test_agent(make_mock_llm(vec!["主Agent"]), "主Agent");

    let mut orchestrator = Orchestrator::new(
        main_agent,
        Some(dev_agent.clone()),
        None,
        Some(tester_agent.clone()),
        None,
        None,
        None,
    );
    orchestrator.register_team_agent("pm", pm_agent);
    orchestrator.register_team_agent("developer", dev_agent);
    orchestrator.register_team_agent("tester", tester_agent);

    let team = make_test_team();
    let result: TeamResult = orchestrator
        .run_team(&team, "实现用户登录功能")
        .await
        .expect("Team 执行失败");

    // 验证: 3 个角色都执行了
    assert_eq!(result.steps.len(), 3, "应有 3 个步骤 (PM→Developer→Tester)");
    assert_eq!(result.steps[0].role, "pm");
    assert_eq!(result.steps[1].role, "developer");
    assert_eq!(result.steps[2].role, "tester");

    // 验证信号检测
    assert!(result.steps[0].signals.contains(&"decomposed".to_string()));
    assert!(result.steps[1].signals.contains(&"implemented".to_string()));

    // 验证汇总非空
    assert!(!result.summary.is_empty());
    assert!(result.summary.contains("pm:"));
    assert!(result.summary.contains("developer:"));
    assert!(result.summary.contains("tester:"));
}

#[tokio::test]
async fn test_orchestrator_team_no_entry_role_errors() {
    // 所有角色都被 handoff.to 引用 → 循环,无入口角色
    let team = Team {
        name: "cyclic-team".into(),
        description: "循环团队".into(),
        npcs: vec![
            TeamNpc {
                role: "a".into(),
                sop: None,
            },
            TeamNpc {
                role: "b".into(),
                sop: None,
            },
        ],
        handoff: vec![
            HandoffRule {
                from: "a".into(),
                to: vec!["b".into()],
                trigger: "a 发出 \"done\" 信号".into(),
            },
            HandoffRule {
                from: "b".into(),
                to: vec!["a".into()],
                trigger: "b 发出 \"done\" 信号".into(),
            },
        ],
        merge: None,
    };

    let main_agent = make_test_agent(make_mock_llm(vec!["ok"]), "主Agent");
    let orchestrator = Orchestrator::new(main_agent, None, None, None, None, None, None);

    let result = orchestrator.run_team(&team, "测试任务").await;
    assert!(result.is_err(), "循环团队应返回错误");
    let err_msg = result.unwrap_err().to_string();
    assert!(err_msg.contains("无入口角色"), "错误信息应包含'无入口角色'");
}

#[tokio::test]
async fn test_orchestrator_team_unregistered_role_errors() {
    let team = Team {
        name: "missing-agent-team".into(),
        description: "缺少Agent的团队".into(),
        npcs: vec![TeamNpc {
            role: "pm".into(),
            sop: None,
        }],
        handoff: vec![],
        merge: None,
    };

    let main_agent = make_test_agent(make_mock_llm(vec!["ok"]), "主Agent");
    let orchestrator = Orchestrator::new(main_agent, None, None, None, None, None, None);

    let result = orchestrator.run_team(&team, "测试任务").await;
    assert!(result.is_err(), "未注册角色应返回错误");
    let err_msg = result.unwrap_err().to_string();
    assert!(err_msg.contains("未注册"), "错误信息应包含'未注册'");
}

#[tokio::test]
async fn test_signal_parsing_formats() {
    use devnpc::adapter::orchestrator::parse_signals;

    // [SIGNAL:xxx] 格式
    let sigs = parse_signals("任务完成\n[SIGNAL:done]");
    assert_eq!(sigs, vec!["done".to_string()]);

    // ## 信号: xxx 格式
    let sigs = parse_signals("任务完成\n## 信号: completed");
    assert_eq!(sigs, vec!["completed".to_string()]);

    // 混合格式
    let sigs = parse_signals("[SIGNAL:decomposed]\n## 信号: verified");
    assert_eq!(sigs, vec!["decomposed".to_string(), "verified".to_string()]);

    // 无信号
    let sigs = parse_signals("普通输出,无信号");
    assert!(sigs.is_empty());
}

#[tokio::test]
async fn test_handoff_trigger_logic() {
    use devnpc::adapter::orchestrator::handoff_triggered;

    // 无信号约束 → 直接触发
    let rule = HandoffRule {
        from: "a".into(),
        to: vec!["b".into()],
        trigger: "无条件触发".into(),
    };
    assert!(handoff_triggered(&rule, &[]));

    // 有信号约束,信号已发出 → 触发
    let rule = HandoffRule {
        from: "pm".into(),
        to: vec!["dev".into()],
        trigger: "pm 发出 \"decomposed\" 信号".into(),
    };
    assert!(handoff_triggered(&rule, &["decomposed".into()]));

    // 有信号约束,信号未发出 → 不触发
    assert!(!handoff_triggered(&rule, &["other".into()]));
    assert!(!handoff_triggered(&rule, &[]));
}

#[tokio::test]
async fn test_memory_store_e2e_save_and_retrieve() {
    let config = MemoryConfig {
        enabled: true,
        db_path: ":memory:".to_string(),
    };
    let store = MemoryStore::new(config);
    store.initialize().expect("初始化失败");

    // 保存任务记录
    let record = devnpc::adapter::memory::TaskRecord {
        task_description: "实现用户认证模块".to_string(),
        result_summary: "认证模块已完成,包含JWT和OAuth".to_string(),
        modified_files: vec!["src/auth.rs".to_string(), "src/oauth.rs".to_string()],
        duration_secs: 3600,
        token_consumption: 50000,
        success: true,
        created_at: "2026-08-01T10:00:00Z".to_string(),
    };
    store.save_task_record(record).expect("保存失败");

    // 保存修复经验
    let exp = devnpc::adapter::memory::FixExperience {
        failure_type: "编译错误".to_string(),
        error_message: "E0277: trait bound not satisfied".to_string(),
        root_cause: "缺少 Clone trait 实现".to_string(),
        fix_method: "为结构体派生 Clone".to_string(),
        success: true,
        created_at: "2026-08-01T11:00:00Z".to_string(),
    };
    store.save_fix_experience(exp).expect("保存失败");

    // 检索相关记忆 (使用空格分词,确保匹配到任务记录和修复经验)
    let results = store
        .retrieve_relevant("认证 编译错误")
        .expect("检索失败");
    assert!(!results.is_empty(), "应检索到相关记忆");

    // 验证同时检索到任务记录和修复经验
    let has_task = results.iter().any(|r| r.contains("[任务]"));
    let has_fix = results.iter().any(|r| r.contains("[修复]"));
    assert!(has_task, "应包含任务记录");
    assert!(has_fix, "应包含修复经验");
}

#[tokio::test]
async fn test_memory_store_disabled_does_nothing() {
    let config = MemoryConfig {
        enabled: false,
        db_path: ":memory:".to_string(),
    };
    let store = MemoryStore::new(config);

    // 禁用状态下所有操作都应是空操作
    let results = store.retrieve_relevant("任何任务").expect("不应报错");
    assert!(results.is_empty());

    store
        .save_task_record(devnpc::adapter::memory::TaskRecord {
            task_description: "test".into(),
            result_summary: "test".into(),
            modified_files: vec![],
            duration_secs: 0,
            token_consumption: 0,
            success: true,
            created_at: String::new(),
        })
        .expect("禁用状态下保存不应报错");
}

#[tokio::test]
async fn test_report_data_construction_from_ci_outcome() {
    use devnpc::report::collector::{Trajectory, TrajectoryEvent};

    // 模拟轨迹采集
    let mut trajectory = Trajectory::new();
    trajectory.record_llm_call(0);
    trajectory.record_llm_call(1);
    trajectory.record_tool_call("read_file", true);
    trajectory.record_tool_call("edit_file", true);

    assert_eq!(trajectory.events.len(), 4);
    let llm_count = trajectory
        .events
        .iter()
        .filter(|e| matches!(e, TrajectoryEvent::LlmCall { .. }))
        .count();
    let tool_count = trajectory
        .events
        .iter()
        .filter(|e| matches!(e, TrajectoryEvent::ToolCall { .. }))
        .count();
    assert_eq!(llm_count, 2);
    assert_eq!(tool_count, 2);

    // 验证 CI 结果类型转换
    let passed = CiOutcome::Passed {
        mr_iid: 42,
        pipeline_id: 100,
        attempts: 1,
    };
    let failed = CiOutcome::Failed {
        mr_iid: 42,
        last_error: "编译失败".into(),
        attempts: 3,
    };
    let timeout = CiOutcome::Timeout {
        mr_iid: 42,
        stage: "test".into(),
    };
    let error = CiOutcome::Error {
        mr_iid: 0,
        reason: "创建 MR 失败".into(),
    };

    assert!(matches!(passed, CiOutcome::Passed { .. }));
    assert!(matches!(failed, CiOutcome::Failed { .. }));
    assert!(matches!(timeout, CiOutcome::Timeout { .. }));
    assert!(matches!(error, CiOutcome::Error { .. }));
    // 关键: Error 不应被误判为 Passed
    assert!(!matches!(error, CiOutcome::Passed { .. }));
}

#[tokio::test]
async fn test_classify_task_complexity_routing() {
    use devnpc::adapter::orchestrator::{classify_task_complexity, TaskComplexity};

    // 复杂任务 (应路由到 complex_model)
    assert_eq!(
        classify_task_complexity("修复登录 bug"),
        TaskComplexity::Complex
    );
    assert_eq!(
        classify_task_complexity("implement new feature"),
        TaskComplexity::Complex
    );
    assert_eq!(
        classify_task_complexity("重构数据库层"),
        TaskComplexity::Complex
    );

    // 简单任务 (应路由到 simple_model)
    assert_eq!(
        classify_task_complexity("阅读 README 文件"),
        TaskComplexity::Simple
    );
    assert_eq!(
        classify_task_complexity("explain how auth works"),
        TaskComplexity::Simple
    );
    assert_eq!(
        classify_task_complexity("审查这个 PR"),
        TaskComplexity::Simple
    );

    // 复杂优先于简单 (同时命中时)
    assert_eq!(
        classify_task_complexity("fix and review the bug"),
        TaskComplexity::Complex
    );
}

#[tokio::test]
async fn test_usage_stats_accumulation() {
    use devnpc::adapter::orchestrator::UsageStats;

    let stats = UsageStats {
        input_tokens: 1000,
        output_tokens: 500,
        llm_calls: 5,
        estimated_cost_usd: 0.0,
    };

    assert_eq!(stats.total_tokens(), 1500);
    // 默认费率估算 (provider 未返回 cost 时)
    let cost = stats.estimated_cost_or_default();
    assert!(cost > 0.0, "估算成本应大于 0");

    // 有 provider 返回的 cost 时使用它
    let stats_with_cost = UsageStats {
        input_tokens: 1000,
        output_tokens: 500,
        llm_calls: 5,
        estimated_cost_usd: 0.05,
    };
    assert_eq!(stats_with_cost.estimated_cost_or_default(), 0.05);
}
