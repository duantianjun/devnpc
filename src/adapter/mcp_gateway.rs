//! MCP Gateway: 管理 MCP 客户端连接
//!
//! 利用 adk-rust 的 mcp feature，建立统一的 MCP 协议入口。
//! 支持两种传输方式:
//! - stdio: 通过子进程启动 MCP 服务器 (如 codemap)
//! - http:  通过 streamable HTTP 连接远程 MCP 服务器 (MCP 2025-06-18 spec)

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use serde::Deserialize;

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

    /// 从 YAML 配置文件加载 MCP 服务器
    ///
    /// 扫描 `mcp_servers_dir` 下的所有 `.yml` 文件,解析为 `McpServerDesc` 并注册。
    /// YAML 格式见 `npc-config/mcp-servers/codemap.yml`。
    /// 支持环境变量展开: `${ENV_VAR}` 会被替换为对应的环境变量值。
    pub async fn load_from_yaml(&self, mcp_servers_dir: &std::path::Path) -> Result<usize> {
        if !self.config.enabled {
            return Ok(0);
        }

        if !mcp_servers_dir.exists() {
            tracing::debug!(dir = %mcp_servers_dir.display(), "MCP 服务器配置目录不存在,跳过加载");
            return Ok(0);
        }

        let mut count = 0;
        let entries = std::fs::read_dir(mcp_servers_dir).map_err(|e| {
            crate::error::DevnpcError::Config(format!(
                "读取 MCP 配置目录失败 ({}): {e}",
                mcp_servers_dir.display()
            ))
        })?;

        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("yml") {
                continue;
            }

            let content = match std::fs::read_to_string(&path) {
                Ok(c) => c,
                Err(e) => {
                    tracing::warn!(file = %path.display(), error = %e, "读取 MCP 配置文件失败,跳过");
                    continue;
                }
            };

            // YAML 文件格式为列表 (数组),每个元素是一个服务器配置
            let servers: Vec<YamlMcpServer> = match serde_yaml::from_str(&content) {
                Ok(s) => s,
                Err(e) => {
                    tracing::warn!(file = %path.display(), error = %e, "解析 MCP 配置 YAML 失败,跳过");
                    continue;
                }
            };

            for srv in servers {
                let desc = srv.to_mcp_server_desc();
                tracing::info!(name = %desc.name, file = %path.display(), "从 YAML 加载 MCP 服务器");
                self.register_server(desc).await;
                count += 1;
            }
        }

        if count > 0 {
            tracing::info!(count = count, "从 YAML 配置加载 MCP 服务器完成");
        }

        Ok(count)
    }
}

/// YAML 格式的 MCP 服务器配置 (用于反序列化)
#[derive(Debug, Clone, Deserialize)]
struct YamlMcpServer {
    name: String,
    /// "stdio" 或 "http"
    transport: String,
    /// stdio: 命令名
    command: Option<String>,
    /// stdio: 命令参数
    args: Option<Vec<String>>,
    /// http: 服务器 URL
    url: Option<String>,
    /// http: Bearer token (支持 ${ENV_VAR} 展开)
    bearer_token: Option<String>,
    /// http: API Key 认证
    api_key: Option<YamlApiKey>,
    /// http: 连接超时 (秒)
    timeout_secs: Option<u64>,
}

/// YAML 格式的 API Key 认证
#[derive(Debug, Clone, Deserialize)]
struct YamlApiKey {
    /// Header 名称 (如 "X-API-Key")
    header: String,
    /// Key 值 (支持 ${ENV_VAR} 展开)
    key: String,
}

impl YamlMcpServer {
    /// 转换为 McpServerDesc,支持环境变量展开
    fn to_mcp_server_desc(&self) -> McpServerDesc {
        let mut desc = match self.transport.as_str() {
            "stdio" => {
                let cmd = self.command.clone().unwrap_or_default();
                let args = self.args.clone().unwrap_or_default();
                McpServerDesc::stdio(self.name.clone(), cmd, args)
            }
            "http" => {
                let url = self.url.clone().unwrap_or_default();
                McpServerDesc::http(self.name.clone(), url)
            }
            _ => {
                tracing::warn!(transport = %self.transport, name = %self.name, "未知传输方式,默认使用 stdio");
                McpServerDesc::stdio(
                    self.name.clone(),
                    self.command.clone().unwrap_or_default(),
                    self.args.clone().unwrap_or_default(),
                )
            }
        };

        // Bearer token (展开环境变量)
        if let Some(token) = &self.bearer_token {
            let expanded = expand_env_vars(token);
            desc = desc.with_bearer(expanded);
        }

        // API Key 认证 (展开环境变量)
        if let Some(api_key) = &self.api_key {
            let header_expanded = expand_env_vars(&api_key.header);
            let key_expanded = expand_env_vars(&api_key.key);
            desc = desc.with_api_key(header_expanded, key_expanded);
        }

        // 超时
        if let Some(secs) = self.timeout_secs {
            desc = desc.with_timeout(secs);
        }

        desc
    }
}

