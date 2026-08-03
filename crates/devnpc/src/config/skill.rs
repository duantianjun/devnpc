//! Skill 抽象: 特定领域专家知识的声明式配置
//!
//! Skill 叠加在 Role + Sop 之上,为 Agent 注入领域特定指令和工具约束:
//! - Role: 谁 (身份 + 工具白名单)
//! - Sop:  怎么做 (流程步骤)
//! - Skill: 做什么 (领域专家知识,如"前端开发"/"数据库优化"/"安全审计")
//!
//! Skill 通过关键字匹配任务描述,自动选择最合适的技能注入。
//! 多个 Skill 可叠加使用 (如 "前端开发" + "安全审计")。

use std::collections::HashMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::error::{DevnpcError, Result};
use crate::trigger::parser::TaskKind;

/// Skill 适用场景
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq, Default)]
pub struct SkillScenarios {
    /// 任务类型匹配 (implement/fix/test/refactor/review)
    #[serde(default)]
    pub task_kinds: Vec<String>,
    /// 关键词匹配 (任务描述中包含这些词时触发)
    #[serde(default)]
    pub keywords: Vec<String>,
}

/// Skill 定义
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct Skill {
    pub name: String,
    #[serde(default)]
    pub description: String,
    /// 注入到 Agent 指令末尾的专家知识 (领域规范、注意事项、最佳实践)
    #[serde(default)]
    pub instruction: String,
    /// Skill 限制的工具子集 (空表示不限制,继承 Role 的工具集)
    #[serde(default)]
    pub tools: Vec<String>,
    /// 适用场景 (任务类型 + 关键词)
    #[serde(default)]
    pub scenarios: SkillScenarios,
    /// 优先级 (高优先级 Skill 在多匹配时排在前面)
    #[serde(default)]
    pub priority: i32,
}

/// Skill 注册表
#[derive(Debug, Clone, Default)]
pub struct SkillRegistry {
    pub skills: HashMap<String, Skill>,
}

impl SkillRegistry {
    /// 从目录加载所有 .yml 文件
    pub fn load(dir: &Path) -> Result<Self> {
        let mut registry = SkillRegistry::default();

        if !dir.is_dir() {
            return Ok(registry);
        }

        for entry in std::fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            let ext = path.extension().and_then(|e| e.to_str()).unwrap_or_default();
            if ext != "yml" && ext != "yaml" {
                continue;
            }
            let content = std::fs::read_to_string(&path)?;
            let skill: Skill = serde_yaml::from_str(&content).map_err(|e| {
                DevnpcError::Config(format!("解析 {} 失败: {e}", path.display()))
            })?;
            tracing::debug!(skill = %skill.name, "已加载技能");
            registry.skills.insert(skill.name.clone(), skill);
        }

        tracing::info!(count = registry.skills.len(), "Skill 注册表已加载");
        Ok(registry)
    }

    /// 按名称查找 Skill
    pub fn get(&self, name: &str) -> Option<&Skill> {
        self.skills.get(name)
    }

    /// 根据任务类型和描述匹配最佳 Skill
    ///
    /// 匹配规则:
    /// 1. 任务类型匹配: skill.scenarios.task_kinds 包含任务类型
    /// 2. 关键词匹配: 任务描述包含 skill.scenarios.keywords 中任一词
    /// 3. 多匹配时按 priority 降序返回
    pub fn match_skills(&self, task_kind: &TaskKind, description: &str) -> Vec<&Skill> {
        let kind_str = task_kind_str(task_kind);
        let desc_lower = description.to_lowercase();

        let mut matched: Vec<&Skill> = self
            .skills
            .values()
            .filter(|skill| {
                // 任务类型匹配
                let kind_match = skill.scenarios.task_kinds.iter().any(|k| k == kind_str);
                // 关键词匹配 (任一关键词命中即匹配)
                let keyword_match = skill
                    .scenarios
                    .keywords
                    .iter()
                    .any(|kw| desc_lower.contains(&kw.to_lowercase()));
                kind_match || keyword_match
            })
            .collect();

        // 按 priority 降序排序 (高优先级在前)
        matched.sort_by_key(|s| std::cmp::Reverse(s.priority));
        matched
    }

    /// 列出所有 Skill 名称 (按优先级降序)
    pub fn list(&self) -> Vec<&str> {
        let mut entries: Vec<(&str, i32)> = self
            .skills
            .iter()
            .map(|(name, skill)| (name.as_str(), skill.priority))
            .collect();
        entries.sort_by_key(|(_, p)| std::cmp::Reverse(*p));
        entries.into_iter().map(|(n, _)| n).collect()
    }
}

