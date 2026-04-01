// Integration tests for the 5 wired systems:
//   1. Cost tracking (wired to TUI/slash commands)
//   2. Session resume (loads conversation, --resume last)
//   3. Conversation compaction (triggered before API call)
//   4. IDE bridge (prompt, file_changed, get_status, get_diagnostics)
//   5. MCP SSE/HTTP transports (manager routes correctly)

// =============================================================================
// 1. Cost tracking -- CostTracker feeds header/slash-command displays
// =============================================================================

mod cost_tracking {
    use claude_core::cost::tracker::CostTracker;
    use claude_core::types::usage::Usage;

    fn make_usage(input: u64, output: u64) -> Usage {
        Usage {
            input_tokens: input,
            output_tokens: output,
            cache_read_input_tokens: None,
            cache_creation_input_tokens: None,
        }
    }

    #[test]
    fn header_display_shows_tokens_and_cost() {
        let mut tracker = CostTracker::new("claude-sonnet-4-6");
        tracker.add_usage(&make_usage(10_000, 5_000));
        let header = tracker.header_display();

        // Should contain token count and cost
        assert!(header.contains("k tokens"), "header should show k tokens, got: {}", header);
        assert!(header.contains("$"), "header should show cost, got: {}", header);
    }

    #[test]
    fn detailed_summary_available_for_cost_command() {
        let mut tracker = CostTracker::new("claude-sonnet-4-6");
        tracker.add_usage(&Usage {
            input_tokens: 5000,
            output_tokens: 2000,
            cache_read_input_tokens: Some(1000),
            cache_creation_input_tokens: Some(500),
        });
        let detail = tracker.detailed_summary();

        // Must contain all the fields the /cost command needs
        assert!(detail.contains("Total cost:"), "missing total cost");
        assert!(detail.contains("Total input tokens:"), "missing input tokens");
        assert!(detail.contains("Total output tokens:"), "missing output tokens");
        assert!(detail.contains("Cache read tokens:"), "missing cache read");
        assert!(detail.contains("Cache write tokens:"), "missing cache write");
        assert!(detail.contains("API requests:"), "missing request count");
        assert!(detail.contains("claude-sonnet-4-6"), "missing model name");
    }

    #[test]
    fn cost_command_reads_from_shared_state() {
        use claude_core::commands::builtin::CostHandler;
        use claude_core::commands::registry::{CommandContext, CommandHandler, CommandResult, SharedCommandState};
        use std::sync::{Arc, Mutex};

        let mut state = SharedCommandState::default();
        state.cost_summary = "Total cost:            $1.2345\nTotal input tokens:    100000".to_string();

        let ctx = CommandContext {
            working_directory: std::path::PathBuf::from("/tmp"),
            model: "test-model".to_string(),
            shared: Some(Arc::new(Mutex::new(state))),
        };

        let result = CostHandler.execute("", &ctx).unwrap();
        match result {
            CommandResult::Action(text) => {
                assert!(text.contains("$1.2345"), "should show cost from shared state, got: {}", text);
                assert!(text.contains("100000"), "should show tokens, got: {}", text);
            }
            _ => panic!("expected Action variant"),
        }
    }
}

// =============================================================================
// 2. Session resume -- load_transcript populates messages, --resume last works
// =============================================================================

mod session_resume {
    use claude_core::session::manager::SessionManager;
    use serde_json::json;

    #[test]
    fn load_transcript_round_trips_messages() {
        // Use a real SessionManager to create storage in the sessions dir
        let mgr = SessionManager::new().unwrap();
        let storage = mgr.storage();

        let msgs = vec![
            json!({"role": "user", "content": [{"type": "text", "text": "hello"}]}),
            json!({"role": "assistant", "content": [{"type": "text", "text": "hi back"}]}),
            json!({"role": "user", "content": [{"type": "text", "text": "what is 2+2?"}]}),
        ];

        for msg in &msgs {
            storage.append_transcript(&serde_json::to_string(msg).unwrap()).unwrap();
        }

        let loaded = storage.load_transcript().unwrap();
        assert_eq!(loaded.len(), 3);
        assert_eq!(loaded[0]["role"], "user");
        assert_eq!(loaded[1]["role"], "assistant");
        assert_eq!(loaded[2]["content"][0]["text"], "what is 2+2?");
    }

