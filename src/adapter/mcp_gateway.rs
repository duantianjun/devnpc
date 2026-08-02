//! MCP Gateway: 管理 MCP 客户端连接
//!
//! 利用 adk-rust 的 mcp feature，建立统一的 MCP 协议入口。
//! 支持两种传输方式:
//! - stdio: 通过子进程启动 MCP 服务器 (如 codemap)
//! - http:  通过 streamable HTTP 连接远程 MCP 服务器 (MCP 2025-06-18 spec)

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use adk_rust::tool::{McpAuth, McpHttpClientBuilder, McpToolset};
use adk_rust::Toolset;
use rmcp::{ServiceExt, transport::TokioChildProcess};
use tokio::process::Command;
use tokio::sync::RwLock;

use crate::config::McpConfig;
use crate::error::Result;

/// MCP 认证配置 (用于 HTTP 传输)
#[derive(Debug, Clone, Default)]
pub struct McpAuthConfig {
    /// Bearer token (Authorization: Bearer <token>)
    pub bearer_token: Option<String>,
    /// 自定义 header 鉴权 (header_name, key)
    pub api_key: Option<(String, String)>,
}

impl McpAuthConfig {
    pub fn is_configured(&self) -> bool {
        self.bearer_token.is_some() || self.api_key.is_some()
    }

    /// 转换为 adk-rust 的 McpAuth
    fn to_mcp_auth(&self) -> McpAuth {
        if let Some(token) = &self.bearer_token {
            return McpAuth::bearer(token.clone());
        }
        if let Some((header, key)) = &self.api_key {
            return McpAuth::api_key(header.clone(), key.clone());
        }
        McpAuth::default()
    }
}

/// MCP 服务器描述
#[derive(Debug, Clone)]
pub struct McpServerDesc {
    pub name: String,
    /// "stdio" 或 "http"
    pub transport: String,
    pub command: Option<String>,
    pub args: Vec<String>,
    pub url: Option<String>,
    /// HTTP 传输的认证配置 (可选)
    pub auth: McpAuthConfig,
    /// HTTP 连接超时 (秒, 默认 30)
    pub timeout_secs: Option<u64>,
}

impl McpServerDesc {
    /// 创建 stdio 类型的服务器描述
    pub fn stdio(name: impl Into<String>, command: impl Into<String>, args: Vec<String>) -> Self {
        Self {
            name: name.into(),
            transport: "stdio".to_string(),
            command: Some(command.into()),
            args,
            url: None,
            auth: McpAuthConfig::default(),
            timeout_secs: None,
        }
    }

