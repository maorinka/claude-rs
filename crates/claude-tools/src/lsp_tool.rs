use anyhow::Result;
use async_trait::async_trait;
use once_cell::sync::Lazy;
use serde_json::{json, Value};
use std::path::Path;
use std::sync::Arc;
use tokio::sync::RwLock;
use tokio_util::sync::CancellationToken;

use crate::registry::{ProgressSender, ToolExecutor, ToolUseContext};
use claude_core::lsp::manager::LspManager;
use claude_core::types::events::ToolResultData;

// ---------------------------------------------------------------------------
// Shared LspManager singleton
// ---------------------------------------------------------------------------

/// Process-wide LspManager, lazily initialised.
///
/// Mirrors the TS pattern where `getLspServerManager()` returns a global
/// singleton. The `Arc<RwLock<..>>` allows concurrent reads (diagnostics /
/// hover) and exclusive writes (server start / register).
static LSP_MANAGER: Lazy<Arc<RwLock<LspManager>>> =
    Lazy::new(|| Arc::new(RwLock::new(LspManager::new())));

/// Get a reference to the global LspManager.
pub fn get_lsp_manager() -> Arc<RwLock<LspManager>> {
    LSP_MANAGER.clone()
}

/// Register a language server globally. Called during startup.
pub async fn register_lsp_server(
    language_id: &str,
    command: &str,
    args: &[String],
    extensions: &[String],
) {
    let mut mgr = LSP_MANAGER.write().await;
    mgr.register_server(language_id, command, args, extensions);
}

/// Set the workspace root URI on the global manager.
pub async fn set_lsp_root_uri(root_uri: String) {
    let mut mgr = LSP_MANAGER.write().await;
    mgr.set_root_uri(root_uri);
}

/// Check if any LSP server is registered (used for tool visibility).
pub async fn is_lsp_available() -> bool {
    let mgr = LSP_MANAGER.read().await;
    mgr.registered_count() > 0
}

/// Shut down all LSP servers (called at process exit).
pub async fn shutdown_lsp_servers() {
    let mgr = LSP_MANAGER.read().await;
    mgr.shutdown().await;
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Convert a file path to a file:// URI.
fn path_to_file_uri(file_path: &str) -> String {
    if file_path.starts_with('/') {
        format!("file://{}", file_path)
    } else {
        let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("/"));
        let absolute = cwd.join(file_path);
        format!("file://{}", absolute.display())
    }
}

/// Map TS-style operation names to LSP method strings and build params.
fn operation_to_method_and_params(
    operation: &str,
    file_path: &str,
    line: u64,
    character: u64,
) -> Result<(&'static str, Value)> {
    let uri = path_to_file_uri(file_path);
    // Convert from 1-based (user input) to 0-based (LSP protocol)
    let position = json!({
        "line": line.saturating_sub(1),
        "character": character.saturating_sub(1)
    });

    match operation {
        "goToDefinition" => Ok((
            "textDocument/definition",
            json!({ "textDocument": { "uri": uri }, "position": position }),
        )),
        "findReferences" => Ok((
            "textDocument/references",
            json!({
                "textDocument": { "uri": uri },
                "position": position,
                "context": { "includeDeclaration": true }
            }),
        )),
        "hover" => Ok((
            "textDocument/hover",
            json!({ "textDocument": { "uri": uri }, "position": position }),
        )),
        "documentSymbol" => Ok((
            "textDocument/documentSymbol",
            json!({ "textDocument": { "uri": uri } }),
        )),
        "workspaceSymbol" => Ok(("workspace/symbol", json!({ "query": "" }))),
        "goToImplementation" => Ok((
            "textDocument/implementation",
            json!({ "textDocument": { "uri": uri }, "position": position }),
        )),
        "prepareCallHierarchy" => Ok((
            "textDocument/prepareCallHierarchy",
            json!({ "textDocument": { "uri": uri }, "position": position }),
        )),
        "incomingCalls" => Ok((
            "textDocument/prepareCallHierarchy",
            json!({ "textDocument": { "uri": uri }, "position": position }),
        )),
        "outgoingCalls" => Ok((
            "textDocument/prepareCallHierarchy",
            json!({ "textDocument": { "uri": uri }, "position": position }),
        )),
        "diagnostics" => Ok((
            "textDocument/diagnostic",
            json!({ "textDocument": { "uri": uri } }),
        )),
        _ => Err(anyhow::anyhow!(
            "Unknown LSP operation: '{}'. Supported: goToDefinition, findReferences, \
             hover, documentSymbol, workspaceSymbol, goToImplementation, \
             prepareCallHierarchy, incomingCalls, outgoingCalls, diagnostics",
            operation
        )),
    }
}

