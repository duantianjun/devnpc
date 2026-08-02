//! Orchestrator Agent: 任务拆解、分发、结果汇总
//!
//! 将子 Agent 调用封装为 FunctionTool，通过 Orchestrator Agent 统一调度。
//! 子 Agent 不直接相互调用，通过 Orchestrator 传递中间结果，保持解耦。
//!
//! 模型路由: 通过 `classify_task_complexity` 在 Agent 构建前判定任务复杂度,
//! 主 Agent 按任务类型选择 simple_model 或 complex_model。
//!
//! Token 估算: 累积 LlmResponse.usage_metadata (真实 provider 计费数据),
//! 替代固定公式 (llm_calls * 500), 报告更准确。
//!
//! Team 编排: 通过 `run_team` 按 Team 配置驱动多 Agent 协作流程 (PM→Developer→Tester),
//! 基于 handoff 规则和信号传递 (decomposed/implemented) 串联子 Agent。

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

use adk_rust::agent::LlmAgent;
use adk_rust::runner::Runner;
use adk_rust::session::{CreateRequest, InMemorySessionService, SessionService};
use adk_rust::{Content, SessionId, UsageMetadata, UserId, Llm};
use futures::StreamExt;

use crate::adapter::memory::MemoryStore;
use crate::config::npc_config::{HandoffRule, Team};
use crate::error::Result;

/// 任务复杂度分类
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskComplexity {
    /// 简单任务: 阅读/搜索/总结/解释,使用小模型降低成本
    Simple,
    /// 复杂任务: 修复/实现/重构/添加/优化,使用大模型保证质量
    Complex,
}

/// 根据任务描述判定复杂度,用于模型路由
///
/// 规则:
/// - 命中复杂关键词 (fix/implement/refactor/...) → Complex
/// - 否则命中简单关键词 (read/search/explain/...) → Simple
/// - 都不命中 → Complex (保守起见,大模型对小任务也能胜任)
pub fn classify_task_complexity(description: &str) -> TaskComplexity {
    let lower = description.to_lowercase();

    const COMPLEX_KEYWORDS: &[&str] = &[
        "fix", "implement", "refactor", "add", "modify", "optimize", "migrate", "build",
        "deploy", "refactor", "rewrite", "重构", "修复", "实现", "添加", "修改", "优化",
        "迁移", "编译", "部署", "重写", "改造", "完善",
    ];
    const SIMPLE_KEYWORDS: &[&str] = &[
        "read", "search", "list", "find", "explain", "summarize", "outline", "describe",
        "review", "阅读", "查找", "列出", "解释", "总结", "大纲", "描述", "审查", "查看",
        "查询", "梳理",
    ];

    let complex_hit = COMPLEX_KEYWORDS.iter().any(|k| lower.contains(k));
    if complex_hit {
        return TaskComplexity::Complex;
    }

    let simple_hit = SIMPLE_KEYWORDS.iter().any(|k| lower.contains(k));
    if simple_hit {
        return TaskComplexity::Simple;
    }

    // 默认复杂,避免小模型能力不足导致任务失败
    TaskComplexity::Complex
}

/// 子 Agent 类型
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubAgentKind {
    /// 代码修改 Agent
    Code,
    /// CI 修复 Agent
    Fix,
    /// 代码审查 Agent
    Review,
}

/// 真实 token 使用统计 (从 provider 返回的 usage_metadata 累积)
#[derive(Debug, Clone, Default)]
pub struct UsageStats {
    /// 输入 token (prompt tokens)
    pub input_tokens: i64,
    /// 输出 token (candidates tokens)
    pub output_tokens: i64,
    /// LLM 调用次数 (有 usage_metadata 的非 partial 事件数)
    pub llm_calls: u64,
    /// 估算成本 (USD, provider 返回的 cost 字段累加; 若 provider 不提供则按默认费率估算)
    pub estimated_cost_usd: f64,
}

impl UsageStats {
    /// 总 token 数
    pub fn total_tokens(&self) -> i64 {
        self.input_tokens + self.output_tokens
    }

    /// 当 provider 未返回 cost 时, 用默认费率估算
    /// - input: $1.5/M tokens (DeepSeek/OpenAI 入门级)
    /// - output: $2.0/M tokens
    pub fn estimated_cost_or_default(&self) -> f64 {
        if self.estimated_cost_usd > 0.0 {
            self.estimated_cost_usd
        } else {
            (self.input_tokens as f64 * 0.000_001_5) + (self.output_tokens as f64 * 0.000_002_0)
        }
    }
}

