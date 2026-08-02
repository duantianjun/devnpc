//! npc-config 角色与 SOP 配置加载
//!
//! 从 npc-config/ 目录加载 YAML 配置:
//! - roles/*.yml: 角色定义 (system_prompt + 工具白名单 + 默认 SOP)
//! - sops/*.yml: SOP 流程定义 (步骤 + 每步预期工具 + 提示)
//! - teams/*.yml: 团队编排 (角色组合 + handoff 规则)
//! - skills/*.yml: Skill 技能定义 (领域专家知识 + 适用场景)
//!
//! 加载后通过 build_role_instruction 将 role.system_prompt + sop.steps 组合为完整指令,
//! 注入到 LlmAgentBuilder.instruction(),实现"能力 = 角色 + 流程 + 技能"的声明式配置。

use std::collections::HashMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::error::{DevnpcError, Result};
pub use crate::config::skill::{Skill, SkillRegistry, SkillScenarios};

/// SOP 步骤
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct SopStep {
    pub name: String,
    #[serde(default)]
    pub expected_tools: Vec<String>,
    #[serde(default)]
    pub hint: String,
}

/// SOP 流程定义
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct Sop {
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub steps: Vec<SopStep>,
}

/// 角色定义
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct Role {
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub system_prompt: String,
    #[serde(default)]
    pub max_iterations: Option<u32>,
    #[serde(default)]
    pub default_sop: Option<String>,
    /// 角色允许使用的工具名 (简称,如 outline/view_symbol;加载时映射到实际工具名)
    #[serde(default)]
    pub tools: Vec<String>,
}

/// 团队编排 handoff 规则
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct HandoffRule {
    pub from: String,
    pub to: Vec<String>,
    pub trigger: String,
}

/// 团队编排 merge 策略
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct MergeStrategy {
    pub strategy: String,
}

/// 团队编排配置
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct TeamNpc {
    pub role: String,
    pub sop: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct Team {
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub npcs: Vec<TeamNpc>,
    #[serde(default)]
    pub handoff: Vec<HandoffRule>,
    #[serde(default)]
    pub merge: Option<MergeStrategy>,
}

/// npc-config 完整配置
#[derive(Debug, Clone, Default)]
pub struct NpcConfig {
    pub roles: HashMap<String, Role>,
    pub sops: HashMap<String, Sop>,
    pub teams: HashMap<String, Team>,
    pub skills: SkillRegistry,
}

impl NpcConfig {
    /// 从指定目录加载所有 YAML 配置
    ///
    /// 目录结构:
    /// ```text
    /// npc-config/
    ///   roles/*.yml
    ///   sops/*.yml
    ///   teams/*.yml
    ///   skills/*.yml
    /// ```
    pub fn load(base_dir: &Path) -> Result<Self> {
        let mut config = NpcConfig::default();

        let roles_dir = base_dir.join("roles");
        if roles_dir.is_dir() {
            config.roles = load_dir(&roles_dir)?;
        }

        let sops_dir = base_dir.join("sops");
        if sops_dir.is_dir() {
            config.sops = load_dir(&sops_dir)?;
        }

        let teams_dir = base_dir.join("teams");
        if teams_dir.is_dir() {
            config.teams = load_dir(&teams_dir)?;
        }

        let skills_dir = base_dir.join("skills");
        if skills_dir.is_dir() {
            config.skills = SkillRegistry::load(&skills_dir)?;
        }

        tracing::info!(
            roles = config.roles.len(),
            sops = config.sops.len(),
            teams = config.teams.len(),
            skills = config.skills.skills.len(),
            "npc-config 已加载"
        );

        Ok(config)
    }

    /// 按角色名查找 Role
    pub fn role(&self, name: &str) -> Option<&Role> {
        self.roles.get(name)
    }

    /// 按 SOP 名查找 Sop
    pub fn sop(&self, name: &str) -> Option<&Sop> {
        self.sops.get(name)
    }

    /// 按团队名查找 Team
    pub fn team(&self, name: &str) -> Option<&Team> {
        self.teams.get(name)
    }

    /// 查找角色的默认 SOP,返回 (role, sop) 二元组
    ///
    /// 若 role.default_sop 不存在或指向的 SOP 不存在,sop 返回 None
    pub fn role_with_default_sop(&self, role_name: &str) -> Option<(&Role, Option<&Sop>)> {
        let role = self.role(role_name)?;
        let sop = role
            .default_sop
            .as_deref()
            .and_then(|sop_name| self.sop(sop_name));
        Some((role, sop))
    }
}

/// 从目录加载所有 .yml 文件,反序列化为 HashMap<name, T>
///
/// 每个文件必须包含 `name` 字段作为 HashMap 的 key。
fn load_dir<T>(dir: &Path) -> Result<HashMap<String, T>>
where
    T: for<'de> Deserialize<'de> + Clone + serde::Serialize,
{
    let mut map = HashMap::new();
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        // 只处理 .yml / .yaml
        if !path.is_file() {
            continue;
        }
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or_default();
        if ext != "yml" && ext != "yaml" {
            continue;
        }
        let content = std::fs::read_to_string(&path)?;
        let item: T = serde_yaml::from_str(&content)
            .map_err(|e| DevnpcError::Config(format!("解析 {} 失败: {e}", path.display())))?;
        // 读取 name 字段作为 key (通过 JSON 中转避免泛型 trait 约束)
        let json = serde_json::to_value(&item)
            .map_err(|e| DevnpcError::Config(format!("序列化 {} 失败: {e}", path.display())))?;
        let name = json
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| DevnpcError::Config(format!("{} 缺少 name 字段", path.display())))?
            .to_string();
        map.insert(name, item);
    }
    Ok(map)
}

