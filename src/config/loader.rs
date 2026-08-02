//! 配置加载器: 环境变量 + .devnpc.md + 默认值三层合并
//!
//! 优先级 (高 → 低): 环境变量 > .devnpc.md > 内置默认。

use std::path::Path;

use crate::config::devnpc_md::{parse_devnpc_md, DevnpcMdFrontMatter, ParsedDevnpcMd};
use crate::config::env;
use crate::config::{
    CiConfig, CommandConfig, Config, ContextConfig, GitlabConfig, Limits, LlmConfig,
    LogParserConfig, ProjectConfig, ReadFileConfig, ReportConfig, ReportTarget, SopMode,
    SummaryConfig,
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
    model_routing_var: &str,
    // 新增集中配置环境变量名
    cmd_allowlist_var: &str,
    cmd_denylist_var: &str,
    default_timeout_var: &str,
    read_file_max_lines_var: &str,
    log_parser_max_failures_var: &str,
    key_file_patterns_var: &str,
    summary_readme_lines_var: &str,
    summary_main_rs_lines_var: &str,
    summary_other_lines_var: &str,
    ctx_max_commits_var: &str,
    ctx_max_pipelines_var: &str,
    ctx_max_failures_var: &str,
    ci_poll_interval_var: &str,
    ci_poll_timeout_var: &str,
    ci_pipeline_timeout_var: &str,
    ci_max_retries_var: &str,
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
    let model_routing = env::get_model_routing(model_routing_var)?.unwrap_or_default();

    // 4. 新增集中配置: 环境变量覆盖 → 默认值
    let command = CommandConfig {
        allowlist: env::get_vec(cmd_allowlist_var).unwrap_or_default(),
        denylist: env::get_vec(cmd_denylist_var).unwrap_or_default(),
        default_timeout_secs: env::get_u64(default_timeout_var)?.unwrap_or(120),
    };
    // 如果 allowlist/denylist 为空, 用默认值
    let command = if command.allowlist.is_empty() && command.denylist.is_empty() {
        CommandConfig::default()
    } else {
        CommandConfig {
            allowlist: if command.allowlist.is_empty() {
                CommandConfig::default().allowlist
            } else {
                command.allowlist
            },
            denylist: if command.denylist.is_empty() {
                CommandConfig::default().denylist
            } else {
                command.denylist
            },
            default_timeout_secs: command.default_timeout_secs,
        }
    };

    let read_file = ReadFileConfig {
        max_lines: env::get_usize(read_file_max_lines_var)?.unwrap_or(200),
    };

    let log_parser = LogParserConfig {
        max_failures: env::get_usize(log_parser_max_failures_var)?.unwrap_or(10),
    };

    let summary = {
        let default_summary = SummaryConfig::default();
        let patterns = env::get_vec(key_file_patterns_var)
            .filter(|v| !v.is_empty())
            .unwrap_or(default_summary.key_file_patterns);
        SummaryConfig {
            key_file_patterns: patterns,
            readme_lines: env::get_usize(summary_readme_lines_var)?.unwrap_or(default_summary.readme_lines),
            main_rs_lines: env::get_usize(summary_main_rs_lines_var)?.unwrap_or(default_summary.main_rs_lines),
            other_lines: env::get_usize(summary_other_lines_var)?.unwrap_or(default_summary.other_lines),
        }
    };

    let context = ContextConfig {
        max_recent_commits: env::get_usize(ctx_max_commits_var)?.unwrap_or(20),
        max_recent_pipelines: env::get_usize(ctx_max_pipelines_var)?.unwrap_or(5),
        max_ci_history_failures: env::get_usize(ctx_max_failures_var)?.unwrap_or(5),
    };

    let ci = CiConfig {
        poll_interval_secs: env::get_u64(ci_poll_interval_var)?.unwrap_or(10),
        poll_timeout_secs: env::get_u64(ci_poll_timeout_var)?.unwrap_or(300),
        pipeline_timeout_secs: env::get_u64(ci_pipeline_timeout_var)?.unwrap_or(1800),
        max_retries: env::get_u8(ci_max_retries_var)?.unwrap_or(3),
    };

    Ok(Config {
        llm: LlmConfig {
            api_key,
            base_url,
            model,
            provider: "deepseek".to_string(),
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
        model_routing,
        report: ReportConfig {
            target: report_target,
        },
        command,
        read_file,
        log_parser,
        summary,
        context,
        ci,
    })
}

/// 生产环境配置加载 (使用标准环境变量名)
pub fn load() -> Result<Config> {
    let cwd = std::env::current_dir().ok();
    let devnpc_md_path = cwd.as_ref().map(|p| p.join(".devnpc.md"));
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
        "DEVNPC_MODEL_ROUTING",
        // 新增集中配置环境变量名
        "DEVNPC_COMMAND_ALLOWLIST",
        "DEVNPC_COMMAND_DENYLIST",
        "DEVNPC_DEFAULT_TIMEOUT_SECS",
        "DEVNPC_READ_FILE_MAX_LINES",
        "DEVNPC_LOG_PARSER_MAX_FAILURES",
        "DEVNPC_KEY_FILE_PATTERNS",
        "DEVNPC_SUMMARY_README_LINES",
        "DEVNPC_SUMMARY_MAIN_RS_LINES",
        "DEVNPC_SUMMARY_OTHER_LINES",
        "DEVNPC_CONTEXT_MAX_COMMITS",
        "DEVNPC_CONTEXT_MAX_PIPELINES",
        "DEVNPC_CONTEXT_MAX_CI_FAILURES",
        "DEVNPC_CI_POLL_INTERVAL_SECS",
        "DEVNPC_CI_POLL_TIMEOUT_SECS",
        "DEVNPC_CI_PIPELINE_TIMEOUT_SECS",
        "DEVNPC_CI_MAX_RETRIES",
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
        unsafe { std::env::set_var("DEVNPC_TEST_MERGE_API_KEY", "sk-merge"); }
        unsafe { std::env::set_var("DEVNPC_TEST_MERGE_BASE_URL", "https://api.test.com/v1"); }
        unsafe { std::env::set_var("DEVNPC_TEST_MERGE_MODEL", "test-model"); }
        unsafe { std::env::set_var("DEVNPC_TEST_MERGE_GITLAB_URL", "https://gitlab.test.com"); }
        unsafe { std::env::set_var("DEVNPC_TEST_MERGE_GITLAB_TOKEN", "gl-token"); }
        unsafe { std::env::set_var("DEVNPC_TEST_MERGE_PROJECT_ID", "42"); }
        unsafe { std::env::set_var("DEVNPC_TEST_MERGE_MAX_ITERATIONS", "30"); }
        unsafe { std::env::set_var("DEVNPC_TEST_MERGE_MAX_CI_RETRIES", "5"); }
        unsafe { std::env::set_var("DEVNPC_TEST_MERGE_SOP_MODE", "strict"); }

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
            "DEVNPC_TEST_MERGE_MODEL_ROUTING",
            // 新参数: 不设置,使用默认值
            "DEVNPC_TEST_MERGE_CMD_ALLOWLIST",
            "DEVNPC_TEST_MERGE_CMD_DENYLIST",
            "DEVNPC_TEST_MERGE_DEFAULT_TIMEOUT",
            "DEVNPC_TEST_MERGE_READ_FILE_MAX_LINES",
            "DEVNPC_TEST_MERGE_LOG_PARSER_MAX_FAILURES",
            "DEVNPC_TEST_MERGE_KEY_FILE_PATTERNS",
            "DEVNPC_TEST_MERGE_SUMMARY_README_LINES",
            "DEVNPC_TEST_MERGE_SUMMARY_MAIN_RS_LINES",
            "DEVNPC_TEST_MERGE_SUMMARY_OTHER_LINES",
            "DEVNPC_TEST_MERGE_CTX_MAX_COMMITS",
            "DEVNPC_TEST_MERGE_CTX_MAX_PIPELINES",
            "DEVNPC_TEST_MERGE_CTX_MAX_FAILURES",
            "DEVNPC_TEST_MERGE_CI_POLL_INTERVAL",
            "DEVNPC_TEST_MERGE_CI_POLL_TIMEOUT",
            "DEVNPC_TEST_MERGE_CI_PIPELINE_TIMEOUT",
            "DEVNPC_TEST_MERGE_CI_MAX_RETRIES",
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
        // 模型路由默认
        assert!(config.model_routing.simple_model.is_empty());
        // 新增集中配置使用默认值
        assert_eq!(config.command.default_timeout_secs, 120);
        assert_eq!(config.read_file.max_lines, 200);
        assert_eq!(config.log_parser.max_failures, 10);
        assert_eq!(config.context.max_recent_commits, 20);
        assert_eq!(config.ci.poll_interval_secs, 10);

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
            unsafe { std::env::remove_var(key); }
        }
    }

    #[test]
    fn load_fails_when_required_llm_env_missing() {
        unsafe { std::env::remove_var("DEVNPC_TEST_FAIL_API_KEY"); }
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
            "DEVNPC_TEST_FAIL_MODEL_ROUTING",
            "DEVNPC_TEST_FAIL_CMD_ALLOWLIST",
            "DEVNPC_TEST_FAIL_CMD_DENYLIST",
            "DEVNPC_TEST_FAIL_DEFAULT_TIMEOUT",
            "DEVNPC_TEST_FAIL_READ_FILE_MAX_LINES",
            "DEVNPC_TEST_FAIL_LOG_PARSER_MAX_FAILURES",
            "DEVNPC_TEST_FAIL_KEY_FILE_PATTERNS",
            "DEVNPC_TEST_FAIL_SUMMARY_README_LINES",
            "DEVNPC_TEST_FAIL_SUMMARY_MAIN_RS_LINES",
            "DEVNPC_TEST_FAIL_SUMMARY_OTHER_LINES",
            "DEVNPC_TEST_FAIL_CTX_MAX_COMMITS",
            "DEVNPC_TEST_FAIL_CTX_MAX_PIPELINES",
            "DEVNPC_TEST_FAIL_CTX_MAX_FAILURES",
            "DEVNPC_TEST_FAIL_CI_POLL_INTERVAL",
            "DEVNPC_TEST_FAIL_CI_POLL_TIMEOUT",
            "DEVNPC_TEST_FAIL_CI_PIPELINE_TIMEOUT",
            "DEVNPC_TEST_FAIL_CI_MAX_RETRIES",
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
        unsafe { std::env::set_var("DEVNPC_TEST_DEFAULT_API_KEY", "sk"); }
        unsafe { std::env::set_var("DEVNPC_TEST_DEFAULT_BASE_URL", "https://api.test.com/v1"); }
        unsafe { std::env::set_var("DEVNPC_TEST_DEFAULT_MODEL", "m"); }
        unsafe { std::env::set_var("DEVNPC_TEST_DEFAULT_GITLAB_URL", "https://gl.test.com"); }
        unsafe { std::env::set_var("DEVNPC_TEST_DEFAULT_GITLAB_TOKEN", "t"); }
        unsafe { std::env::set_var("DEVNPC_TEST_DEFAULT_PROJECT_ID", "1"); }
        unsafe { std::env::remove_var("DEVNPC_TEST_DEFAULT_MAX_ITERATIONS"); }
        unsafe { std::env::remove_var("DEVNPC_TEST_DEFAULT_MAX_CI_RETRIES"); }
        unsafe { std::env::remove_var("DEVNPC_TEST_DEFAULT_SOP_MODE"); }
        unsafe { std::env::remove_var("DEVNPC_TEST_DEFAULT_REPORT_TARGET"); }

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
            "DEVNPC_TEST_DEFAULT_MODEL_ROUTING",
            "DEVNPC_TEST_DEFAULT_CMD_ALLOWLIST",
            "DEVNPC_TEST_DEFAULT_CMD_DENYLIST",
            "DEVNPC_TEST_DEFAULT_DEFAULT_TIMEOUT",
            "DEVNPC_TEST_DEFAULT_READ_FILE_MAX_LINES",
            "DEVNPC_TEST_DEFAULT_LOG_PARSER_MAX_FAILURES",
            "DEVNPC_TEST_DEFAULT_KEY_FILE_PATTERNS",
            "DEVNPC_TEST_DEFAULT_SUMMARY_README_LINES",
            "DEVNPC_TEST_DEFAULT_SUMMARY_MAIN_RS_LINES",
            "DEVNPC_TEST_DEFAULT_SUMMARY_OTHER_LINES",
            "DEVNPC_TEST_DEFAULT_CTX_MAX_COMMITS",
            "DEVNPC_TEST_DEFAULT_CTX_MAX_PIPELINES",
            "DEVNPC_TEST_DEFAULT_CTX_MAX_FAILURES",
            "DEVNPC_TEST_DEFAULT_CI_POLL_INTERVAL",
            "DEVNPC_TEST_DEFAULT_CI_POLL_TIMEOUT",
            "DEVNPC_TEST_DEFAULT_CI_PIPELINE_TIMEOUT",
            "DEVNPC_TEST_DEFAULT_CI_MAX_RETRIES",
            None,
        )
        .unwrap();

        assert_eq!(config.limits.max_iterations, 20);
        assert_eq!(config.limits.max_ci_retries, 3);
        assert_eq!(config.project.sop_mode, SopMode::Soft);
        assert_eq!(config.project.branch_prefix, "npc");
        assert_eq!(config.report.target, ReportTarget::Artifact);
        assert!(config.model_routing.simple_model.is_empty());
        // 新增集中配置默认值
        assert_eq!(config.command.default_timeout_secs, 120);
        assert_eq!(config.read_file.max_lines, 200);
        assert_eq!(config.log_parser.max_failures, 10);
        assert_eq!(config.summary.readme_lines, 30);
        assert_eq!(config.context.max_recent_commits, 20);
        assert_eq!(config.ci.poll_interval_secs, 10);

        for key in [
            "DEVNPC_TEST_DEFAULT_API_KEY",
            "DEVNPC_TEST_DEFAULT_BASE_URL",
            "DEVNPC_TEST_DEFAULT_MODEL",
            "DEVNPC_TEST_DEFAULT_GITLAB_URL",
            "DEVNPC_TEST_DEFAULT_GITLAB_TOKEN",
            "DEVNPC_TEST_DEFAULT_PROJECT_ID",
        ] {
            unsafe { std::env::remove_var(key); }
        }
    }

    #[test]
    fn load_uses_devnpc_md_max_ci_retries_when_env_missing() {
        unsafe { std::env::set_var("DEVNPC_TEST_MD_API_KEY", "sk"); }
        unsafe { std::env::set_var("DEVNPC_TEST_MD_BASE_URL", "https://api.test.com/v1"); }
        unsafe { std::env::set_var("DEVNPC_TEST_MD_MODEL", "m"); }
        unsafe { std::env::set_var("DEVNPC_TEST_MD_GITLAB_URL", "https://gl.test.com"); }
        unsafe { std::env::set_var("DEVNPC_TEST_MD_GITLAB_TOKEN", "t"); }
        unsafe { std::env::set_var("DEVNPC_TEST_MD_PROJECT_ID", "1"); }
        unsafe { std::env::remove_var("DEVNPC_TEST_MD_MAX_CI_RETRIES"); }

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
            "DEVNPC_TEST_MD_MODEL_ROUTING",
            "DEVNPC_TEST_MD_CMD_ALLOWLIST",
            "DEVNPC_TEST_MD_CMD_DENYLIST",
            "DEVNPC_TEST_MD_DEFAULT_TIMEOUT",
            "DEVNPC_TEST_MD_READ_FILE_MAX_LINES",
            "DEVNPC_TEST_MD_LOG_PARSER_MAX_FAILURES",
            "DEVNPC_TEST_MD_KEY_FILE_PATTERNS",
            "DEVNPC_TEST_MD_SUMMARY_README_LINES",
            "DEVNPC_TEST_MD_SUMMARY_MAIN_RS_LINES",
            "DEVNPC_TEST_MD_SUMMARY_OTHER_LINES",
            "DEVNPC_TEST_MD_CTX_MAX_COMMITS",
            "DEVNPC_TEST_MD_CTX_MAX_PIPELINES",
            "DEVNPC_TEST_MD_CTX_MAX_FAILURES",
            "DEVNPC_TEST_MD_CI_POLL_INTERVAL",
            "DEVNPC_TEST_MD_CI_POLL_TIMEOUT",
            "DEVNPC_TEST_MD_CI_PIPELINE_TIMEOUT",
            "DEVNPC_TEST_MD_CI_MAX_RETRIES",
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
            unsafe { std::env::remove_var(key); }
        }
    }

    #[test]
    fn load_new_config_fields_from_env() {
        unsafe { std::env::set_var("DEVNPC_TEST_NEW_API_KEY", "sk"); }
        unsafe { std::env::set_var("DEVNPC_TEST_NEW_BASE_URL", "https://api.test.com/v1"); }
        unsafe { std::env::set_var("DEVNPC_TEST_NEW_MODEL", "m"); }
        unsafe { std::env::set_var("DEVNPC_TEST_NEW_GITLAB_URL", "https://gl.test.com"); }
        unsafe { std::env::set_var("DEVNPC_TEST_NEW_GITLAB_TOKEN", "t"); }
        unsafe { std::env::set_var("DEVNPC_TEST_NEW_PROJECT_ID", "1"); }
        // 设置新配置
        unsafe { std::env::set_var("DEVNPC_TEST_NEW_CMD_ALLOWLIST", "cargo,rustc,echo"); }
        unsafe { std::env::set_var("DEVNPC_TEST_NEW_CMD_DENYLIST", "rm,curl"); }
        unsafe { std::env::set_var("DEVNPC_TEST_NEW_DEFAULT_TIMEOUT", "300"); }
        unsafe { std::env::set_var("DEVNPC_TEST_NEW_READ_FILE_MAX_LINES", "500"); }
        unsafe { std::env::set_var("DEVNPC_TEST_NEW_LOG_PARSER_MAX_FAILURES", "20"); }
        unsafe { std::env::set_var("DEVNPC_TEST_NEW_KEY_FILE_PATTERNS", "Cargo.toml,README.md"); }
        unsafe { std::env::set_var("DEVNPC_TEST_NEW_SUMMARY_README_LINES", "50"); }
        unsafe { std::env::set_var("DEVNPC_TEST_NEW_SUMMARY_MAIN_RS_LINES", "80"); }
        unsafe { std::env::set_var("DEVNPC_TEST_NEW_SUMMARY_OTHER_LINES", "30"); }
        unsafe { std::env::set_var("DEVNPC_TEST_NEW_CTX_MAX_COMMITS", "50"); }
        unsafe { std::env::set_var("DEVNPC_TEST_NEW_CTX_MAX_PIPELINES", "10"); }
        unsafe { std::env::set_var("DEVNPC_TEST_NEW_CTX_MAX_FAILURES", "10"); }
        unsafe { std::env::set_var("DEVNPC_TEST_NEW_CI_POLL_INTERVAL", "5"); }
        unsafe { std::env::set_var("DEVNPC_TEST_NEW_CI_POLL_TIMEOUT", "600"); }
        unsafe { std::env::set_var("DEVNPC_TEST_NEW_CI_PIPELINE_TIMEOUT", "3600"); }
        unsafe { std::env::set_var("DEVNPC_TEST_NEW_CI_MAX_RETRIES", "5"); }

        let config = load_internal(
            "DEVNPC_TEST_NEW_API_KEY",
            "DEVNPC_TEST_NEW_BASE_URL",
            "DEVNPC_TEST_NEW_MODEL",
            "DEVNPC_TEST_NEW_GITLAB_URL",
            "DEVNPC_TEST_NEW_GITLAB_TOKEN",
            "DEVNPC_TEST_NEW_PROJECT_ID",
            "DEVNPC_TEST_NEW_MAX_ITERATIONS",
            "DEVNPC_TEST_NEW_MAX_CI_RETRIES",
            "DEVNPC_TEST_NEW_SOP_MODE",
            "DEVNPC_TEST_NEW_REPORT_TARGET",
            "DEVNPC_TEST_NEW_MODEL_ROUTING",
            "DEVNPC_TEST_NEW_CMD_ALLOWLIST",
            "DEVNPC_TEST_NEW_CMD_DENYLIST",
            "DEVNPC_TEST_NEW_DEFAULT_TIMEOUT",
            "DEVNPC_TEST_NEW_READ_FILE_MAX_LINES",
            "DEVNPC_TEST_NEW_LOG_PARSER_MAX_FAILURES",
            "DEVNPC_TEST_NEW_KEY_FILE_PATTERNS",
            "DEVNPC_TEST_NEW_SUMMARY_README_LINES",
            "DEVNPC_TEST_NEW_SUMMARY_MAIN_RS_LINES",
            "DEVNPC_TEST_NEW_SUMMARY_OTHER_LINES",
            "DEVNPC_TEST_NEW_CTX_MAX_COMMITS",
            "DEVNPC_TEST_NEW_CTX_MAX_PIPELINES",
            "DEVNPC_TEST_NEW_CTX_MAX_FAILURES",
            "DEVNPC_TEST_NEW_CI_POLL_INTERVAL",
            "DEVNPC_TEST_NEW_CI_POLL_TIMEOUT",
            "DEVNPC_TEST_NEW_CI_PIPELINE_TIMEOUT",
            "DEVNPC_TEST_NEW_CI_MAX_RETRIES",
            None,
        )
        .unwrap();

        // 验证新配置字段
        assert_eq!(config.command.default_timeout_secs, 300);
        assert!(config.command.allowlist.contains(&"cargo".to_string()));
        assert!(config.command.denylist.contains(&"rm".to_string()));
        assert_eq!(config.read_file.max_lines, 500);
        assert_eq!(config.log_parser.max_failures, 20);
        assert_eq!(config.summary.key_file_patterns.len(), 2);
        assert_eq!(config.summary.readme_lines, 50);
        assert_eq!(config.summary.main_rs_lines, 80);
        assert_eq!(config.summary.other_lines, 30);
        assert_eq!(config.context.max_recent_commits, 50);
        assert_eq!(config.context.max_recent_pipelines, 10);
        assert_eq!(config.context.max_ci_history_failures, 10);
        assert_eq!(config.ci.poll_interval_secs, 5);
        assert_eq!(config.ci.poll_timeout_secs, 600);
        assert_eq!(config.ci.pipeline_timeout_secs, 3600);
        assert_eq!(config.ci.max_retries, 5);

        // 清理
        for key in [
            "DEVNPC_TEST_NEW_API_KEY",
            "DEVNPC_TEST_NEW_BASE_URL",
            "DEVNPC_TEST_NEW_MODEL",
            "DEVNPC_TEST_NEW_GITLAB_URL",
            "DEVNPC_TEST_NEW_GITLAB_TOKEN",
            "DEVNPC_TEST_NEW_PROJECT_ID",
            "DEVNPC_TEST_NEW_CMD_ALLOWLIST",
            "DEVNPC_TEST_NEW_CMD_DENYLIST",
            "DEVNPC_TEST_NEW_DEFAULT_TIMEOUT",
            "DEVNPC_TEST_NEW_READ_FILE_MAX_LINES",
            "DEVNPC_TEST_NEW_LOG_PARSER_MAX_FAILURES",
            "DEVNPC_TEST_NEW_KEY_FILE_PATTERNS",
            "DEVNPC_TEST_NEW_SUMMARY_README_LINES",
            "DEVNPC_TEST_NEW_SUMMARY_MAIN_RS_LINES",
            "DEVNPC_TEST_NEW_SUMMARY_OTHER_LINES",
            "DEVNPC_TEST_NEW_CTX_MAX_COMMITS",
            "DEVNPC_TEST_NEW_CTX_MAX_PIPELINES",
            "DEVNPC_TEST_NEW_CTX_MAX_FAILURES",
            "DEVNPC_TEST_NEW_CI_POLL_INTERVAL",
            "DEVNPC_TEST_NEW_CI_POLL_TIMEOUT",
            "DEVNPC_TEST_NEW_CI_PIPELINE_TIMEOUT",
            "DEVNPC_TEST_NEW_CI_MAX_RETRIES",
        ] {
            unsafe { std::env::remove_var(key); }
        }
    }
}