const http = require("http");
const crypto = require("crypto");
const path = require("path");
const vscode = require("vscode");

let server;
let latest = null;

function nowMs() {
  return Date.now();
}

function hash(value) {
  return crypto.createHash("sha256").update(String(value || "")).digest("hex").slice(0, 16);
}

function sanitize(value, limit = 120) {
  if (!value) return null;
  const compact = String(value).replace(/\s+/g, " ").trim();
  if (!compact) return null;
  return compact.slice(0, limit);
}

function workspaceSummary() {
  const folder = vscode.workspace.workspaceFolders && vscode.workspace.workspaceFolders[0];
  if (!folder) return null;
  return {
    name: sanitize(folder.name, 80),
    hash: hash(folder.uri.fsPath),
  };
}

function windowFocused() {
  return !vscode.window.state || vscode.window.state.focused !== false;
}

function buildFocusSnapshot() {
  if (!windowFocused()) {
    return {
      status: "unavailable",
      observed_at_ms: nowMs(),
      reason: "VS Code window is not foreground focused",
      focused_app: "VS Code",
      window_focused: false,
      confidence: 0,
      reliability: "unavailable",
    };
  }

  const activeEditor = vscode.window.activeTextEditor;
  const activeTerminal = vscode.window.activeTerminal;
  const workspace = workspaceSummary();

  if (activeEditor) {
    const document = activeEditor.document;
    const selection = activeEditor.selection;
    const basename = path.basename(document.fileName || document.uri.path || "untitled");
    return {
      status: "ok",
      observed_at_ms: nowMs(),
      focused_app: "VS Code",
      focused_window: sanitize(`${basename} - Visual Studio Code`, 140),
      focused_control_id: `vscode:editor:${hash(document.uri.toString())}`,
      focused_control_label: "VS Code editor",
      focused_control_role: "editor",
      editable_target_known: true,
      text_cursor_known: true,
      terminal_like: false,
      window_focused: true,
      cursor_line: selection.active.line + 1,
      cursor_column: selection.active.character + 1,
      selected_text_length: document.getText(selection).length,
      file_basename: sanitize(basename, 80),
      workspace,
      confidence: 0.95,
      reliability: "reliable",
    };
  }

  if (activeTerminal) {
    return {
      status: "ok",
      observed_at_ms: nowMs(),
      focused_app: "VS Code",
      focused_window: "Terminal - Visual Studio Code",
      focused_control_id: `vscode:terminal:${hash(activeTerminal.name)}`,
      focused_control_label: "VS Code integrated terminal",
      focused_control_role: "terminal",
      editable_target_known: false,
      text_cursor_known: false,
      terminal_like: true,
      window_focused: true,
      terminal_name_hash: hash(activeTerminal.name),
      workspace,
      confidence: 0.95,
      reliability: "reliable",
    };
  }

  return {
    status: "unavailable",
    observed_at_ms: nowMs(),
    reason: "VS Code focus adapter has no active editor or terminal",
    focused_app: "VS Code",
    window_focused: true,
    confidence: 0,
    reliability: "unavailable",
  };
}

function refreshFocus() {
  latest = buildFocusSnapshot();
}

function activate(context) {
  refreshFocus();
  context.subscriptions.push(vscode.window.onDidChangeActiveTextEditor(refreshFocus));
  context.subscriptions.push(vscode.window.onDidChangeActiveTerminal(refreshFocus));
  context.subscriptions.push(vscode.window.onDidChangeTextEditorSelection(refreshFocus));
  context.subscriptions.push(vscode.window.onDidChangeWindowState(refreshFocus));

  const port = Number(process.env.KRIA_VSCODE_FOCUS_PORT || "47323");
  server = http.createServer((req, res) => {
    if (req.url !== "/focus") {
      res.writeHead(404, { "Content-Type": "application/json" });
      res.end(JSON.stringify({ status: "unavailable", reason: "unknown endpoint" }));
      return;
    }
    refreshFocus();
    res.writeHead(200, {
      "Content-Type": "application/json",
      "Access-Control-Allow-Origin": "http://127.0.0.1",
    });
    res.end(JSON.stringify(latest));
  });
  server.listen(port, "127.0.0.1");
  context.subscriptions.push({
    dispose() {
      if (server) server.close();
    },
  });
}

function deactivate() {
  if (server) server.close();
}

module.exports = { activate, deactivate };