/// 将 role.system_prompt + sop.steps 组合为完整 Agent 指令
///
/// 输出格式:
/// ```text
/// {role.system_prompt}
///
/// ## 执行流程 (SOP: {sop.name})
///
/// ### 1. {step1.name}
/// 预期工具: {expected_tools}
/// 提示: {hint}
///
/// ### 2. {step2.name}
/// ...
/// ```
///
/// 若 sop 为 None,仅返回 role.system_prompt。
pub fn build_role_instruction(role: &Role, sop: Option<&Sop>) -> String {
    let mut instruction = role.system_prompt.clone();

    if let Some(sop) = sop
        && !sop.steps.is_empty()
    {
        instruction.push_str("\n\n## 执行流程 (SOP: ");
        instruction.push_str(&sop.name);
        instruction.push_str(")\n");

        for (i, step) in sop.steps.iter().enumerate() {
            instruction.push_str(&format!("\n### {}. {}\n", i + 1, step.name));
            if !step.expected_tools.is_empty() {
                instruction.push_str(&format!(
                    "预期工具: {}\n",
                    step.expected_tools.join(", ")
                ));
            }
            if !step.hint.is_empty() {
                instruction.push_str(&format!("提示: {}\n", step.hint));
            }
        }

        instruction.push_str("\n请按上述流程执行,每步完成后进入下一步。");
    }

    instruction
}

/// 按角色工具白名单过滤实际工具列表
///
/// role.tools 中使用简称 (如 "outline"),实际工具名为 "aft_outline" 等。
/// 匹配规则: 实际工具名等于简称,或以 "_{简称}" 结尾。
/// role.tools 为空时返回所有工具 (不做过滤)。
///
/// 返回过滤后的工具名列表 (用于后续 Arc<dyn Tool> 筛选)。
pub fn filter_tool_names<'a>(
    all_tool_names: &'a [String],
    role_tool_short_names: &'a [String],
) -> Vec<&'a String> {
    if role_tool_short_names.is_empty() {
        return all_tool_names.iter().collect();
    }

    all_tool_names
        .iter()
        .filter(|full_name| {
            // 精确匹配
            if role_tool_short_names.contains(*full_name) {
                return true;
            }
            // 后缀匹配: aft_outline 匹配 "outline"
            role_tool_short_names.iter().any(|short| {
                *full_name == short || full_name.ends_with(&format!("_{short}"))
            })
        })
        .collect()
}

