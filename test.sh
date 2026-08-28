#!/usr/bin/env bash
set -euo pipefail

for command in cargo i3-msg jq xdotool; do
  if ! command -v "$command" >/dev/null 2>&1; then
    echo "Missing required command: $command" >&2
    exit 1
  fi
done

echo "Building the large-graph example..."
cargo build --release --example large_graph

# Only consider X11 windows managed by i3. GPUI also creates short-lived helper
# windows which xdotool can see, but i3 criteria cannot target.
managed_windows() {
  i3-msg -t get_tree \
    | jq -r '.. | objects | select(.window? != null) | .window'
}
existing_windows="$(managed_windows)"

GPUG_NODE_COUNT="${GPUG_NODE_COUNT:-10000}" \
GPUG_EDGE_PROBABILITY="${GPUG_EDGE_PROBABILITY:-0.00001}" \
  target/release/examples/large_graph &
app_pid=$!

cleanup_on_error() {
  kill "$app_pid" 2>/dev/null || true
}
trap cleanup_on_error ERR INT TERM

window_id=""
for _ in $(seq 1 100); do
  while read -r candidate; do
    if [[ -n "$candidate" ]] && ! grep -qxF "$candidate" <<<"$existing_windows"; then
      window_id="$candidate"
      break
    fi
  done < <(managed_windows)

  [[ -n "$window_id" ]] && break
  sleep 0.1
done

if [[ -z "$window_id" ]]; then
  echo "Could not locate the new GPUG window. Is this running in an X11/i3 session?" >&2
  exit 1
fi

# Let GPUI finish applying its initial window bounds before i3 overrides them.
sleep 0.75
i3-msg "[id=\"$window_id\"] floating enable, resize set 1280 px 800 px, move position center, focus" \
  >/dev/null
xdotool windowactivate --sync "$window_id"
xdotool windowraise "$window_id"

# Apply the bounds once more in case a late startup configure event raced i3.
sleep 0.25
i3-msg "[id=\"$window_id\"] resize set 1580 px 928 px, move position center, focus" \
  >/dev/null
xdotool windowraise "$window_id"

trap - ERR INT TERM
echo "GPUG is running in floating window $window_id (process $app_pid)."