fn format_lsp_result(operation: &str, result: &Value, cwd: &Path) -> (String, usize, usize) {
    match operation {
        "goToDefinition" | "goToImplementation" => {
            let locations = normalize_locations(result);
            let valid = locations
                .into_iter()
                .filter(|location| location.uri.is_some())
                .collect::<Vec<_>>();
            let formatted = format_definition_locations(&valid, cwd);
            let file_count = count_unique_location_files(&valid, cwd);
            (formatted, valid.len(), file_count)
        }
        "findReferences" => {
            let valid = result
                .as_array()
                .map(|items| {
                    items
                        .iter()
                        .filter_map(value_to_location)
                        .filter(|location| location.uri.is_some())
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            let formatted = format_reference_locations(&valid, cwd);
            let file_count = count_unique_location_files(&valid, cwd);
            (formatted, valid.len(), file_count)
        }
        "hover" => {
            let formatted = format_hover_result(result);
            let count = if result.is_null() { 0 } else { 1 };
            (formatted, count, count)
        }
        "documentSymbol" => {
            let symbols = result.as_array().cloned().unwrap_or_default();
            let count = count_document_symbols(&symbols);
            let formatted = format_document_symbols(&symbols);
            let file_count = if symbols.is_empty() { 0 } else { 1 };
            (formatted, count, file_count)
        }
        "workspaceSymbol" => {
            let symbols = result.as_array().cloned().unwrap_or_default();
            let valid = symbols
                .into_iter()
                .filter(|symbol| {
                    symbol
                        .pointer("/location/uri")
                        .and_then(Value::as_str)
                        .is_some()
                })
                .collect::<Vec<_>>();
            let formatted = format_workspace_symbols(&valid, cwd);
            let file_count = count_unique_symbol_files(&valid, cwd);
            (formatted, valid.len(), file_count)
        }
        "prepareCallHierarchy" => {
            let items = result.as_array().cloned().unwrap_or_default();
            let formatted = format_prepare_call_hierarchy(&items, cwd);
            let file_count = count_unique_call_item_files(&items, cwd);
            (formatted, items.len(), file_count)
        }
        "incomingCalls" => {
            let calls = result.as_array().cloned().unwrap_or_default();
            let formatted = format_incoming_calls(&calls, cwd);
            let file_count = count_unique_call_side_files(&calls, "from", cwd);
            (formatted, calls.len(), file_count)
        }
        "outgoingCalls" => {
            let calls = result.as_array().cloned().unwrap_or_default();
            let formatted = format_outgoing_calls(&calls, cwd);
            let file_count = count_unique_call_side_files(&calls, "to", cwd);
            (formatted, calls.len(), file_count)
        }
        _ => {
            let count = result
                .as_array()
                .map(Vec::len)
                .unwrap_or(usize::from(!result.is_null()));
            (result.to_string(), count, 0)
        }
    }
}

#[derive(Debug, Clone)]
struct LspLocation {
    uri: Option<String>,
    line: u64,
    character: u64,
}

fn normalize_locations(result: &Value) -> Vec<LspLocation> {
    if let Some(items) = result.as_array() {
        return items.iter().filter_map(value_to_location).collect();
    }
    value_to_location(result).into_iter().collect()
}

fn value_to_location(value: &Value) -> Option<LspLocation> {
    if value.is_null() {
        return None;
    }
    let uri = value
        .get("uri")
        .or_else(|| value.get("targetUri"))
        .and_then(Value::as_str)
        .map(str::to_string);
    let range = value
        .get("range")
        .or_else(|| value.get("targetSelectionRange"))
        .or_else(|| value.get("targetRange"))?;
    let start = range.get("start")?;
    Some(LspLocation {
        uri,
        line: start.get("line").and_then(Value::as_u64).unwrap_or(0) + 1,
        character: start.get("character").and_then(Value::as_u64).unwrap_or(0) + 1,
    })
}

fn format_definition_locations(locations: &[LspLocation], cwd: &Path) -> String {
    match locations.len() {
        0 => "No definition found. This may occur if the cursor is not on a symbol, or if the definition is in an external library not indexed by the LSP server.".to_string(),
        1 => format!("Defined in {}", format_location(&locations[0], Some(cwd))),
        len => {
            let list = locations
                .iter()
                .map(|location| format!("  {}", format_location(location, Some(cwd))))
                .collect::<Vec<_>>()
                .join("\n");
            format!("Found {len} definitions:\n{list}")
        }
    }
}

fn format_reference_locations(locations: &[LspLocation], cwd: &Path) -> String {
    if locations.is_empty() {
        return "No references found. This may occur if the symbol has no usages, or if the LSP server has not fully indexed the workspace.".to_string();
    }
    if locations.len() == 1 {
        return format!(
            "Found 1 reference:\n  {}",
            format_location(&locations[0], Some(cwd))
        );
    }

    let grouped = group_locations_by_file(locations, cwd);
    let mut lines = vec![format!(
        "Found {} references across {} files:",
        locations.len(),
        grouped.len()
    )];
    for (file, locations) in grouped {
        lines.push(format!("\n{file}:"));
        for location in locations {
            lines.push(format!("  Line {}:{}", location.line, location.character));
        }
    }
    lines.join("\n")
}

fn format_hover_result(result: &Value) -> String {
    if result.is_null() {
        return "No hover information available. This may occur if the cursor is not on a symbol, or if the LSP server has not fully indexed the file.".to_string();
    }
    let content = result
        .get("contents")
        .map(extract_markup_text)
        .unwrap_or_default();
    if let Some(start) = result.get("range").and_then(|range| range.get("start")) {
        let line = start.get("line").and_then(Value::as_u64).unwrap_or(0) + 1;
        let character = start.get("character").and_then(Value::as_u64).unwrap_or(0) + 1;
        return format!("Hover info at {line}:{character}:\n\n{content}");
    }
    content
}

fn extract_markup_text(value: &Value) -> String {
    if let Some(items) = value.as_array() {
        return items
            .iter()
            .map(extract_markup_text)
            .collect::<Vec<_>>()
            .join("\n\n");
    }
    if let Some(text) = value.as_str() {
        return text.to_string();
    }
    value
        .get("value")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string()
}

fn format_document_symbols(symbols: &[Value]) -> String {
    if symbols.is_empty() {
        return "No symbols found in document. This may occur if the file is empty, not supported by the LSP server, or if the server has not fully indexed the file.".to_string();
    }
    if symbols
        .first()
        .and_then(|symbol| symbol.get("location"))
        .is_some()
    {
        return format_workspace_symbols(symbols, Path::new(""));
    }

    let mut lines = vec!["Document symbols:".to_string()];
    for symbol in symbols {
        format_document_symbol_node(symbol, 0, &mut lines);
    }
    lines.join("\n")
}

fn format_document_symbol_node(symbol: &Value, indent: usize, lines: &mut Vec<String>) {
    let prefix = "  ".repeat(indent);
    let name = symbol.get("name").and_then(Value::as_str).unwrap_or("");
    let kind = symbol_kind_to_string(symbol.get("kind").and_then(Value::as_u64).unwrap_or(0));
    let mut line = format!("{prefix}{name} ({kind})");
    if let Some(detail) = symbol.get("detail").and_then(Value::as_str) {
        line.push(' ');
        line.push_str(detail);
    }
    let symbol_line = symbol
        .pointer("/range/start/line")
        .and_then(Value::as_u64)
        .unwrap_or(0)
        + 1;
    line.push_str(&format!(" - Line {symbol_line}"));
    lines.push(line);

    if let Some(children) = symbol.get("children").and_then(Value::as_array) {
        for child in children {
            format_document_symbol_node(child, indent + 1, lines);
        }
    }
}

fn format_workspace_symbols(symbols: &[Value], cwd: &Path) -> String {
    if symbols.is_empty() {
        return "No symbols found in workspace. This may occur if the workspace is empty, or if the LSP server has not finished indexing the project.".to_string();
    }
    let mut lines = vec![format!(
        "Found {} {} in workspace:",
        symbols.len(),
        plural(symbols.len(), "symbol")
    )];
    let grouped = group_symbols_by_file(symbols, cwd);
    for (file, symbols) in grouped {
        lines.push(format!("\n{file}:"));
        for symbol in symbols {
            let name = symbol.get("name").and_then(Value::as_str).unwrap_or("");
            let kind =
                symbol_kind_to_string(symbol.get("kind").and_then(Value::as_u64).unwrap_or(0));
            let line_no = symbol
                .pointer("/location/range/start/line")
                .and_then(Value::as_u64)
                .unwrap_or(0)
                + 1;
            let mut text = format!("  {name} ({kind}) - Line {line_no}");
            if let Some(container) = symbol.get("containerName").and_then(Value::as_str) {
                text.push_str(&format!(" in {container}"));
            }
            lines.push(text);
        }
    }
    lines.join("\n")
}

fn format_prepare_call_hierarchy(items: &[Value], cwd: &Path) -> String {
    match items.len() {
        0 => "No call hierarchy item found at this position".to_string(),
        1 => format!("Call hierarchy item: {}", format_call_item(&items[0], cwd)),
        len => {
            let mut lines = vec![format!("Found {len} call hierarchy items:")];
            for item in items {
                lines.push(format!("  {}", format_call_item(item, cwd)));
            }
            lines.join("\n")
        }
    }
}

fn format_incoming_calls(calls: &[Value], cwd: &Path) -> String {
    if calls.is_empty() {
        return "No incoming calls found (nothing calls this function)".to_string();
    }
    format_calls(calls, "from", "incoming", "calls at", cwd)
}

fn format_outgoing_calls(calls: &[Value], cwd: &Path) -> String {
    if calls.is_empty() {
        return "No outgoing calls found (this function calls nothing)".to_string();
    }
    format_calls(calls, "to", "outgoing", "called from", cwd)
}

fn format_calls(
    calls: &[Value],
    side: &str,
    label: &str,
    ranges_label: &str,
    cwd: &Path,
) -> String {
    let mut lines = vec![format!(
        "Found {} {} {}:",
        calls.len(),
        label,
        plural(calls.len(), "call")
    )];
    let grouped = group_calls_by_file(calls, side, cwd);
    for (file, calls) in grouped {
        lines.push(format!("\n{file}:"));
        for call in calls {
            let Some(item) = call.get(side) else {
                continue;
            };
            let name = item.get("name").and_then(Value::as_str).unwrap_or("");
            let kind = symbol_kind_to_string(item.get("kind").and_then(Value::as_u64).unwrap_or(0));
            let line = item
                .pointer("/range/start/line")
                .and_then(Value::as_u64)
                .unwrap_or(0)
                + 1;
            let mut text = format!("  {name} ({kind}) - Line {line}");
            if let Some(ranges) = call
                .get("fromRanges")
                .and_then(Value::as_array)
                .filter(|r| !r.is_empty())
            {
                let sites = ranges
                    .iter()
                    .map(|range| {
                        let line = range
                            .pointer("/start/line")
                            .and_then(Value::as_u64)
                            .unwrap_or(0)
                            + 1;
                        let character = range
                            .pointer("/start/character")
                            .and_then(Value::as_u64)
                            .unwrap_or(0)
                            + 1;
                        format!("{line}:{character}")
                    })
                    .collect::<Vec<_>>()
                    .join(", ");
                text.push_str(&format!(" [{ranges_label}: {sites}]"));
            }
            lines.push(text);
        }
    }
    lines.join("\n")
}

fn format_call_item(item: &Value, cwd: &Path) -> String {
    let name = item.get("name").and_then(Value::as_str).unwrap_or("");
    let kind = symbol_kind_to_string(item.get("kind").and_then(Value::as_u64).unwrap_or(0));
    let file = item
        .get("uri")
        .and_then(Value::as_str)
        .map(|uri| format_uri(uri, Some(cwd)))
        .unwrap_or_else(|| "<unknown location>".to_string());
    let line = item
        .pointer("/range/start/line")
        .and_then(Value::as_u64)
        .unwrap_or(0)
        + 1;
    let mut text = format!("{name} ({kind}) - {file}:{line}");
    if let Some(detail) = item.get("detail").and_then(Value::as_str) {
        text.push_str(&format!(" [{detail}]"));
    }
    text
}

fn format_location(location: &LspLocation, cwd: Option<&Path>) -> String {
    let file = location
        .uri
        .as_deref()
        .map(|uri| format_uri(uri, cwd))
        .unwrap_or_else(|| "<unknown location>".to_string());
    format!("{file}:{}:{}", location.line, location.character)
}

fn format_uri(uri: &str, cwd: Option<&Path>) -> String {
    let mut path = uri.strip_prefix("file://").unwrap_or(uri).to_string();
    if path.starts_with('/') && path.as_bytes().get(2) == Some(&b':') {
        path.remove(0);
    }
    path = path.replace("%20", " ");
    if let Some(cwd) = cwd {
        let absolute = Path::new(&path);
        if let Ok(relative) = absolute.strip_prefix(cwd) {
            let relative = relative.to_string_lossy().replace('\\', "/");
            if !relative.is_empty() && relative.len() < path.len() && !relative.starts_with("../..")
            {
                return relative;
            }
        }
    }
    path.replace('\\', "/")
}

fn group_locations_by_file<'a>(
    locations: &'a [LspLocation],
    cwd: &Path,
) -> Vec<(String, Vec<&'a LspLocation>)> {
    let mut groups: Vec<(String, Vec<&LspLocation>)> = Vec::new();
    for location in locations {
        let file = location
            .uri
            .as_deref()
            .map(|uri| format_uri(uri, Some(cwd)))
            .unwrap_or_else(|| "<unknown location>".to_string());
        push_grouped(&mut groups, file, location);
    }
    groups
}

fn group_symbols_by_file<'a>(symbols: &'a [Value], cwd: &Path) -> Vec<(String, Vec<&'a Value>)> {
    let mut groups: Vec<(String, Vec<&Value>)> = Vec::new();
    for symbol in symbols {
        if let Some(uri) = symbol.pointer("/location/uri").and_then(Value::as_str) {
            push_grouped(&mut groups, format_uri(uri, Some(cwd)), symbol);
        }
    }
    groups
}

