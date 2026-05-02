// prism-runtime/src/ipc/protocol.rs
//
// Wire protocol types for the OpenRender IPC format.
// Any host application implementing this protocol can communicate with the runtime.

use serde::{Serialize, Deserialize};
use serde_json::Value;

/// An IPC request sent to the host application.
#[derive(Debug, Serialize, Deserialize)]
pub struct IpcRequest {
    /// Namespace (e.g. "sysdata", "config", "control").
    pub ns: String,
    /// Command name within the namespace.
    pub cmd: String,
    /// Optional JSON arguments.
    pub args: Option<Value>,
}

/// An IPC response from the host application.
#[derive(Debug, Serialize, Deserialize)]
pub struct IpcResponse {
    pub ok: bool,
    pub data: Option<Value>,
    pub error: Option<String>,
}

impl IpcRequest {
    /// Create a simple request with no arguments.
    pub fn new(ns: impl Into<String>, cmd: impl Into<String>) -> Self {
        Self {
            ns: ns.into(),
            cmd: cmd.into(),
            args: None,
        }
    }

    /// Create a request with arguments.
    pub fn with_args(ns: impl Into<String>, cmd: impl Into<String>, args: Value) -> Self {
        Self {
            ns: ns.into(),
            cmd: cmd.into(),
            args: Some(args),
        }
    }
}

