//! MCP Gateway: 管理 MCP 客户端连接
//!
//! 利用 adk-rust 的 mcp feature，建立统一的 MCP 协议入口。
//! 支持 stdio 传输方式，通过 rmcp 和 adk-rust 的 McpToolset 连接 MCP 服务器。

use std::collections::HashMap;
use std::sync::Arc;

use adk_rust::tool::McpToolset;
use adk_rust::Toolset;
use rmcp::{ServiceExt, transport::TokioChildProcess};
use tokio::process::Command;
use tokio::sync::RwLock;

use crate::config::McpConfig;
use crate::error::Result;

/// MCP 服务器描述
#[derive(Debug, Clone)]
pub struct McpServerDesc {
    pub name: String,
    /// "stdio" 或 "http"
    pub transport: String,
    pub command: Option<String>,
    pub args: Vec<String>,
    pub url: Option<String>,
}

/// MCP Gateway: 管理多个 MCP 服务器连接
pub struct McpGateway {
    config: McpConfig,
    servers: Arc<RwLock<HashMap<String, McpServerDesc>>>,
    /// 已连接的 McpToolset 工具集
    toolsets: Arc<RwLock<Vec<Arc<dyn Toolset>>>>,
}

impl McpGateway {
    pub fn new(config: McpConfig) -> Self {
        Self {
            config,
            servers: Arc::new(RwLock::new(HashMap::new())),
            toolsets: Arc::new(RwLock::new(Vec::new())),
        }
    }

    /// 注册 MCP 服务器描述
    pub async fn register_server(&self, desc: McpServerDesc) {
        let mut servers = self.servers.write().await;
        servers.insert(desc.name.clone(), desc);
    }

    /// 连接所有已注册的 MCP 服务器，收集工具集
    ///
    /// 连接成功后，工具集可通过 `take_toolsets()` 获取并添加到 Agent。
    pub async fn connect_all(&self) -> Result<()> {
        if !self.config.enabled {
            return Ok(());
        }

        let servers = self.servers.read().await;
        let mut toolsets = self.toolsets.write().await;

        for (name, desc) in servers.iter() {
            match desc.transport.as_str() {
                "stdio" => {
                    if let Some(cmd) = &desc.command {
                        match Self::connect_stdio(name, cmd, &desc.args).await {
                            Ok(toolset) => {
                                tracing::info!(server = %name, "MCP 服务器连接成功");
                                toolsets.push(toolset as Arc<dyn Toolset>);
                            }
                            Err(e) => {
                                tracing::warn!(server = %name, error = %e, "MCP 服务器连接失败");
                            }
                        }
                    }
                }
                "http" => {
                    if let Some(url) = &desc.url {
                        tracing::warn!(server = %name, url = %url, "HTTP MCP 传输暂未实现");
                    }
                }
                _ => {
                    tracing::warn!(transport = %desc.transport, "不支持的 MCP 传输方式");
                }
            }
        }

        Ok(())
    }

    /// 通过 stdio 连接 MCP 服务器，返回 McpToolset
    async fn connect_stdio(name: &str, cmd: &str, args: &[String]) -> Result<Arc<McpToolset>> {
        let mut command = Command::new(cmd);
        command.args(args);

        let transport = TokioChildProcess::new(command).map_err(|e| {
            crate::error::DevnpcError::Config(format!("MCP 传输创建失败 ({name}): {e}"))
        })?;

        let client = ().serve(transport).await.map_err(|e| {
            crate::error::DevnpcError::Config(format!("MCP 客户端连接失败 ({name}): {e}"))
        })?;

        let toolset = McpToolset::new(client);
        Ok(Arc::new(toolset))
    }

    /// 获取已连接的 MCP 工具集
    pub async fn take_toolsets(&self) -> Vec<Arc<dyn Toolset>> {
        let mut toolsets = self.toolsets.write().await;
        std::mem::take(&mut *toolsets)
    }

    /// 启动 codemap 子进程 (stdio 模式)
    pub async fn start_codemap(&self) -> Result<()> {
        if !self.config.enabled {
            return Ok(());
        }

        let cmd = &self.config.codemap_path;
        let data_dir = &self.config.codemap_data_dir;

        tracing::info!(cmd = %cmd, data_dir = %data_dir, "注册 codemap MCP 服务器描述");

        self.register_server(McpServerDesc {
            name: "codemap".to_string(),
            transport: "stdio".to_string(),
            command: Some(cmd.clone()),
            args: vec!["serve".to_string(), "--data-dir".to_string(), data_dir.clone()],
            url: None,
        }).await;

        Ok(())
    }
}