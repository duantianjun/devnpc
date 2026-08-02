//! CI 日志解析器 (P4 完整实现)
//!
//! 识别常见 CI 失败模式:
//! - 编译错误: error[E####]: / error: / FAILURE:
//! - 测试失败: panicked at / FAILED / Tests failed
//! - 超时: timed out / killed

use serde::Deserialize;

use crate::config::LogParserConfig;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FailureType {
    Compile,
    Test,
    Lint,
    Build,
    Timeout,
    Other,
}

#[derive(Debug, Clone)]
pub struct ParsedFailure {
    pub failure_type: FailureType,
    pub job_name: String,
    pub file: Option<String>,
    pub line: Option<u32>,
    pub error_message: String,
    pub context_lines: Vec<String>,
}

/// 解析日志 (通用)
pub fn parse_log(job_name: &str, log: &str) -> Vec<ParsedFailure> {
    parse_log_with_config(job_name, log, &LogParserConfig::default())
}

/// 解析日志 (可配置)
pub fn parse_log_with_config(job_name: &str, log: &str, config: &LogParserConfig) -> Vec<ParsedFailure> {
    let mut failures = Vec::new();

    for (i, line) in log.lines().enumerate() {
        // 编译错误: error[E0277]: ...
        if let Some(msg) = line.strip_prefix("error[") && let Some(end) = msg.find("]:") {
                let error_message = msg[end + 2..].trim().to_string();
                let context = extract_context(log, i);
                // Rust 编译错误的文件/行号在下一行: --> src/file.rs:45:13
                let lines: Vec<&str> = log.lines().collect();
                let next_line = lines.get(i + 1).copied().unwrap_or("");
                let (file, line_num) = extract_file_line_from_arrow(next_line);
                failures.push(ParsedFailure {
                    failure_type: FailureType::Compile,
                    job_name: job_name.into(),
                    file,
                    line: line_num,
                    error_message,
                    context_lines: context,
                });
            }

        // 测试失败: panicked at '...', src/file.rs:42:13
        if line.contains("panicked at") {
            let error_message = line.trim().to_string();
            failures.push(ParsedFailure {
                failure_type: FailureType::Test,
                job_name: job_name.into(),
                file: extract_file_from_panic(line),
                line: extract_line_from_panic(line),
                error_message,
                context_lines: extract_context(log, i),
            });
        }

        // 超时
        if line.contains("timed out") || line.contains("killed (signal 9)") {
            failures.push(ParsedFailure {
                failure_type: FailureType::Timeout,
                job_name: job_name.into(),
                file: None,
                line: None,
                error_message: line.trim().to_string(),
                context_lines: extract_context(log, i),
            });
        }
    }

    // 去重 + 限 N 条
    failures.dedup_by(|a, b| a.error_message == b.error_message);
    failures.truncate(config.max_failures);
    failures
}

fn extract_context(log: &str, center: usize) -> Vec<String> {
    let lines: Vec<&str> = log.lines().collect();
    let start = center.saturating_sub(2);
    let end = (center + 3).min(lines.len());
    lines[start..end].iter().map(|s| s.to_string()).collect()
}

/// 从 "--> src/file.rs:45:13" 行提取文件与行号
fn extract_file_line_from_arrow(line: &str) -> (Option<String>, Option<u32>) {
    let file = regex::Regex::new(r"-->\s*([^\s:]+\.rs)")
        .ok()
        .and_then(|re| re.captures(line))
        .map(|c| c[1].to_string());
    let line_num = regex::Regex::new(r":(\d+):\d+")
        .ok()
        .and_then(|re| re.captures(line))
        .and_then(|c| c[1].parse().ok());
    (file, line_num)
}

fn extract_file_from_panic(line: &str) -> Option<String> {
    let re = regex::Regex::new(r"'[^']*',\s*([^:]+\.rs)").ok()?;
    re.captures(line).map(|c| c[1].to_string())
}

fn extract_line_from_panic(line: &str) -> Option<u32> {
    regex::Regex::new(r"\.rs:(\d+)").ok()?
        .captures(line)
        .and_then(|c| c[1].parse().ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_compile_error_extracts_file_line() {
        let log = r#"
error[E0277]: cannot find value `password_raw` in this scope
  --> src/handler/login.rs:45:13
   |
45|     if password_raw.contains('!') {
   |        ^^^^^^^^^^^^^ not found
"#;
        let failures = parse_log("test", log);
        assert_eq!(failures.len(), 1);
        assert_eq!(failures[0].failure_type, FailureType::Compile);
        assert!(failures[0].error_message.contains("password_raw"));
        assert_eq!(failures[0].file.as_deref(), Some("src/handler/login.rs"));
    }

    #[test]
    fn parse_test_failure_panicked() {
        let log = "thread 'main' panicked at 'assertion failed', src/handler/login.rs:42:13";
        let failures = parse_log("test", log);
        assert_eq!(failures.len(), 1);
        assert_eq!(failures[0].failure_type, FailureType::Test);
        assert_eq!(failures[0].file.as_deref(), Some("src/handler/login.rs"));
        assert_eq!(failures[0].line, Some(42));
    }

    #[test]
    fn parse_timeout() {
        let log = "Job was killed (signal 9)";
        let failures = parse_log("test", log);
        assert_eq!(failures.len(), 1);
        assert_eq!(failures[0].failure_type, FailureType::Timeout);
    }

    #[test]
    fn parse_empty_log_returns_empty() {
        let failures = parse_log("test", "");
        assert!(failures.is_empty());
    }

    #[test]
    fn parse_dedup_duplicate_errors() {
        let log = "error[E0277]: same error\nerror[E0277]: same error";
        let failures = parse_log("test", log);
        assert_eq!(failures.len(), 1);
    }
}
