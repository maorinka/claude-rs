use claude_tools::build_default_registry;
use serde_json::json;

#[test]
fn test_agent_tool_schema_has_team_name() {
    let reg = build_default_registry();
    let agent = reg.get("Agent").expect("Agent tool should be registered");
    let schema = agent.input_schema();

    // Verify team_name is in the schema properties
    assert!(
        schema["properties"]["team_name"].is_object(),
        "Agent tool schema should have team_name property"
    );
    assert_eq!(
        schema["properties"]["team_name"]["type"], "string",
        "team_name should be a string"
    );
}

#[test]
fn test_agent_tool_schema_has_name() {
    let reg = build_default_registry();
    let agent = reg.get("Agent").expect("Agent tool should be registered");
    let schema = agent.input_schema();

    // Verify name is in the schema properties
    assert!(
        schema["properties"]["name"].is_object(),
        "Agent tool schema should have name property"
    );
    assert_eq!(
        schema["properties"]["name"]["type"], "string",
        "name should be a string"
    );
}

#[test]
fn test_agent_tool_schema_has_mode() {
    let reg = build_default_registry();
    let agent = reg.get("Agent").expect("Agent tool should be registered");
    let schema = agent.input_schema();

    // Verify mode is in the schema properties (for team permission mode)
    assert!(
        schema["properties"]["mode"].is_object(),
        "Agent tool schema should have mode property"
    );
}

#[test]
fn test_agent_tool_schema_has_required_prompt() {
    let reg = build_default_registry();
    let agent = reg.get("Agent").expect("Agent tool should be registered");
    let schema = agent.input_schema();

    let required = schema["required"]
        .as_array()
        .expect("should have required array");
    assert!(
        required.iter().any(|v| v == "prompt"),
        "prompt should be required"
    );
}

#[test]
fn test_agent_tool_has_alias() {
    let reg = build_default_registry();
    // The Agent tool has "agent" as an alias
    let by_alias = reg.get("agent");
    assert!(
        by_alias.is_some(),
        "Agent tool should be findable by alias 'agent'"
    );
}

#[test]
fn test_team_create_tool_schema() {
    let reg = build_default_registry();
    let tool = reg
        .get("TeamCreate")
        .expect("TeamCreate should be registered");
    let schema = tool.input_schema();

    assert_eq!(schema["type"], "object");
    let required = schema["required"].as_array().expect("should have required");
    assert!(required.iter().any(|v| v == "team_name"));
    assert!(schema["properties"]["team_name"].is_object());
    assert!(schema["properties"]["description"].is_object());
    assert!(schema["properties"]["agent_type"].is_object());
}

#[test]
fn test_team_delete_tool_schema() {
    let reg = build_default_registry();
    let tool = reg
        .get("TeamDelete")
        .expect("TeamDelete should be registered");
    let schema = tool.input_schema();

    assert_eq!(schema["type"], "object");
    assert!(schema["properties"]["team_name"].is_object());
}
