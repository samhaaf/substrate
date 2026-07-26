//! stream-json → CcwEvent taxonomy mapping (recon Deliverable 1). Every `type`
//! discriminator has an arm; nothing is dropped.

use serde_json::json;
use substrate_ccw::event::CcwEvent;
use substrate_ccw::stream::{parse_line, parse_raw_line};

#[test]
fn system_init_is_surfaced_whole() {
    let v = json!({
        "type": "system", "subtype": "init", "session_id": "s1", "cwd": "/x",
        "model": "claude-fable-5", "tools": ["Bash", "Read"], "mcp_servers": [],
        "permissionMode": "bypassPermissions", "slash_commands": ["/usage"],
        "apiKeySource": "none", "agents": []
    });
    let evs = parse_line(&v);
    assert_eq!(evs.len(), 1);
    match &evs[0] {
        CcwEvent::SystemInit { model, tools, permission_mode, .. } => {
            assert_eq!(model.as_deref(), Some("claude-fable-5"));
            assert_eq!(tools, &vec!["Bash".to_string(), "Read".to_string()]);
            assert_eq!(permission_mode.as_deref(), Some("bypassPermissions"));
        }
        other => panic!("expected SystemInit, got {other:?}"),
    }
}

#[test]
fn assistant_text_thinking_and_agent_tooluse() {
    let v = json!({
        "type": "assistant",
        "message": {
            "model": "claude-opus-4-8",
            "content": [
                {"type": "thinking", "thinking": "hmm", "signature": "sig"},
                {"type": "text", "text": "hello"},
                {"type": "tool_use", "id": "toolu_1", "name": "Agent",
                 "input": {"description": "d", "subagent_type": "general-purpose", "run_in_background": true, "prompt": "go"}}
            ],
            "usage": {"input_tokens": 10, "output_tokens": 2, "cache_read_input_tokens": 1, "cache_creation_input_tokens": 3}
        }
    });
    let evs = parse_line(&v);
    // thinking, text, agent_spawn, tool_use, message_usage
    assert!(evs.iter().any(|e| matches!(e, CcwEvent::Thinking { .. })));
    assert!(evs.iter().any(|e| matches!(e, CcwEvent::AssistantText { text, .. } if text == "hello")));
    assert!(evs.iter().any(|e| matches!(e, CcwEvent::AgentSpawn { run_in_background, .. } if *run_in_background)));
    assert!(evs.iter().any(|e| matches!(e, CcwEvent::ToolUse { name, .. } if name == "Agent")));
    match evs.iter().find(|e| matches!(e, CcwEvent::MessageUsage { .. })).unwrap() {
        CcwEvent::MessageUsage { input_tokens, cache_creation_tokens, .. } => {
            assert_eq!(*input_tokens, 10);
            assert_eq!(*cache_creation_tokens, 3);
        }
        _ => unreachable!(),
    }
}

#[test]
fn async_agent_launch_is_tracked() {
    let v = json!({
        "type": "user",
        "toolUseResult": {"isAsync": true, "status": "async_launched", "agentId": "agent-x",
                          "toolUseId": "toolu_1", "outputFile": "/tmp/a.jsonl"},
        "message": {"content": [{"type": "tool_result", "tool_use_id": "toolu_1", "content": "launched"}]}
    });
    let evs = parse_line(&v);
    let launched = evs.iter().find(|e| matches!(e, CcwEvent::AgentLaunched { .. })).unwrap();
    match launched {
        CcwEvent::AgentLaunched { agent_id, is_async, status, output_file, .. } => {
            assert_eq!(agent_id, "agent-x");
            assert!(*is_async);
            assert_eq!(status, "async_launched");
            assert_eq!(output_file.as_deref(), Some("/tmp/a.jsonl"));
        }
        _ => unreachable!(),
    }
    assert!(evs.iter().any(|e| matches!(e, CcwEvent::ToolResult { .. })));
}

#[test]
fn synthetic_429_becomes_limit_hit() {
    let v = json!({
        "type": "assistant",
        "message": {"model": "<synthetic>", "content": [{"type": "text",
            "text": "You have hit your session limit resets 2:40am (America/Chicago)"}], "usage": {}},
        "error": "rate_limit", "isApiErrorMessage": true, "apiErrorStatus": 429
    });
    let evs = parse_line(&v);
    assert_eq!(evs.len(), 1);
    match &evs[0] {
        CcwEvent::LimitHit { status, reset, text, .. } => {
            assert_eq!(*status, Some(429));
            assert!(reset.as_deref().unwrap().contains("2:40am"));
            assert!(text.contains("session limit"));
        }
        other => panic!("expected LimitHit, got {other:?}"),
    }
}

#[test]
fn result_carries_model_usage() {
    let v = json!({
        "type": "result", "subtype": "success", "is_error": false,
        "total_cost_usd": 0.01, "num_turns": 1, "duration_ms": 50, "stop_reason": "end_turn",
        "modelUsage": {"claude-fable-5": {"inputTokens": 105, "outputTokens": 25, "costUSD": 0.01}}
    });
    match &parse_line(&v)[0] {
        CcwEvent::ResultTurn { subtype, total_cost_usd, model_usage, .. } => {
            assert_eq!(subtype, "success");
            assert_eq!(*total_cost_usd, Some(0.01));
            assert!(model_usage.get("claude-fable-5").is_some());
        }
        other => panic!("expected ResultTurn, got {other:?}"),
    }
}

#[test]
fn unknown_and_stream_event_never_dropped() {
    // A type aui buckets as "unknown" — CCW keeps it.
    let unknown = json!({"type": "hook_event", "hook": "PreToolUse"});
    assert!(matches!(parse_line(&unknown)[0], CcwEvent::Unknown { .. }));

    let se = json!({"type": "stream_event", "event": {"type": "content_block_delta"}});
    assert!(matches!(parse_line(&se)[0], CcwEvent::StreamEvent { .. }));

    let ald = json!({"type": "agent_listing_delta", "delta": []});
    assert!(matches!(parse_line(&ald)[0], CcwEvent::AgentListingDelta { .. }));

    // Non-JSON garbage → Unknown, still not dropped.
    assert!(matches!(parse_raw_line("not json")[0], CcwEvent::Unknown { .. }));
    assert!(parse_raw_line("   ").is_empty());
}

#[test]
fn event_kind_matches_serialized_tag() {
    let e = CcwEvent::AssistantText { text: "x".into(), parent_tool_use_id: None };
    assert_eq!(e.kind(), "assistant_text");
    let json = serde_json::to_value(&e).unwrap();
    assert_eq!(json["kind"], "assistant_text");
    // round-trip
    let back: CcwEvent = serde_json::from_value(json).unwrap();
    assert_eq!(back, e);
}