    /// 创建 http 类型的服务器描述
    pub fn http(name: impl Into<String>, url: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            transport: "http".to_string(),
            command: None,
            args: Vec::new(),
            url: Some(url.into()),
            auth: McpAuthConfig::default(),
            timeout_secs: None,
        }
    }

    /// 设置 Bearer token 认证
    pub fn with_bearer(mut self, token: impl Into<String>) -> Self {
        self.auth.bearer_token = Some(token.into());
        self
    }

    /// 设置 API Key 认证
    pub fn with_api_key(mut self, header: impl Into<String>, key: impl Into<String>) -> Self {
        self.auth.api_key = Some((header.into(), key.into()));
        self
    }

    /// 设置连接超时
    pub fn with_timeout(mut self, secs: u64) -> Self {
        self.timeout_secs = Some(secs);
        self
    }
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
                                tracing::info!(server = %name, "MCP (stdio) 服务器连接成功");
                                toolsets.push(toolset);
                            }
                            Err(e) => {
                                tracing::warn!(server = %name, error = %e, "MCP (stdio) 服务器连接失败");
                            }
                        }
                    }
                }
                "http" => {
                    if let Some(url) = &desc.url {
                        match Self::connect_http(name, url, &desc.auth, desc.timeout_secs).await {
                            Ok(toolset) => {
                                tracing::info!(server = %name, url = %url, "MCP (http) 服务器连接成功");
                                toolsets.push(toolset);
                            }
                            Err(e) => {
                                tracing::warn!(server = %name, url = %url, error = %e, "MCP (http) 服务器连接失败");
                            }
                        }
                    }
                }
                _ => {
                    tracing::warn!(transport = %desc.transport, "不支持的 MCP 传输方式");
                }
            }
        }

        Ok(())
    }

    /// 通过 stdio 连接 MCP 服务器，返回 trait 对象工具集
    async fn connect_stdio(name: &str, cmd: &str, args: &[String]) -> Result<Arc<dyn Toolset>> {
        let mut command = Command::new(cmd);
        command.args(args);

        let transport = TokioChildProcess::new(command).map_err(|e| {
            crate::error::DevnpcError::Config(format!("MCP 传输创建失败 ({name}): {e}"))
        })?;

        let client = ().serve(transport).await.map_err(|e| {
            crate::error::DevnpcError::Config(format!("MCP 客户端连接失败 ({name}): {e}"))
        })?;

        let toolset: McpToolset = McpToolset::new(client);
        Ok(Arc::new(toolset))
    }

    /// 通过 streamable HTTP 连接 MCP 服务器，返回 trait 对象工具集
    ///
    /// 使用 adk-rust 的 McpHttpClientBuilder，支持 MCP 2025-06-18 spec 的
    /// streamable HTTP 协议 (JSON + SSE 响应, 透明 session 管理)。
    async fn connect_http(
        name: &str,
        url: &str,
        auth: &McpAuthConfig,
        timeout_secs: Option<u64>,
    ) -> Result<Arc<dyn Toolset>> {
        let mut builder = McpHttpClientBuilder::new(url);

        if auth.is_configured() {
            builder = builder.with_auth(auth.to_mcp_auth());
        }

        if let Some(secs) = timeout_secs {
            builder = builder.timeout(Duration::from_secs(secs));
        }

        let toolset = builder.connect().await.map_err(|e| {
            crate::error::DevnpcError::Config(format!(
                "MCP HTTP 客户端连接失败 ({name}, url={url}): {e}"
            ))
        })?;

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

        self.register_server(McpServerDesc::stdio(
            "codemap",
            cmd.clone(),
            vec!["serve".to_string(), "--data-dir".to_string(), data_dir.clone()],
        ))
        .await;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mcp_server_desc_stdio_builder() {
        let desc = McpServerDesc::stdio("test", "codemap", vec!["serve".into()]);
        assert_eq!(desc.transport, "stdio");
        assert_eq!(desc.command.as_deref(), Some("codemap"));
        assert!(desc.url.is_none());
        assert!(!desc.auth.is_configured());
    }

    #[test]
    fn test_mcp_server_desc_http_builder() {
        let desc = McpServerDesc::http("remote", "https://mcp.example.com/sse")
            .with_bearer("token-123")
            .with_timeout(60);
        assert_eq!(desc.transport, "http");
        assert_eq!(desc.url.as_deref(), Some("https://mcp.example.com/sse"));
        assert!(desc.auth.is_configured());
        assert_eq!(desc.timeout_secs, Some(60));
    }

    #[test]
    fn test_mcp_auth_config_bearer() {
        let cfg = McpAuthConfig {
            bearer_token: Some("tok".into()),
            api_key: None,
        };
        assert!(cfg.is_configured());
        let auth = cfg.to_mcp_auth();
        match &auth {
            McpAuth::Bearer(t) => assert_eq!(t, "tok"),
            _ => panic!("expected Bearer"),
        }
    }

    #[test]
    fn test_mcp_auth_config_api_key() {
        let cfg = McpAuthConfig {
            bearer_token: None,
            api_key: Some(("X-API-Key".into(), "key123".into())),
        };
        assert!(cfg.is_configured());
        let auth = cfg.to_mcp_auth();
        match &auth {
            McpAuth::ApiKey { header, key } => {
                assert_eq!(header, "X-API-Key");
                assert_eq!(key, "key123");
            }
            _ => panic!("expected ApiKey"),
        }
    }

    #[test]
    fn test_mcp_auth_config_empty_returns_default() {
        let cfg = McpAuthConfig::default();
        assert!(!cfg.is_configured());
        let auth = cfg.to_mcp_auth();
        assert!(matches!(auth, McpAuth::None));
    }

    #[tokio::test]
    async fn test_disabled_gateway_skips_connect() {
        let config = McpConfig {
            enabled: false,
            codemap_path: "codemap".into(),
            codemap_data_dir: ".codemap".into(),
        };
        let gateway = McpGateway::new(config);
        // 即使注册了服务器,connect_all 也应直接返回
        gateway
            .register_server(McpServerDesc::http("x", "http://localhost:9999"))
            .await;
        gateway.connect_all().await.unwrap();
        let toolsets = gateway.take_toolsets().await;
        assert!(toolsets.is_empty());
    }
}
