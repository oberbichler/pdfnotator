#!/usr/bin/env bash
# Record docs/demo.gif and docs/demo.mp4: a scripted user (PDFNOTATOR_DEMO=animate)
# comments on examples/proposal.typ while scripts/demo-agent.py acts as the agent.
# macOS only (uses screencapture). Requires a release build, typst and ffmpeg.
set -euo pipefail
cd "$(dirname "$0")/.."

BIN=src-tauri/target/release/bundle/macos/PDFnotator.app/Contents/MacOS/pdfnotator
PORT=${PORT:-4848}
WORK=$(mktemp -d)
trap 'kill -TERM ${APP:-} ${AGENT:-} 2>/dev/null || true; rm -rf "$WORK"' EXIT

cp examples/proposal.typ "$WORK/proposal.typ"       # the agent edits a copy
cp examples/report.typ "$WORK/report.typ"           # opened as a second tab at the end
PDFNOTATOR_PORT=$PORT PDFNOTATOR_DEMO=animate "$BIN" "$WORK/proposal.typ" >/dev/null 2>&1 &
APP=$!
until curl -sf "http://127.0.0.1:$PORT/api/status" | grep -q proposal; do sleep 0.2; done

WID=$(swift -e "import CoreGraphics
let l = CGWindowListCopyWindowInfo(.optionAll, kCGNullWindowID) as! [[String: Any]]
for w in l where (w[\"kCGWindowOwnerPID\"] as? Int) == $APP && (w[\"kCGWindowLayer\"] as? Int) == 0
  && ((w[\"kCGWindowBounds\"] as? [String: Double])?[\"Height\"] ?? 0) > 500 { print(w[\"kCGWindowNumber\"]!); break }")

python3 scripts/demo-agent.py "$PORT" "$WORK/proposal.typ" "$WORK/done" "$WORK/report.typ" 2>/dev/null &
AGENT=$!

# capture the window as fast as possible, remembering when each frame was taken
mkdir "$WORK/frames"
python3 - "$WID" "$WORK/frames" "$WORK/done" <<'PY'
import os, subprocess, sys, time
wid, out, done = sys.argv[1], sys.argv[2], sys.argv[3]
start, stop_at, i, times = time.time(), None, 0, []
while stop_at is None or time.time() < stop_at:
    t = time.time() - start
    subprocess.run(["screencapture", "-x", "-o", f"-l{wid}", "-t", "png", f"{out}/{i:05d}.png"], check=True)
    times.append(t); i += 1
    if stop_at is None and os.path.exists(done):
        stop_at = time.time() + 3.5                   # agent listens again: keep the end visible
with open(f"{out}/list.txt", "w") as f:               # ffmpeg concat list with real frame durations
    for n, (a, b) in enumerate(zip(times, times[1:] + [times[-1] + 0.1])):
        f.write(f"file '{n:05d}.png'\nduration {b - a:.3f}\n")
PY

FILTER="fps=15,scale=1200:-2:flags=lanczos"
ffmpeg -y -loglevel error -f concat -safe 0 -i "$WORK/frames/list.txt" \
  -vf "$FILTER" -c:v libx264 -pix_fmt yuv420p -crf 20 -movflags +faststart docs/demo.mp4
# GIF from the video: the encoder smooths capture noise, which makes the GIF ~10× smaller
ffmpeg -y -loglevel error -i docs/demo.mp4 \
  -vf "fps=12,scale=1100:-2:flags=lanczos,split[a][b];[a]palettegen=max_colors=160:stats_mode=diff[p];[b][p]paletteuse=dither=none:diff_mode=rectangle" \
  docs/demo.gif
ls -lh docs/demo.gif docs/demo.mp4