fn group_calls_by_file<'a>(
    calls: &'a [Value],
    side: &str,
    cwd: &Path,
) -> Vec<(String, Vec<&'a Value>)> {
    let mut groups: Vec<(String, Vec<&Value>)> = Vec::new();
    for call in calls {
        if let Some(uri) = call
            .pointer(&format!("/{side}/uri"))
            .and_then(Value::as_str)
        {
            push_grouped(&mut groups, format_uri(uri, Some(cwd)), call);
        }
    }
    groups
}

fn push_grouped<'a, T>(groups: &mut Vec<(String, Vec<&'a T>)>, key: String, item: &'a T) {
    if let Some((_, items)) = groups.iter_mut().find(|(existing, _)| existing == &key) {
        items.push(item);
    } else {
        groups.push((key, vec![item]));
    }
}

fn count_unique_location_files(locations: &[LspLocation], cwd: &Path) -> usize {
    group_locations_by_file(locations, cwd).len()
}

fn count_unique_symbol_files(symbols: &[Value], cwd: &Path) -> usize {
    group_symbols_by_file(symbols, cwd).len()
}

fn count_unique_call_item_files(items: &[Value], cwd: &Path) -> usize {
    let mut groups: Vec<(String, Vec<&Value>)> = Vec::new();
    for item in items {
        if let Some(uri) = item.get("uri").and_then(Value::as_str) {
            push_grouped(&mut groups, format_uri(uri, Some(cwd)), item);
        }
    }
    groups.len()
}

