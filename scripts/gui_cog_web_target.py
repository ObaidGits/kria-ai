#!/usr/bin/env python3
"""
Phase 2 — deterministic WEB TARGET for GUI Cognition click/type verification.

A tiny stdlib HTTP server that serves a controlled form page and RECORDS what
actually happens in the DOM (typed text, button clicks, form submit). The test
harness drives KRIA to type/click on this page, then reads the recorded events
back — REAL ground truth of the in-page effect, with NO CDP/websocket/deps.

The page also mirrors the live input value into document.title
("KGTEST|field=<value>|last=<event>") so xdotool can read it as a second check.

Endpoints:
  GET  /            -> the form page (text field is autofocused)
  POST /record      -> page posts {type, id, value} here on input/click/submit
  GET  /state       -> JSON list of recorded events
  GET  /reset       -> clear recorded events

Run:  python3 scripts/gui_cog_web_target.py [port]   (default 8765)
"""
import json
import sys
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

PORT = int(sys.argv[1]) if len(sys.argv) > 1 else 8765
_events = []
_lock = threading.Lock()

PAGE = """<!doctype html><html><head><meta charset="utf-8">
<title>KGTEST|field=|last=</title>
<style>body{font:18px sans-serif;margin:40px;background:#f4f4f8}
input,button{font:18px sans-serif;padding:10px;margin:8px;display:block}
button{cursor:pointer}</style></head>
<body>
<h2>KRIA GUI Cognition Test Form</h2>
<form id="f" onsubmit="rec('submit', 'f', document.getElementById('field').value); return false;">
  <input id="field" name="field" type="text" autofocus placeholder="type here"
         oninput="rec('input','field',this.value); document.title='KGTEST|field='+this.value+'|last=input'">
  <button id="save" type="button" onclick="rec('click','save','')">Save</button>
  <button id="cancel" type="button" onclick="rec('click','cancel','')">Cancel</button>
  <button id="submit" type="submit">Submit</button>
</form>
<pre id="log"></pre>
<script>
function rec(type,id,value){
  document.title='KGTEST|field='+(document.getElementById('field').value)+'|last='+type+':'+id;
  fetch('/record',{method:'POST',headers:{'Content-Type':'application/json'},
    body:JSON.stringify({type:type,id:id,value:value})}).catch(()=>{});
  var l=document.getElementById('log'); l.textContent+=type+' '+id+' '+value+'\\n';
}
window.addEventListener('load',function(){document.getElementById('field').focus();});
</script>
</body></html>"""


class H(BaseHTTPRequestHandler):
    def log_message(self, *a):  # silence
        pass

    def _send(self, code, body, ctype="application/json"):
        b = body.encode("utf-8")
        self.send_response(code)
        self.send_header("Content-Type", ctype)
        self.send_header("Content-Length", str(len(b)))
        self.end_headers()
        self.wfile.write(b)

    def do_GET(self):
        if self.path == "/" or self.path.startswith("/?"):
            self._send(200, PAGE, "text/html; charset=utf-8")
        elif self.path == "/state":
            with _lock:
                self._send(200, json.dumps(_events))
        elif self.path == "/reset":
            with _lock:
                _events.clear()
            self._send(200, json.dumps({"ok": True}))
        else:
            self._send(404, json.dumps({"error": "not found"}))

    def do_POST(self):
        if self.path == "/record":
            n = int(self.headers.get("Content-Length", "0"))
            try:
                ev = json.loads(self.rfile.read(n).decode("utf-8"))
            except Exception:  # noqa: BLE001
                ev = {"type": "parse_error"}
            with _lock:
                _events.append(ev)
            self._send(200, json.dumps({"ok": True}))
        else:
            self._send(404, json.dumps({"error": "not found"}))


def main():
    srv = ThreadingHTTPServer(("127.0.0.1", PORT), H)
    print(f"[web-target] serving http://127.0.0.1:{PORT}/  (Ctrl+C to stop)")
    try:
        srv.serve_forever()
    except KeyboardInterrupt:
        pass


if __name__ == "__main__":
    main()
