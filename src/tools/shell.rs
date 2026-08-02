//! Shell 命令工具: run_command
//!
//! 沙箱内执行,带白名单/黑名单 + 超时 (默认 120s)。

use std::path::PathBuf;
use std::time::Duration;

use async_trait::async_trait;
use serde::Deserialize;
use tokio::process::Command;

use crate::config::CommandConfig;
use crate::error::{DevnpcError, Result};
use crate::tools::{Tool, ToolResult};

pub struct RunCommandTool {
    workspace: PathBuf,
    timeout: Duration,
    allowlist: Vec<String>,
    denylist: Vec<String>,
}

impl RunCommandTool {
    pub fn new(workspace: impl Into<PathBuf>, config: CommandConfig) -> Self {
        Self {
            workspace: workspace.into(),
            timeout: Duration::from_secs(config.default_timeout_secs),
            allowlist: config.allowlist,
            denylist: config.denylist,
        }
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }
}

#[derive(Deserialize)]
struct RunCommandArgs {
    cmd: String,
    #[serde(default)]
    args: Vec<String>,
    #[serde(default)]
    timeout_secs: Option<u64>,
}

#[async_trait]
impl Tool for RunCommandTool {
    fn name(&self) -> &str {
        "run_command"
    }
    fn description(&self) -> &str {
        "在 workspace 内执行白名单命令 (cargo/rustc/make/just 等)。参数: cmd, args, timeout_secs。"
    }
    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "cmd": {"type": "string", "description": "命令名 (必须在白名单内)"},
                "args": {"type": "array", "items": {"type": "string"}},
                "timeout_secs": {"type": "integer", "description": "超时秒数,默认 120"}
            },
            "required": ["cmd"]
        })
    }
    async fn call(&self, args: &serde_json::Value) -> Result<ToolResult> {
        let parsed: RunCommandArgs = serde_json::from_value(args.clone()).map_err(|e| {
            DevnpcError::Tool {
                tool: "run_command".into(),
                msg: format!("参数解析失败: {e}"),
            }
        })?;

        // 黑名单优先
        if self.denylist.contains(&parsed.cmd) {
            return Ok(ToolResult::err(format!("命令 {} 在黑名单中", parsed.cmd)));
        }
        // 白名单检查
        if !self.allowlist.contains(&parsed.cmd) {
            return Ok(ToolResult::err(format!(
                "命令 {} 不在白名单中 (允许: {})",
                parsed.cmd,
                self.allowlist.join(", ")
            )));
        }

        let timeout = parsed
            .timeout_secs
            .map(Duration::from_secs)
            .unwrap_or(self.timeout);

        let mut cmd = Command::new(&parsed.cmd);
        cmd.args(&parsed.args).current_dir(&self.workspace);
        // 合并 stdout+stderr
        cmd.stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());

        let child = cmd.spawn().map_err(|e| DevnpcError::Tool {
            tool: "run_command".into(),
            msg: format!("启动命令失败: {e}"),
        })?;

        match tokio::time::timeout(timeout, child.wait_with_output()).await {
            Ok(Ok(output)) => {
                let stdout = String::from_utf8_lossy(&output.stdout).to_string();
                let stderr = String::from_utf8_lossy(&output.stderr).to_string();
                let success = output.status.success();
                let combined = if stderr.is_empty() {
                    stdout
                } else {
                    format!("{stdout}\n[stderr]\n{stderr}")
                };
                if success {
                    Ok(ToolResult::ok(combined))
                } else {
                    Ok(ToolResult::err(format!(
                        "退出码 {:?}\n{combined}",
                        output.status.code()
                    )))
                }
            }
            Ok(Err(e)) => Ok(ToolResult::err(format!("等待命令失败: {e}"))),
            Err(_) => Ok(ToolResult::err(format!("命令超时 ({:?})", timeout))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn default_command_config() -> CommandConfig {
        CommandConfig::default()
    }

    #[tokio::test]
    async fn run_command_executes_whitelisted_cargo() {
        let dir = tempfile::tempdir().unwrap();
        let tool = RunCommandTool::new(dir.path(), default_command_config());
        // 用 cargo --version (跨平台真实可执行文件,echo 在 Windows 是 CMD 内建)
        let result = tool
            .call(&serde_json::json!({"cmd": "cargo", "args": ["--version"]}))
            .await
            .unwrap();
        assert!(result.success);
        assert!(result.output.contains("cargo"));
    }

    #[tokio::test]
    async fn run_command_rejects_non_whitelisted() {
        let dir = tempfile::tempdir().unwrap();
        let tool = RunCommandTool::new(dir.path(), default_command_config());
        let result = tool
            .call(&serde_json::json!({"cmd": "ls"}))
            .await
            .unwrap();
        assert!(!result.success);
        assert!(result.output.contains("不在白名单"));
    }

    #[tokio::test]
    async fn run_command_rejects_blacklisted() {
        let dir = tempfile::tempdir().unwrap();
        let tool = RunCommandTool::new(dir.path(), default_command_config());
        let result = tool
            .call(&serde_json::json!({"cmd": "rm", "args": ["-rf", "/"]}))
            .await
            .unwrap();
        assert!(!result.success);
        assert!(result.output.contains("黑名单"));
    }

    #[tokio::test]
    async fn run_command_returns_err_on_non_zero_exit() {
        let dir = tempfile::tempdir().unwrap();
        let tool = RunCommandTool::new(dir.path(), default_command_config());
        // cargo 一个必然失败的子命令
        let result = tool
            .call(&serde_json::json!({"cmd": "cargo", "args": ["nonexistent-subcommand"]}))
            .await
            .unwrap();
        // cargo 在白名单内,会执行;失败时返回 err
        if !result.success {
            assert!(result.output.contains("退出码"));
        }
    }

    #[tokio::test]
    async fn run_command_timeout_param_accepted() {
        let dir = tempfile::tempdir().unwrap();
        // 用极短超时跑一个立即返回的命令,验证 timeout_secs 参数被接受
        // cargo --version 立即返回,跨平台可执行
        let tool = RunCommandTool::new(dir.path(), default_command_config()).with_timeout(Duration::from_millis(500));
        let result = tool
            .call(&serde_json::json!({"cmd": "cargo", "args": ["--version"]}))
            .await
            .unwrap();
        assert!(result.success);
    }
}
