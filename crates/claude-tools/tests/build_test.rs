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
    assert!(reg.get("LSP").is_some());
}

#[test]
fn test_default_registry_schemas() {
    let reg = build_default_registry();
    let schemas = reg.schemas();
    assert!(
        schemas.len() >= 27,
        "expected at least 27 tools, got {}",
        schemas.len()
    );
    for schema in &schemas {
        assert!(schema.get("name").is_some());
        assert!(schema.get("input_schema").is_some());
    }
}
