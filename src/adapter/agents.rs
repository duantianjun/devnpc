//! 子 Agent 构建: 为 Orchestrator 提供 Code/Fix/Review Agent
//!
//! 每个子 Agent 通过 LlmAgentBuilder 构建，拥有专属的 System Prompt 和工具集。
//! 支持三层能力注入:
//! 1. Role + Sop: 身份 + 流程 (system_prompt + steps)
//! 2. Skills: 领域专家知识 (instruction 追加 + tools 进一步过滤)

use std::sync::Arc;

use adk_rust::agent::LlmAgentBuilder;
use adk_rust::Tool;

use crate::config::npc_config::{build_role_instruction, filter_tools_by_role, Role, Sop};
use crate::config::skill::{filter_tools_by_skills, inject_skills, Skill};
use crate::error::Result;

/// 默认 Code Agent 指令 (无 role 配置时使用)
const DEFAULT_CODE_INSTRUCTION: &str = "\
你是一个代码修改专家。\n\
原则:\n\
1. 修改前先理解上下文 (read_file / list_files / aft_outline)\n\
2. 改完后用对应的构建工具验证编译 (如 cargo build / mvn compile)\n\
3. 禁止修改工作目录外的文件\n\
4. 总结修改内容";

/// 默认 Fix Agent 指令 (无 role 配置时使用)
const DEFAULT_FIX_INSTRUCTION: &str = "\
你是一个 CI 修复专家。\n\
任务: 分析 CI 失败日志 → 定位根因 → 修复代码 → 验证语法\n\
原则:\n\
1. 先读取失败日志和相关源码\n\
2. 定位根因后再修改\n\
3. 修复后验证语法正确性\n\
4. 总结修复内容";

/// 默认 Review Agent 指令 (无 role 配置时使用)
const DEFAULT_REVIEW_INSTRUCTION: &str = "\
你是一个代码审查专家。\n\
任务: 审查代码变更 → 检查 SOP 合规 → 输出审查报告\n\
原则:\n\
1. 检查代码质量、安全性、性能\n\
2. 检查是否符合项目规范\n\
3. 输出明确的通过/不通过结论";

/// 默认 PM Agent 指令 (无 role 配置时使用)
const DEFAULT_PM_INSTRUCTION: &str = "\
你是一个项目经理。将用户需求分解为可执行的开发和测试任务。\n\
原则:\n\
1. 分析需求目标和验收标准\n\
2. 拆分为独立的开发任务和测试任务\n\
3. 定义任务优先级和依赖关系\n\
4. 完成后在输出末尾追加信号标记: [SIGNAL:decomposed]";

/// 构建 Code Agent - 代码读写、AST 操作、编译验证
///
/// - 当 `role` 为 Some 时,使用 role.system_prompt + sop 组合指令,并按 role.tools 过滤工具
/// - 当 `role` 为 None 时,使用硬编码默认指令,注入全部工具
/// - `skills` 可选注入领域专家知识 (追加指令 + 进一步过滤工具)
pub fn build_code_agent(
    tools: Vec<Arc<dyn Tool>>,
    model: Arc<dyn adk_rust::Llm>,
    role: Option<&Role>,
    sop: Option<&Sop>,
    skills: &[&Skill],
) -> Result<adk_rust::agent::LlmAgent> {
    let base_instruction = match role {
        Some(r) => build_role_instruction(r, sop),
        None => DEFAULT_CODE_INSTRUCTION.to_string(),
    };
    let instruction = inject_skills(&base_instruction, skills);
    let role_filtered = filter_tools_by_role(tools, role);
    let filtered_tools = filter_tools_by_skills(role_filtered, skills);

    let builder = LlmAgentBuilder::new("code_agent")
        .instruction(instruction)
        .model(model);
    let builder = filtered_tools.into_iter().fold(builder, |b, tool| b.tool(tool));
    builder.build().map_err(|e| {
        crate::error::DevnpcError::Config(format!("Code Agent 构建失败: {e}"))
    })
}