fn count_unique_call_side_files(calls: &[Value], side: &str, cwd: &Path) -> usize {
    group_calls_by_file(calls, side, cwd).len()
}

fn count_document_symbols(symbols: &[Value]) -> usize {
    symbols
        .iter()
        .map(|symbol| {
            1 + symbol
                .get("children")
                .and_then(Value::as_array)
                .map(|children| count_document_symbols(children))
                .unwrap_or(0)
        })
        .sum()
}

fn symbol_kind_to_string(kind: u64) -> &'static str {
    match kind {
        1 => "File",
        2 => "Module",
        3 => "Namespace",
        4 => "Package",
        5 => "Class",
        6 => "Method",
        7 => "Property",
        8 => "Field",
        9 => "Constructor",
        10 => "Enum",
        11 => "Interface",
        12 => "Function",
        13 => "Variable",
        14 => "Constant",
        15 => "String",
        16 => "Number",
        17 => "Boolean",
        18 => "Array",
        19 => "Object",
        20 => "Key",
        21 => "Null",
        22 => "EnumMember",
        23 => "Struct",
        24 => "Event",
        25 => "Operator",
        26 => "TypeParameter",
        _ => "Unknown",
    }
}

fn plural(count: usize, word: &str) -> String {
    if count == 1 {
        word.to_string()
    } else {
        format!("{word}s")
    }
}

