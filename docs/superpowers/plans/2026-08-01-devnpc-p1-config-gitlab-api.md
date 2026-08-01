# devnpc P1: 配置系统 + GitLab API 客户端 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 实现 `Config::load()` 完整逻辑 (env + `.devnpc.md` 解析) 与 `GitlabClient` 所有 API 方法 (reqwest 调用 GitLab REST v4),补充单元测试,使 `devnpc config` 命令能打印合并后的配置,`GitlabClient` 能从真实 GitLab 读取 Issue/MR/Pipeline/Notes。

**Architecture:** 配置系统采用三层合并: 环境变量 (最高优先级) > `.devnpc.md` (项目级) > 内置默认值。`.devnpc.md` 用 YAML front matter + Markdown 正文,front matter 由 `serde_yaml` 反序列化为 `ProjectConfig`,正文整体作为 `guidelines_markdown`。GitLab API 客户端基于 `reqwest`,统一封装 `GET/POST` 请求,处理状态码与错误,所有方法实现 `GitlabApi` trait,便于用 `mockall` mock 测试。

**Tech Stack:** Rust 2021, `serde`/`serde_yaml` (配置反序列化), `reqwest` (HTTP), `async-trait` (异步 trait), `tokio` (异步运行时), `mockall` (测试 mock), `tempfile` (测试临时文件), `wiremock` 不引入 (用 `mockall` mock trait 即可,避免新依赖)。

**参考文档:**
- [2026-08-01-devnpc-design.md](../specs/2026-08-01-devnpc-design.md) 第 4 节配置系统、第 5 节记忆来源、第 10.5 节目录结构。
- GitLab REST API v4 文档: https://docs.gitlab.com/ee/api/rest/

**P0 现状:** 所有模块骨架已就位,`Config::load()` 与 `GitlabClient` 方法返回 `unimplemented!`/占位错误。本计划替换占位实现,保持 P0 测试 (13 个) 不被破坏,新增 P1 测试。

---

## File Structure

P1 修改以下文件,职责单一:

| 文件 | 职责 | 改动类型 |
|---|---|---|
| `src/error.rs` | 新增 `Reqwest` 与 `Yaml` 错误变体 | 修改 |
| `src/config/mod.rs` | `Config::load()` 调用 loader,`ProjectConfig` 加 `max_ci_retries` 字段 | 修改 |
| `src/config/env.rs` | 环境变量读取 (已有 `get_required`/`get_or_default`,补充 `u32`/`u8`/`bool` 解析) | 修改 |
| `src/config/loader.rs` | `.devnpc.md` 读取 + front matter 解析 + 三层合并 | 重写 (从占位) |
| `src/config/devnpc_md.rs` | `.devnpc.md` front matter 反序列化结构 + 解析函数 | 新建 |
| `src/gitlab_api/mod.rs` | trait 方法补充 (可选) | 修改 (如需) |
| `src/gitlab_api/client.rs` | `GitlabClient` 实现所有 `GitlabApi` 方法 + 内部 `get`/`post` 封装 | 重写 |
| `src/gitlab_api/client_tests.rs` | `GitlabClient` 单元测试 (mock HTTP via `mockall` 或 `wiremock`) | 新建 |
| `src/main.rs` | `print_config` 调用 `Config::load()` 打印 | 修改 |
| `Cargo.toml` | 新增 `wiremock` dev-dependency | 修改 |

**关键设计决策:**
- **HTTP mock 选型:** 用 `wiremock` (异步 HTTP mock) 测试真实 reqwest 调用,比 `mockall` mock trait 更能验证 URL/header/反序列化。引入为 dev-dependency。
- **`.devnpc.md` 解析:** 用简单字符串分割 front matter (`---` 分隔),不引入 `markdown` crate。front matter 用 `serde_yaml::from_str`,正文整体作为 `guidelines_markdown`。
- **配置合并优先级:** env 覆盖 `.devnpc.md` 覆盖默认。`max_ci_retries`/`sop_mode` 在 `.devnpc.md` 有则用,环境变量 `DEVNPC_MAX_CI_RETRIES`/`DEVNPC_SOP_MODE` 进一步覆盖。
- **P1 不实现 roles/sops YAML 加载:** 设计文档第 4.6 节的 `load_roles`/`load_sops` 放 P6 (Role/SOP 系统),P1 仅保留注释占位。`Config` 结构体的 `roles`/`sops` 字段在 P0 已注释,P1 保持注释。

---

### Task 1: 新增 `wiremock` dev-dependency 与错误变体

**Files:**
- Modify: `Cargo.toml`
- Modify: `src/error.rs`

- [ ] **Step 1: 在 Cargo.toml 添加 wiremock dev-dependency**