/// 将 TaskKind 转为字符串 (与 YAML 配置中的 task_kinds 字段对齐)
fn task_kind_str(kind: &TaskKind) -> &'static str {
    match kind {
        TaskKind::Implement => "implement",
        TaskKind::Fix => "fix",
        TaskKind::Test => "test",
        TaskKind::Refactor => "refactor",
        TaskKind::Review => "review",
    }
}

/// 将多个 Skill 的 instruction 追加到现有指令末尾
///
/// 格式:
/// ```text
/// {base_instruction}
///
/// ## 专家技能: {skill1.name}
/// {skill1.instruction}
///
/// ## 专家技能: {skill2.name}
/// {skill2.instruction}
/// ```
pub fn inject_skills(base_instruction: &str, skills: &[&Skill]) -> String {
    if skills.is_empty() {
        return base_instruction.to_string();
    }

    let mut result = base_instruction.to_string();
    for skill in skills {
        if !skill.instruction.is_empty() {
            result.push_str(&format!("\n\n## 专家技能: {}\n{}", skill.name, skill.instruction));
        }
    }
    result
}

/// 按 Skill 的 tools 限制进一步过滤工具 (取 Role 过滤后工具与 Skill 允许工具的交集)
///
/// 若所有 skills 的 tools 都为空,则不限制 (返回原工具列表)。
/// 若任一 skill 的 tools 非空,则只保留该 skill 允许的工具。
/// 多个 skill 时取并集。
pub fn filter_tools_by_skills(
    tools: Vec<std::sync::Arc<dyn adk_rust::Tool>>,
    skills: &[&Skill],
) -> Vec<std::sync::Arc<dyn adk_rust::Tool>> {
    // 收集所有 skill 的 tools 约束
    let constraints: Vec<&Vec<String>> = skills
        .iter()
        .filter(|s| !s.tools.is_empty())
        .map(|s| &s.tools)
        .collect();

    if constraints.is_empty() {
        return tools; // 无约束,不限制
    }

    // 取所有 skill 允许的工具的并集
    let allowed: std::collections::HashSet<&String> = constraints
        .iter()
        .flat_map(|c| c.iter())
        .collect();

    tools
        .into_iter()
        .filter(|t| {
            let name = t.name().to_string();
            // 精确匹配或后缀匹配
            allowed.contains(&name)
                || allowed.iter().any(|short| name.ends_with(&format!("_{short}")))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn make_skill(
        name: &str,
        instruction: &str,
        tools: Vec<&str>,
        task_kinds: Vec<&str>,
        keywords: Vec<&str>,
        priority: i32,
    ) -> Skill {
        Skill {
            name: name.into(),
            description: String::new(),
            instruction: instruction.into(),
            tools: tools.into_iter().map(String::from).collect(),
            scenarios: SkillScenarios {
                task_kinds: task_kinds.into_iter().map(String::from).collect(),
                keywords: keywords.into_iter().map(String::from).collect(),
            },
            priority,
        }
    }

    fn setup_skills_dir() -> TempDir {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("frontend.yml"),
            "name: frontend\ndescription: 前端开发\ninstruction: |\n  你是前端专家,遵循 React 最佳实践。\ntools:\n  - view_symbol\n  - edit_symbol\nscenarios:\n  task_kinds: [implement]\n  keywords: [前端, react, vue, css, ui]\npriority: 10\n",
        )
        .unwrap();
        fs::write(
            dir.path().join("security.yml"),
            "name: security\ndescription: 安全审计\ninstruction: |\n  检查 SQL 注入、XSS、CSRF 等漏洞。\ntools: []\nscenarios:\n  task_kinds: [review, fix]\n  keywords: [安全, 漏洞, security, xss, sql注入]\npriority: 20\n",
        )
        .unwrap();
        fs::write(
            dir.path().join("database.yml"),
            "name: database\ndescription: 数据库优化\ninstruction: |\n  关注索引、N+1 查询、事务边界。\ntools:\n  - view_symbol\n  - search_symbols\nscenarios:\n  task_kinds: [implement, refactor]\n  keywords: [数据库, sql, 索引, database, db]\npriority: 15\n",
        )
        .unwrap();
        dir
    }

    #[test]
    fn load_skill_registry_reads_all_yamls() {
        let dir = setup_skills_dir();
        let registry = SkillRegistry::load(dir.path()).unwrap();
        assert_eq!(registry.skills.len(), 3);
        assert!(registry.get("frontend").is_some());
        assert!(registry.get("security").is_some());
        assert!(registry.get("database").is_some());
    }

    #[test]
    fn load_skill_registry_empty_dir_returns_default() {
        let dir = tempfile::tempdir().unwrap();
        let registry = SkillRegistry::load(dir.path()).unwrap();
        assert!(registry.skills.is_empty());
    }

    #[test]
    fn load_skill_registry_nonexistent_dir_returns_default() {
        let registry = SkillRegistry::load(std::path::Path::new("/nonexistent/path/xyz")).unwrap();
        assert!(registry.skills.is_empty());
    }

    #[test]
    fn load_skill_registry_ignores_non_yaml() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("readme.txt"), "not yaml").unwrap();
        fs::write(
            dir.path().join("valid.yml"),
            "name: test\ndescription: x\ninstruction: y\n",
        )
        .unwrap();
        let registry = SkillRegistry::load(dir.path()).unwrap();
        assert_eq!(registry.skills.len(), 1);
    }

    #[test]
    fn match_skills_by_task_kind() {
        let registry = SkillRegistry {
            skills: HashMap::from([
                ("frontend".into(), make_skill("frontend", "前端", vec![], vec!["implement"], vec![], 10)),
                ("security".into(), make_skill("security", "安全", vec![], vec!["review"], vec![], 20)),
            ]),
        };

        // implement 任务应匹配 frontend
        let matched = registry.match_skills(&TaskKind::Implement, "添加新功能");
        assert_eq!(matched.len(), 1);
        assert_eq!(matched[0].name, "frontend");
    }

    #[test]
    fn match_skills_by_keyword() {
        let registry = SkillRegistry {
            skills: HashMap::from([
                ("frontend".into(), make_skill("frontend", "前端", vec![], vec![], vec!["react", "css"], 10)),
                ("database".into(), make_skill("database", "DB", vec![], vec![], vec!["sql", "索引"], 15)),
            ]),
        };

        // 描述包含 "react" 应匹配 frontend
        let matched = registry.match_skills(&TaskKind::Implement, "用 react 重写页面");
        assert_eq!(matched.len(), 1);
        assert_eq!(matched[0].name, "frontend");
    }

    #[test]
    fn match_skills_multiple_match_sorted_by_priority() {
        let registry = SkillRegistry {
            skills: HashMap::from([
                ("frontend".into(), make_skill("frontend", "前端", vec![], vec!["implement"], vec!["ui"], 10)),
                ("database".into(), make_skill("database", "DB", vec![], vec!["implement"], vec!["sql"], 15)),
                ("security".into(), make_skill("security", "安全", vec![], vec!["fix"], vec![], 20)),
            ]),
        };

        // implement 任务匹配 frontend 和 database,应按 priority 降序
        let matched = registry.match_skills(&TaskKind::Implement, "实现 ui 和 sql 查询");
        assert_eq!(matched.len(), 2);
        assert_eq!(matched[0].name, "database"); // priority 15
        assert_eq!(matched[1].name, "frontend"); // priority 10
    }

    #[test]
    fn match_skills_no_match_returns_empty() {
        let registry = SkillRegistry {
            skills: HashMap::from([
                ("frontend".into(), make_skill("frontend", "前端", vec![], vec!["implement"], vec!["react"], 10)),
            ]),
        };

        let matched = registry.match_skills(&TaskKind::Fix, "修复编译错误");
        assert!(matched.is_empty());
    }

    #[test]
    fn match_skills_empty_registry_returns_empty() {
        let registry = SkillRegistry::default();
        let matched = registry.match_skills(&TaskKind::Implement, "任何描述");
        assert!(matched.is_empty());
    }

    #[test]
    fn match_skills_case_insensitive_keyword() {
        let registry = SkillRegistry {
            skills: HashMap::from([
                ("db".into(), make_skill("db", "DB", vec![], vec![], vec!["SQL"], 10)),
            ]),
        };

        // 小写 sql 应匹配大写 SQL 关键词
        let matched = registry.match_skills(&TaskKind::Implement, "优化 sql 查询");
        assert_eq!(matched.len(), 1);
    }

    #[test]
    fn inject_skills_empty_returns_base() {
        let base = "你是开发者。";
        let result = inject_skills(base, &[]);
        assert_eq!(result, base);
    }

    #[test]
    fn inject_skills_single_appends_instruction() {
        let base = "你是开发者。";
        let skill = make_skill("security", "检查漏洞", vec![], vec![], vec![], 10);
        let result = inject_skills(base, &[&skill]);
        assert!(result.contains("你是开发者。"));
        assert!(result.contains("## 专家技能: security"));
        assert!(result.contains("检查漏洞"));
    }

    #[test]
    fn inject_skills_multiple_appends_all() {
        let base = "你是开发者。";
        let s1 = make_skill("frontend", "前端规范", vec![], vec![], vec![], 10);
        let s2 = make_skill("security", "安全规范", vec![], vec![], vec![], 20);
        let result = inject_skills(base, &[&s1, &s2]);
        assert!(result.contains("## 专家技能: frontend"));
        assert!(result.contains("前端规范"));
        assert!(result.contains("## 专家技能: security"));
        assert!(result.contains("安全规范"));
    }

    #[test]
    fn inject_skills_skips_empty_instruction() {
        let base = "你是开发者。";
        let skill = make_skill("empty", "", vec![], vec![], vec![], 10);
        let result = inject_skills(base, &[&skill]);
        assert_eq!(result, base); // 空 instruction 不追加
    }

    #[test]
    fn filter_tools_by_skills_no_constraint_returns_all() {
        use adk_rust::tool::FunctionTool;
        let tools: Vec<std::sync::Arc<dyn adk_rust::Tool>> = vec![
            std::sync::Arc::new(FunctionTool::new("read_file", "r", |_, _| Box::pin(async { Ok(serde_json::json!({})) }))),
            std::sync::Arc::new(FunctionTool::new("run_command", "r", |_, _| Box::pin(async { Ok(serde_json::json!({})) }))),
        ];
        let skills: Vec<&Skill> = vec![]; // 无 skill → 无约束
        let result = filter_tools_by_skills(tools, &skills);
        assert_eq!(result.len(), 2);
    }

    #[test]
    fn filter_tools_by_skills_with_constraint() {
        use adk_rust::tool::FunctionTool;
        let tools: Vec<std::sync::Arc<dyn adk_rust::Tool>> = vec![
            std::sync::Arc::new(FunctionTool::new("aft_view_symbol", "r", |_, _| Box::pin(async { Ok(serde_json::json!({})) }))),
            std::sync::Arc::new(FunctionTool::new("run_command", "r", |_, _| Box::pin(async { Ok(serde_json::json!({})) }))),
        ];
        let skill = make_skill("frontend", "前端", vec!["view_symbol"], vec![], vec![], 10);
        let result = filter_tools_by_skills(tools, &[&skill]);
        // 仅保留 aft_view_symbol (后缀匹配 view_symbol)
        assert_eq!(result.len(), 1);
    }

    #[test]
    fn filter_tools_by_skills_multiple_skills_union() {
        use adk_rust::tool::FunctionTool;
        let tools: Vec<std::sync::Arc<dyn adk_rust::Tool>> = vec![
            std::sync::Arc::new(FunctionTool::new("aft_view_symbol", "r", |_, _| Box::pin(async { Ok(serde_json::json!({})) }))),
            std::sync::Arc::new(FunctionTool::new("aft_edit_symbol", "r", |_, _| Box::pin(async { Ok(serde_json::json!({})) }))),
            std::sync::Arc::new(FunctionTool::new("run_command", "r", |_, _| Box::pin(async { Ok(serde_json::json!({})) }))),
        ];
        let s1 = make_skill("frontend", "", vec!["view_symbol"], vec![], vec![], 10);
        let s2 = make_skill("db", "", vec!["run_command"], vec![], vec![], 10);
        // 并集: view_symbol + run_command → 保留 aft_view_symbol 和 run_command
        let result = filter_tools_by_skills(tools, &[&s1, &s2]);
        assert_eq!(result.len(), 2);
    }

    #[test]
    fn list_skills_sorted_by_priority() {
        let registry = SkillRegistry {
            skills: HashMap::from([
                ("low".into(), make_skill("low", "", vec![], vec![], vec![], 5)),
                ("high".into(), make_skill("high", "", vec![], vec![], vec![], 100)),
                ("mid".into(), make_skill("mid", "", vec![], vec![], vec![], 50)),
            ]),
        };
        let list = registry.list();
        assert_eq!(list, vec!["high", "mid", "low"]);
    }

    #[test]
    fn load_real_skills_dir() {
        let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("npc-config/skills");
        if !path.exists() {
            return;
        }
        let registry = SkillRegistry::load(&path).unwrap();
        assert!(!registry.skills.is_empty(), "应加载到示例技能");
    }
}
