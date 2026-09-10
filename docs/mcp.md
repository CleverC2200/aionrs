# MCP (Model Context Protocol) Integration

## Overview

MCP allows the agent to connect to external tool servers, extending the bundled tool suite with tools from the MCP server ecosystem.

## Configuring MCP Servers

Declare MCP servers in the config file:

```toml
# Stdio transport: launch a local subprocess
[mcp.servers.filesystem]
transport = "stdio"
command = "npx"
args = ["-y", "@modelcontextprotocol/server-filesystem", "/Users/me/project"]

[mcp.servers.github]
transport = "stdio"
command = "npx"
args = ["-y", "@modelcontextprotocol/server-github"]
env = { GITHUB_TOKEN = "ghp_xxx" }
startup_timeout_ms = 30000

# SSE transport: connect to a remote SSE server
[mcp.servers.database]
transport = "sse"
url = "http://localhost:3001/sse"

# Streamable HTTP transport: HTTP POST communication
[mcp.servers.remote-tools]
transport = "streamable-http"
url = "https://tools.example.com/mcp"
headers = { Authorization = "Bearer xxx" }
```

## Transport Types

| Transport | Description | Use Case |
|-----------|-------------|----------|
| `stdio` | Launch local subprocess, communicate via stdin/stdout | Local MCP servers (npx, uvx) |
| `sse` | GET for SSE event stream, POST for requests | Remote MCP servers |
| `streamable-http` | HTTP POST, supports SSE streaming responses | Remote MCP servers |

## Startup Timeout

Configured MCP servers are connected concurrently during startup. Each server
has a startup timeout covering transport connection, `initialize`, and
`tools/list`. The default is `30000` milliseconds.

```toml
[mcp.servers.slow-tools]
transport = "stdio"
command = "npx"
args = ["-y", "slow-mcp-server"]
startup_timeout_ms = 60000
```

Increase `startup_timeout_ms` for servers that need extra time for first-run
setup, package downloads, remote authentication, or slow network handshakes.

## Deferred Loading

MCP tools can be registered as "deferred" — their full schema is not loaded into the system prompt at startup, reducing initial token usage. The LLM discovers deferred tools via the `ToolSearch` tool when needed.

```toml
[mcp.servers.large-toolset]
transport = "stdio"
command = "npx"
args = ["-y", "my-mcp-server"]
deferred = true    # Don't load tool schemas at startup
```

| `deferred` | Behavior |
|------------|----------|
| `false` (default for config servers) | Tool schemas included in system prompt at startup |
| `true` | Tools registered but schemas loaded on-demand via ToolSearch |

Use `deferred = true` for MCP servers with many tools to keep the initial system prompt small.

After a successful `ToolSearch`, matching schemas are included in subsequent model
requests. Activation is stored independently of message history and survives
compaction and session resume. Resume reconnects configured MCP servers and resolves
saved tool names against their current schemas; it never restores stale schemas or
bypasses the current tool policy. Plan mode and tool-free summarization requests
continue to apply their normal restrictions.

## Tool Naming

- MCP tool names are used directly when there's no conflict
- On conflict with built-in or other MCP tools, names are auto-prefixed: `mcp__{server}__{tool}`

## Tool Resource Links

Tool results containing `resource_link` are resolved with `resources/read` on
the same MCP server and session. Small resource text is included inline.
Resources larger than 4,000 bytes instead return a `ReadMcpResource` call hint,
so they do not overflow the model's tool-output budget. This read-only tool
accepts `server`, `uri`, a JSON `pointer` (empty for the root), and `offset`.
Large JSON containers return a paginated index of child pointers and names;
follow a child pointer to read its value, or `next_offset` to continue the index.
Large strings and non-JSON text use character-offset pages. Index pages are
explicitly incomplete, never partial JSON presented as complete data.

Only resources returned by tools in the current manager session can be read
through this tool; each read still goes through the originating server's
authorization and validation. Nothing is persisted locally. The URI is never
fetched directly over HTTP or read from the filesystem. A resource read failure remains a tool
error instead of returning an incomplete result as success. Binary-only
resources are not supported by this text-output path.

## Startup Flow

1. Connect to all configured MCP servers
2. Perform MCP protocol handshake (`initialize`) for each server
3. Discover available tools (`tools/list`)
4. Register tools in the tool registry — the agent uses them like built-in tools
5. Gracefully close all connections on exit
