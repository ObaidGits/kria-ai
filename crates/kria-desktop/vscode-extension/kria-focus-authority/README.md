# KRIA Focus Authority VS Code Extension

Manual development install:

```bash
cd crates/kria-desktop/vscode-extension/kria-focus-authority
code --extensionDevelopmentPath="$PWD"
```

The extension exposes a localhost-only metadata endpoint:

```text
http://127.0.0.1:47323/focus
```

It reports active editor/terminal focus metadata only. It never returns source
code contents, selected text contents, clipboard contents, or full workspace
paths. KRIA rejects responses older than 1000 ms.
