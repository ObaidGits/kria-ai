#!/usr/bin/env bash
# Decisive probe: does extension v2.1.0 (Main.activateWindow) raise a
# background window on this GNOME Wayland session?
# Picks a non-focused normal window, activates it, re-checks focus.
set -u
TOK=$(cat "$HOME/.kria/gui_ext_token" 2>/dev/null)
call() { busctl --user call ai.kria.ActiveWindow /ai/kria/ActiveWindow ai.kria.ActiveWindow "$@" 2>&1; }

echo "== Ping (expect version 2.1.0) =="
call Ping s "$TOK"
echo

echo "== ListWindows =="
LW=$(call ListWindows s "$TOK")
echo "$LW" | python3 -c '
import sys,json,re
raw=sys.stdin.read()
m=re.search(r"\"\{.*\}\"",raw,re.S) or re.search(r"\{.*\}",raw,re.S)
s=raw
# busctl prints: s "<json>"  -> strip leading  s "
s=s.strip()
if s.startswith("s "): s=s[2:].strip()
s=s.strip().strip("\"")
s=s.encode().decode("unicode_escape")
d=json.loads(s)
wins=d.get("windows",[])
focused=[w for w in wins if w.get("focused")]
notf=[w for w in wins if not w.get("focused") and w.get("wm_class") not in ("gjs",)]
print("focused:",[(w["id"],w["title"]) for w in focused])
print("candidates:",[(w["id"],w["title"]) for w in notf])
import os
if notf:
    open("/tmp/_probe_target.txt","w").write(notf[0]["id"])
    print("TARGET_ID="+notf[0]["id"])
'
echo
TARGET=$(cat /tmp/_probe_target.txt 2>/dev/null)
if [ -z "$TARGET" ]; then echo "no background candidate found"; exit 1; fi
echo "== ActivateWindow id=$TARGET =="
call ActivateWindow ss "$TOK" "$TARGET"
echo
sleep 1
echo "== GetFocusedWindow (expect id=$TARGET) =="
call GetFocusedWindow s "$TOK"
