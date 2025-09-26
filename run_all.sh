#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "$0")" && pwd)"
LOG_DIR="$ROOT/logs"
mkdir -p "$LOG_DIR"

# kill all children on exit or signals
trap 'echo; echo stopping...; kill 0 2>/dev/null || true' INT TERM EXIT

prefix() {
  local name="$1"
  awk -v n="[$name]" '{ print n, $0 }'
}

start() {
  local name="$1"
  shift
  local cmd="$*"

  echo "starting $name"
  bash -lc "$cmd" \
    > >({ prefix "$name" | tee -a "$LOG_DIR/$name.log"; }) \
    2> >({ prefix "$name" | tee -a "$LOG_DIR/$name.log" >&2; }) &
}

# helpers
WP="$ROOT/wait-port.sh"

# qdrant
start qdrant     "cd \"$ROOT\" && ./qdrant.sh"

# memory and rag services wait for qdrant 6333
start memory     "cd \"$ROOT/mcp-memory\"    && \"$WP\" 127.0.0.1 6333 && cargo run --release"
start rag_page   "cd \"$ROOT/mcp-rag-page\"  && \"$WP\" 127.0.0.1 6333 && cargo run --release"
start rag_rules  "cd \"$ROOT/mcp-rag-rules\" && \"$WP\" 127.0.0.1 6333 && cargo run --release"
start rag_faq    "cd \"$ROOT/mcp-rag-faq\"   && \"$WP\" 127.0.0.1 6333 && cargo run --release"

# scraper has no deps listed
start scraper    "cd \"$ROOT/mcp-scraper\"   && cargo run --release"

# staff and programme wait for 8002 and 8000
start staff      "cd \"$ROOT/mcp-staff\"     && \"$WP\" 127.0.0.1 8002 && \"$WP\" 127.0.0.1 8000 && cargo run --release"
start programme  "cd \"$ROOT/mcp-programme\" && \"$WP\" 127.0.0.1 8002 && \"$WP\" 127.0.0.1 8000 && cargo run --release"

# urska waits for many ports
start urska      "cd \"$ROOT/core\" && \"$WP\" 127.0.0.1 8001 && \"$WP\" 127.0.0.1 8003 && \"$WP\" 127.0.0.1 8005 && \"$WP\" 127.0.0.1 8006 && \"$WP\" 127.0.0.1 8007 && cargo run --release"

# optional inspector, set INSPECTOR=1 to enable
if [[ "${INSPECTOR:-0}" = "1" ]]; then
  if command -v node >/dev/null && node -v | grep -q '^v22'; then
    start inspector "cd \"$ROOT\" && npx @modelcontextprotocol/inspector"
  else
    NVM_DIR="${NVM_DIR:-$HOME/.nvm}"
    if [[ -s \"$NVM_DIR/nvm.sh\" ]]; then
      start inspector "cd \"$ROOT\" && . \"$NVM_DIR/nvm.sh\" && nvm use 22 && npx @modelcontextprotocol/inspector"
    else
      echo "skipping inspector, need node 22 or nvm"
    fi
  fi
fi

echo "all processes launched. logs in $LOG_DIR"
echo "press ctrl c to stop"
wait