Modify `Cargo.toml` 的 `[dev-dependencies]` 节,在 `tempfile = "3"` 之后添加:

```toml
wiremock = "0.6"
```

- [ ] **Step 2: 在 error.rs 新增 Reqwest 与 Yaml 错误变体**

Modify `src/error.rs`,在 `Io(#[from] std::io::Error)` 之前添加两个变体:

```rust
    #[error("GitLab API 请求失败: {0}")]
    Reqwest(#[from] reqwest::Error),

    #[error("YAML 解析错误: {0}")]
    Yaml(#[from] serde_yaml::Error),
```

完整 `DevnpcError` enum 的末尾应为:

```rust
    #[error("路径越界: {path} 不在 workspace 内")]
    PathTraversal { path: String },

    #[error("GitLab API 请求失败: {0}")]
    Reqwest(#[from] reqwest::Error),

    #[error("YAML 解析错误: {0}")]
    Yaml(#[from] serde_yaml::Error),

    #[error(transparent)]
    Io(#[from] std::io::Error),
}
```

- [ ] **Step 3: 验证编译**

Run: `cargo build`
Expected: 编译成功 (wiremock 会下载)。

- [ ] **Step 4: 提交**

```bash
git add Cargo.toml Cargo.lock src/error.rs
git commit -m "feat: 新增 wiremock dev-dep + Reqwest/Yaml 错误变体"
```

---

### Task 2: `.devnpc.md` 解析 (front matter + 正文)

**Files:**
- Create: `src/config/devnpc_md.rs`
- Modify: `src/config/mod.rs`

- [ ] **Step 1: 写失败测试 - 解析 .devnpc.md**

Create `src/config/devnpc_md.rs`:

```rust
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
    let body = lines.get(end + 1..).map(|s| s.join("\n")).unwrap_or_default();
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
        assert_eq!(parsed.guidelines_markdown, "# 正文\n");
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
        assert!(matches!(result.unwrap_err(), crate::error::DevnpcError::Yaml(_)));
    }
}
```

- [ ] **Step 2: 在 config/mod.rs 注册 devnpc_md 子模块**

Modify `src/config/mod.rs`,在 `pub mod env;` 之后添加:

```rust
pub mod devnpc_md;
```

- [ ] **Step 3: 运行测试验证**

Run: `cargo test --lib config::devnpc_md`
Expected: 6 个测试 PASS。

- [ ] **Step 4: 提交**

```bash
git add src/config/devnpc_md.rs src/config/mod.rs
git commit -m "feat: .devnpc.md 解析 (front matter + 正文)"
```

---

### Task 3: 环境变量解析扩展 (u32/u8/bool/SopMode)

**Files:**
- Modify: `src/config/env.rs`

- [ ] **Step 1: 写失败测试 - 类型化环境变量解析**

Replace `src/config/env.rs` 完整内容为:

