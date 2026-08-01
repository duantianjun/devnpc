//! 配置加载器: 环境变量 + .devnpc.md + 默认值三层合并
//!
//! 优先级 (高 → 低): 环境变量 > .devnpc.md > 内置默认。
//! roles/sops YAML 加载放 P6。

use std::path::Path;

use crate::config::devnpc_md::{parse_devnpc_md, DevnpcMdFrontMatter, ParsedDevnpcMd};
use crate::config::env;
use crate::config::{
    Config, GitlabConfig, Limits, LlmConfig, ProjectConfig, ReportConfig, ReportTarget, SopMode,
};
use crate::error::{DevnpcError, Result};

/// 从指定 .devnpc.md 路径读取并解析,文件不存在返回默认
fn read_devnpc_md(path: Option<&Path>) -> Result<ParsedDevnpcMd> {
    let Some(path) = path else {
        return Ok(ParsedDevnpcMd::default());
    };
    if !path.exists() {
        return Ok(ParsedDevnpcMd::default());
    }
    let content = std::fs::read_to_string(path)?;
    parse_devnpc_md(&content)
}

/// 把 front matter 转成 ProjectConfig (未填字段留默认,等合并阶段补)
fn project_config_from_front_matter(
    fm: &DevnpcMdFrontMatter,
    guidelines: String,
) -> ProjectConfig {
    let sop_mode = fm
        .sop_mode
        .as_deref()
        .map(|s| match s {
            "strict" => SopMode::Strict,
            _ => SopMode::Soft,
        })
        .unwrap_or_default();
    ProjectConfig {
        sop_mode,
        forbidden_paths: fm.forbidden_paths.clone().unwrap_or_default(),
        required_checks: fm.required_checks.clone().unwrap_or_default(),
        branch_prefix: fm
            .branch_prefix
            .clone()
            .unwrap_or_else(|| "npc".to_string()),
        max_ci_retries: fm.max_ci_retries,
        guidelines_markdown: guidelines,
    }
}

/// 内部加载函数 (接受环境变量名参数,便于测试隔离)
///
/// 生产代码用 `Config::load()` 调用此函数并传入标准变量名。
#[allow(clippy::too_many_arguments)]
fn load_internal(
    api_key_var: &str,
    base_url_var: &str,
    model_var: &str,
    gitlab_url_var: &str,
    gitlab_token_var: &str,
    project_id_var: &str,
    max_iter_var: &str,
    max_ci_var: &str,
    sop_mode_var: &str,
    report_target_var: &str,
    devnpc_md_path: Option<&Path>,
) -> Result<Config> {
    // 1. 必需环境变量
    let api_key = env::get_required(api_key_var)?;
    let base_url = env::get_required(base_url_var)?;
    let model = env::get_required(model_var)?;
    let gitlab_url = env::get_required(gitlab_url_var)?;
    let gitlab_token = env::get_required(gitlab_token_var)?;
    let project_id_str = env::get_required(project_id_var)?;
    let project_id: u64 = project_id_str.parse().map_err(|_| {
        DevnpcError::Config(format!("环境变量 {project_id_var} 不是有效 u64: {project_id_str}"))
    })?;

    // 2. .devnpc.md
    let parsed_md = read_devnpc_md(devnpc_md_path)?;
    let mut project =
        project_config_from_front_matter(&parsed_md.front_matter, parsed_md.guidelines_markdown);

    // 3. 可选环境变量 (覆盖 .devnpc.md)
    let max_iterations = env::get_u32(max_iter_var)?.unwrap_or(20);
    let max_ci_retries = match env::get_u8(max_ci_var)? {
        Some(v) => v,
        None => project.max_ci_retries.unwrap_or(3),
    };
    if let Some(mode) = env::get_sop_mode(sop_mode_var)? {
        project.sop_mode = mode;
    }
    let report_target = env::get_report_target(report_target_var)?.unwrap_or(ReportTarget::Artifact);

    Ok(Config {
        llm: LlmConfig {
            api_key,
            base_url,
            model,
        },
        gitlab: GitlabConfig {
            url: gitlab_url,
            token: gitlab_token,
            project_id,
        },
        limits: Limits {
            max_iterations,
            max_ci_retries,
        },
        project,
        report: ReportConfig {
            target: report_target,
        },
    })
}