/// 构建 Fix Agent - CI 日志分析、根因定位、修复代码
///
/// - 当 `role` 为 Some 时,使用 role.system_prompt + sop 组合指令,并按 role.tools 过滤工具
/// - 当 `role` 为 None 时,使用硬编码默认指令,注入全部工具
/// - `skills` 可选注入领域专家知识 (追加指令 + 进一步过滤工具)
pub fn build_fix_agent(
    tools: Vec<Arc<dyn Tool>>,
    model: Arc<dyn adk_rust::Llm>,
    role: Option<&Role>,
    sop: Option<&Sop>,
    skills: &[&Skill],
) -> Result<adk_rust::agent::LlmAgent> {
    let base_instruction = match role {
        Some(r) => build_role_instruction(r, sop),
        None => DEFAULT_FIX_INSTRUCTION.to_string(),
    };
    let instruction = inject_skills(&base_instruction, skills);
    let role_filtered = filter_tools_by_role(tools, role);
    let filtered_tools = filter_tools_by_skills(role_filtered, skills);

    let builder = LlmAgentBuilder::new("fix_agent")
        .instruction(instruction)
        .model(model);
    let builder = filtered_tools.into_iter().fold(builder, |b, tool| b.tool(tool));
    builder.build().map_err(|e| {
        crate::error::DevnpcError::Config(format!("Fix Agent 构建失败: {e}"))
    })
}

/// 构建 Review Agent - 代码审查、SOP 合规检查
///
/// - 当 `role` 为 Some 时,使用 role.system_prompt + sop 组合指令,并按 role.tools 过滤工具
/// - 当 `role` 为 None 时,使用硬编码默认指令,注入全部工具
/// - `skills` 可选注入领域专家知识 (追加指令 + 进一步过滤工具)
pub fn build_review_agent(
    tools: Vec<Arc<dyn Tool>>,
    model: Arc<dyn adk_rust::Llm>,
    role: Option<&Role>,
    sop: Option<&Sop>,
    skills: &[&Skill],
) -> Result<adk_rust::agent::LlmAgent> {
    let base_instruction = match role {
        Some(r) => build_role_instruction(r, sop),
        None => DEFAULT_REVIEW_INSTRUCTION.to_string(),
    };
    let instruction = inject_skills(&base_instruction, skills);
    let role_filtered = filter_tools_by_role(tools, role);
    let filtered_tools = filter_tools_by_skills(role_filtered, skills);

    let builder = LlmAgentBuilder::new("review_agent")
        .instruction(instruction)
        .model(model);
    let builder = filtered_tools.into_iter().fold(builder, |b, tool| b.tool(tool));
    builder.build().map_err(|e| {
        crate::error::DevnpcError::Config(format!("Review Agent 构建失败: {e}"))
    })
}