/// Orchestrator: 负责任务编排
pub struct Orchestrator {
    /// 主 Agent (Orchestrator 自身)
    pub agent: Arc<LlmAgent>,
    /// 子 Agent
    pub code_agent: Option<Arc<LlmAgent>>,
    pub fix_agent: Option<Arc<LlmAgent>>,
    pub review_agent: Option<Arc<LlmAgent>>,
    /// 简单模型 (小模型，用于阅读/搜索)
    pub simple_model: Option<Arc<dyn Llm>>,
    /// 复杂模型 (大模型，用于改码/修复/推理)
    pub complex_model: Option<Arc<dyn Llm>>,
    /// 长期记忆存储器
    pub memory_store: Option<MemoryStore>,
    /// 累积的 token 使用统计 (跨多次 run_*_agent 调用)
    usage_stats: Arc<Mutex<UsageStats>>,
    /// Team 编排用: 角色 → Agent 映射 (pm/developer/tester 等)
    team_agents: HashMap<String, Arc<LlmAgent>>,
}

/// Team 执行单步结果
#[derive(Debug, Clone)]
pub struct TeamStep {
    /// 执行的角色名
    pub role: String,
    /// 输入指令
    pub instruction: String,
    /// Agent 输出
    pub output: String,
    /// 检测到的信号 (decomposed/implemented 等)
    pub signals: Vec<String>,
}

/// Team 执行结果
#[derive(Debug, Clone, Default)]
pub struct TeamResult {
    /// 各角色执行步骤 (按执行顺序)
    pub steps: Vec<TeamStep>,
    /// 最终汇总文本
    pub summary: String,
}

impl Orchestrator {
    pub fn new(
        agent: Arc<LlmAgent>,
        code_agent: Option<Arc<LlmAgent>>,
        fix_agent: Option<Arc<LlmAgent>>,
        review_agent: Option<Arc<LlmAgent>>,
        simple_model: Option<Arc<dyn Llm>>,
        complex_model: Option<Arc<dyn Llm>>,
        memory_store: Option<MemoryStore>,
    ) -> Self {
        Self {
            agent,
            code_agent,
            fix_agent,
            review_agent,
            simple_model,
            complex_model,
            memory_store,
            usage_stats: Arc::new(Mutex::new(UsageStats::default())),
            team_agents: HashMap::new(),
        }
    }

    /// 注册 Team 编排用的角色 Agent
    ///
    /// 在 `run_team` 调用前注册所有 team.npcs 中声明的角色对应的 Agent。
    /// 同一角色重复注册会覆盖旧值。
    pub fn register_team_agent(&mut self, role: &str, agent: Arc<LlmAgent>) {
        self.team_agents.insert(role.to_string(), agent);
    }

    /// 查询已注册的角色
    pub fn team_agent(&self, role: &str) -> Option<&Arc<LlmAgent>> {
        self.team_agents.get(role)
    }

    /// 按 Team 配置执行多 Agent 协作流程
    ///
    /// 执行模型:
    /// 1. 找到入口角色 (在 team.npcs 中但不是任何 handoff.to 的角色)
    /// 2. 顺序执行入口角色 Agent
    /// 3. 从 Agent 输出解析信号 (格式: `[SIGNAL:xxx]` 或 `## 信号: xxx`)
    /// 4. 匹配 handoff 规则: 若 rule.from == 当前角色 且 (trigger 信号已发出 或 trigger 无信号约束)
    ///    → 递归执行 rule.to 中所有角色
    /// 5. 汇总所有步骤的输出
    ///
    /// 防环: 用 `executed` 集合记录已执行角色,避免循环引用导致的死循环。
    pub async fn run_team(&self, team: &Team, task: &str) -> Result<TeamResult> {
        let mut result = TeamResult::default();
        let mut executed: HashSet<String> = HashSet::new();

        let entry_roles = find_entry_roles(team);
        if entry_roles.is_empty() {
            return Err(crate::error::DevnpcError::Config(format!(
                "团队 {} 无入口角色 (所有角色都被 handoff.to 引用,存在循环)",
                team.name
            )));
        }

        let mut current_input = task.to_string();
        for role in &entry_roles {
            self.execute_role_chain(team, role, &current_input, &mut executed, &mut result)
                .await?;
            // 后续入口角色使用最后一步输出作为输入 (串联)
            if let Some(last) = result.steps.last() {
                current_input = last.output.clone();
            }
        }

        // 合并结果 (按 merge 策略; single-mr: 拼接所有步骤)
        result.summary = result
            .steps
            .iter()
            .map(|s| format!("## {}:\n{}", s.role, s.output))
            .collect::<Vec<_>>()
            .join("\n\n");

        Ok(result)
    }

