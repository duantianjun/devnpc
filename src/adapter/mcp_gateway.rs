//! MCP Gateway: 管理 MCP 客户端连接
//!
//! 利用 adk-rust 的 mcp feature，建立统一的 MCP 协议入口。
//! 支持 stdio 和 streamable HTTP 两种传输方式。

use std::collections::HashMap;
use std::sync::Arc;

use adk_rust::Tool;
use tok