use kria_core::tools::registry::build_default_registry;

#[test]
fn registry_has_handlers_for_every_registered_tool() {
    let registry = build_default_registry();
    let defs = registry.list_defs();
    assert!(
        !defs.is_empty(),
        "tool registry should not be empty in production profile"
    );

    let mut missing = Vec::new();
    for def in defs {
        if registry.get_handler(&def.name).is_none() {
            missing.push(def.name);
        }
    }

    assert!(
        missing.is_empty(),
        "missing handlers for registered tools: {:?}",
        missing
    );
}

#[test]
fn critical_tool_presence_matrix() {
    let registry = build_default_registry();
    let critical = [
        "check_system_health",
        "web_search",
        "get_weather",
        "get_news",
        "install_package",
        "execute_fleet_command",
        "list_files",
        "read_file",
        "remember_fact",
        "recall_fact",
    ];

    let mut missing = Vec::new();
    for name in critical {
        if registry.get_def(name).is_none() || registry.get_handler(name).is_none() {
            missing.push(name.to_string());
        }
    }

    assert!(missing.is_empty(), "critical tool matrix missing entries: {:?}", missing);
}

#[test]
fn unknown_tool_negative_path() {
    let registry = build_default_registry();
    assert!(registry.get_def("__nonexistent_tool__").is_none());
    assert!(registry.get_handler("__nonexistent_tool__").is_none());
}