    /// 递归执行角色链: 执行当前角色 → 解析信号 → 匹配 handoff → 递归执行下游角色
    async fn execute_role_chain(
        &self,
        team: &Team,
        role: &str,
        input: &str,
        executed: &mut HashSet<String>,
        result: &mut TeamResult,
    ) -> Result<()> {
        if executed.contains(role) {
            tracing::debug!(role = %role, "角色已执行,跳过 (防环)");
            return Ok(());
        }
        executed.insert(role.to_string());

        let agent = self.team_agents.get(role).ok_or_else(|| {
            crate::error::DevnpcError::Config(format!(
                "团队角色 {} 未注册 Agent,请先调用 register_team_agent",
                role
            ))
        })?;

        // 计算该角色应发出的信号 (从 handoff 规则提取 trigger 中的信号名)
        let expected_signals: Vec<String> = team
            .handoff
            .iter()
            .filter(|r| r.from == role)
            .filter_map(|r| parse_trigger_signal(&r.trigger))
            .collect();

        // 在输入末尾追加信号提醒,确保 Agent 输出可被 parse_signals 识别
        let team_input = if expected_signals.is_empty() {
            input.to_string()
        } else {
            format!(
                "{}\n\n## 协作信号\n完成本阶段任务后,请在输出末尾追加信号标记: {}",
                input,
                expected_signals
                    .iter()
                    .map(|s| format!("[SIGNAL:{}]", s))
                    .collect::<Vec<_>>()
                    .join(" 或 ")
            )
        };

        let app_name = format!("devnpc-team-{}", role);
        tracing::info!(role = %role, expected_signals = ?expected_signals, "执行 Team 角色");
        let output = self.run_sub_agent(agent, &app_name, &team_input).await?;
        let signals = parse_signals(&output);

        tracing::info!(
            role = %role,
            signals = ?signals,
            output_len = output.len(),
            "Team 角色执行完成"
        );

        result.steps.push(TeamStep {
            role: role.to_string(),
            instruction: input.to_string(),
            output: output.clone(),
            signals: signals.clone(),
        });

        // 匹配 handoff 规则,递归执行下游角色
        for rule in &team.handoff {
            if rule.from != role {
                continue;
            }
            let triggered = handoff_triggered(rule, &signals);
            if !triggered {
                tracing::debug!(
                    from = %rule.from,
                    to = ?rule.to,
                    trigger = %rule.trigger,
                    "handoff 信号未满足,跳过"
                );
                continue;
            }
            tracing::info!(
                from = %rule.from,
                to = ?rule.to,
                "handoff 触发,执行下游角色"
            );
            for next_role in &rule.to {
                Box::pin(self.execute_role_chain(team, next_role, &output, executed, result))
                    .await?;
            }
        }

        Ok(())
    }

    /// 根据任务复杂度选择模型
    ///
    /// - 简单任务 (阅读/搜索/总结) → simple_model
    /// - 复杂任务 (修复/实现/重构) → complex_model
    /// - 若目标模型未配置,则回退到另一个已配置的模型
    pub fn pick_model_for_task(&self, description: &str) -> Option<&Arc<dyn Llm>> {
        let complexity = classify_task_complexity(description);
        let (primary, fallback) = match complexity {
            TaskComplexity::Simple => (&self.simple_model, &self.complex_model),
            TaskComplexity::Complex => (&self.complex_model, &self.simple_model),
        };
        if primary.is_some() {
            tracing::info!(?complexity, target = "primary", "模型路由命中");
            primary.as_ref()
        } else if fallback.is_some() {
            tracing::info!(?complexity, target = "fallback", "模型路由回退");
            fallback.as_ref()
        } else {
            tracing::warn!(?complexity, "模型路由: simple_model 和 complex_model 均未配置");
            None
        }
    }