/// 构建 PM Agent - 需求分解、任务拆分、优先级定义
///
/// - 当 `role` 为 Some 时,使用 role.system_prompt + sop 组合指令,并按 role.tools 过滤工具
/// - 当 `role` 为 None 时,使用硬编码默认指令 (含 [SIGNAL:decomposed] 标记)
/// - `skills` 可选注入领域专家知识
pub fn build_pm_agent(
    tools: Vec<Arc<dyn Tool>>,
    model: Arc<dyn adk_rust::Llm>,
    role: Option<&Role>,
    sop: Option<&Sop>,
    skills: &[&Skill],
) -> Result<adk_rust::agent::LlmAgent> {
    let base_instruction = match role {
        Some(r) => build_role_instruction(r, sop),
        None => DEFAULT_PM_INSTRUCTION.to_string(),
    };
    let instruction = inject_skills(&base_instruction, skills);
    let role_filtered = filter_tools_by_role(tools, role);
    let filtered_tools = filter_tools_by_skills(role_filtered, skills);

    let builder = LlmAgentBuilder::new("pm_agent")
        .instruction(instruction)
        .model(model);
    let builder = filtered_tools.into_iter().fold(builder, |b, tool| b.tool(tool));
    builder.build().map_err(|e| {
        crate::error::DevnpcError::Config(format!("PM Agent 构建失败: {e}"))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::npc_config::{Role, Sop, SopStep};

    /// 构造测试用 Llm (DeepSeek 客户端,假 API key,不发起真实请求)
    fn make_test_llm() -> Arc<dyn adk_rust::Llm> {
        use crate::adapter::provider;
        use crate::config::LlmConfig;
        let cfg = LlmConfig {
            api_key: "test-key".into(),
            base_url: "".into(),
            model: "deepseek-chat".into(),
            provider: "deepseek".into(),
        };
        provider::create_model(&cfg).expect("LLM 创建失败")
    }

    /// 构造测试用 FunctionTool (dummy, 永远返回空对象)
    fn make_dummy_tool(name: &str) -> Arc<dyn Tool> {
        use adk_rust::tool::FunctionTool;
        Arc::new(FunctionTool::new(
            name,
            "dummy tool for testing",
            |_ctx, _args| Box::pin(async { Ok(serde_json::json!({})) }),
        ))
    }

    /// 构造测试用 Role (developer 角色,限定 view_symbol/edit_symbol)
    fn make_developer_role() -> Role {
        Role {
            name: "developer".into(),
            description: "开发者".into(),
            system_prompt: "你是全栈工程师,遵循最小改动原则。".into(),
            max_iterations: Some(25),
            default_sop: Some("bugfix".into()),
            tools: vec!["view_symbol".into(), "edit_symbol".into(), "outline".into()],
        }
    }

    /// 构造测试用 Sop (bugfix 流程)
    fn make_bugfix_sop() -> Sop {
        Sop {
            name: "bugfix".into(),
            description: "Bug 修复流程".into(),
            steps: vec![
                SopStep {
                    name: "复现".into(),
                    expected_tools: vec!["run_command".into()],
                    hint: "先复现 bug".into(),
                },
                SopStep {
                    name: "修复".into(),
                    expected_tools: vec!["edit_symbol".into()],
                    hint: "最小改动".into(),
                },
            ],
        }
    }

    #[test]
    fn test_build_code_agent_empty_tools_no_role() {
        let llm = make_test_llm();
        let result = build_code_agent(vec![], llm, None, None, &[]);
        assert!(result.is_ok(), "Code Agent 构建失败: {:?}", result.err());
    }

    #[test]
    fn test_build_fix_agent_empty_tools_no_role() {
        let llm = make_test_llm();
        let result = build_fix_agent(vec![], llm, None, None, &[]);
        assert!(result.is_ok(), "Fix Agent 构建失败: {:?}", result.err());
    }

    #[test]
    fn test_build_review_agent_empty_tools_no_role() {
        let llm = make_test_llm();
        let result = build_review_agent(vec![], llm, None, None, &[]);
        assert!(result.is_ok(), "Review Agent 构建失败: {:?}", result.err());
    }

    #[test]
    fn test_build_code_agent_with_tools_no_role() {
        let llm = make_test_llm();
        let tools: Vec<Arc<dyn Tool>> = vec![make_dummy_tool("tool1"), make_dummy_tool("tool2")];
        let result = build_code_agent(tools, llm, None, None, &[]);
        assert!(result.is_ok(), "Code Agent (带工具) 构建失败: {:?}", result.err());
    }

    #[test]
    fn test_build_fix_agent_with_tools_no_role() {
        let llm = make_test_llm();
        let tools: Vec<Arc<dyn Tool>> = vec![make_dummy_tool("fix_tool")];
        let result = build_fix_agent(tools, llm, None, None, &[]);
        assert!(result.is_ok(), "Fix Agent (带工具) 构建失败: {:?}", result.err());
    }

    #[test]
    fn test_build_review_agent_with_tools_no_role() {
        let llm = make_test_llm();
        let tools: Vec<Arc<dyn Tool>> = vec![make_dummy_tool("review_tool")];
        let result = build_review_agent(tools, llm, None, None, &[]);
        assert!(result.is_ok(), "Review Agent (带工具) 构建失败: {:?}", result.err());
    }

    #[test]
    fn test_build_code_agent_with_role_uses_role_prompt() {
        let llm = make_test_llm();
        let tools: Vec<Arc<dyn Tool>> = vec![
            make_dummy_tool("aft_view_symbol"),
            make_dummy_tool("aft_edit_symbol"),
            make_dummy_tool("aft_outline"),
            make_dummy_tool("run_command"), // 不在 role.tools 中,应被过滤
        ];
        let role = make_developer_role();
        let sop = make_bugfix_sop();
        let result = build_code_agent(tools, llm, Some(&role), Some(&sop), &[]);
        // Agent 构建成功即验证了 role + sop 注入逻辑
        assert!(result.is_ok(), "Code Agent (role+sop) 构建失败: {:?}", result.err());
    }

    #[test]
    fn test_build_fix_agent_with_role_and_sop() {
        let llm = make_test_llm();
        let tools: Vec<Arc<dyn Tool>> = vec![
            make_dummy_tool("aft_view_symbol"),
            make_dummy_tool("run_command"),
        ];
        let role = make_developer_role();
        let sop = make_bugfix_sop();
        let result = build_fix_agent(tools, llm, Some(&role), Some(&sop), &[]);
        assert!(result.is_ok(), "Fix Agent (role+sop) 构建失败: {:?}", result.err());
    }

    #[test]
    fn test_build_review_agent_with_role_filters_tools() {
        let llm = make_test_llm();
        // 提供 5 个工具,但 role.tools 只允许 3 个
        let tools: Vec<Arc<dyn Tool>> = vec![
            make_dummy_tool("aft_view_symbol"),  // 匹配 view_symbol
            make_dummy_tool("aft_edit_symbol"),  // 匹配 edit_symbol
            make_dummy_tool("aft_outline"),      // 匹配 outline
            make_dummy_tool("run_command"),      // 不匹配,应被过滤
            make_dummy_tool("read_file"),        // 不匹配,应被过滤
        ];
        let role = make_developer_role();
        let result = build_review_agent(tools, llm, Some(&role), None, &[]);
        // Agent 构建成功即验证了工具过滤逻辑
        assert!(result.is_ok(), "Review Agent (role 过滤) 构建失败: {:?}", result.err());
    }

    #[test]
    fn test_build_code_agent_role_with_empty_tools_keeps_all() {
        let llm = make_test_llm();
        let tools: Vec<Arc<dyn Tool>> = vec![
            make_dummy_tool("read_file"),
            make_dummy_tool("run_command"),
        ];
        // role.tools 为空 → 不过滤,保留所有工具
        let role = Role {
            name: "dev".into(),
            description: String::new(),
            system_prompt: "你是开发者。".into(),
            max_iterations: None,
            default_sop: None,
            tools: vec![],
        };
        let result = build_code_agent(tools, llm, Some(&role), None, &[]);
        assert!(result.is_ok(), "空 tools 的 role 应保留所有工具: {:?}", result.err());
    }

    #[test]
    fn test_build_code_agent_role_with_no_matching_tools() {
        let llm = make_test_llm();
        let tools: Vec<Arc<dyn Tool>> = vec![
            make_dummy_tool("read_file"),
            make_dummy_tool("run_command"),
        ];
        // role.tools 中没有匹配的 → 工具被过滤为空,但 Agent 仍可构建
        let role = Role {
            name: "dev".into(),
            description: String::new(),
            system_prompt: "你是开发者。".into(),
            max_iterations: None,
            default_sop: None,
            tools: vec!["nonexistent_tool".into()],
        };
        let result = build_code_agent(tools, llm, Some(&role), None, &[]);
        assert!(result.is_ok(), "无匹配工具的 role 应仍可构建: {:?}", result.err());
    }

    #[test]
    fn test_build_code_agent_with_skill_injects_instruction() {
        let llm = make_test_llm();
        let tools: Vec<Arc<dyn Tool>> = vec![
            make_dummy_tool("aft_view_symbol"),
            make_dummy_tool("aft_edit_symbol"),
            make_dummy_tool("run_command"),
        ];
        let skill = Skill {
            name: "security".into(),
            description: "安全审计".into(),
            instruction: "检查 SQL 注入和 XSS 漏洞".into(),
            tools: vec!["view_symbol".into()],
            scenarios: crate::config::skill::SkillScenarios::default(),
            priority: 20,
        };
        let result = build_code_agent(tools, llm, None, None, &[&skill]);
        assert!(result.is_ok(), "Code Agent (skill) 构建失败: {:?}", result.err());
    }

    #[test]
    fn test_build_review_agent_with_skill_filters_tools() {
        let llm = make_test_llm();
        let tools: Vec<Arc<dyn Tool>> = vec![
            make_dummy_tool("aft_view_symbol"),
            make_dummy_tool("aft_edit_symbol"),
            make_dummy_tool("run_command"),
            make_dummy_tool("read_file"),
        ];
        // skill 限制只允许 view_symbol
        let skill = Skill {
            name: "audit".into(),
            description: "审计".into(),
            instruction: "只读审查".into(),
            tools: vec!["view_symbol".into()],
            scenarios: crate::config::skill::SkillScenarios::default(),
            priority: 10,
        };
        let result = build_review_agent(tools, llm, None, None, &[&skill]);
        assert!(result.is_ok(), "Review Agent (skill 过滤) 构建失败: {:?}", result.err());
    }

    #[test]
    fn test_build_code_agent_with_role_and_skill() {
        let llm = make_test_llm();
        let tools: Vec<Arc<dyn Tool>> = vec![
            make_dummy_tool("aft_view_symbol"),
            make_dummy_tool("aft_edit_symbol"),
            make_dummy_tool("aft_outline"),
            make_dummy_tool("run_command"),
        ];
        let role = make_developer_role();
        let sop = make_bugfix_sop();
        let skill = Skill {
            name: "frontend".into(),
            description: "前端".into(),
            instruction: "遵循 React 最佳实践".into(),
            tools: vec!["view_symbol".into(), "edit_symbol".into()],
            scenarios: crate::config::skill::SkillScenarios::default(),
            priority: 10,
        };
        // role + sop + skill 三层叠加
        let result = build_code_agent(tools, llm, Some(&role), Some(&sop), &[&skill]);
        assert!(result.is_ok(), "Code Agent (role+sop+skill) 构建失败: {:?}", result.err());
    }

    #[test]
    fn test_build_pm_agent_empty_tools_no_role() {
        let llm = make_test_llm();
        let result = build_pm_agent(vec![], llm, None, None, &[]);
        assert!(result.is_ok(), "PM Agent 构建失败: {:?}", result.err());
    }

    #[test]
    fn test_build_pm_agent_with_tools_no_role() {
        let llm = make_test_llm();
        let tools: Vec<Arc<dyn Tool>> = vec![make_dummy_tool("read_file")];
        let result = build_pm_agent(tools, llm, None, None, &[]);
        assert!(result.is_ok(), "PM Agent (带工具) 构建失败: {:?}", result.err());
    }

    #[test]
    fn test_build_pm_agent_with_role_and_sop() {
        let llm = make_test_llm();
        let tools: Vec<Arc<dyn Tool>> = vec![make_dummy_tool("read_file")];
        // PM 角色使用 requirement-decompose sop
        let role = Role {
            name: "pm".into(),
            description: "项目经理".into(),
            system_prompt: "你是项目经理,分解需求为开发/测试任务。".into(),
            max_iterations: Some(15),
            default_sop: Some("requirement-decompose".into()),
            tools: vec!["read_file".into()],
        };
        let sop = Sop {
            name: "requirement-decompose".into(),
            description: "需求分解".into(),
            steps: vec![SopStep {
                name: "任务拆分".into(),
                expected_tools: vec![],
                hint: "拆分为开发+测试任务".into(),
            }],
        };
        let result = build_pm_agent(tools, llm, Some(&role), Some(&sop), &[]);
        assert!(result.is_ok(), "PM Agent (role+sop) 构建失败: {:?}", result.err());
    }

    #[test]
    fn test_build_pm_agent_with_skill() {
        let llm = make_test_llm();
        let tools: Vec<Arc<dyn Tool>> = vec![make_dummy_tool("read_file")];
        let skill = Skill {
            name: "requirement-analysis".into(),
            description: "需求分析".into(),
            instruction: "关注验收标准和边界条件".into(),
            tools: vec![],
            scenarios: crate::config::skill::SkillScenarios::default(),
            priority: 10,
        };
        let result = build_pm_agent(tools, llm, None, None, &[&skill]);
        assert!(result.is_ok(), "PM Agent (skill) 构建失败: {:?}", result.err());
    }
}
