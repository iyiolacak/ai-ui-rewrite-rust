//! Frontend tools: functions the model can call that run inside the app,
//! ported from `Tool` (`execute`) and the tool approval flow of assistant-ui.
//!
//! After a model step, the run loop executes every open call of a registered
//! tool that has an `execute` function and feeds the results to the model in
//! the next step. A tool marked [`Tool::requires_approval`] first pauses the
//! run with a pending approval on its call; `respond_to_tool_approval` records
//! the user's decision and resumes the run.

use crate::abort::AbortSignal;
use crate::adapter::ToolDefinition;
use crate::types::JsonObject;
use serde_json::Value;
use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;

/// What a running tool receives besides its arguments.
#[derive(Clone, Debug)]
pub struct ToolContext {
    /// The id of the call being run.
    pub tool_call_id: String,
    /// Aborts when the run is cancelled; long tools should stop.
    pub abort_signal: AbortSignal,
}

/// A tool's outcome: a JSON result for the model, or an error message that
/// becomes an error result.
pub type ToolFuture = Pin<Box<dyn Future<Output = Result<Value, String>>>>;

type ExecuteFn = dyn Fn(JsonObject, ToolContext) -> ToolFuture;
type PromptFn = dyn Fn(&str, &JsonObject) -> String;

/// A frontend tool: a schema for the model and, optionally, code to run in the app.
#[derive(Clone)]
pub struct Tool {
    pub(crate) description: Option<String>,
    pub(crate) parameters: Value,
    pub(crate) requires_approval: bool,
    pub(crate) execute: Option<Rc<ExecuteFn>>,
    pub(crate) prompt: Option<Rc<PromptFn>>,
}

impl Tool {
    /// A tool with a JSON Schema for its arguments.
    pub fn new(parameters: Value) -> Self {
        Self {
            description: None,
            parameters,
            requires_approval: false,
            execute: None,
            prompt: None,
        }
    }

    /// What the tool does, for the model.
    pub fn description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    /// Ask the user before every call.
    pub fn requires_approval(mut self) -> Self {
        self.requires_approval = true;
        self
    }

    /// The question shown with the approval: `(tool name, arguments)`.
    pub fn approval_prompt(
        mut self,
        prompt: impl Fn(&str, &JsonObject) -> String + 'static,
    ) -> Self {
        self.prompt = Some(Rc::new(prompt));
        self
    }

    /// Run the tool in the app. Without this, calls are left for the host
    /// (human tools answered through `add_tool_result`).
    pub fn execute<F, Fut>(mut self, execute: F) -> Self
    where
        F: Fn(JsonObject, ToolContext) -> Fut + 'static,
        Fut: Future<Output = Result<Value, String>> + 'static,
    {
        self.execute = Some(Rc::new(move |args, context| {
            Box::pin(execute(args, context))
        }));
        self
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            description: self.description.clone(),
            parameters: self.parameters.clone(),
        }
    }
}

/// The tools of a thread, by name.
#[derive(Clone, Default)]
pub struct ToolRegistry {
    tools: BTreeMap<String, Tool>,
}

impl ToolRegistry {
    /// An empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a tool under `name`.
    pub fn with(mut self, name: impl Into<String>, tool: Tool) -> Self {
        self.tools.insert(name.into(), tool);
        self
    }

    /// The tool registered under `name`.
    pub fn get(&self, name: &str) -> Option<&Tool> {
        self.tools.get(name)
    }

    /// What adapters send to the model.
    pub fn definitions(&self) -> BTreeMap<String, ToolDefinition> {
        self.tools
            .iter()
            .map(|(n, t)| (n.clone(), t.definition()))
            .collect()
    }
}

/// The approval question for a call.
pub(crate) fn approval_prompt(tool: &Tool, name: &str, args: &JsonObject) -> String {
    match &tool.prompt {
        Some(prompt) => prompt(name, args),
        None => format!("Allow the assistant to use {name}?"),
    }
}