```rust
//! 环境变量读取与类型解析

use crate::config::SopMode;
use crate::error::{DevnpcError, Result};

/// 从环境变量读取字符串,缺失则返回错误
pub fn get_required(var: &str) -> Result<String> {
    std::env::var(var).map_err(|_| DevnpcError::MissingEnv { var: var.into() })
}

/// 从环境变量读取字符串,缺失返回默认值
pub fn get_or_default(var: &str, default: &str) -> String {
    std::env::var(var).unwrap_or_else(|_| default.into())
}

/// 读取可选字符串环境变量 (缺失返回 None)
pub fn get_optional(var: &str) -> Option<String> {
    std::env::var(var).ok()
}

/// 读取并解析为 u32,缺失返回 None,解析失败返回错误
pub fn get_u32(var: &str) -> Result<Option<u32>> {
    match std::env::var(var) {
        Ok(s) => s
            .parse::<u32>()
            .map(Some)
            .map_err(|_| DevnpcError::Config(format!("环境变量 {var} 不是有效 u32: {s}"))),
        Err(_) => Ok(None),
    }
}

/// 读取并解析为 u8,缺失返回 None,解析失败返回错误
pub fn get_u8(var: &str) -> Result<Option<u8>> {
    match std::env::var(var) {
        Ok(s) => s
            .parse::<u8>()
            .map(Some)
            .map_err(|_| DevnpcError::Config(format!("环境变量 {var} 不是有效 u8: {s}"))),
        Err(_) => Ok(None),
    }
}

/// 读取并解析 SopMode,缺失返回 None,非法值返回错误
pub fn get_sop_mode(var: &str) -> Result<Option<SopMode>> {
    match std::env::var(var) {
        Ok(s) => match s.as_str() {
            "soft" => Ok(Some(SopMode::Soft)),
            "strict" => Ok(Some(SopMode::Strict)),
            _ => Err(DevnpcError::Config(format!(
                "环境变量 {var} 必须是 soft|strict,实际: {s}"
            ))),
        },
        Err(_) => Ok(None),
    }
}

/// 读取并解析 ReportTarget,缺失返回 None,非法值返回错误
pub fn get_report_target(var: &str) -> Result<Option<crate::config::ReportTarget>> {
    use crate::config::ReportTarget;
    match std::env::var(var) {
        Ok(s) => match s.as_str() {
            "artifact" => Ok(Some(ReportTarget::Artifact)),
            "pages" => Ok(Some(ReportTarget::Pages)),
            "none" => Ok(Some(ReportTarget::None)),
            _ => Err(DevnpcError::Config(format!(
                "环境变量 {var} 必须是 artifact|pages|none,实际: {s}"
            ))),
        },
        Err(_) => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // 注意: 环境变量测试用唯一前缀 DEVNPC_TEST_ENV_ 避免并行污染
    const PREFIX: &str = "DEVNPC_TEST_ENV_";

    fn set_var(key: &str, val: &str) {
        // safety: 测试串行或用唯一 key
        std::env::set_var(key, val);
    }

    fn remove_var(key: &str) {
        std::env::remove_var(key);
    }

    #[test]
    fn get_required_returns_value_when_set() {
        let key = format!("{PREFIX}REQUIRED_SET");
        set_var(&key, "abc");
        assert_eq!(get_required(&key).unwrap(), "abc");
        remove_var(&key);
    }

    #[test]
    fn get_required_returns_error_when_missing() {
        let key = format!("{PREFIX}REQUIRED_MISSING");
        remove_var(&key);
        let err = get_required(&key).unwrap_err();
        assert!(matches!(err, DevnpcError::MissingEnv { .. }));
    }

    #[test]
    fn get_or_default_returns_default_when_missing() {
        let key = format!("{PREFIX}DEFAULT_MISSING");
        remove_var(&key);
        assert_eq!(get_or_default(&key, "fallback"), "fallback");
    }

    #[test]
    fn get_u32_parses_valid() {
        let key = format!("{PREFIX}U32_VALID");
        set_var(&key, "42");
        assert_eq!(get_u32(&key).unwrap(), Some(42));
        remove_var(&key);
    }

    #[test]
    fn get_u32_returns_none_when_missing() {
        let key = format!("{PREFIX}U32_MISSING");
        remove_var(&key);
        assert_eq!(get_u32(&key).unwrap(), None);
    }

    #[test]
    fn get_u32_returns_error_on_invalid() {
        let key = format!("{PREFIX}U32_INVALID");
        set_var(&key, "not-a-number");
        assert!(get_u32(&key).is_err());
        remove_var(&key);
    }

    #[test]
    fn get_u8_parses_valid() {
        let key = format!("{PREFIX}U8_VALID");
        set_var(&key, "5");
        assert_eq!(get_u8(&key).unwrap(), Some(5));
        remove_var(&key);
    }

    #[test]
    fn get_sop_mode_parses_soft() {
        let key = format!("{PREFIX}SOP_SOFT");
        set_var(&key, "soft");
        assert_eq!(get_sop_mode(&key).unwrap(), Some(SopMode::Soft));
        remove_var(&key);
    }

    #[test]
    fn get_sop_mode_parses_strict() {
        let key = format!("{PREFIX}SOP_STRICT");
        set_var(&key, "strict");
        assert_eq!(get_sop_mode(&key).unwrap(), Some(SopMode::Strict));
        remove_var(&key);
    }

    #[test]
    fn get_sop_mode_returns_error_on_invalid() {
        let key = format!("{PREFIX}SOP_INVALID");
        set_var(&key, "loud");
        assert!(get_sop_mode(&key).is_err());
        remove_var(&key);
    }

    #[test]
    fn get_report_target_parses_pages() {
        let key = format!("{PREFIX}RT_PAGES");
        set_var(&key, "pages");
        assert_eq!(
            get_report_target(&key).unwrap(),
            Some(crate::config::ReportTarget::Pages)
        );
        remove_var(&key);
    }
}
```

- [ ] **Step 2: 运行测试验证**

Run: `cargo test --lib config::env`
Expected: 11 个测试 PASS。

- [ ] **Step 3: 提交**

```bash
git add src/config/env.rs
git commit -m "feat: 环境变量类型解析 (u32/u8/SopMode/ReportTarget)"
```

---

### Task 4: ProjectConfig 加 max_ci_retries 字段 + 从 front matter 转换

**Files:**
- Modify: `src/config/mod.rs`

- [ ] **Step 1: 在 ProjectConfig 加 max_ci_retries 字段**

Modify `src/config/mod.rs` 的 `ProjectConfig` struct,在 `branch_prefix` 之后、`guidelines_markdown` 之前添加字段:

