#!/usr/bin/env node
"use strict";

/**
 * Minimal standards-compliant MCP stdio server (newline-delimited JSON-RPC 2.0).
 *
 * Used by CPP Milestone-6 tests to validate the provider-neutral boundary with a
 * SECOND, non-OpenClaw provider — proving the architecture is not overfit to
 * OpenClaw. It implements exactly the MCP subset KRIA's `mcp::client::McpClient`
 * speaks: `initialize`, `notifications/initialized`, `tools/list`, `tools/call`,
 * `ping`.
 *
 * Transport: one JSON object per line on stdin/stdout (matches the line-delimited
 * BufReader transport in `crates/kria-core/src/mcp/client.rs`).
 *
 * Tools exposed:
 *  - reverse_text: reverses the input string
 *  - word_count:   counts words in the input string
 */

const TOOLS = [
  {
    name: "reverse_text",
    description: "Reverse the characters of a text string and return the reversed string.",
    inputSchema: {
      type: "object",
      properties: { text: { type: "string", description: "Text to reverse" } },
      required: ["text"],
    },
  },
  {
    name: "word_count",
    description: "Count the number of words in a text string.",
    inputSchema: {
      type: "object",
      properties: { text: { type: "string", description: "Text to count words in" } },
      required: ["text"],
    },
  },
];

function send(obj) {
  process.stdout.write(JSON.stringify(obj) + "\n");
}

function result(id, res) {
  send({ jsonrpc: "2.0", id, result: res });
}

function error(id, code, message) {
  send({ jsonrpc: "2.0", id, error: { code, message } });
}

function handleCall(id, params) {
  const name = params && params.name;
  const args = (params && params.arguments) || {};
  const tool = TOOLS.find((t) => t.name === name);
  if (!tool) {
    error(id, -32602, `Unknown tool: ${name}`);
    return;
  }
  let text;
  try {
    if (name === "reverse_text") {
      text = String(args.text || "").split("").reverse().join("");
    } else if (name === "word_count") {
      const words = String(args.text || "").trim().split(/\s+/).filter(Boolean);
      text = JSON.stringify({ words: words.length });
    } else {
      text = "";
    }
  } catch (e) {
    result(id, { content: [{ type: "text", text: `Error: ${e.message}` }], isError: true });
    return;
  }
  result(id, { content: [{ type: "text", text }], isError: false });
}

function handle(msg) {
  const { id, method, params } = msg;
  // Notifications carry no id → no response.
  if (id === undefined || id === null) {
    return;
  }
  switch (method) {
    case "initialize":
      result(id, {
        protocolVersion: "2024-11-05",
        capabilities: { tools: {} },
        serverInfo: { name: "kria-mcp-stub", version: "1.0.0" },
      });
      break;
    case "tools/list":
      result(id, { tools: TOOLS });
      break;
    case "tools/call":
      handleCall(id, params);
      break;
    case "ping":
      result(id, {});
      break;
    default:
      error(id, -32601, `Method not found: ${method}`);
  }
}

let buffer = "";
process.stdin.setEncoding("utf8");
process.stdin.on("data", (chunk) => {
  buffer += chunk;
  let nl;
  while ((nl = buffer.indexOf("\n")) !== -1) {
    const line = buffer.slice(0, nl).trim();
    buffer = buffer.slice(nl + 1);
    if (!line) continue;
    try {
      handle(JSON.parse(line));
    } catch (e) {
      process.stderr.write(`[mcp-stub] parse error: ${e.message}\n`);
    }
  }
});
process.stdin.on("end", () => process.exit(0));
process.stderr.write("[mcp-stub] ready\n");