    /// 累积单次 LLM 调用的 usage_metadata 到全局统计
    ///
    /// 只统计非 partial 事件 (即每次 model call 的最终结果), 避免重复计数。
    fn accumulate_usage(&self, usage: Option<&UsageMetadata>) {
        let Some(u) = usage else { return };
        let mut stats = self.usage_stats.lock().expect("usage_stats mutex poisoned");
        stats.input_tokens += u.prompt_token_count as i64;
        stats.output_tokens += u.candidates_token_count as i64;
        if let Some(cost) = u.cost {
            stats.estimated_cost_usd += cost;
        }
        stats.llm_calls += 1;
    }

    /// 取出累积的 token 使用统计 (drain), 用于报告
    pub fn take_usage_stats(&self) -> UsageStats {
        let mut stats = self.usage_stats.lock().expect("usage_stats mutex poisoned");
        std::mem::take(&mut *stats)
    }

    /// 当前累积的 token 使用统计 (不 drain), 用于运行中查询
    pub fn usage_stats(&self) -> UsageStats {
        let stats = self.usage_stats.lock().expect("usage_stats mutex poisoned");
        stats.clone()
    }

    /// 通用子 Agent 执行逻辑 (供 run_fix_agent / run_code_agent / run_review_agent 复用)
    ///
    /// 每次 invocation 创建独立的 InMemorySessionService, 避免子 Agent 间状态污染。
    /// 累积每个非 partial 事件的 usage_metadata 到全局统计。
    async fn run_sub_agent(
        &self,
        agent: &Arc<LlmAgent>,
        app_name: &str,
        instruction: &str,
    ) -> Result<String> {
        let session_service: Arc<dyn SessionService> = Arc::new(InMemorySessionService::new());
        let session_id = format!("{}-{}", app_name, uuid::Uuid::new_v4());
        let session_id_typed = SessionId::try_from(session_id.as_str()).map_err(|e| {
            crate::error::DevnpcError::Config(format!("SessionId 创建失败: {e}"))
        })?;

        session_service
            .create(CreateRequest {
                app_name: app_name.to_string(),
                user_id: "devnpc".to_string(),
                session_id: Some(session_id_typed.to_string()),
                state: std::collections::HashMap::new(),
            })
            .await
            .map_err(|e| crate::error::DevnpcError::Config(format!("会话创建失败: {e}")))?;

        let runner = Runner::builder()
            .app_name(app_name)
            .agent(agent.clone())
            .session_service(session_service)
            .build()
            .map_err(|e| crate::error::DevnpcError::Config(format!("{app_name} Runner 构建失败: {e}")))?;

        let content = Content::new("user").with_text(instruction);
        let user_id = UserId::new("devnpc").map_err(|e| {
            crate::error::DevnpcError::Config(format!("UserId 创建失败: {e}"))
        })?;

        let mut stream = runner
            .run(user_id, session_id_typed, content)
            .await
            .map_err(|e| crate::error::DevnpcError::Config(format!("{app_name} 执行失败: {e}")))?;

        let mut result = String::new();
        while let Some(event_result) = stream.next().await {
            if let Ok(event) = event_result {
                // 累积 usage (只统计非 partial 事件,避免重复计数)
                if !event.llm_response.partial {
                    self.accumulate_usage(event.llm_response.usage_metadata.as_ref());
                }
                if event.is_final_response()
                    && let Some(content) = &event.llm_response.content
                {
                    for part in &content.parts {
                        if let Some(text) = part.text() {
                            result.push_str(text);
                        }
                    }
                }
            }
        }

        Ok(result)
    }

    /// 运行主 Agent 执行任务
    pub async fn run(
        &self,
        user_input: &str,
        session_service: Arc<dyn SessionService>,
        session_id: &str,
        initial_state: std::collections::HashMap<String, adk_rust::serde_json::Value>,
    ) -> Result<String> {
        let session_id_typed = SessionId::try_from(session_id).map_err(|e| {
            crate::error::DevnpcError::Config(format!("SessionId 创建失败: {e}"))
        })?;

        session_service
            .create(CreateRequest {
                app_name: "devnpc".to_string(),
                user_id: "devnpc".to_string(),
                session_id: Some(session_id_typed.to_string()),
                state: initial_state,
            })
            .await
            .map_err(|e| crate::error::DevnpcError::Config(format!("会话创建失败: {e}")))?;

        let runner = Runner::builder()
            .app_name("devnpc")
            .agent(self.agent.clone())
            .session_service(session_service)
            .build()
            .map_err(|e| crate::error::DevnpcError::Config(format!("Runner 构建失败: {e}")))?;

        let content = Content::new("user").with_text(user_input);
        let user_id = UserId::new("devnpc").map_err(|e| {
            crate::error::DevnpcError::Config(format!("UserId 创建失败: {e}"))
        })?;

        let mut stream = runner
            .run(user_id, session_id_typed, content)
            .await
            .map_err(|e| crate::error::DevnpcError::Config(format!("Agent 执行失败: {e}")))?;

        let mut final_text = String::new();
        while let Some(event_result) = stream.next().await {
            if let Ok(event) = event_result {
                if !event.llm_response.partial {
                    self.accumulate_usage(event.llm_response.usage_metadata.as_ref());
                }
                if event.is_final_response()
                    && let Some(content) = &event.llm_response.content
                {
                    for part in &content.parts {
                        if let Some(text) = part.text() {
                            final_text.push_str(text);
                        }
                    }
                }
            }
        }

        Ok(final_text)
    }

