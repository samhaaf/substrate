//! The canonical spawn recipe + tool clean-slate flag construction (recon
//! Deliverables 3 + 4). This is how Leverage hands agents engineered tools only.

use substrate_ccw::spawn::{SpawnConfig, ToolConfig};

fn base(tools: ToolConfig) -> SpawnConfig {
    SpawnConfig {
        model: "fable".into(),
        session_id: "sess-1".into(),
        cwd: "/work".into(),
        append_system_prompt: Some("role".into()),
        add_dirs: vec!["/extra".into()],
        permission_mode: Some("bypassPermissions".into()),
        tools,
        max_budget_usd: None,
        fallback_model: None,
    }
}

/// Find the value immediately following a flag (or panic).
fn val_after<'a>(args: &'a [String], flag: &str) -> &'a str {
    let i = args.iter().position(|a| a == flag).unwrap_or_else(|| panic!("no {flag}"));
    args.get(i + 1).map(String::as_str).unwrap_or_else(|| panic!("no value after {flag}"))
}

#[test]
fn always_streams_json_and_scopes() {
    let args = base(ToolConfig::default()).build_args(true);
    assert!(args.iter().any(|a| a == "-p"));
    assert_eq!(val_after(&args, "--output-format"), "stream-json");
    assert!(args.iter().any(|a| a == "--verbose"));
    assert_eq!(val_after(&args, "--model"), "fable");
    assert_eq!(val_after(&args, "--append-system-prompt"), "role");
    assert_eq!(val_after(&args, "--add-dir"), "/extra");
    assert_eq!(val_after(&args, "--permission-mode"), "bypassPermissions");
}

#[test]
fn first_turn_mints_resume_continues() {
    let first = base(ToolConfig::default()).build_args(true);
    assert_eq!(val_after(&first, "--session-id"), "sess-1");
    assert!(!first.iter().any(|a| a == "--resume"));

    let cont = base(ToolConfig::default()).build_args(false);
    assert_eq!(val_after(&cont, "--resume"), "sess-1");
    assert!(!cont.iter().any(|a| a == "--session-id"));
}

#[test]
fn clean_slate_disables_all_builtins() {
    // `--tools ""` is THE clean-slate switch (recon Deliverable 4).
    let tc = ToolConfig { clean_slate: true, ..Default::default() };
    let args = base(tc).build_args(true);
    let i = args.iter().position(|a| a == "--tools").expect("--tools present");
    assert_eq!(args[i + 1], "", "--tools must be followed by an empty string");
}

#[test]
fn injects_custom_agents_and_fences() {
    let tc = ToolConfig {
        clean_slate: true,
        agents_json: Some(r#"{"reviewer":{"description":"r","prompt":"p"}}"#.into()),
        plugin_dirs: vec!["/plugins/inv".into()],
        allowed_tools: vec!["Bash(git *)".into()],
        disallowed_tools: vec!["WebFetch".into()],
        ..Default::default()
    };
    let args = base(tc).build_args(true);
    assert!(val_after(&args, "--agents").contains("reviewer"));
    assert_eq!(val_after(&args, "--plugin-dir"), "/plugins/inv");
    assert_eq!(val_after(&args, "--allowedTools"), "Bash(git *)");
    assert_eq!(val_after(&args, "--disallowedTools"), "WebFetch");
}

#[test]
fn explicit_toolset_when_not_clean_slate() {
    let tc = ToolConfig { tools: Some("Bash,Edit,Read".into()), ..Default::default() };
    let args = base(tc).build_args(true);
    assert_eq!(val_after(&args, "--tools"), "Bash,Edit,Read");
}

#[test]
fn env_strips_key_and_keeps_subagents_alive() {
    let cfg = base(ToolConfig::default());
    let env = cfg.env(Some("/cfg/dir"));
    // API key removed (use subscription auth).
    assert!(env.iter().any(|(k, v)| k == "ANTHROPIC_API_KEY" && v.is_none()));
    // The two subagent-survival vars (the idle-illusion fix).
    assert!(env.iter().any(|(k, v)| k == "CLAUDE_CODE_PRINT_BG_WAIT_CEILING_MS" && v.as_deref() == Some("0")));
    assert!(env.iter().any(|(k, v)| k == "CLAUDE_ASYNC_AGENT_STALL_TIMEOUT_MS" && v.as_deref() == Some("3600000")));
    // Account selection.
    assert!(env.iter().any(|(k, v)| k == "CLAUDE_CONFIG_DIR" && v.as_deref() == Some("/cfg/dir")));
    // Machine default clears it.
    let env2 = cfg.env(None);
    assert!(env2.iter().any(|(k, v)| k == "CLAUDE_CONFIG_DIR" && v.is_none()));
}
