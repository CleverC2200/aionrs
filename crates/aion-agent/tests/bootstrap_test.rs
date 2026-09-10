use std::sync::Arc;

use aion_agent::bootstrap::AgentBootstrap;
use aion_agent::output::null_sink::NullSink;
use aion_config::compat::ProviderCompat;
use aion_config::config::{Config, ProviderType};

fn minimal_config() -> Config {
    Config {
        provider_label: "openai".into(),
        provider: ProviderType::OpenAI,
        api_key: "sk-test".into(),
        base_url: "http://localhost:0".into(),
        model: "gpt-test-model".into(),
        max_tokens: Some(1024),
        max_turns: Some(5),
        max_tool_call_malformed_turns: Some(3),
        max_tool_call_failure_turns: Some(3),
        system_prompt: None,
        thinking: None,
        prompt_caching: false,
        compat: ProviderCompat::openai_defaults(),
        tools: Default::default(),
        session: Default::default(),
        compact: Default::default(),
        plan: Default::default(),
        shell: Default::default(),
        file_cache: Default::default(),
        hooks: Default::default(),
        bedrock: None,
        vertex: None,
        mcp: Default::default(),
        logging: Default::default(),
    }
}

fn null_output() -> Arc<dyn aion_agent::output::OutputSink> {
    Arc::new(NullSink)
}

#[tokio::test]
async fn bootstrap_builds_engine_with_model_in_prompt() {
    let config = minimal_config();
    let result = AgentBootstrap::new(config, "/tmp/test-workspace", null_output())
        .build()
        .await
        .expect("bootstrap should succeed");

    assert!(!result.engine.tool_names().is_empty());
    assert!(!result.has_mcp);
    assert!(result.mcp_managers.is_empty());
}

#[tokio::test]
async fn bootstrap_registers_all_expected_tools() {
    let config = minimal_config();
    let result = AgentBootstrap::new(config, "/tmp/test-workspace", null_output())
        .build()
        .await
        .unwrap();

    let names = result.engine.tool_names();

    for expected in &["Read", "Write", "Edit", "ExecCommand", "Grep", "Glob"] {
        assert!(names.iter().any(|n| n == expected), "missing built-in tool: {expected}");
    }

    assert!(names.iter().any(|n| n == "Skill"), "SkillTool should be registered");
    assert!(names.iter().any(|n| n == "Spawn"), "SpawnTool should be registered");
    assert!(
        names.iter().any(|n| n == "ToolSearch"),
        "ToolSearchTool should be registered"
    );
}

#[tokio::test]
async fn bootstrap_plan_tools_when_enabled() {
    let mut config = minimal_config();
    config.plan.enabled = true;

    let result = AgentBootstrap::new(config, "/tmp/test-workspace", null_output())
        .build()
        .await
        .unwrap();

    let names = result.engine.tool_names();
    assert!(
        names.iter().any(|n| n == "EnterPlanMode"),
        "EnterPlanMode should be registered when plan.enabled"
    );
    assert!(
        names.iter().any(|n| n == "ExitPlanMode"),
        "ExitPlanMode should be registered when plan.enabled"
    );
}

#[tokio::test]
async fn bootstrap_no_plan_tools_when_disabled() {
    let mut config = minimal_config();
    config.plan.enabled = false;

    let result = AgentBootstrap::new(config, "/tmp/test-workspace", null_output())
        .build()
        .await
        .unwrap();

    let names = result.engine.tool_names();
    assert!(
        !names.iter().any(|n| n == "EnterPlanMode"),
        "EnterPlanMode should NOT be registered when plan.disabled"
    );
}

#[tokio::test]
async fn bootstrap_no_mcp_when_no_servers() {
    let config = minimal_config();
    let result = AgentBootstrap::new(config, "/tmp/test-workspace", null_output())
        .build()
        .await
        .unwrap();

    assert!(!result.has_mcp);
    assert!(result.mcp_managers.is_empty());
}

#[tokio::test]
async fn bootstrap_with_custom_system_prompt() {
    let mut config = minimal_config();
    config.system_prompt = Some("You are a pirate assistant.".into());

    let _result = AgentBootstrap::new(config, "/tmp/test-workspace", null_output())
        .build()
        .await
        .unwrap();
}