    /// 运行 Code Agent (代码修改)
    pub async fn run_code_agent(&self, instruction: &str) -> Result<String> {
        let agent = self.code_agent.as_ref().ok_or_else(|| {
            crate::error::DevnpcError::Config("Code Agent 未配置".to_string())
        })?;
        self.run_sub_agent(agent, "devnpc-code", instruction).await
    }

    /// 运行 Fix Agent 执行 CI 修复
    pub async fn run_fix_agent(&self, instruction: &str) -> Result<String> {
        let agent = self.fix_agent.as_ref().ok_or_else(|| {
            crate::error::DevnpcError::Config("Fix Agent 未配置".to_string())
        })?;
        self.run_sub_agent(agent, "devnpc-fix", instruction).await
    }

    /// 运行 Review Agent (代码审查)
    pub async fn run_review_agent(&self, instruction: &str) -> Result<String> {
        let agent = self.review_agent.as_ref().ok_or_else(|| {
            crate::error::DevnpcError::Config("Review Agent 未配置".to_string())
        })?;
        self.run_sub_agent(agent, "devnpc-review", instruction).await
    }

    /// 并行运行多个子 Agent (按 SubAgentKind 分发)
    ///
    /// 适用场景: 一个用户请求中包含多个独立子任务 (例如 "修复 bug A 并审查 PR B"),
    /// 可同时启动 code_agent / fix_agent / review_agent 多个实例。
    /// 每个 invocation 使用独立的 SessionService, 互不干扰。
    ///
    /// 注意: 调用方需 `Arc<Orchestrator>` 才能并行 borrow self。
    pub async fn run_agents_parallel(
        self: Arc<Self>,
        tasks: Vec<(SubAgentKind, String)>,
    ) -> Vec<Result<String>> {
        if tasks.is_empty() {
            return Vec::new();
        }
        tracing::info!(count = tasks.len(), "并行启动子 Agent");

        let futures: Vec<_> = tasks
            .into_iter()
            .map(|(kind, instruction)| {
                let this = self.clone();
                async move {
                    match kind {
                        SubAgentKind::Code => this.run_code_agent(&instruction).await,
                        SubAgentKind::Fix => this.run_fix_agent(&instruction).await,
                        SubAgentKind::Review => this.run_review_agent(&instruction).await,
                    }
                }
            })
            .collect();

        futures::future::join_all(futures).await
    }

    /// 运行主 Agent 执行任务 (带记忆注入)
    pub async fn run_with_memory(
        &self,
        user_input: &str,
        session_service: Arc<dyn SessionService>,
        session_id: &str,
        initial_state: std::collections::HashMap<String, adk_rust::serde_json::Value>,
    ) -> Result<String> {
        // 检索相关记忆并注入
        if let Some(ref store) = self.memory_store
            && let Ok(history) = store.retrieve_relevant(user_input)
            && !history.is_empty()
        {
            tracing::info!(count = history.len(), "注入历史记忆到 Agent 上下文");
            let enriched_input = format!(
                "{}\n\n## 历史相关记忆\n{}",
                user_input,
                history.join("\n---\n")
            );
            return self.run(&enriched_input, session_service, session_id, initial_state).await;
        }
        self.run(user_input, session_service, session_id, initial_state).await
    }
}