/// 生产环境配置加载 (使用标准环境变量名)
pub fn load() -> Result<Config> {
    let devnpc_md_path = std::env::current_dir().ok().map(|p| p.join(".devnpc.md"));
    load_internal(
        "DEVNPC_API_KEY",
        "DEVNPC_BASE_URL",
        "DEVNPC_MODEL",
        "GITLAB_URL",
        "GITLAB_TOKEN",
        "CI_PROJECT_ID",
        "DEVNPC_MAX_ITERATIONS",
        "DEVNPC_MAX_CI_RETRIES",
        "DEVNPC_SOP_MODE",
        "DEVNPC_REPORT_TARGET",
        devnpc_md_path.as_deref(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{ReportTarget, SopMode};
    use std::path::PathBuf;

    /// 构造一个临时 .devnpc.md 文件并返回其路径
    ///
    /// 用 `keep()` 消费 TempDir,避免析构删除文件 (测试用,允许泄漏)。
    fn write_devnpc_md(content: &str) -> PathBuf {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join(".devnpc.md");
        std::fs::write(&file_path, content).unwrap();
        let dir_path = dir.keep();
        dir_path.join(".devnpc.md")
    }

    #[test]
    fn merge_env_overrides_devnpc_md_overrides_defaults() {
        // 设环境变量
        std::env::set_var("DEVNPC_TEST_MERGE_API_KEY", "sk-merge");
        std::env::set_var("DEVNPC_TEST_MERGE_BASE_URL", "https://api.test.com/v1");
        std::env::set_var("DEVNPC_TEST_MERGE_MODEL", "test-model");
        std::env::set_var("DEVNPC_TEST_MERGE_GITLAB_URL", "https://gitlab.test.com");
        std::env::set_var("DEVNPC_TEST_MERGE_GITLAB_TOKEN", "gl-token");
        std::env::set_var("DEVNPC_TEST_MERGE_PROJECT_ID", "42");
        std::env::set_var("DEVNPC_TEST_MERGE_MAX_ITERATIONS", "30");
        std::env::set_var("DEVNPC_TEST_MERGE_MAX_CI_RETRIES", "5");
        std::env::set_var("DEVNPC_TEST_MERGE_SOP_MODE", "strict");

        let md_path = write_devnpc_md(
            "---\nsop_mode: soft\nmax_ci_retries: 2\nbranch_prefix: \"npc\"\n---\n# 规范\n",
        );

        let config = load_internal(
            "DEVNPC_TEST_MERGE_API_KEY",
            "DEVNPC_TEST_MERGE_BASE_URL",
            "DEVNPC_TEST_MERGE_MODEL",
            "DEVNPC_TEST_MERGE_GITLAB_URL",
            "DEVNPC_TEST_MERGE_GITLAB_TOKEN",
            "DEVNPC_TEST_MERGE_PROJECT_ID",
            "DEVNPC_TEST_MERGE_MAX_ITERATIONS",
            "DEVNPC_TEST_MERGE_MAX_CI_RETRIES",
            "DEVNPC_TEST_MERGE_SOP_MODE",
            "DEVNPC_TEST_MERGE_REPORT_TARGET",
            Some(&md_path),
        )
        .unwrap();

        // env 覆盖 .devnpc.md
        assert_eq!(config.llm.api_key, "sk-merge");
        assert_eq!(config.limits.max_iterations, 30);
        assert_eq!(config.limits.max_ci_retries, 5);
        assert_eq!(config.project.sop_mode, SopMode::Strict);
        // .devnpc.md 的 branch_prefix 保留 (env 无覆盖)
        assert_eq!(config.project.branch_prefix, "npc");
        // 正文
        assert!(config.project.guidelines_markdown.contains("# 规范"));
        // 报告默认 artifact
        assert_eq!(config.report.target, ReportTarget::Artifact);

        // 清理
        for key in [
            "DEVNPC_TEST_MERGE_API_KEY",
            "DEVNPC_TEST_MERGE_BASE_URL",
            "DEVNPC_TEST_MERGE_MODEL",
            "DEVNPC_TEST_MERGE_GITLAB_URL",
            "DEVNPC_TEST_MERGE_GITLAB_TOKEN",
            "DEVNPC_TEST_MERGE_PROJECT_ID",
            "DEVNPC_TEST_MERGE_MAX_ITERATIONS",
            "DEVNPC_TEST_MERGE_MAX_CI_RETRIES",
            "DEVNPC_TEST_MERGE_SOP_MODE",
        ] {
            std::env::remove_var(key);
        }
    }

    #[test]
    fn load_fails_when_required_llm_env_missing() {
        std::env::remove_var("DEVNPC_TEST_FAIL_API_KEY");
        let result = load_internal(
            "DEVNPC_TEST_FAIL_API_KEY",
            "DEVNPC_TEST_FAIL_BASE_URL",
            "DEVNPC_TEST_FAIL_MODEL",
            "DEVNPC_TEST_FAIL_GITLAB_URL",
            "DEVNPC_TEST_FAIL_GITLAB_TOKEN",
            "DEVNPC_TEST_FAIL_PROJECT_ID",
            "DEVNPC_TEST_FAIL_MAX_ITERATIONS",
            "DEVNPC_TEST_FAIL_MAX_CI_RETRIES",
            "DEVNPC_TEST_FAIL_SOP_MODE",
            "DEVNPC_TEST_FAIL_REPORT_TARGET",
            None,
        );
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            crate::error::DevnpcError::MissingEnv { .. }
        ));
    }

    #[test]
    fn load_uses_defaults_when_optional_missing() {
        std::env::set_var("DEVNPC_TEST_DEFAULT_API_KEY", "sk");
        std::env::set_var("DEVNPC_TEST_DEFAULT_BASE_URL", "https://api.test.com/v1");
        std::env::set_var("DEVNPC_TEST_DEFAULT_MODEL", "m");
        std::env::set_var("DEVNPC_TEST_DEFAULT_GITLAB_URL", "https://gl.test.com");
        std::env::set_var("DEVNPC_TEST_DEFAULT_GITLAB_TOKEN", "t");
        std::env::set_var("DEVNPC_TEST_DEFAULT_PROJECT_ID", "1");
        std::env::remove_var("DEVNPC_TEST_DEFAULT_MAX_ITERATIONS");
        std::env::remove_var("DEVNPC_TEST_DEFAULT_MAX_CI_RETRIES");
        std::env::remove_var("DEVNPC_TEST_DEFAULT_SOP_MODE");
        std::env::remove_var("DEVNPC_TEST_DEFAULT_REPORT_TARGET");

        let config = load_internal(
            "DEVNPC_TEST_DEFAULT_API_KEY",
            "DEVNPC_TEST_DEFAULT_BASE_URL",
            "DEVNPC_TEST_DEFAULT_MODEL",
            "DEVNPC_TEST_DEFAULT_GITLAB_URL",
            "DEVNPC_TEST_DEFAULT_GITLAB_TOKEN",
            "DEVNPC_TEST_DEFAULT_PROJECT_ID",
            "DEVNPC_TEST_DEFAULT_MAX_ITERATIONS",
            "DEVNPC_TEST_DEFAULT_MAX_CI_RETRIES",
            "DEVNPC_TEST_DEFAULT_SOP_MODE",
            "DEVNPC_TEST_DEFAULT_REPORT_TARGET",
            None,
        )
        .unwrap();

        assert_eq!(config.limits.max_iterations, 20);
        assert_eq!(config.limits.max_ci_retries, 3);
        assert_eq!(config.project.sop_mode, SopMode::Soft);
        assert_eq!(config.project.branch_prefix, "npc");
        assert_eq!(config.report.target, ReportTarget::Artifact);

        for key in [
            "DEVNPC_TEST_DEFAULT_API_KEY",
            "DEVNPC_TEST_DEFAULT_BASE_URL",
            "DEVNPC_TEST_DEFAULT_MODEL",
            "DEVNPC_TEST_DEFAULT_GITLAB_URL",
            "DEVNPC_TEST_DEFAULT_GITLAB_TOKEN",
            "DEVNPC_TEST_DEFAULT_PROJECT_ID",
        ] {
            std::env::remove_var(key);
        }
    }

    #[test]
    fn load_uses_devnpc_md_max_ci_retries_when_env_missing() {
        std::env::set_var("DEVNPC_TEST_MD_API_KEY", "sk");
        std::env::set_var("DEVNPC_TEST_MD_BASE_URL", "https://api.test.com/v1");
        std::env::set_var("DEVNPC_TEST_MD_MODEL", "m");
        std::env::set_var("DEVNPC_TEST_MD_GITLAB_URL", "https://gl.test.com");
        std::env::set_var("DEVNPC_TEST_MD_GITLAB_TOKEN", "t");
        std::env::set_var("DEVNPC_TEST_MD_PROJECT_ID", "1");
        std::env::remove_var("DEVNPC_TEST_MD_MAX_CI_RETRIES");

        let md_path = write_devnpc_md("---\nmax_ci_retries: 7\n---\n正文");
        let config = load_internal(
            "DEVNPC_TEST_MD_API_KEY",
            "DEVNPC_TEST_MD_BASE_URL",
            "DEVNPC_TEST_MD_MODEL",
            "DEVNPC_TEST_MD_GITLAB_URL",
            "DEVNPC_TEST_MD_GITLAB_TOKEN",
            "DEVNPC_TEST_MD_PROJECT_ID",
            "DEVNPC_TEST_MD_MAX_ITERATIONS",
            "DEVNPC_TEST_MD_MAX_CI_RETRIES",
            "DEVNPC_TEST_MD_SOP_MODE",
            "DEVNPC_TEST_MD_REPORT_TARGET",
            Some(&md_path),
        )
        .unwrap();

        // .devnpc.md 指定 7,env 缺失,用 7
        assert_eq!(config.limits.max_ci_retries, 7);

        for key in [
            "DEVNPC_TEST_MD_API_KEY",
            "DEVNPC_TEST_MD_BASE_URL",
            "DEVNPC_TEST_MD_MODEL",
            "DEVNPC_TEST_MD_GITLAB_URL",
            "DEVNPC_TEST_MD_GITLAB_TOKEN",
            "DEVNPC_TEST_MD_PROJECT_ID",
        ] {
            std::env::remove_var(key);
        }
    }
}