```rust
/// .devnpc.md 解析结果
#[derive(Debug, Clone, Default)]
pub struct ProjectConfig {
    pub sop_mode: SopMode,
    pub forbidden_paths: Vec<String>,
    pub required_checks: Vec<String>,
    pub branch_prefix: String,
    pub max_ci_retries: Option<u8>,
    pub guidelines_markdown: String,
}
```

注: `max_ci_retries: Option<u8>` 用 None 表示"未在 .devnpc.md 指定,用环境变量或默认"。

- [ ] **Step 2: 验证编译 (Default 仍可用)**

Run: `cargo build`
Expected: 编译成功 (`Option<u8>` 默认 None)。

- [ ] **Step 3: 提交**

```bash
git add src/config/mod.rs
git commit -m "feat: ProjectConfig 加 max_ci_retries 字段"
```

---

### Task 5: 配置加载器 (loader.rs) - 三层合并

**Files:**
- Modify: `src/config/loader.rs`
- Modify: `src/config/mod.rs`

- [ ] **Step 1: 写失败测试 - Config::load 合并逻辑**

先在 `src/config/loader.rs` 末尾追加测试模块 (Step 2 会实现 `load_internal` 使其通过):

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{SopMode, ReportTarget};
    use std::path::PathBuf;

    /// 构造一个临时 .devnpc.md 文件并返回其路径
    ///
    /// 用 `into_path()` 消费 TempDir,避免析构删除文件 (测试用,允许泄漏)。
    fn write_devnpc_md(content: &str) -> PathBuf {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join(".devnpc.md");
        std::fs::write(&file_path, content).unwrap();
        let dir_path = dir.into_path(); // 消费 TempDir,保留目录
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
```

- [ ] **Step 2: 实现 load_internal 与合并逻辑**

Replace `src/config/loader.rs` 的占位内容 (保留 Step 1 的 tests 模块),在文件顶部添加实现:

```rust
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
    let project_id: u64 = project_id_str
        .parse()
        .map_err(|_| DevnpcError::Config(format!("环境变量 {project_id_var} 不是有效 u64: {project_id_str}")))?;

    // 2. .devnpc.md
    let parsed_md = read_devnpc_md(devnpc_md_path)?;
    let mut project = project_config_from_front_matter(&parsed_md.front_matter, parsed_md.guidelines_markdown);

    // 3. 可选环境变量 (覆盖 .devnpc.md)
    let max_iterations = env::get_u32(max_iter_var)?.unwrap_or(20);
    let max_ci_retries = match env::get_u8(max_ci_var)? {
        Some(v) => v,
        None => project.max_ci_retries.unwrap_or(3),
    };
    if let Some(mode) = env::get_sop_mode(sop_mode_var)? {
        project.sop_mode = mode;
    }
    let report_target = env::get_report_target(report_target_var)?
        .unwrap_or(ReportTarget::Artifact);

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
```

- [ ] **Step 3: 修改 Config::load 调用 loader**

Modify `src/config/mod.rs` 的 `Config::load()`,替换占位实现:

```rust
impl Config {
    /// 加载配置 (env + .devnpc.md + 默认值三层合并)
    pub fn load() -> Result<Self> {
        loader::load()
    }
}
```

- [ ] **Step 4: 运行测试验证**

Run: `cargo test --lib config::loader`
Expected: 4 个测试 PASS。

注: 测试用唯一环境变量前缀 `DEVNPC_TEST_*` 避免并行污染。若 CI 报并行冲突,改为 `cargo test --lib config::loader -- --test-threads=1`。

- [ ] **Step 5: 提交**

```bash
git add src/config/loader.rs src/config/mod.rs
git commit -m "feat: 配置加载器三层合并 (env > .devnpc.md > 默认)"
```

---

### Task 6: GitlabClient - get_issue 实现

**Files:**
- Modify: `src/gitlab_api/client.rs`

- [ ] **Step 1: 写失败测试 - get_issue (wiremock)**

在 `src/gitlab_api/client.rs` 末尾追加测试模块:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::gitlab_api::GitlabApi;
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn client_for(server: &MockServer) -> GitlabClient {
        GitlabClient::new(server.uri(), "test-token")
    }

    #[tokio::test]
    async fn get_issue_returns_parsed_issue() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v4/projects/1/issues/42"))
            .and(header("PRIVATE-TOKEN", "test-token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "iid": 42,
                "title": "登录 bug",
                "description": "无法登录",
                "state": "opened",
                "web_url": "https://gitlab.test.com/issues/42"
            })))
            .mount(&server)
            .await;

        let client = client_for(&server);
        let issue = client.get_issue(1, 42).await.unwrap();
        assert_eq!(issue.iid, 42);
        assert_eq!(issue.title, "登录 bug");
        assert_eq!(issue.description.as_deref(), Some("无法登录"));
        assert_eq!(issue.state, "opened");
    }

    #[tokio::test]
    async fn get_issue_returns_not_found_error_on_404() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v4/projects/1/issues/999"))
            .respond_with(ResponseTemplate::new(404).set_body_string("404 Not Found"))
            .mount(&server)
            .await;

        let client = client_for(&server);
        let result = client.get_issue(1, 999).await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(matches!(err, crate::error::DevnpcError::GitlabApi { status: 404, .. }));
    }
}
```

- [ ] **Step 2: 实现 GitlabClient 内部 get/post 封装 + get_issue**

Replace `src/gitlab_api/client.rs` 完整内容为:

```rust
//! GitLab HTTP 客户端 (reqwest 实现 GitlabApi trait)
//!
//! 统一封装 GET/POST,处理状态码与错误。

use async_trait::async_trait;
use reqwest::StatusCode;

use crate::error::{DevnpcError, Result};

use super::{CreateMrReq, GitlabApi, Issue, MergeRequest, Note, Pipeline};

/// reqwest 实现
pub struct GitlabClient {
    base_url: String,
    token: String,
    http: reqwest::Client,
}

impl GitlabClient {
    pub fn new(base_url: impl Into<String>, token: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into().trim_end_matches('/').to_string(),
            token: token.into(),
            http: reqwest::Client::new(),
        }
    }

    /// 发 GET 请求,返回反序列化的 JSON。
    /// 404 返回 GitlabNotFound,其他非 2xx 返回 GitlabApi。
    async fn get<T: serde::de::DeserializeOwned>(&self, url: &str) -> Result<T> {
        let resp = self
            .http
            .get(url)
            .header("PRIVATE-TOKEN", &self.token)
            .send()
            .await?;
        let status = resp.status();
        if status == StatusCode::NOT_FOUND {
            let body = resp.text().await.unwrap_or_default();
            return Err(DevnpcError::GitlabNotFound { resource: body });
        }
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(DevnpcError::GitlabApi {
                status: status.as_u16(),
                body,
            });
        }
        Ok(resp.json::<T>().await?)
    }

    /// 发 POST 请求,返回反序列化的 JSON。
    async fn post<T: serde::de::DeserializeOwned>(
        &self,
        url: &str,
        form: &[(&str, &str)],
    ) -> Result<T> {
        let resp = self
            .http
            .post(url)
            .header("PRIVATE-TOKEN", &self.token)
            .form(form)
            .send()
            .await?;
        let status = resp.status();
        if status == StatusCode::NOT_FOUND {
            let body = resp.text().await.unwrap_or_default();
            return Err(DevnpcError::GitlabNotFound { resource: body });
        }
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(DevnpcError::GitlabApi {
                status: status.as_u16(),
                body,
            });
        }
        Ok(resp.json::<T>().await?)
    }

    fn issue_url(&self, project_id: u64, iid: u64) -> String {
        format!("{}/api/v4/projects/{}/issues/{}", self.base_url, project_id, iid)
    }

    fn mr_url(&self, project_id: u64, iid: u64) -> String {
        format!("{}/api/v4/projects/{}/merge_requests/{}", self.base_url, project_id, iid)
    }

    fn mr_notes_url(&self, project_id: u64, mr_iid: u64) -> String {
        format!(
            "{}/api/v4/projects/{}/merge_requests/{}/notes",
            self.base_url, project_id, mr_iid
        )
    }

    fn issue_notes_url(&self, project_id: u64, iid: u64) -> String {
        format!(
            "{}/api/v4/projects/{}/issues/{}/notes",
            self.base_url, project_id, iid
        )
    }

    fn pipelines_url(&self, project_id: u64) -> String {
        format!("{}/api/v4/projects/{}/pipelines", self.base_url, project_id)
    }
}

#[async_trait]
impl GitlabApi for GitlabClient {
    async fn get_issue(&self, project_id: u64, iid: u64) -> Result<Issue> {
        let url = self.issue_url(project_id, iid);
        self.get(&url).await
    }

    async fn get_mr(&self, project_id: u64, iid: u64) -> Result<MergeRequest> {
        let url = self.mr_url(project_id, iid);
        self.get(&url).await
    }

    async fn create_mr(&self, project_id: u64, req: CreateMrReq) -> Result<MergeRequest> {
        let url = format!("{}/api/v4/projects/{}/merge_requests", self.base_url, project_id);
        let draft_str = if req.draft { "true" } else { "false" };
        // GitLab MR API: title 前缀 "Draft: " 表草稿,这里简化用 title 直传
        let title = if req.draft {
            format!("Draft: {}", req.title)
        } else {
            req.title.clone()
        };
        self.post(&url, &[
            ("source_branch", &req.source_branch),
            ("target_branch", &req.target_branch),
            ("title", &title),
            ("description", &req.description),
        ])
        .await
    }

    async fn get_pipelines(&self, project_id: u64) -> Result<Vec<Pipeline>> {
        let url = self.pipelines_url(project_id);
        self.get(&url).await
    }

    async fn get_issue_notes(&self, project_id: u64, iid: u64) -> Result<Vec<Note>> {
        let url = self.issue_notes_url(project_id, iid);
        self.get(&url).await
    }

    async fn get_mr_notes(&self, project_id: u64, mr_iid: u64) -> Result<Vec<Note>> {
        let url = self.mr_notes_url(project_id, mr_iid);
        self.get(&url).await
    }

    async fn create_mr_note(&self, project_id: u64, mr_iid: u64, body: &str) -> Result<Note> {
        let url = self.mr_notes_url(project_id, mr_iid);
        self.post(&url, &[("body", body)]).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gitlab_api::GitlabApi;
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn client_for(server: &MockServer) -> GitlabClient {
        GitlabClient::new(server.uri(), "test-token")
    }

    #[tokio::test]
    async fn get_issue_returns_parsed_issue() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v4/projects/1/issues/42"))
            .and(header("PRIVATE-TOKEN", "test-token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "iid": 42,
                "title": "登录 bug",
                "description": "无法登录",
                "state": "opened",
                "web_url": "https://gitlab.test.com/issues/42"
            })))
            .mount(&server)
            .await;

        let client = client_for(&server);
        let issue = client.get_issue(1, 42).await.unwrap();
        assert_eq!(issue.iid, 42);
        assert_eq!(issue.title, "登录 bug");
        assert_eq!(issue.description.as_deref(), Some("无法登录"));
        assert_eq!(issue.state, "opened");
    }

    #[tokio::test]
    async fn get_issue_returns_not_found_error_on_404() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v4/projects/1/issues/999"))
            .respond_with(ResponseTemplate::new(404).set_body_string("404 Not Found"))
            .mount(&server)
            .await;

        let client = client_for(&server);
        let result = client.get_issue(1, 999).await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(matches!(err, crate::error::DevnpcError::GitlabApi { status: 404, .. }));
    }
}
```

- [ ] **Step 3: 运行测试验证**

Run: `cargo test --lib gitlab_api::client`
Expected: 2 个测试 PASS。

- [ ] **Step 4: 提交**

```bash
git add src/gitlab_api/client.rs
git commit -m "feat: GitlabClient get_issue 实现 (reqwest + wiremock 测试)"
```

---

### Task 7: GitlabClient - get_mr 与 create_mr 测试

**Files:**
- Modify: `src/gitlab_api/client.rs` (仅测试模块追加)

- [ ] **Step 1: 追加 get_mr 与 create_mr 测试**

在 `src/gitlab_api/client.rs` 的 `mod tests` 内追加 (在最后一个 `}` 之前):

```rust
    #[tokio::test]
    async fn get_mr_returns_parsed_mr() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v4/projects/1/merge_requests/7"))
            .and(header("PRIVATE-TOKEN", "test-token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "iid": 7,
                "title": "feat: add login",
                "description": "实现登录",
                "state": "opened",
                "source_branch": "npc/1-login",
                "target_branch": "main",
                "web_url": "https://gitlab.test.com/mrs/7",
                "draft": false,
                "work_in_progress": false
            })))
            .mount(&server)
            .await;

        let client = client_for(&server);
        let mr = client.get_mr(1, 7).await.unwrap();
        assert_eq!(mr.iid, 7);
        assert_eq!(mr.source_branch, "npc/1-login");
        assert_eq!(mr.target_branch, "main");
        assert!(!mr.draft);
    }

    #[tokio::test]
    async fn create_mr_posts_form_and_returns_mr() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/v4/projects/1/merge_requests"))
            .and(header("PRIVATE-TOKEN", "test-token"))
            .and(wiremock::matchers::body_string_contains("source_branch=npc%2F1-login"))
            .and(wiremock::matchers::body_string_contains("target_branch=main"))
            .respond_with(ResponseTemplate::new(201).set_body_json(serde_json::json!({
                "iid": 8,
                "title": "Draft: feat: login",
                "description": "desc",
                "state": "opened",
                "source_branch": "npc/1-login",
                "target_branch": "main",
                "web_url": "https://gitlab.test.com/mrs/8",
                "draft": true,
                "work_in_progress": true
            })))
            .mount(&server)
            .await;

        let client = client_for(&server);
        let req = CreateMrReq {
            source_branch: "npc/1-login".into(),
            target_branch: "main".into(),
            title: "feat: login".into(),
            description: "desc".into(),
            draft: true,
        };
        let mr = client.create_mr(1, req).await.unwrap();
        assert_eq!(mr.iid, 8);
        assert!(mr.draft);
    }

    #[tokio::test]
    async fn get_mr_returns_api_error_on_500() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v4/projects/1/merge_requests/1"))
            .respond_with(ResponseTemplate::new(500).set_body_string("server error"))
            .mount(&server)
            .await;

        let client = client_for(&server);
        let err = client.get_mr(1, 1).await.unwrap_err();
        assert!(matches!(err, crate::error::DevnpcError::GitlabApi { status: 500, .. }));
    }
```

- [ ] **Step 2: 运行测试验证**

Run: `cargo test --lib gitlab_api::client`
Expected: 5 个测试 PASS (原 2 + 新 3)。

- [ ] **Step 3: 提交**

```bash
git add src/gitlab_api/client.rs
git commit -m "test: GitlabClient get_mr/create_mr 测试 (wiremock)"
```

---

### Task 8: GitlabClient - pipelines 与 notes 测试

**Files:**
- Modify: `src/gitlab_api/client.rs` (仅测试模块追加)

- [ ] **Step 1: 追加 pipelines/notes 测试**

在 `src/gitlab_api/client.rs` 的 `mod tests` 内追加:

```rust
    #[tokio::test]
    async fn get_pipelines_returns_list() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v4/projects/1/pipelines"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
                { "id": 100, "status": "success", "ref": "main", "sha": "abc123", "web_url": "https://gl.test/p/100" },
                { "id": 101, "status": "failed", "ref": "npc/1-x", "sha": "def456", "web_url": "https://gl.test/p/101" }
            ])))
            .mount(&server)
            .await;

        let client = client_for(&server);
        let pipelines = client.get_pipelines(1).await.unwrap();
        assert_eq!(pipelines.len(), 2);
        assert_eq!(pipelines[0].id, 100);
        assert_eq!(pipelines[0].status, "success");
        assert_eq!(pipelines[0].ref_.as_deref(), Some("main"));
        assert_eq!(pipelines[1].status, "failed");
    }

    #[tokio::test]
    async fn get_issue_notes_returns_list() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v4/projects/1/issues/42/notes"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
                {
                    "id": 1,
                    "body": "@devnpc 修复登录",
                    "author": { "id": 10, "username": "alice", "name": "Alice" },
                    "created_at": "2026-08-01T10:00:00Z"
                }
            ])))
            .mount(&server)
            .await;

        let client = client_for(&server);
        let notes = client.get_issue_notes(1, 42).await.unwrap();
        assert_eq!(notes.len(), 1);
        assert_eq!(notes[0].body, "@devnpc 修复登录");
        assert_eq!(notes[0].author.username, "alice");
    }

    #[tokio::test]
    async fn get_mr_notes_returns_list() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v4/projects/1/merge_requests/7/notes"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
                {
                    "id": 5,
                    "body": "CI 通过",
                    "author": { "id": 11, "username": "bob", "name": "Bob" },
                    "created_at": "2026-08-01T11:00:00Z"
                }
            ])))
            .mount(&server)
            .await;

        let client = client_for(&server);
        let notes = client.get_mr_notes(1, 7).await.unwrap();
        assert_eq!(notes[0].author.name, "Bob");
    }

    #[tokio::test]
    async fn create_mr_note_posts_body_and_returns_note() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/v4/projects/1/merge_requests/7/notes"))
            .and(wiremock::matchers::body_string_contains("body=CI+%E9%80%9A%E8%BF%87"))
            .respond_with(ResponseTemplate::new(201).set_body_json(serde_json::json!({
                "id": 9,
                "body": "CI 通过",
                "author": { "id": 99, "username": "devnpc", "name": "devnpc bot" },
                "created_at": "2026-08-01T12:00:00Z"
            })))
            .mount(&server)
            .await;

        let client = client_for(&server);
        let note = client.create_mr_note(1, 7, "CI 通过").await.unwrap();
        assert_eq!(note.id, 9);
        assert_eq!(note.author.username, "devnpc");
    }

    #[tokio::test]
    async fn get_pipelines_returns_not_found_on_404() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v4/projects/999/pipelines"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;

        let client = client_for(&server);
        let err = client.get_pipelines(999).await.unwrap_err();
        assert!(matches!(err, crate::error::DevnpcError::GitlabNotFound { .. }));
    }
```

- [ ] **Step 2: 运行测试验证**

Run: `cargo test --lib gitlab_api::client`
Expected: 10 个测试 PASS (原 5 + 新 5)。

- [ ] **Step 3: 提交**

```bash
git add src/gitlab_api/client.rs
git commit -m "test: GitlabClient pipelines/notes 测试 (wiremock)"
```

---

### Task 9: CLI config 命令接入真实 Config::load

**Files:**
- Modify: `src/main.rs`

- [ ] **Step 1: 修改 print_config 调用 Config::load**

Modify `src/main.rs` 的 `print_config` 函数,替换占位:

```rust
fn print_config() -> Result<()> {
    match devnpc::config::Config::load() {
        Ok(config) => {
            println!("=== devnpc 配置 ===");
            println!("LLM:");
            println!("  base_url: {}", config.llm.base_url);
            println!("  model: {}", config.llm.model);
            println!("  api_key: {}***", &config.llm.api_key.chars().take(4).collect::<String>());
            println!("GitLab:");
            println!("  url: {}", config.gitlab.url);
            println!("  project_id: {}", config.gitlab.project_id);
            println!("Limits:");
            println!("  max_iterations: {}", config.limits.max_iterations);
            println!("  max_ci_retries: {}", config.limits.max_ci_retries);
            println!("Project:");
            println!("  sop_mode: {:?}", config.project.sop_mode);
            println!("  branch_prefix: {}", config.project.branch_prefix);
            println!("  forbidden_paths: {:?}", config.project.forbidden_paths);
            println!("  required_checks: {:?}", config.project.required_checks);
            println!("  guidelines_markdown_len: {}", config.project.guidelines_markdown.len());
            println!("Report:");
            println!("  target: {:?}", config.report.target);
            Ok(())
        }
        Err(e) => {
            eprintln!("配置加载失败: {e}");
            Err(e)
        }
    }
}
```

- [ ] **Step 2: 验证编译**

Run: `cargo build`
Expected: 编译成功。

- [ ] **Step 3: 手动验证 config 命令**

设测试环境变量并运行 (PowerShell):

```powershell
$env:DEVNPC_API_KEY="sk-test1234567890"; $env:DEVNPC_BASE_URL="https://api.deepseek.com/v1"; $env:DEVNPC_MODEL="deepseek-chat"; $env:GITLAB_URL="https://gitlab.test.com"; $env:GITLAB_TOKEN="gl-token"; $env:CI_PROJECT_ID="42"; cargo run --quiet -- config
```

Expected: 打印配置摘要,`api_key: sk-t***` 脱敏,`project_id: 42`。

- [ ] **Step 4: 提交**

```bash
git add src/main.rs
git commit -m "feat: CLI config 命令接入 Config::load"
```

---

### Task 10: 全量测试 + clippy + 最终验收

**Files:**
- 无新文件

- [ ] **Step 1: 全量编译与测试**

Run:

```bash
cargo build
cargo test
cargo build --release
```

Expected:
- `cargo build`: 成功
- `cargo test`: 全部 PASS (P0 的 13 + P1 新增约 25 = 约 38 个测试)
- `cargo build --release`: 成功

- [ ] **Step 2: 运行 clippy**

Run: `cargo clippy -- -D warnings`
Expected: 无 warning。若有 dead_code warning (如未用的 URL 构造方法),加 `#[allow(dead_code)]` 或删除。

- [ ] **Step 3: P1 验收检查**

逐项确认:

- [ ] `cargo build` 成功
- [ ] `cargo test` 全部 PASS
- [ ] `cargo clippy -- -D warnings` 无 warning
- [ ] `cargo run -- config` 在设环境变量后正常打印配置
- [ ] `Config::load()` 三层合并逻辑有测试覆盖 (env 覆盖 .devnpc.md 覆盖默认)
- [ ] `.devnpc.md` 解析有 6 个测试 (front matter/正文/边界)
- [ ] `GitlabClient` 7 个 API 方法均有 wiremock 测试
- [ ] 404 返回 `GitlabNotFound`,其他非 2xx 返回 `GitlabApi`
- [ ] git 历史清晰 (约 9 个新提交)

- [ ] **Step 4: 提交验收记录 (可选)**

若有 clippy 修复,提交:

```bash
git add -A
git commit -m "chore: P1 验收 - clippy 修复"
```

---

## P1 完成后的 Next Steps

P1 (配置系统 + GitLab API 客户端) 完成后,为 **P2 (研发记忆 + Git 操作)** 编写下一份计划。P2 将:

1. 实现 `GitOps` (clone/checkout_branch/commit/push/recent_commits) 调用系统 git 命令
2. 实现 `Context::build()` 并行聚合 (Issue + 相关 PR + 评论 + 仓库目录树 + CI 历史)
3. 实现 `repo_index` (目录树构建 + 关键文件选择)
4. 验收: `devnpc run --task "@devnpc 修复 #42"` 能加载配置 → 连 GitLab 取 Issue/PR → 聚合 Context (P2 暂不调 LLM,只打印 Context 摘要)

每个后续阶段在上一阶段完成后编写计划,确保基于实际代码、精确可执行。
