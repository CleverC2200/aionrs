use std::collections::BTreeSet;

use aion_types::tool::ToolDef;

use crate::Tool;
use crate::tool_search::ToolSearchTool;

pub struct ToolRegistry {
    tools: Vec<Box<dyn Tool>>,
    activated_tools: BTreeSet<String>,
}

impl Default for ToolRegistry {
    fn default() -> Self {
        Self::new()
    }
}
impl ToolRegistry {
    pub fn new() -> Self {
        Self {
            tools: Vec::new(),
            activated_tools: BTreeSet::new(),
        }
    }

    pub fn register(&mut self, tool: Box<dyn Tool>) {
        self.tools.push(tool);
    }

    /// Promote schemas after a successful ToolSearch execution. The caller must
    /// pass the executed query, never names parsed from model or tool output.
    pub fn activate_deferred_tools(&mut self, query: &str) {
        let definitions = self.to_tool_defs();
        let names = ToolSearchTool::matching_tools(&definitions, query)
            .into_iter()
            .map(|tool| tool.name.clone())
            .collect::<Vec<_>>();
        self.activated_tools.extend(names);
    }

    /// Persist schema activation independently of compactable message history.
    pub fn activated_tool_names(&self) -> Vec<String> {
        self.activated_tools.iter().cloned().collect()
    }

    /// Restore names against the current registry; schemas and permissions are
    /// always taken from the current tools and the engine's current policy.
    pub fn restore_activated_tools(&mut self, names: &[String]) {
        for name in names {
            if self.get(name).is_some_and(|tool| tool.is_deferred()) {
                self.activated_tools.insert(name.clone());
            }
        }
    }

    /// Find a tool by name
    pub fn get(&self, name: &str) -> Option<&dyn Tool> {
        self.tools.iter().find(|t| t.name() == name).map(|t| t.as_ref())
    }

    /// Get all registered tool names
    pub fn tool_names(&self) -> Vec<String> {
        self.tools.iter().map(|t| t.name().to_string()).collect()
    }

    /// Generate API tool definitions for all registered tools
    pub fn to_tool_defs(&self) -> Vec<ToolDef> {
        self.tools
            .iter()
            .map(|t| ToolDef {
                name: t.name().to_string(),
                description: t.description().to_string(),
                input_schema: t.input_schema(),
                deferred: t.is_deferred() && !self.activated_tools.contains(t.name()),
            })
            .collect()
    }

    /// Generate API tool definitions for tools matching a predicate.
    ///
    /// Used by plan mode to restrict the tool set sent to the LLM.
    pub fn to_tool_defs_filtered<F>(&self, filter: F) -> Vec<ToolDef>
    where
        F: Fn(&dyn Tool) -> bool,
    {
        self.tools
            .iter()
            .filter(|t| filter(t.as_ref()))
            .map(|t| ToolDef {
                name: t.name().to_string(),
                description: t.description().to_string(),
                input_schema: t.input_schema(),
                deferred: t.is_deferred() && !self.activated_tools.contains(t.name()),
            })
            .collect()
    }
}

#[cfg(test)]
#[path = "registry_test.rs"]
mod registry_test;