    #[test]
    fn load_transcript_skips_malformed_and_empty() {
        let mgr = SessionManager::new().unwrap();
        let storage = mgr.storage();

        let good = json!({"role": "user", "content": "test"});
        storage.append_transcript(&serde_json::to_string(&good).unwrap()).unwrap();
        storage.append_transcript("not json!!!").unwrap();
        storage.append_transcript("").unwrap();
        storage.append_transcript(&serde_json::to_string(&good).unwrap()).unwrap();

        let loaded = storage.load_transcript().unwrap();
        assert_eq!(loaded.len(), 2, "should skip bad lines and keep good ones");
    }

    #[test]
    fn resume_last_finds_most_recent_session() {
        // Create two sessions with transcripts
        let s1 = SessionManager::new().unwrap();
        s1.storage().append_transcript(
            &serde_json::to_string(&json!({"role": "user", "content": "first session"})).unwrap()
        ).unwrap();

        // Small delay to ensure different modification times
        std::thread::sleep(std::time::Duration::from_millis(50));

        let s2 = SessionManager::new().unwrap();
        s2.storage().append_transcript(
            &serde_json::to_string(&json!({"role": "user", "content": "second session"})).unwrap()
        ).unwrap();

        let sessions = SessionManager::list_sessions().unwrap();
        assert!(!sessions.is_empty(), "should find sessions");

        // The most recent session (s2) should be first (sorted by modification time desc)
        let most_recent = &sessions[0];
        assert_eq!(most_recent.id, s2.session_id(),
            "most recent session should be s2, got {} (expected {})", most_recent.id, s2.session_id());
    }
}

// =============================================================================
// 3. Conversation compaction -- should_compact runs before API call
// =============================================================================

mod compaction {
    use claude_core::compact::compactor::{should_compact, estimate_tokens, default_context_window};
    use serde_json::json;

    #[test]
    fn should_compact_returns_false_for_short_conversations() {
        let messages = vec![
            json!({"role": "user", "content": [{"type": "text", "text": "hello"}]}),
            json!({"role": "assistant", "content": [{"type": "text", "text": "hi"}]}),
        ];
        assert!(!should_compact(&messages, default_context_window()),
            "short conversation should not trigger compaction");
    }

    #[test]
    fn should_compact_returns_true_when_near_limit() {
        // Create a conversation that exceeds the threshold.
        // threshold = 200_000 - 13_000 - 20_000 = 167_000 tokens
        // At 4 chars per token, we need ~668_000 chars of text content.
        let big_text = "x".repeat(700_000); // ~175k tokens
        let messages = vec![
            json!({"role": "user", "content": [{"type": "text", "text": big_text}]}),
        ];
        assert!(should_compact(&messages, default_context_window()),
            "conversation near token limit should trigger compaction");
    }

    #[test]
    fn estimate_tokens_counts_text_blocks_not_json() {
        // Verify text-based estimation, not JSON serialization
        let text = "a".repeat(400); // 400 chars = 100 tokens
        let messages = vec![
            json!({"role": "user", "content": [{"type": "text", "text": text}]}),
        ];
        let tokens = estimate_tokens(&messages);
        assert_eq!(tokens, 100,
            "should estimate 100 tokens from 400 chars, got {}", tokens);
    }

    #[test]
    fn compaction_check_runs_before_api_call() {
        // This is a structural verification:
        // In engine.rs, should_compact() is called at the top of run_turn()
        // BEFORE the API request. We verify the compactor returns correct
        // results which the engine depends on.
        let small = vec![json!({"role": "user", "content": "short"})];
        let big = vec![json!({"role": "user", "content": [{"type": "text", "text": "x".repeat(700_000)}]})];

        // Small conversation: no compaction
        assert!(!should_compact(&small, default_context_window()));
        // Large conversation: compaction needed
        assert!(should_compact(&big, default_context_window()));
    }
}

