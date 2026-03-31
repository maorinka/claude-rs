use claude_tools::build_default_registry;

#[test]
fn test_default_registry_has_all_phase1_tools() {
    let reg = build_default_registry();
    assert!(reg.get("Bash").is_some());
    assert!(reg.get("Read").is_some());
    assert!(reg.get("Write").is_some());
    assert!(reg.get("Edit").is_some());
    assert!(reg.get("Grep").is_some());
    assert!(reg.get("Glob").is_some());
}

#[test]
fn test_default_registry_has_all_new_tools() {
    let reg = build_default_registry();
    assert!(reg.get("WebFetch").is_some());
    assert!(reg.get("WebSearch").is_some());
    assert!(reg.get("TaskCreate").is_some());
    assert!(reg.get("TaskList").is_some());
    assert!(reg.get("TaskUpdate").is_some());
    assert!(reg.get("TaskGet").is_some());
    assert!(reg.get("TaskStop").is_some());
    assert!(reg.get("TaskOutput").is_some());
    assert!(reg.get("NotebookEdit").is_some());
    assert!(reg.get("Agent").is_some());
}

#[test]
fn test_default_registry_has_team_tools() {
    let reg = build_default_registry();
    assert!(reg.get("TeamCreate").is_some());
    assert!(reg.get("TeamDelete").is_some());
}

#[test]
fn test_default_registry_schemas() {
    let reg = build_default_registry();
    let schemas = reg.schemas();
    // 24 original tools + 2 team tools = 26 total
    assert_eq!(schemas.len(), 26);
    for schema in &schemas {
        assert!(schema.get("name").is_some());
        assert!(schema.get("input_schema").is_some());
    }
}