// ---------------------------------------------------------------------------
// Tool
// ---------------------------------------------------------------------------

pub struct LSPTool;

#[async_trait]
impl ToolExecutor for LSPTool {
    fn name(&self) -> &str {
        "LSP"
    }

    fn description(&self) -> String {
        "Run Language Server Protocol actions on source files. Supports: \
         goToDefinition, findReferences, hover, documentSymbol, workspaceSymbol, \
         goToImplementation, prepareCallHierarchy, incomingCalls, outgoingCalls, diagnostics. \
         Uses the project's registered language servers for real code intelligence."
            .to_string()
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "operation": {
                    "type": "string",
                    "enum": [
                        "goToDefinition",
                        "findReferences",
                        "hover",
                        "documentSymbol",
                        "workspaceSymbol",
                        "goToImplementation",
                        "prepareCallHierarchy",
                        "incomingCalls",
                        "outgoingCalls"
                    ],
                    "description": "The LSP operation to perform."
                },
                "filePath": {
                    "type": "string",
                    "description": "The absolute or relative path to the file."
                },
                "line": {
                    "type": "integer",
                    "description": "The 1-based line number (required for position-based operations)."
                },
                "character": {
                    "type": "integer",
                    "description": "The 1-based character offset (required for position-based operations)."
                }
            },
            "required": ["operation", "filePath", "line", "character"],
            "additionalProperties": false
        })
    }

    fn is_read_only(&self, _input: &Value) -> bool {
        true
    }

    fn is_concurrency_safe(&self, _input: &Value) -> bool {
        true
    }

    async fn call(
        &self,
        input: &Value,
        ctx: &ToolUseContext,
        _cancel: CancellationToken,
        _progress: Option<ProgressSender>,
    ) -> Result<ToolResultData> {
        let operation = input
            .get("operation")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let file_path = input.get("filePath").and_then(|v| v.as_str()).unwrap_or("");

        if operation.is_empty() {
            return Ok(ToolResultData {
                data: json!({ "error": "missing required field: operation" }),
                is_error: true,
            });
        }
        if file_path.is_empty() {
            return Ok(ToolResultData {
                data: json!({ "error": "missing required field: filePath" }),
                is_error: true,
            });
        }

        // Resolve absolute path
        let absolute_path = if Path::new(file_path).is_absolute() {
            file_path.to_string()
        } else {
            let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("/"));
            cwd.join(file_path).to_string_lossy().to_string()
        };

        let line = input.get("line").and_then(|v| v.as_u64()).unwrap_or(1);
        let character = input.get("character").and_then(|v| v.as_u64()).unwrap_or(1);

        // Special case: diagnostics go through the manager's get_diagnostics
        if operation == "diagnostics" {
            return self.handle_diagnostics(&absolute_path, file_path).await;
        }

        // Map operation to LSP method and params
        let (method, params) =
            match operation_to_method_and_params(operation, &absolute_path, line, character) {
                Ok(v) => v,
                Err(e) => {
                    return Ok(ToolResultData {
                        data: json!({ "error": e.to_string() }),
                        is_error: true,
                    });
                }
            };

        // Open the file in the LSP server if it exists
        if Path::new(&absolute_path).is_file() {
            match tokio::fs::read_to_string(&absolute_path).await {
                Ok(content) => {
                    let mgr = LSP_MANAGER.read().await;
                    let _ = mgr.open_file(&absolute_path, &content).await;
                }
                Err(e) => {
                    tracing::debug!("Could not read file for LSP didOpen: {}", e);
                }
            }
        }

        // Send the request through the LspManager
        let mgr = LSP_MANAGER.read().await;
        let result = mgr.send_request(&absolute_path, method, params).await;

        match result {
            Ok(Some(value)) => {
                // Handle two-step call hierarchy for incomingCalls/outgoingCalls
                let final_value = if operation == "incomingCalls" || operation == "outgoingCalls" {
                    self.handle_call_hierarchy(&mgr, &absolute_path, operation, &value)
                        .await
                        .unwrap_or(value)
                } else {
                    value
                };

                let (formatted, result_count, file_count) =
                    format_lsp_result(operation, &final_value, &ctx.working_directory);

                Ok(ToolResultData {
                    data: json!({
                        "operation": operation,
                        "filePath": file_path,
                        "result": formatted,
                        "resultCount": result_count,
                        "fileCount": file_count
                    }),
                    is_error: false,
                })
            }
            Ok(None) => {
                let ext = Path::new(file_path)
                    .extension()
                    .and_then(|e| e.to_str())
                    .unwrap_or("unknown");
                Ok(ToolResultData {
                    data: json!({
                        "operation": operation,
                        "filePath": file_path,
                        "result": format!("No LSP server available for file type: .{}", ext),
                        "resultCount": 0
                    }),
                    is_error: false,
                })
            }
            Err(e) => Ok(ToolResultData {
                data: json!({
                    "operation": operation,
                    "filePath": file_path,
                    "result": format!("Error performing {}: {}", operation, e),
                    "resultCount": 0
                }),
                is_error: false,
            }),
        }
    }
}

