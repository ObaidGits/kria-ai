#!/usr/bin/env bash
# Verify extension v2.2.0 CaptureScreen sees native Wayland windows.
# Opens a long file in Text Editor, activates it, captures via extension,
# OCRs the PNG — real window content (line N / "Text Editor") => capture works.
set -u
TOK=$(cat "$HOME/.kria/gui_ext_token" 2>/dev/null)
D=ai.kria.ActiveWindow; OP=/ai/kria/ActiveWindow; IF=ai.kria.ActiveWindow
call(){ gdbus call --session --dest "$D" --object-path "$OP" --method "$IF.$1" "${@:2}"; }

echo "== Ping (expect 2.2.1) =="
busctl --user call "$D" "$OP" "$IF" Ping s "$TOK"
echo

seq 1 500 | sed 's/^/line /' > /tmp/kria_scroll_test.txt
gio launch /usr/share/applications/org.gnome.TextEditor.desktop /tmp/kria_scroll_test.txt >/dev/null 2>&1 &
sleep 4
# focus the text editor window
LW=$(busctl --user call "$D" "$OP" "$IF" ListWindows s "$TOK")
ID=$(echo "$LW" | python3 -c '
import sys,re,json
s=sys.stdin.read().strip()
if s.startswith("s "): s=s[2:].strip().strip("\"")
d=json.loads(s.encode().decode("unicode_escape"))
for w in d.get("windows",[]):
    if "TextEditor" in (w.get("wm_class") or ""): print(w["id"]); break
')
echo "TextEditor window id=$ID"
busctl --user call "$D" "$OP" "$IF" ActivateWindow ss "$TOK" "$ID"
sleep 1
OUT=/tmp/kria_cap_probe.png
rm -f "$OUT"
echo "== CaptureScreen =="
busctl --user call "$D" "$OP" "$IF" CaptureScreen ss "$TOK" "$OUT"
sleep 1
ls -l "$OUT" 2>&1
echo "== OCR of captured frame (expect 'line N' / 'Text Editor') =="
tesseract "$OUT" - 2>/dev/null | grep -iE "line [0-9]|Text Editor" | head -8
echo "== distinct-content check =="
python3 -c "
from PIL import Image
im=Image.open('$OUT').convert('RGB')
c=im.getcolors(maxcolors=10_000_000) or []
print('distinct colors:', len(c), 'size', im.size)
"