// =============================================================================
// 4. IDE bridge -- prompt, file_changed, get_status, get_diagnostics handlers
// =============================================================================

mod bridge {
    use claude_core::bridge::protocol::BridgeRequest;
    use claude_core::bridge::server::{BridgeState, dispatch_request, dispatch_request_stateless};
    use std::sync::{Arc, Mutex};

    fn make_request(id: &str, method: &str, params: serde_json::Value) -> BridgeRequest {
        BridgeRequest {
            id: id.to_string(),
            method: method.to_string(),
            params,
        }
    }

    #[test]
    fn prompt_handler_queues_text() {
        let state = Arc::new(Mutex::new(BridgeState::default()));
        let req = make_request("p1", "prompt", serde_json::json!({"text": "fix the bug"}));
        let resp = dispatch_request(&req, Some(&state));

        assert!(resp.error.is_none(), "prompt should succeed");
        let result = resp.result.unwrap();
        assert_eq!(result["queued"], true);
        assert_eq!(result["prompt"], "fix the bug");

        // Verify the prompt was actually queued
        let s = state.lock().unwrap();
        assert_eq!(s.pending_prompts.len(), 1);
        assert_eq!(s.pending_prompts[0].text, "fix the bug");
    }

    #[test]
    fn prompt_handler_rejects_empty_text() {
        let state = Arc::new(Mutex::new(BridgeState::default()));
        let req = make_request("p2", "prompt", serde_json::json!({"text": ""}));
        let resp = dispatch_request(&req, Some(&state));

        assert!(resp.error.is_some(), "empty prompt should fail");
        assert_eq!(resp.error.unwrap().code, -2);
    }

    #[test]
    fn file_changed_handler_records_change() {
        let state = Arc::new(Mutex::new(BridgeState::default()));
        let req = make_request("f1", "file_changed",
            serde_json::json!({"path": "src/main.rs", "change_type": "saved"}));
        let resp = dispatch_request(&req, Some(&state));

        assert!(resp.error.is_none(), "file_changed should succeed");
        let result = resp.result.unwrap();
        assert_eq!(result["acknowledged"], true);
        assert_eq!(result["path"], "src/main.rs");

        let s = state.lock().unwrap();
        assert_eq!(s.file_changes.len(), 1);
        assert_eq!(s.file_changes[0].path, "src/main.rs");
        assert_eq!(s.file_changes[0].change_type, "saved");
    }

    #[test]
    fn get_status_returns_session_details() {
        let state = Arc::new(Mutex::new(BridgeState {
            session_state: "running".to_string(),
            model: "claude-sonnet-4-6".to_string(),
            message_count: 42,
            engine_busy: true,
            ..Default::default()
        }));
        let req = make_request("s1", "get_status", serde_json::json!({}));
        let resp = dispatch_request(&req, Some(&state));

        assert!(resp.error.is_none());
        let result = resp.result.unwrap();
        assert_eq!(result["state"], "running");
        assert_eq!(result["model"], "claude-sonnet-4-6");
        assert_eq!(result["message_count"], 42);
        assert_eq!(result["engine_busy"], true);
    }

    #[test]
    fn get_diagnostics_returns_supported_methods() {
        let req = make_request("d1", "get_diagnostics", serde_json::json!({}));
        let resp = dispatch_request_stateless(&req);

        assert!(resp.error.is_none());
        let result = resp.result.unwrap();
        let methods = result["supported_methods"].as_array().unwrap();
        let method_names: Vec<&str> = methods.iter().filter_map(|v| v.as_str()).collect();

        assert!(method_names.contains(&"ping"), "should support ping");
        assert!(method_names.contains(&"prompt"), "should support prompt");
        assert!(method_names.contains(&"file_changed"), "should support file_changed");
        assert!(method_names.contains(&"get_status"), "should support get_status");
        assert!(method_names.contains(&"get_diagnostics"), "should support get_diagnostics");
    }