/// 找到 Team 的入口角色: 在 team.npcs 中但不在任何 handoff.to 中的角色
///
/// 入口角色是协作流程的起点 (如 PM),下游角色 (developer/tester) 通过 handoff 触发。
/// 若所有角色都被 handoff.to 引用,说明存在循环,返回空 Vec (调用方据此报错)。
fn find_entry_roles(team: &Team) -> Vec<String> {
    let all_to: HashSet<String> = team
        .handoff
        .iter()
        .flat_map(|r| r.to.iter().cloned())
        .collect();
    team.npcs
        .iter()
        .map(|n| n.role.clone())
        .filter(|role| !all_to.contains(role))
        .collect()
}

/// 解析 handoff trigger 字段中的信号名
///
/// trigger 格式: `pm 发出 "decomposed" 信号` → 提取双引号内的 `decomposed`
/// 若 trigger 不包含双引号信号,返回 None (表示无信号约束,直接触发)。
fn parse_trigger_signal(trigger: &str) -> Option<String> {
    let start = trigger.find('"')?;
    let rest = &trigger[start + 1..];
    let end = rest.find('"')?;
    Some(rest[..end].to_string())
}

/// 判断 handoff 规则是否被触发
///
/// - 若 rule.trigger 不含信号约束 (无双引号) → 直接触发 (返回 true)
/// - 若 rule.trigger 含信号约束 → 检查 signals 中是否包含该信号
fn handoff_triggered(rule: &HandoffRule, signals: &[String]) -> bool {
    let Some(expected) = parse_trigger_signal(&rule.trigger) else {
        return true; // 无信号约束
    };
    signals.iter().any(|s| s == &expected)
}

