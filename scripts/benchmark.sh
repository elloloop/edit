#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 1 ]]; then
  echo "Usage: ./scripts/benchmark.sh path/to/project" >&2
  exit 1
fi

PROJECT="$(cd "$1" && pwd)"
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TMP_DIR="$(mktemp -d)"
LARGE_FILE="$TMP_DIR/large.txt"
trap 'rm -rf "$TMP_DIR"; cleanup_pid "${EDIT_PID:-}"; cleanup_pid "${GUI_PID:-}"; cleanup_pid "${CODE_PID:-}"' EXIT

cleanup_pid() {
  local pid="${1:-}"
  if [[ -n "$pid" ]] && kill -0 "$pid" 2>/dev/null; then
    kill "$pid" 2>/dev/null || true
    wait "$pid" 2>/dev/null || true
  fi
}

rss_kb() {
  ps -o rss= -p "$1" | tr -d ' '
}

startup_ms() {
  "$ROOT/target/debug/edit" --benchmark "$1" | tail -n 1 | tr -d '[:space:]'
}

launch_rss() {
  local name="$1"
  shift
  "$@" >/dev/null 2>&1 &
  local pid=$!
  sleep 5
  local rss="$(rss_kb "$pid")"
  printf "%s\t%s\t%s\n" "$name" "$pid" "$rss"
}

launch_tui_rss() {
  local name="$1"
  shift
  script -q /dev/null "$@" >/dev/null 2>&1 &
  local wrapper_pid=$!
  sleep 5
  local child_pid
  child_pid="$(pgrep -fn "$1.*$PROJECT" || true)"
  local rss=""
  if [[ -n "$child_pid" ]]; then
    rss="$(rss_kb "$child_pid")"
  fi
  printf "%s\t%s\t%s\n" "$name" "$wrapper_pid" "$rss"
}

python3 - <<'PY' > "$LARGE_FILE"
for i in range(50000):
    print(f"line {i:05d} lorem ipsum dolor sit amet")
PY

(cd "$ROOT" && cargo build -p edit -p edit-gui >/dev/null)

EDIT_STARTUP="$(startup_ms "$PROJECT")"
GUI_STARTUP="n/a"
CODE_STARTUP="n/a"
LARGE_STARTUP="$(startup_ms "$LARGE_FILE")"

read -r _ EDIT_PID EDIT_RSS < <(launch_tui_rss "edit" "$ROOT/target/debug/edit" "$PROJECT")
read -r _ GUI_PID GUI_RSS < <(launch_rss "edit-gui" "$ROOT/target/debug/edit-gui" "$PROJECT")

if command -v code >/dev/null 2>&1; then
  CODE_STARTUP="$(python3 - "$PROJECT" <<'PY'
import subprocess, sys, time
project = sys.argv[1]
start = time.perf_counter()
proc = subprocess.Popen(["code", "--new-window", project], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
time.sleep(1.5)
print(int((time.perf_counter() - start) * 1000))
proc.terminate()
try:
    proc.wait(timeout=5)
except Exception:
    proc.kill()
PY
)"
  read -r _ CODE_PID CODE_RSS < <(launch_rss "code" code --new-window "$PROJECT")
else
  CODE_RSS="n/a"
fi

printf "%-10s | %-10s | %-10s | %-14s\n" "editor" "rss_kb" "startup_ms" "large_file_ms"
printf "%-10s-+-%-10s-+-%-10s-+-%-14s\n" "----------" "----------" "----------" "--------------"
printf "%-10s | %-10s | %-10s | %-14s\n" "edit" "$EDIT_RSS" "$EDIT_STARTUP" "$LARGE_STARTUP"
printf "%-10s | %-10s | %-10s | %-14s\n" "edit-gui" "$GUI_RSS" "$GUI_STARTUP" "n/a"
printf "%-10s | %-10s | %-10s | %-14s\n" "code" "${CODE_RSS:-n/a}" "$CODE_STARTUP" "n/a"
