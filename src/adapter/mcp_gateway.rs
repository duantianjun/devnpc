//! MCP Gateway: 管理 MCP 客户端连接
//!
//! 利用 adk-rust 的 mcp feature，建立统一的 MCP 协议入口。
//! 支持 stdio 和 streamable HTTP 两种传输方式。

use std::collections::HashMap;
use std::sync::Arc;

use adk_rust::Tool;
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
}

impl McpGateway {
    pub fn new(config: McpConfig) -> Self {
        Self {
            config,
            servers: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// 注册 MCP 服务器描述
    pub async fn register_server(&self, desc: McpServerDesc) {
        let mut servers = self.servers.write().await;
        servers.insert(desc.name.clone(), desc);
    }

    /// 收集所有 MCP 工具 (通过 adk-rust 的 MCP 客户端连接)
    ///
    /// 注意: adk-rust 的 MCP 客户端需要根据实际 API 调整。
    /// 当前返回空列表，后续根据 adk-rust mcp feature 实现具体连接。
    pub async fn collect_mcp_tools(&self) -> Result<Vec<Arc<dyn Tool>>> {
        if !self.config.enabled {
            return Ok(Vec::new());
        }

        let servers = self.servers.read().await;
        let tools: Vec<Arc<dyn Tool>> = Vec::new();

        for (_name, desc) in servers.iter() {
            match desc.transport.as_str() {
                "stdio" => {
                    if let Some(cmd) = &desc.command {
                        tracing::info!(server = %desc.name, cmd = %cmd, "注册 stdio MCP 服务器");
                        // TODO: 根据 adk-rust MCP API 实现 StdioMcpClient 连接
                        let _ = cmd;
                    }
                }
                "http" => {
                    if let Some(url) = &desc.url {
                        tracing::info!(server = %desc.name, url = %url, "注册 HTTP MCP 服务器");
                        // TODO: 根据 adk-rust MCP API 实现 HttpMcpClient 连接
                        let _ = url;
                    }
                }
                _ => {
                    tracing::warn!(transport = %desc.transport, "不支持的 MCP 传输方式");
                }
            }
        }

        Ok(tools)
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