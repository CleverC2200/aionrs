use std::sync::Arc;

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{Value, json};

use aion_protocol::events::ToolCategory;
use aion_tools::Tool;
use aion_types::tool::{JsonSchema, ToolResult};

use super::manager::McpManager;
use super::transport::McpError;

pub(crate) const RESOURCE_TOOL_NAME: &str = "ReadMcpResource";
const PAGE_BYTES: usize = 6_000;
const PAGE_ENTRIES: usize = 20;

pub(crate) struct ResourceReader(pub(crate) Arc<McpManager>);

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ReadInput {
    server: String,
    uri: String,
    #[serde(default)]
    pointer: String,
    #[serde(default)]
    offset: usize,
}

#[async_trait]
impl Tool for ResourceReader {
    fn name(&self) -> &str {
        RESOURCE_TOOL_NAME
    }

    fn description(&self) -> &str {
        "Read a resource returned by an MCP tool in this session. Start with pointer=''. Small JSON values are returned whole; large objects/arrays return a bounded index of child JSON pointers (with names). Follow a pointer for details and next_offset for more entries. Large strings/non-JSON text are paged by character offset. Never guess schema fields before reading them."
    }

    fn input_schema(&self) -> JsonSchema {
        json!({"type":"object", "properties": {
            "server":{"type":"string"}, "uri":{"type":"string"},
            "pointer":{"type":"string", "description":"RFC 6901 JSON pointer from a previous page, or empty for root"},
            "offset":{"type":"integer", "minimum":0, "description":"Use next_offset from the previous page; defaults to 0"}
        }, "required":["server","uri"], "additionalProperties":false})
    }

    fn is_concurrency_safe(&self, _input: &Value) -> bool {
        false
    }
    fn category(&self) -> ToolCategory {
        ToolCategory::Mcp
    }

    async fn execute(&self, input: Value) -> ToolResult {
        let result = match serde_json::from_value::<ReadInput>(input) {
            Ok(input) => {
                self.0
                    .read_tool_resource(&input.server, &input.uri, &input.pointer, input.offset)
                    .await
            }
            Err(_) => Err(McpError::Transport("Invalid resource read arguments".into())),
        };
        match result {
            Ok(content) => ToolResult {
                content,
                is_error: false,
            },
            Err(error) => ToolResult {
                content: error.to_string(),
                is_error: true,
            },
        }
    }
}

/// Produce a complete small value or a navigable index, never a truncated JSON document.
pub(crate) fn resource_page(text: &str, pointer: &str, offset: usize) -> Result<String, McpError> {
    if pointer.len() > 1_024 {
        return Err(McpError::Transport("Resource pointer is too long".into()));
    }
    let parsed = serde_json::from_str::<Value>(text);
    let root = match parsed {
        Ok(value) => value,
        Err(_) if pointer.is_empty() => Value::String(text.to_owned()),
        Err(_) => return Err(McpError::Transport("JSON pointer requires a JSON resource".into())),
    };
    let value = root
        .pointer(pointer)
        .ok_or_else(|| McpError::Transport("Resource JSON pointer not found".into()))?;
    let whole = json!({"pointer":pointer,"complete":true,"value":value}).to_string();
    if offset == 0 && whole.len() <= PAGE_BYTES {
        return Ok(whole);
    }
    if let Value::String(value) = value {
        let total = value.chars().count();
        if offset > total {
            return Err(McpError::Transport("Resource offset out of range".into()));
        }
        // At most 6 encoded JSON bytes per scalar (control characters), leaving room for metadata.
        let part: String = value.chars().skip(offset).take(600).collect();
        let next = offset + part.chars().count();
        return Ok(json!({"pointer":pointer,"complete":next==total,"text":part,
            "offset":offset,"next_offset":(next<total).then_some(next),"total_chars":total})
        .to_string());
    }
    let children: Vec<(String, &Value)> = match value {
        Value::Object(map) => map.iter().map(|(key, value)| (key.clone(), value)).collect(),
        Value::Array(items) => items
            .iter()
            .enumerate()
            .map(|(i, value)| (i.to_string(), value))
            .collect(),
        _ => {
            return Err(McpError::Transport(
                "Resource offset requires a container or text".into(),
            ));
        }
    };
    if offset > children.len() {
        return Err(McpError::Transport("Resource offset out of range".into()));
    }
    let mut entries = Vec::new();
    let mut next = offset;
    for (key, child) in children.iter().skip(offset).take(PAGE_ENTRIES) {
        let path = format!("{}/{}", pointer, key.replace('~', "~0").replace('/', "~1"));
        let kind = match child {
            Value::Array(_) => "array",
            Value::Object(_) => "object",
            Value::String(_) => "string",
            _ => "scalar",
        };
        let mut entry = json!({"pointer":path,"type":kind});
        if let Some(name) = child.get("name").and_then(Value::as_str) {
            entry["name"] = Value::String(name.chars().take(100).collect());
        }
        if let Some(items) = child.as_array() {
            entry["count"] = json!(items.len());
        }
        entries.push(entry);
        if json!({"pointer":pointer,"entries":entries}).to_string().len() > PAGE_BYTES - 200 {
            entries.pop();
            break;
        }
        next += 1;
    }
    if next == offset && offset < children.len() {
        return Err(McpError::Transport("Resource entry exceeds page budget".into()));
    }
    Ok(
        json!({"pointer":pointer,"complete":false,"entries":entries,"offset":offset,
        "next_offset":(next<children.len()).then_some(next),"total_entries":children.len(),
        "message":"Index only; follow child pointers to read values."})
        .to_string(),
    )
}

#[cfg(test)]
#[path = "resource_reader_test.rs"]
mod resource_reader_test;