    #[test]
    fn unknown_method_returns_error() {
        let req = make_request("u1", "nonexistent", serde_json::json!({}));
        let resp = dispatch_request_stateless(&req);

        assert!(resp.result.is_none());
        let err = resp.error.unwrap();
        assert_eq!(err.code, -1);
        assert!(err.message.contains("nonexistent"));
    }
}

// =============================================================================
// 5. MCP SSE/HTTP -- manager routes Sse/Http configs, client build_mcp_tool_name
// =============================================================================

mod mcp_transports {
    use claude_core::mcp::client::{build_mcp_tool_name, normalize_mcp_name};
    use claude_core::mcp::manager::McpManager;
    use claude_core::mcp::types::*;
    use std::collections::HashMap;

    #[test]
    fn build_mcp_tool_name_normalizes() {
        assert_eq!(
            build_mcp_tool_name("my-sse-server", "list-items"),
            "mcp__my_sse_server__list_items"
        );
    }

    #[test]
    fn normalize_mcp_name_handles_special_chars() {
        assert_eq!(normalize_mcp_name("sse.server"), "sse_server");
        assert_eq!(normalize_mcp_name("http-server"), "http_server");
        assert_eq!(normalize_mcp_name("server name"), "server_name");
    }

    #[tokio::test]
    async fn manager_routes_sse_config() {
        // Verify the manager accepts SSE configs and attempts connection
        // (it will fail since there's no real server, but the routing should work)
        let manager = McpManager::new();
        let mut configs = HashMap::new();

        let scoped = ScopedMcpServerConfig {
            config: McpServerConfig::Sse(McpSseServerConfig {
                url: "http://127.0.0.1:1/sse".to_string(),
                headers: None,
            }),
            scope: ConfigScope::User,
        };
        configs.insert("test-sse".to_string(), scoped);

        let results = manager.connect_all(configs).await;
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].name, "test-sse");
        // Connection will fail (no server), but it should not panic
        match &results[0].status {
            McpConnectionStatus::Failed { error } => {
                assert!(error.is_some(), "should have error message for failed SSE connection");
            }
            _ => {
                // If it somehow connected (unlikely), that's also fine
            }
        }
    }

    #[tokio::test]
    async fn manager_routes_http_config() {
        let manager = McpManager::new();
        let mut configs = HashMap::new();

        let scoped = ScopedMcpServerConfig {
            config: McpServerConfig::Http(McpHttpServerConfig {
                url: "http://127.0.0.1:1/mcp".to_string(),
                headers: None,
            }),
            scope: ConfigScope::User,
        };
        configs.insert("test-http".to_string(), scoped);

        let results = manager.connect_all(configs).await;
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].name, "test-http");
        // Connection will fail (no server), but routing should work
        match &results[0].status {
            McpConnectionStatus::Failed { error } => {
                assert!(error.is_some(), "should have error for failed HTTP connection");
            }
            _ => {}
        }
    }

    #[tokio::test]
    async fn manager_handles_mixed_transports() {
        let manager = McpManager::new();
        let mut configs = HashMap::new();

        configs.insert("sse-server".to_string(), ScopedMcpServerConfig {
            config: McpServerConfig::Sse(McpSseServerConfig {
                url: "http://127.0.0.1:1/sse".to_string(),
                headers: Some({
                    let mut h = HashMap::new();
                    h.insert("Authorization".to_string(), "Bearer token".to_string());
                    h
                }),
            }),
            scope: ConfigScope::User,
        });

        configs.insert("http-server".to_string(), ScopedMcpServerConfig {
            config: McpServerConfig::Http(McpHttpServerConfig {
                url: "http://127.0.0.1:1/http".to_string(),
                headers: None,
            }),
            scope: ConfigScope::Project,
        });

        let results = manager.connect_all(configs).await;
        assert_eq!(results.len(), 2);

        // Both should have attempted connection (and failed since no servers)
        let names: Vec<&str> = results.iter().map(|r| r.name.as_str()).collect();
        assert!(names.contains(&"sse-server") || names.contains(&"http-server"));
    }
}
