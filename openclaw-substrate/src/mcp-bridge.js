"use strict";

/**
 * OpenClaw MCP Bridge — Content-Length framed JSON-RPC over stdio.
 *
 * This is the execution engine running inside each ephemeral container.
 * It reads skill definitions from /app/skills/*.json and exposes them
 * via the MCP (Model Context Protocol) JSON-RPC interface.
 *
 * Protocol: Content-Length framed messages on stdin/stdout (LSP-style).
 * - Request:  Content-Length: N\r\n\r\n{...json...}
 * - Response: Content-Length: N\r\n\r\n{...json...}
 */

const fs = require("fs");
const path = require("path");

// ─── Skill Loading ───────────────────────────────────────────────────────────

const SKILLS_DIR = path.join(__dirname, "..", "skills");

function loadSkills() {
  const skills = [];
  if (!fs.existsSync(SKILLS_DIR)) return skills;

  const files = fs.readdirSync(SKILLS_DIR).filter((f) => f.endsWith(".json"));
  for (const file of files) {
    try {
      const raw = fs.readFileSync(path.join(SKILLS_DIR, file), "utf8");
      const skill = JSON.parse(raw);
      if (skill.name && skill.description) {
        skills.push(skill);
      }
    } catch (e) {
      process.stderr.write(`[mcp-bridge] Failed to load skill ${file}: ${e.message}\n`);
    }
  }
  return skills;
}

const skills = loadSkills();

// ─── Content-Length Frame Parser ─────────────────────────────────────────────

class ContentLengthParser {
  constructor(onMessage) {
    this.onMessage = onMessage;
    this.buffer = Buffer.alloc(0);
    this.contentLength = -1;
  }

  feed(chunk) {
    this.buffer = Buffer.concat([this.buffer, chunk]);
    this._parse();
  }

  _parse() {
    while (true) {
      if (this.contentLength === -1) {
        // Look for header terminator
        const headerEnd = this.buffer.indexOf("\r\n\r\n");
        if (headerEnd === -1) return;

        const header = this.buffer.slice(0, headerEnd).toString("utf8");
        const match = header.match(/Content-Length:\s*(\d+)/i);
        if (!match) {
          // Malformed header — skip past it
          this.buffer = this.buffer.slice(headerEnd + 4);
          continue;
        }

        this.contentLength = parseInt(match[1], 10);
        this.buffer = this.buffer.slice(headerEnd + 4);
      }

      // Wait for full body
      if (this.buffer.length < this.contentLength) return;

      const body = this.buffer.slice(0, this.contentLength).toString("utf8");
      this.buffer = this.buffer.slice(this.contentLength);
      this.contentLength = -1;

      try {
        const message = JSON.parse(body);
        this.onMessage(message);
      } catch (e) {
        process.stderr.write(`[mcp-bridge] Failed to parse JSON: ${e.message}\n`);
      }
    }
  }
}

// ─── Response Writer ─────────────────────────────────────────────────────────

function sendResponse(obj) {
  const body = JSON.stringify(obj);
  const header = `Content-Length: ${Buffer.byteLength(body, "utf8")}\r\n\r\n`;
  process.stdout.write(header + body);
}

function jsonRpcResult(id, result) {
  sendResponse({ jsonrpc: "2.0", id, result });
}

function jsonRpcError(id, code, message) {
  sendResponse({ jsonrpc: "2.0", id, error: { code, message } });
}

// ─── MCP Method Handlers ─────────────────────────────────────────────────────

function handleInitialize(id, _params) {
  jsonRpcResult(id, {
    protocolVersion: "2024-11-05",
    capabilities: {
      tools: { listChanged: false },
    },
    serverInfo: {
      name: "openclaw-substrate",
      version: "1.0.0",
    },
  });
}

function handleToolsList(id, _params) {
  const tools = skills.map((skill) => ({
    name: skill.name,
    description: skill.description || "",
    inputSchema: skill.inputSchema || { type: "object", properties: {} },
  }));
  jsonRpcResult(id, { tools });
}

function handleToolsCall(id, params) {
  const { name, arguments: args } = params || {};

  const skill = skills.find((s) => s.name === name);
  if (!skill) {
    jsonRpcError(id, -32602, `Unknown tool: ${name}`);
    return;
  }

  // Execute the skill handler if it has one, otherwise return a stub
  try {
    let result;
    if (skill.handler && typeof skill.handler === "string") {
      // Load and execute handler file
      const handlerPath = path.join(SKILLS_DIR, skill.handler);
      if (fs.existsSync(handlerPath)) {
        const handler = require(handlerPath);
        result = typeof handler === "function" ? handler(args || {}) : handler;
      } else {
        result = { output: `Skill '${name}' handler not found at ${skill.handler}` };
      }
    } else {
      result = { output: `Skill '${name}' executed with args: ${JSON.stringify(args || {})}` };
    }

    jsonRpcResult(id, {
      content: [{ type: "text", text: typeof result === "string" ? result : JSON.stringify(result) }],
      isError: false,
    });
  } catch (e) {
    jsonRpcResult(id, {
      content: [{ type: "text", text: `Error: ${e.message}` }],
      isError: true,
    });
  }
}

// ─── Notifications (no response needed) ──────────────────────────────────────

function handleNotification(method, _params) {
  // notifications/initialized — client confirms init complete
  if (method === "notifications/initialized") {
    process.stderr.write("[mcp-bridge] Client initialized successfully.\n");
  }
}

// ─── Message Dispatch ────────────────────────────────────────────────────────

function handleMessage(msg) {
  const { id, method, params } = msg;

  // Notification (no id)
  if (id === undefined || id === null) {
    handleNotification(method, params);
    return;
  }

  switch (method) {
    case "initialize":
      handleInitialize(id, params);
      break;
    case "tools/list":
      handleToolsList(id, params);
      break;
    case "tools/call":
      handleToolsCall(id, params);
      break;
    default:
      jsonRpcError(id, -32601, `Method not found: ${method}`);
  }
}

// ─── Main ────────────────────────────────────────────────────────────────────

const parser = new ContentLengthParser(handleMessage);

process.stdin.on("data", (chunk) => {
  parser.feed(chunk);
});

process.stdin.on("end", () => {
  process.exit(0);
});

process.stderr.write(`[mcp-bridge] OpenClaw substrate ready. ${skills.length} skill(s) loaded.\n`);
