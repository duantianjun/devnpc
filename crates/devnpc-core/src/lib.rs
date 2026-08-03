//! devnpc-core - 共享类型库
//!
//! 提供 devnpc 和 devnpc-dashboard 共用的数据结构:
//! - 报告类型 (Trajectory/ReportData/CostEstimate)
//! - dashboard 事件协议 (TaskStartedEvent/ExecutionEvent/TaskFinishedEvent)

pub mod error;
pub mod report;
