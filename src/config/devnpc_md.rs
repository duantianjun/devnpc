//! .devnpc.md 解析: YAML front matter + Markdown 正文
//!
//! 格式:
//! ```markdown
//! ---
//! sop_mode: strict
//! forbidden_paths:
//!   - ".gitlab-ci.yml"
//! ---
//! # 项目规范
//! ...正文...
//! ```

use serde::Deserialize;

use crate::error::Result;

/// front matter 反序列化结构 (字段全部可选,缺失用默认)
#[derive(Debug, Clone, Default, Deserialize, PartialEq, Eq)]
pub struct DevnpcMdFrontMatter {
    #[serde(default)]
    pub sop_mode: Option<String>,
    #[serde(default)]
    pub forbidden_paths: Option<Vec<String>>,
    #[serde(default)]
    pub required_checks: Option<Vec<String>>,
    #[serde(default)]
    pub branch_prefix: Option<String>,
    #[serde(default)]
    pub max_ci_retries: Option<u8>,
}

/// .devnpc.md 解析结果: front matter + 正文 markdown
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ParsedDevnpcMd {
    pub front_matter: DevnpcMdFrontMatter,
    pub guidelines_markdown: String,
}

/// 解析 .devnpc.md 内容
///
/// - 无 front matter (文件不以 `---\n` 开头): 整体作为 guidelines_markdown,front_matter 为默认。
/// - 有 front matter: `---` 分隔,首段 YAML 反序列化,剩余为正文。
/// - front matter 为空 (`---\n---`): front_matter 默认,正文为空。
pub fn parse_devnpc_md(content: &str) -> Result<ParsedDevnpcMd> {
    // 必须以 `---\n` 开头才算 front matter
    if !content.starts_with("---\n") && !content.starts_with("---\r\n") {
        return Ok(ParsedDevnpcMd {
            front_matter: DevnpcMdFrontMatter::default(),
            guidelines_markdown: content.to_string(),
        });
    }

    // 找第二个 `---` 行 (front matter 结束)
    let lines: Vec<&str> = content.lines().collect();
    let mut end_idx: Option<usize> = None;
    for (i, line) in lines.iter().enumerate().skip(1) {
        if *line == "---" {
            end_idx = Some(i);
            break;
        }
    }

    let Some(end) = end_idx else {
        // 没有闭合 `---`,整体当正文
        return Ok(ParsedDevnpcMd {
            front_matter: DevnpcMdFrontMatter::default(),
            guidelines_markdown: content.to_string(),
        });
    };

    // YAML 是 lines[1..end]
    let yaml_str: String = lines[1..end].join("\n");
    let front_matter: DevnpcMdFrontMatter = if yaml_str.trim().is_empty() {
        DevnpcMdFrontMatter::default()
    } else {
        serde_yaml::from_str(&yaml_str)?
    };

    // 正文是 lines[end+1..],去掉首行空行
    let body = lines
        .get(end + 1..)
        .map(|s| s.join("\n"))
        .unwrap_or_default();
    let guidelines_markdown = body.trim_start_matches('\n').to_string();

    Ok(ParsedDevnpcMd {
        front_matter,
        guidelines_markdown,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_full_front_matter_and_body() {
        let content = "---\nsop_mode: strict\nforbidden_paths:\n  - \".gitlab-ci.yml\"\n  - \"Cargo.lock\"\nrequired_checks:\n  - \"cargo fmt --check\"\nbranch_prefix: \"npc\"\nmax_ci_retries: 2\n---\n# 项目规范\n\n## 技术栈\n- Rust\n";
        let parsed = parse_devnpc_md(content).unwrap();
        assert_eq!(parsed.front_matter.sop_mode.as_deref(), Some("strict"));
        assert_eq!(
            parsed.front_matter.forbidden_paths.as_deref(),
            Some(&vec![".gitlab-ci.yml".to_string(), "Cargo.lock".to_string()][..])
        );
        assert_eq!(parsed.front_matter.max_ci_retries, Some(2));
        assert_eq!(parsed.front_matter.branch_prefix.as_deref(), Some("npc"));
        assert!(parsed.guidelines_markdown.contains("# 项目规范"));
        assert!(parsed.guidelines_markdown.contains("## 技术栈"));
    }

    #[test]
    fn parse_no_front_matter_returns_all_as_body() {
        let content = "# 仅正文\n没有 front matter";
        let parsed = parse_devnpc_md(content).unwrap();
        assert_eq!(parsed.front_matter, DevnpcMdFrontMatter::default());
        assert_eq!(parsed.guidelines_markdown, content);
    }

    #[test]
    fn parse_empty_front_matter() {
        let content = "---\n---\n# 正文\n";
        let parsed = parse_devnpc_md(content).unwrap();
        assert_eq!(parsed.front_matter, DevnpcMdFrontMatter::default());
        // lines() 会吃掉末尾换行,trim_start_matches 去掉首部空行
        assert_eq!(parsed.guidelines_markdown, "# 正文");
    }

    #[test]
    fn parse_partial_front_matter_missing_fields() {
        let content = "---\nsop_mode: soft\n---\n正文";
        let parsed = parse_devnpc_md(content).unwrap();
        assert_eq!(parsed.front_matter.sop_mode.as_deref(), Some("soft"));
        assert_eq!(parsed.front_matter.forbidden_paths, None);
        assert_eq!(parsed.front_matter.max_ci_retries, None);
        assert_eq!(parsed.guidelines_markdown, "正文");
    }

    #[test]
    fn parse_unclosed_front_matter_falls_back_to_body() {
        let content = "---\nsop_mode: strict\n没有闭合";
        let parsed = parse_devnpc_md(content).unwrap();
        assert_eq!(parsed.front_matter, DevnpcMdFrontMatter::default());
        assert_eq!(parsed.guidelines_markdown, content);
    }

    #[test]
    fn parse_invalid_yaml_returns_error() {
        let content = "---\nsop_mode: [unclosed\n---\n正文";
        let result = parse_devnpc_md(content);
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            crate::error::DevnpcError::Yaml(_)
        ));
    }
}