/// 按角色工具白名单过滤 Tool trait 对象
///
/// 内部先收集 tool.name() (通过 adk-rust 的 Tool trait),再按 role.tools 过滤。
pub fn filter_tools_by_role(
    tools: Vec<std::sync::Arc<dyn adk_rust::Tool>>,
    role: Option<&Role>,
) -> Vec<std::sync::Arc<dyn adk_rust::Tool>> {
    let Some(role) = role else {
        return tools;
    };
    if role.tools.is_empty() {
        return tools;
    }

    // 收集所有工具名,过滤后得到保留的名称集合
    let all_names: Vec<String> = tools.iter().map(|t| t.name().to_string()).collect();
    let keep: Vec<String> = filter_tool_names(&all_names, &role.tools)
        .into_iter()
        .cloned()
        .collect();

    tools
        .into_iter()
        .filter(|t| keep.contains(&t.name().to_string()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;
    use tempfile::TempDir;

    /// 创建临时 npc-config 目录,写入示例角色/SOP/团队
    fn setup_npc_config() -> TempDir {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path();

        fs::create_dir_all(base.join("roles")).unwrap();
        fs::create_dir_all(base.join("sops")).unwrap();
        fs::create_dir_all(base.join("teams")).unwrap();

        fs::write(
            base.join("roles/developer.yml"),
            "name: developer\ndescription: 全栈开发\nsystem_prompt: |\n  你是开发者。\nmax_iterations: 25\ndefault_sop: bugfix\ntools:\n  - view_symbol\n  - edit_symbol\n  - outline\n  - finish\n",
        )
        .unwrap();

        fs::write(
            base.join("sops/bugfix.yml"),
            "name: bugfix\ndescription: Bug 修复\nsteps:\n  - name: 复现\n    expected_tools: [run_command, read_file]\n    hint: 先复现\n  - name: 修复\n    expected_tools: [edit_symbol]\n    hint: 最小改动\n",
        )
        .unwrap();

        fs::write(
            base.join("teams/feature-team.yml"),
            "name: feature-team\ndescription: 功能团队\nnpcs:\n  - role: developer\n    sop: bugfix\nhandoff: []\nmerge:\n  strategy: single-mr\n",
        )
        .unwrap();

        dir
    }

    #[test]
    fn load_npc_config_reads_all_yamls() {
        let dir = setup_npc_config();
        let config = NpcConfig::load(dir.path()).unwrap();

        assert_eq!(config.roles.len(), 1);
        assert_eq!(config.sops.len(), 1);
        assert_eq!(config.teams.len(), 1);

        let role = config.role("developer").unwrap();
        assert_eq!(role.max_iterations, Some(25));
        assert_eq!(role.default_sop.as_deref(), Some("bugfix"));
        assert!(role.system_prompt.contains("你是开发者"));
        assert!(role.tools.contains(&"view_symbol".to_string()));

        let sop = config.sop("bugfix").unwrap();
        assert_eq!(sop.steps.len(), 2);
        assert_eq!(sop.steps[0].name, "复现");
        assert!(sop.steps[0].expected_tools.contains(&"run_command".to_string()));

        let team = config.team("feature-team").unwrap();
        assert_eq!(team.npcs.len(), 1);
        assert_eq!(team.npcs[0].role, "developer");
        assert_eq!(team.merge.as_ref().unwrap().strategy, "single-mr");
    }

    #[test]
    fn load_npc_config_empty_dir_returns_default() {
        let dir = tempfile::tempdir().unwrap();
        let config = NpcConfig::load(dir.path()).unwrap();
        assert!(config.roles.is_empty());
        assert!(config.sops.is_empty());
        assert!(config.teams.is_empty());
    }

    #[test]
    fn load_npc_config_ignores_non_yaml_files() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join("roles")).unwrap();
        fs::write(dir.path().join("roles/readme.txt"), "not yaml").unwrap();
        fs::write(
            dir.path().join("roles/dev.yml"),
            "name: dev\nsystem_prompt: x\n",
        )
        .unwrap();
        let config = NpcConfig::load(dir.path()).unwrap();
        assert_eq!(config.roles.len(), 1);
        assert!(config.role("dev").is_some());
    }

    #[test]
    fn load_npc_config_missing_name_returns_error() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join("roles")).unwrap();
        fs::write(
            dir.path().join("roles/noname.yml"),
            "description: no name field\nsystem_prompt: x\n",
        )
        .unwrap();
        let result = NpcConfig::load(dir.path());
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("name"));
    }

    #[test]
    fn role_with_default_sop_resolves_sop() {
        let dir = setup_npc_config();
        let config = NpcConfig::load(dir.path()).unwrap();
        let (role, sop) = config.role_with_default_sop("developer").unwrap();
        assert_eq!(role.name, "developer");
        assert!(sop.is_some());
        assert_eq!(sop.unwrap().name, "bugfix");
    }

    #[test]
    fn role_with_default_sop_returns_none_when_sop_missing() {
        let dir = setup_npc_config();
        let config = NpcConfig::load(dir.path()).unwrap();
        // developer 的 default_sop 是 bugfix (存在),测试不存在的 role
        let result = config.role_with_default_sop("nonexistent");
        assert!(result.is_none());
    }

    #[test]
    fn build_role_instruction_without_sop() {
        let role = Role {
            name: "test".into(),
            description: String::new(),
            system_prompt: "你是测试者。".into(),
            max_iterations: None,
            default_sop: None,
            tools: vec![],
        };
        let instruction = build_role_instruction(&role, None);
        assert_eq!(instruction, "你是测试者。");
    }

    #[test]
    fn build_role_instruction_with_sop_appends_steps() {
        let role = Role {
            name: "test".into(),
            description: String::new(),
            system_prompt: "你是测试者。".into(),
            max_iterations: None,
            default_sop: None,
            tools: vec![],
        };
        let sop = Sop {
            name: "test-sop".into(),
            description: String::new(),
            steps: vec![
                SopStep {
                    name: "步骤1".into(),
                    expected_tools: vec!["read_file".into()],
                    hint: "读取文件".into(),
                },
                SopStep {
                    name: "步骤2".into(),
                    expected_tools: vec![],
                    hint: String::new(),
                },
            ],
        };
        let instruction = build_role_instruction(&role, Some(&sop));
        assert!(instruction.contains("你是测试者。"));
        assert!(instruction.contains("SOP: test-sop"));
        assert!(instruction.contains("1. 步骤1"));
        assert!(instruction.contains("预期工具: read_file"));
        assert!(instruction.contains("提示: 读取文件"));
        assert!(instruction.contains("2. 步骤2"));
        assert!(instruction.contains("请按上述流程执行"));
    }

    #[test]
    fn build_role_instruction_with_empty_steps_sop() {
        let role = Role {
            name: "test".into(),
            description: String::new(),
            system_prompt: "你是测试者。".into(),
            max_iterations: None,
            default_sop: None,
            tools: vec![],
        };
        let sop = Sop {
            name: "empty".into(),
            description: String::new(),
            steps: vec![],
        };
        let instruction = build_role_instruction(&role, Some(&sop));
        // steps 为空时不附加 SOP 段落
        assert_eq!(instruction, "你是测试者。");
    }

    #[test]
    fn filter_tool_names_exact_match() {
        let all = vec!["read_file".into(), "write_file".into(), "run_command".into()];
        let role_tools = vec!["read_file".into()];
        let result = filter_tool_names(&all, &role_tools);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0], "read_file");
    }

    #[test]
    fn filter_tool_names_suffix_match() {
        let all = vec![
            "aft_outline".into(),
            "aft_view_symbol".into(),
            "read_file".into(),
        ];
        // "outline" 应匹配 "aft_outline"
        let role_tools = vec!["outline".into()];
        let result = filter_tool_names(&all, &role_tools);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0], "aft_outline");
    }

    #[test]
    fn filter_tool_names_empty_role_tools_returns_all() {
        let all = vec!["read_file".into(), "write_file".into()];
        let role_tools: Vec<String> = vec![];
        let result = filter_tool_names(&all, &role_tools);
        assert_eq!(result.len(), 2);
    }

    #[test]
    fn filter_tool_names_no_match_returns_empty() {
        let all = vec!["read_file".into()];
        let role_tools = vec!["nonexistent".into()];
        let result = filter_tool_names(&all, &role_tools);
        assert!(result.is_empty());
    }

    #[test]
    fn filter_tool_names_mixed_match() {
        let all = vec![
            "read_file".into(),
            "aft_outline".into(),
            "aft_view_symbol".into(),
            "run_command".into(),
        ];
        // 同时测试精确匹配和后缀匹配
        let role_tools = vec!["read_file".into(), "outline".into(), "view_symbol".into()];
        let result = filter_tool_names(&all, &role_tools);
        assert_eq!(result.len(), 3);
    }

    #[test]
    fn load_real_npc_config_dir() {
        // 加载项目实际的 npc-config 目录
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("npc-config");
        if !path.exists() {
            return; // 某些构建环境可能没有此目录
        }
        let config = NpcConfig::load(&path).unwrap();
        assert!(config.roles.contains_key("developer"));
        assert!(config.roles.contains_key("pm"));
        assert!(config.roles.contains_key("tester"));
        assert!(config.sops.contains_key("bugfix"));
        assert!(config.sops.contains_key("feature"));
        assert!(config.sops.contains_key("test-gen"));
        assert!(config.sops.contains_key("requirement-decompose"));
        assert!(config.teams.contains_key("feature-team"));
    }
}