impl LSPTool {
    /// Handle diagnostics via the manager's dedicated method.
    async fn handle_diagnostics(
        &self,
        absolute_path: &str,
        display_path: &str,
    ) -> Result<ToolResultData> {
        let mgr = LSP_MANAGER.read().await;
        let diagnostics = mgr.get_diagnostics(absolute_path).await?;

        let diag_values: Vec<Value> = diagnostics
            .iter()
            .map(|d| {
                json!({
                    "range": {
                        "start": { "line": d.range.start.line, "character": d.range.start.character },
                        "end": { "line": d.range.end.line, "character": d.range.end.character }
                    },
                    "severity": d.severity.as_ref().map(|s| s.as_str()),
                    "message": d.message,
                    "source": d.source
                })
            })
            .collect();

        let count = diag_values.len();
        Ok(ToolResultData {
            data: json!({
                "operation": "diagnostics",
                "filePath": display_path,
                "result": diag_values,
                "resultCount": count
            }),
            is_error: false,
        })
    }

    /// Handle the two-step call hierarchy for incomingCalls / outgoingCalls.
    /// Step 1 result (prepareCallHierarchy) gives CallHierarchyItems.
    /// Step 2 requests the actual calls using the first item.
    async fn handle_call_hierarchy(
        &self,
        mgr: &LspManager,
        file_path: &str,
        operation: &str,
        prepare_result: &Value,
    ) -> Option<Value> {
        let items = prepare_result.as_array()?;
        if items.is_empty() {
            return Some(json!([]));
        }

        let call_method = if operation == "incomingCalls" {
            "callHierarchy/incomingCalls"
        } else {
            "callHierarchy/outgoingCalls"
        };

        let params = json!({ "item": items[0] });

        match mgr.send_request(file_path, call_method, params).await {
            Ok(Some(result)) => Some(result),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn make_ctx() -> ToolUseContext {
        ToolUseContext::for_test(
            PathBuf::from("/tmp"),
            std::sync::Arc::new(std::sync::Mutex::new(crate::registry::ReadFileState::new())),
            crate::registry::PermissionMode::Default,
        )
    }

    #[tokio::test]
    async fn test_diagnostics_with_manager() {
        // With no registered servers, diagnostics returns empty (not fake data)
        let tool = LSPTool;
        let input = json!({
            "operation": "diagnostics",
            "filePath": "/nonexistent/file.rs"
        });
        let result = tool
            .call(&input, &make_ctx(), CancellationToken::new(), None)
            .await
            .unwrap();

        assert!(!result.is_error);
        assert_eq!(result.data["operation"], "diagnostics");
        assert_eq!(result.data["resultCount"], 0);
    }

    #[tokio::test]
    async fn test_hover_uses_manager() {
        let tool = LSPTool;
        let input = json!({
            "operation": "hover",
            "filePath": "/some/file.rs",
            "line": 10,
            "character": 5
        });
        let result = tool
            .call(&input, &make_ctx(), CancellationToken::new(), None)
            .await
            .unwrap();

        assert!(!result.is_error);
        assert_eq!(result.data["operation"], "hover");
        // Without a running server, result should indicate no server available
        let result_str = result.data["result"].as_str().unwrap_or("");
        assert!(
            result_str.contains("No LSP server") || result.data["resultCount"] == 0,
            "should indicate no server or have zero results"
        );
    }

    #[tokio::test]
    async fn test_go_to_definition_uses_manager() {
        let tool = LSPTool;
        let input = json!({
            "operation": "goToDefinition",
            "filePath": "/some/file.ts",
            "line": 5,
            "character": 10
        });
        let result = tool
            .call(&input, &make_ctx(), CancellationToken::new(), None)
            .await
            .unwrap();

        assert!(!result.is_error);
        assert_eq!(result.data["operation"], "goToDefinition");
    }

    #[tokio::test]
    async fn test_unknown_operation_returns_error() {
        let tool = LSPTool;
        let input = json!({
            "operation": "nonExistentOp",
            "filePath": "/some/file.rs"
        });
        let result = tool
            .call(&input, &make_ctx(), CancellationToken::new(), None)
            .await
            .unwrap();

        assert!(result.is_error);
        assert!(result.data["error"]
            .as_str()
            .unwrap()
            .contains("Unknown LSP operation"));
    }

    #[tokio::test]
    async fn test_missing_operation_returns_error() {
        let tool = LSPTool;
        let input = json!({
            "filePath": "/some/file.rs"
        });
        let result = tool
            .call(&input, &make_ctx(), CancellationToken::new(), None)
            .await
            .unwrap();

        assert!(result.is_error);
        assert!(result.data["error"].as_str().unwrap().contains("operation"));
    }

    #[tokio::test]
    async fn test_missing_file_path_returns_error() {
        let tool = LSPTool;
        let input = json!({
            "operation": "hover"
        });
        let result = tool
            .call(&input, &make_ctx(), CancellationToken::new(), None)
            .await
            .unwrap();

        assert!(result.is_error);
        assert!(result.data["error"].as_str().unwrap().contains("filePath"));
    }

    #[test]
    fn test_lsp_tool_properties() {
        let tool = LSPTool;
        assert_eq!(tool.name(), "LSP");
        assert!(tool.is_read_only(&json!({})));
        assert!(tool.is_concurrency_safe(&json!({})));
    }

    #[test]
    fn formats_definition_like_ts() {
        let (formatted, count, file_count) = format_lsp_result(
            "goToDefinition",
            &json!([{
                "uri": "file:///tmp/project/src/main.rs",
                "range": { "start": { "line": 4, "character": 2 } }
            }]),
            Path::new("/tmp/project"),
        );
        assert_eq!(formatted, "Defined in src/main.rs:5:3");
        assert_eq!(count, 1);
        assert_eq!(file_count, 1);
    }

    #[test]
    fn formats_references_grouped_by_file_like_ts() {
        let (formatted, count, file_count) = format_lsp_result(
            "findReferences",
            &json!([
                {
                    "uri": "file:///tmp/project/src/main.rs",
                    "range": { "start": { "line": 0, "character": 1 } }
                },
                {
                    "uri": "file:///tmp/project/src/lib.rs",
                    "range": { "start": { "line": 9, "character": 3 } }
                }
            ]),
            Path::new("/tmp/project"),
        );
        assert!(formatted.contains("Found 2 references across 2 files:"));
        assert!(formatted.contains("src/main.rs:"));
        assert!(formatted.contains("Line 1:2"));
        assert_eq!(count, 2);
        assert_eq!(file_count, 2);
    }

    #[test]
    fn test_input_schema_has_all_operations() {
        let tool = LSPTool;
        let schema = tool.input_schema();
        let op_enum = &schema["properties"]["operation"]["enum"];
        let ops: Vec<&str> = op_enum
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect();
        assert!(ops.contains(&"goToDefinition"));
        assert!(ops.contains(&"findReferences"));
        assert!(ops.contains(&"hover"));
        assert!(ops.contains(&"documentSymbol"));
        assert!(ops.contains(&"workspaceSymbol"));
        assert!(ops.contains(&"goToImplementation"));
        assert!(ops.contains(&"prepareCallHierarchy"));
        assert!(ops.contains(&"incomingCalls"));
        assert!(ops.contains(&"outgoingCalls"));
        assert!(!ops.contains(&"diagnostics"));
        assert_eq!(
            schema["required"],
            json!(["operation", "filePath", "line", "character"])
        );
    }

    #[test]
    fn test_path_to_file_uri() {
        let uri = path_to_file_uri("/home/user/test.rs");
        assert_eq!(uri, "file:///home/user/test.rs");
    }

    #[test]
    fn test_operation_to_method() {
        let (method, _) =
            operation_to_method_and_params("goToDefinition", "/test.rs", 10, 5).unwrap();
        assert_eq!(method, "textDocument/definition");

        let (method, _) =
            operation_to_method_and_params("findReferences", "/test.rs", 10, 5).unwrap();
        assert_eq!(method, "textDocument/references");

        let (method, _) = operation_to_method_and_params("hover", "/test.rs", 10, 5).unwrap();
        assert_eq!(method, "textDocument/hover");

        let result = operation_to_method_and_params("badOp", "/test.rs", 10, 5);
        assert!(result.is_err());
    }
}