/// 从 Agent 输出中检测信号标记
///
/// 支持两种格式:
/// - `[SIGNAL:decomposed]` (推荐,机器可读)
/// - `## 信号: decomposed` (中文友好格式)
///
/// 建议在 Role system_prompt 中要求 Agent 完成阶段任务后输出对应信号标记。
fn parse_signals(output: &str) -> Vec<String> {
    let mut signals = Vec::new();
    for line in output.lines() {
        let trimmed = line.trim();
        // [SIGNAL:xxx]
        if let Some(rest) = trimmed.strip_prefix("[SIGNAL:") {
            if let Some(end) = rest.find(']') {
                let sig = rest[..end].trim().to_string();
                if !sig.is_empty() && !signals.contains(&sig) {
                    signals.push(sig);
                }
            }
            continue;
        }
        // ## 信号: xxx (大小写不敏感)
        let lower = trimmed.to_lowercase();
        if let Some(rest) = lower.strip_prefix("## 信号:") {
            let sig = rest.trim().to_string();
            if !sig.is_empty() && !signals.contains(&sig) {
                signals.push(sig);
            }
        }
    }
    signals
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_classify_complex_keywords_en() {
        assert_eq!(classify_task_complexity("fix the login bug"), TaskComplexity::Complex);
        assert_eq!(
            classify_task_complexity("implement user registration"),
            TaskComplexity::Complex
        );
        assert_eq!(
            classify_task_complexity("refactor the auth module"),
            TaskComplexity::Complex
        );
        assert_eq!(
            classify_task_complexity("add new endpoint"),
            TaskComplexity::Complex
        );
        assert_eq!(
            classify_task_complexity("optimize the query"),
            TaskComplexity::Complex
        );
    }

    #[test]
    fn test_classify_complex_keywords_zh() {
        assert_eq!(classify_task_complexity("修复登录 bug"), TaskComplexity::Complex);
        assert_eq!(
            classify_task_complexity("实现用户注册功能"),
            TaskComplexity::Complex
        );
        assert_eq!(classify_task_complexity("重构认证模块"), TaskComplexity::Complex);
        assert_eq!(classify_task_complexity("添加新接口"), TaskComplexity::Complex);
        assert_eq!(classify_task_complexity("优化查询性能"), TaskComplexity::Complex);
    }

    #[test]
    fn test_classify_simple_keywords_en() {
        assert_eq!(classify_task_complexity("read the README"), TaskComplexity::Simple);
        assert_eq!(
            classify_task_complexity("search for usage of foo"),
            TaskComplexity::Simple
        );
        assert_eq!(
            classify_task_complexity("explain how auth works"),
            TaskComplexity::Simple
        );
        assert_eq!(
            classify_task_complexity("summarize the file"),
            TaskComplexity::Simple
        );
        assert_eq!(
            classify_task_complexity("review this PR"),
            TaskComplexity::Simple
        );
    }

    #[test]
    fn test_classify_simple_keywords_zh() {
        assert_eq!(classify_task_complexity("阅读 README"), TaskComplexity::Simple);
        assert_eq!(
            classify_task_complexity("查找 foo 的用法"),
            TaskComplexity::Simple
        );
        assert_eq!(classify_task_complexity("解释认证流程"), TaskComplexity::Simple);
        assert_eq!(classify_task_complexity("总结这个文件"), TaskComplexity::Simple);
        assert_eq!(classify_task_complexity("审查这个 PR"), TaskComplexity::Simple);
    }

    #[test]
    fn test_classify_complex_takes_priority_over_simple() {
        // 同时命中简单和复杂关键词时,复杂优先
        assert_eq!(
            classify_task_complexity("read and fix the bug"),
            TaskComplexity::Complex
        );
        assert_eq!(
            classify_task_complexity("review and refactor the module"),
            TaskComplexity::Complex
        );
    }

    #[test]
    fn test_classify_no_keyword_defaults_to_complex() {
        // 无关键词命中时,默认 Complex (保守策略)
        assert_eq!(classify_task_complexity("hello world"), TaskComplexity::Complex);
        assert_eq!(classify_task_complexity(""), TaskComplexity::Complex);
        assert_eq!(
            classify_task_complexity("do something"),
            TaskComplexity::Complex
        );
    }

    #[test]
    fn test_classify_case_insensitive() {
        assert_eq!(classify_task_complexity("FIX the bug"), TaskComplexity::Complex);
        assert_eq!(
            classify_task_complexity("READ the file"),
            TaskComplexity::Simple
        );
        assert_eq!(classify_task_complexity("Fix"), TaskComplexity::Complex);
    }

    #[test]
    fn test_usage_stats_default() {
        let stats = UsageStats::default();
        assert_eq!(stats.input_tokens, 0);
        assert_eq!(stats.output_tokens, 0);
        assert_eq!(stats.llm_calls, 0);
        assert_eq!(stats.estimated_cost_usd, 0.0);
        assert_eq!(stats.total_tokens(), 0);
    }

    #[test]
    fn test_usage_stats_total_tokens() {
        let stats = UsageStats {
            input_tokens: 1500,
            output_tokens: 800,
            llm_calls: 3,
            estimated_cost_usd: 0.0,
        };
        assert_eq!(stats.total_tokens(), 2300);
    }

    #[test]
    fn test_usage_stats_estimated_cost_or_default_with_provider_cost() {
        // provider 返回了 cost, 直接使用
        let stats = UsageStats {
            input_tokens: 1000,
            output_tokens: 500,
            llm_calls: 1,
            estimated_cost_usd: 0.0123,
        };
        assert!((stats.estimated_cost_or_default() - 0.0123).abs() < 1e-9);
    }

    #[test]
    fn test_usage_stats_estimated_cost_or_default_fallback() {
        // provider 未返回 cost, 按默认费率计算
        // input=1000 * $1.5/M = $0.0015, output=500 * $2.0/M = $0.0010, 总和 $0.0025
        let stats = UsageStats {
            input_tokens: 1000,
            output_tokens: 500,
            llm_calls: 1,
            estimated_cost_usd: 0.0,
        };
        let cost = stats.estimated_cost_or_default();
        assert!((cost - 0.0025).abs() < 1e-9, "expected 0.0025, got {cost}");
    }

    // ===== Team handoff 测试 =====

    fn make_team_npc(role: &str) -> crate::config::npc_config::TeamNpc {
        crate::config::npc_config::TeamNpc {
            role: role.into(),
            sop: None,
        }
    }

    fn make_handoff(from: &str, to: Vec<&str>, trigger: &str) -> HandoffRule {
        HandoffRule {
            from: from.into(),
            to: to.into_iter().map(String::from).collect(),
            trigger: trigger.into(),
        }
    }

    fn make_feature_team() -> Team {
        Team {
            name: "feature-team".into(),
            description: "PM+开发+测试".into(),
            npcs: vec![
                make_team_npc("pm"),
                make_team_npc("developer"),
                make_team_npc("tester"),
            ],
            handoff: vec![
                make_handoff("pm", vec!["developer", "tester"], "pm 发出 \"decomposed\" 信号"),
                make_handoff("developer", vec!["tester"], "developer 发出 \"implemented\" 信号"),
            ],
            merge: Some(crate::config::npc_config::MergeStrategy {
                strategy: "single-mr".into(),
            }),
        }
    }

    #[test]
    fn test_find_entry_roles_returns_pm() {
        let team = make_feature_team();
        let entries = find_entry_roles(&team);
        // pm 是入口角色 (不在任何 handoff.to 中)
        assert_eq!(entries, vec!["pm"]);
    }

    #[test]
    fn test_find_entry_roles_empty_when_cycle() {
        let team = Team {
            name: "cycle".into(),
            description: String::new(),
            npcs: vec![make_team_npc("a"), make_team_npc("b")],
            handoff: vec![
                make_handoff("a", vec!["b"], "x"),
                make_handoff("b", vec!["a"], "y"),
            ],
            merge: None,
        };
        // a 和 b 互相引用,无入口角色
        let entries = find_entry_roles(&team);
        assert!(entries.is_empty());
    }

    #[test]
    fn test_parse_trigger_signal_extracts_quoted() {
        assert_eq!(
            parse_trigger_signal("pm 发出 \"decomposed\" 信号"),
            Some("decomposed".into())
        );
        assert_eq!(
            parse_trigger_signal("developer 发出 \"implemented\" 信号"),
            Some("implemented".into())
        );
    }

    #[test]
    fn test_parse_trigger_signal_none_when_no_quote() {
        // 无双引号 → 无信号约束
        assert_eq!(parse_trigger_signal("完成后触发"), None);
        assert_eq!(parse_trigger_signal(""), None);
    }

    #[test]
    fn test_handoff_triggered_with_signal_match() {
        let rule = make_handoff("pm", vec!["dev"], "pm 发出 \"decomposed\" 信号");
        assert!(handoff_triggered(&rule, &["decomposed".into()]));
        assert!(handoff_triggered(&rule, &["other".into(), "decomposed".into()]));
    }

    #[test]
    fn test_handoff_triggered_signal_not_matched() {
        let rule = make_handoff("pm", vec!["dev"], "pm 发出 \"decomposed\" 信号");
        assert!(!handoff_triggered(&rule, &["implemented".into()]));
        assert!(!handoff_triggered(&rule, &[]));
    }

    #[test]
    fn test_handoff_triggered_when_no_signal_constraint() {
        // trigger 无双引号 → 直接触发
        let rule = make_handoff("pm", vec!["dev"], "完成后自动触发");
        assert!(handoff_triggered(&rule, &[]));
        assert!(handoff_triggered(&rule, &["anything".into()]));
    }

    #[test]
    fn test_parse_signals_signal_bracket_format() {
        let output = "任务完成\n[SIGNAL:decomposed]\n下一步";
        let signals = parse_signals(output);
        assert_eq!(signals, vec!["decomposed"]);
    }

    #[test]
    fn test_parse_signals_chinese_format() {
        let output = "需求已拆分\n## 信号: decomposed\n继续";
        let signals = parse_signals(output);
        assert_eq!(signals, vec!["decomposed"]);
    }

    #[test]
    fn test_parse_signals_multiple_distinct() {
        let output = "[SIGNAL:decomposed]\n实现完成\n## 信号: implemented";
        let signals = parse_signals(output);
        assert_eq!(signals, vec!["decomposed", "implemented"]);
    }

    #[test]
    fn test_parse_signals_deduplicates() {
        let output = "[SIGNAL:decomposed]\n[SIGNAL:decomposed]";
        let signals = parse_signals(output);
        assert_eq!(signals, vec!["decomposed"]);
    }

    #[test]
    fn test_parse_signals_empty_output() {
        assert!(parse_signals("").is_empty());
        assert!(parse_signals("无信号文本").is_empty());
    }

    #[test]
    fn test_parse_signals_case_insensitive_chinese_prefix() {
        // ## 信号: 大小写不敏感
        let output = "## 信号: implemented";
        let signals = parse_signals(output);
        assert_eq!(signals, vec!["implemented"]);
    }

    #[test]
    fn test_team_result_default_empty() {
        let result = TeamResult::default();
        assert!(result.steps.is_empty());
        assert!(result.summary.is_empty());
    }

    #[test]
    fn test_team_step_fields() {
        let step = TeamStep {
            role: "pm".into(),
            instruction: "拆分需求".into(),
            output: "已完成".into(),
            signals: vec!["decomposed".into()],
        };
        assert_eq!(step.role, "pm");
        assert_eq!(step.signals, vec!["decomposed"]);
    }
}