#[tokio::test]
async fn bootstrap_with_agents_md_in_workspace() {
    let tmp = tempfile::TempDir::new().unwrap();
    let workspace = tmp.path();
    std::fs::write(workspace.join("AGENTS.md"), "PROJECT_RULES_MARKER").unwrap();

    let config = minimal_config();
    let _result = AgentBootstrap::new(config, workspace.to_string_lossy().as_ref(), null_output())
        .build()
        .await
        .unwrap();
}

#[tokio::test]
async fn bootstrap_config_accessor_returns_config() {
    let config = minimal_config();
    let bootstrap = AgentBootstrap::new(config, "/tmp/ws", null_output());
    assert_eq!(bootstrap.config().model, "gpt-test-model");
    assert_eq!(bootstrap.config().max_tokens, Some(1024));
}

#[tokio::test]
async fn bootstrap_with_external_provider() {
    let config = minimal_config();
    let provider = aion_providers::create_provider(&config);

    let result = AgentBootstrap::new(config, "/tmp/test-workspace", null_output())
        .provider(provider)
        .build()
        .await
        .unwrap();

    assert!(!result.engine.tool_names().is_empty());
}

// Exercises real MCP transport, provider projection, session persistence and the
// engine loop against local servers; no external model or business data is used.
#[tokio::test]
async fn mcp_tools_survive_tool_round_continuation_compaction_and_resume_on_wire() {
    use aion_agent::session::SessionManager;
    use aion_config::config::{McpServerConfig, TransportType};
    use aion_types::message::ContentBlock;
    use serde_json::{Value, json};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, Request, ResponseTemplate};

    const FOLLOW_UPS: usize = 36;
    const RESUME_REQUEST: usize = 3 + FOLLOW_UPS + 2;

    let server = MockServer::start().await;
    let connections = Arc::new(AtomicUsize::new(0));
    let queries = Arc::new(AtomicUsize::new(0));
    let seen_connections = connections.clone();
    let seen_queries = queries.clone();
    Mock::given(method("POST")).and(path("/mcp"))
        .respond_with(move |request: &Request| {
            let body: Value = request.body_json().unwrap();
            let result = match body["method"].as_str().unwrap() {
                "initialize" => {
                    seen_connections.fetch_add(1, Ordering::SeqCst);
                    json!({"protocolVersion":"2025-03-26","capabilities":{"tools":{}},"serverInfo":{"name":"fixture","version":"1"}})
                }
                "notifications/initialized" => return ResponseTemplate::new(202),
                "tools/list" => json!({"tools":[{"name":"query_business_data","description":"Query business data","inputSchema":{"type":"object","properties":{"queries":{"type":"array","items":{"type":"object"}}},"required":["queries"]}}]}),
                "tools/call" => {
                    assert_eq!(body["params"]["name"], "query_business_data");
                    assert!(body["params"]["arguments"]["queries"].is_array());
                    seen_queries.fetch_add(1, Ordering::SeqCst);
                    json!({"content":[{"type":"text","text":"[]"}],"isError":false})
                }
                method => panic!("unexpected MCP method: {method}"),
            };
            ResponseTemplate::new(200).set_body_json(json!({"jsonrpc":"2.0","id":body["id"],"result":result}))
        }).mount(&server).await;

    let model_turn = Arc::new(AtomicUsize::new(0));
    let seen_turn = model_turn.clone();
    Mock::given(method("POST")).and(path("/v1/chat/completions"))
        .respond_with(move |_: &Request| {
            let turn = seen_turn.fetch_add(1, Ordering::SeqCst);
            let (delta, finish) = match turn {
                0 => (json!({"tool_calls":[{"index":0,"id":"search-1","type":"function","function":{"name":"ToolSearch","arguments":"{\"query\":\"query_business_data\"}"}}]}), "tool_calls"),
                1 | RESUME_REQUEST => (json!({"tool_calls":[{"index":0,"id":"query-1","type":"function","function":{"name":"query_business_data","arguments":"{\"queries\":[]}"}}]}), "tool_calls"),
                _ => (json!({"content":"The query completed. Continue using the available business tools."}), "stop"),
            };
            let chunk = json!({"id":"completion","object":"chat.completion.chunk","model":"test","choices":[{"index":0,"delta":delta,"finish_reason":null}]});
            let end = json!({"id":"completion","object":"chat.completion.chunk","model":"test","choices":[{"index":0,"delta":{},"finish_reason":finish}]});
            ResponseTemplate::new(200).insert_header("content-type", "text/event-stream")
                .set_body_string(format!("data: {chunk}\n\ndata: {end}\n\ndata: [DONE]\n\n"))
        }).mount(&server).await;

    let workspace = tempfile::tempdir().unwrap();
    let mut config = minimal_config();
    config.base_url = format!("{}/v1", server.uri());
    config.tools.auto_approve = true;
    config.session.enabled = true;
    config.session.directory = workspace.path().join("sessions").display().to_string();
    config.mcp.servers.insert(
        "gea-gateway".into(),
        McpServerConfig {
            transport: TransportType::StreamableHttp,
            command: None,
            args: None,
            env: None,
            url: Some(format!("{}/mcp", server.uri())),
            headers: None,
            deferred: Some(true),
            startup_timeout_ms: Some(1000),
        },
    );
    let mut runtime = AgentBootstrap::new(config.clone(), workspace.path().display().to_string(), null_output())
        .build()
        .await
        .unwrap();
    assert!(runtime.has_mcp);
    runtime
        .engine
        .init_session(
            "openai",
            &workspace.path().display().to_string(),
            Some("old-conversation"),
        )
        .unwrap();
    runtime.engine.run("Query the business data", "first").await.unwrap();
    assert_eq!(queries.load(Ordering::SeqCst), 1);
    for turn in 0..FOLLOW_UPS {
        runtime
            .engine
            .run("Continue with the same tools", &format!("follow-up-{turn}"))
            .await
            .unwrap();
    }
    let sessions = SessionManager::new(config.session.directory.clone().into(), 100);
    assert!(
        sessions.load("old-conversation").unwrap().messages.len() >= 72,
        "exercise a long existing conversation before compaction"
    );
    runtime.engine.run("/compact", "compact").await.unwrap();
    runtime
        .engine
        .run("Continue after compaction", "after-compact")
        .await
        .unwrap();

    let saved = sessions.load("old-conversation").unwrap();
    assert_eq!(saved.activated_tools, vec!["query_business_data"]);
    assert!(
        saved
            .messages
            .iter()
            .flat_map(|m| &m.content)
            .any(|block| { matches!(block, ContentBlock::Text { text } if text.contains("[Conversation compacted]")) }),
        "the test must actually compact history"
    );
    drop(runtime);
    let mut resumed = AgentBootstrap::new(config, workspace.path().display().to_string(), null_output())
        .resume(saved)
        .build()
        .await
        .unwrap();
    resumed
        .engine
        .run("Query again in the restored conversation", "resumed")
        .await
        .unwrap();
    assert_eq!(connections.load(Ordering::SeqCst), 2, "resume must reconnect MCP");
    assert_eq!(
        queries.load(Ordering::SeqCst),
        2,
        "the restored MCP connection must execute the second query"
    );

    let requests = server.received_requests().await.unwrap();
    let model_requests: Vec<Value> = requests
        .iter()
        .filter(|r| r.url.path() == "/v1/chat/completions")
        .map(|r| r.body_json().unwrap())
        .collect();
    assert!(
        model_requests.len() == FOLLOW_UPS + 7,
        "must exercise tool results, follow-up, compaction and resume"
    );
    let ordinary: Vec<_> = model_requests.iter().filter(|body| body["tools"].is_array()).collect();
    assert_eq!(
        ordinary.len(),
        FOLLOW_UPS + 6,
        "only the compaction summarizer should omit tools"
    );
    let initial_count = ordinary[0]["tools"].as_array().unwrap().len();
    for (index, request) in ordinary.iter().enumerate() {
        let tools = request["tools"].as_array().unwrap();
        assert_eq!(
            tools.len(),
            initial_count,
            "MCP tools disappeared from model request {index}"
        );
        let query = tools
            .iter()
            .find(|t| t["function"]["name"] == "query_business_data")
            .unwrap();
        if index == 0 {
            assert!(
                query["function"]["parameters"]["properties"]
                    .as_object()
                    .unwrap()
                    .is_empty()
            );
        } else {
            assert_eq!(
                query["function"]["parameters"]["properties"]["queries"]["type"],
                "array"
            );
        }
    }
}