/// 展开字符串中的 `${ENV_VAR}` 为环境变量值
///
/// 若环境变量未设置,替换为空字符串。
fn expand_env_vars(s: &str) -> String {
    let mut result = s.to_string();
    while let Some(start) = result.find("${") {
        if let Some(end) = result[start..].find('}') {
            let var_name = &result[start + 2..start + end];
            let value = std::env::var(var_name).unwrap_or_default();
            result = format!("{}{}{}", &result[..start], value, &result[start + end + 1..]);
        } else {
            break; // 无匹配的 },停止
        }
    }
    result
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

    #[test]
    fn test_expand_env_vars_no_vars() {
        let result = expand_env_vars("plain-string");
        assert_eq!(result, "plain-string");
    }

    #[test]
    fn test_expand_env_vars_with_var() {
        // SAFETY: 测试串行执行,不与其他线程共享此环境变量。
        unsafe {
            std::env::set_var("TEST_MCP_TOKEN", "secret123");
        }
        let result = expand_env_vars("Bearer ${TEST_MCP_TOKEN}");
        assert_eq!(result, "Bearer secret123");
        // SAFETY: 同上。
        unsafe {
            std::env::remove_var("TEST_MCP_TOKEN");
        }
    }

    #[test]
    fn test_expand_env_vars_unset_var() {
        let result = expand_env_vars("${UNDEFINED_VAR_12345}");
        assert_eq!(result, "");
    }

    #[test]
    fn test_expand_env_vars_multiple_vars() {
        // SAFETY: 测试串行执行,不与其他线程共享这些环境变量。
        unsafe {
            std::env::set_var("TEST_VAR_A", "value_a");
            std::env::set_var("TEST_VAR_B", "value_b");
        }
        let result = expand_env_vars("${TEST_VAR_A}-${TEST_VAR_B}");
        assert_eq!(result, "value_a-value_b");
        // SAFETY: 同上。
        unsafe {
            std::env::remove_var("TEST_VAR_A");
            std::env::remove_var("TEST_VAR_B");
        }
    }

    #[tokio::test]
    async fn test_load_from_yaml_disabled_gateway() {
        let config = McpConfig {
            enabled: false,
            codemap_path: "codemap".into(),
            codemap_data_dir: ".codemap".into(),
        };
        let gateway = McpGateway::new(config);
        let dir = std::path::Path::new("npc-config/mcp-servers");
        let count = gateway.load_from_yaml(dir).await.unwrap();
        assert_eq!(count, 0, "禁用的 Gateway 不应加载任何服务器");
    }

    #[tokio::test]
    async fn test_load_from_yaml_nonexistent_dir() {
        let config = McpConfig {
            enabled: true,
            codemap_path: "codemap".into(),
            codemap_data_dir: ".codemap".into(),
        };
        let gateway = McpGateway::new(config);
        let dir = std::path::Path::new("nonexistent-dir-12345");
        let count = gateway.load_from_yaml(dir).await.unwrap();
        assert_eq!(count, 0, "不存在的目录应返回 0");
    }

    #[tokio::test]
    async fn test_yaml_mcp_server_to_desc_stdio() {
        let srv = YamlMcpServer {
            name: "test-stdio".into(),
            transport: "stdio".into(),
            command: Some("my-tool".into()),
            args: Some(vec!["--port".into(), "8080".into()]),
            url: None,
            bearer_token: None,
            api_key: None,
            timeout_secs: None,
        };
        let desc = srv.to_mcp_server_desc();
        assert_eq!(desc.name, "test-stdio");
        assert_eq!(desc.transport, "stdio");
        assert_eq!(desc.command.as_deref(), Some("my-tool"));
        assert_eq!(desc.args, vec!["--port".to_string(), "8080".to_string()]);
        assert!(desc.url.is_none());
    }

    #[tokio::test]
    async fn test_yaml_mcp_server_to_desc_http_with_auth() {
        // SAFETY: 测试串行执行,不与其他线程共享此环境变量。
        unsafe {
            std::env::set_var("TEST_YAML_TOKEN", "tok-abc");
        }
        let srv = YamlMcpServer {
            name: "test-http".into(),
            transport: "http".into(),
            command: None,
            args: None,
            url: Some("https://mcp.example.com/sse".into()),
            bearer_token: Some("${TEST_YAML_TOKEN}".into()),
            api_key: None,
            timeout_secs: Some(45),
        };
        let desc = srv.to_mcp_server_desc();
        assert_eq!(desc.name, "test-http");
        assert_eq!(desc.transport, "http");
        assert_eq!(desc.url.as_deref(), Some("https://mcp.example.com/sse"));
        assert!(desc.auth.is_configured());
        assert_eq!(desc.timeout_secs, Some(45));
        // SAFETY: 同上。
        unsafe {
            std::env::remove_var("TEST_YAML_TOKEN");
        }
    }

    #[tokio::test]
    async fn test_yaml_mcp_server_to_desc_http_with_api_key() {
        // SAFETY: 测试串行执行,不与其他线程共享此环境变量。
        unsafe {
            std::env::set_var("TEST_API_KEY_VAL", "key-secret");
        }
        let srv = YamlMcpServer {
            name: "test-apikey".into(),
            transport: "http".into(),
            command: None,
            args: None,
            url: Some("https://mcp.example.com/api".into()),
            bearer_token: None,
            api_key: Some(YamlApiKey {
                header: "X-API-Key".into(),
                key: "${TEST_API_KEY_VAL}".into(),
            }),
            timeout_secs: None,
        };
        let desc = srv.to_mcp_server_desc();
        assert_eq!(desc.name, "test-apikey");
        assert!(desc.auth.is_configured());
        // SAFETY: 同上。
        unsafe {
            std::env::remove_var("TEST_API_KEY_VAL");
        }
    }
}
