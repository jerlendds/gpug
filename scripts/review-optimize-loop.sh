#!/usr/bin/env bash
set -euo pipefail

iteration="${1:-candidate}"
node_count="${GPUG_NODE_COUNT:-10000}"
frames="${GPUG_BENCH_FRAMES:-30}"
edge_probability="${GPUG_EDGE_PROBABILITY:-0.00001}"
samples="${GPUG_BENCH_SAMPLES:-3}"
artifact_root="${GPUG_ARTIFACT_DIR:-artifacts/performance}"
iteration_dir="$artifact_root/$iteration"
mkdir -p "$iteration_dir"

echo "[review] formatting, compile checks, tests, and lints"
cargo fmt --all -- --check
cargo check --workspace --all-targets
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings

echo "[benchmark] $node_count nodes for $frames layout frames"
cargo run --quiet --release --example layout_benchmark -- \
  "$node_count" "$frames" "$edge_probability" "$samples" \
  | tee "$iteration_dir/benchmark.json"

echo "[screenshot] building and launching the large graph"
cargo build --quiet --release --example large_graph
existing_windows="$(xdotool search --name '.*' 2>/dev/null || true)"
GPUG_NODE_COUNT="$node_count" GPUG_EDGE_PROBABILITY="$edge_probability" \
  target/release/examples/large_graph &
app_pid=$!
cleanup() { kill "$app_pid" 2>/dev/null || true; }
trap cleanup EXIT

window_id=""
for _ in $(seq 1 50); do
  while read -r candidate; do
    if [[ -n "$candidate" ]] && ! grep -qx "$candidate" <<<"$existing_windows"; then
      window_id="$candidate"
    fi
  done < <(xdotool search --name '.*' 2>/dev/null || true)
  [[ -n "$window_id" ]] && break
  sleep 0.1
done
if [[ -z "$window_id" ]]; then
  echo "Could not locate the GPUG window; a graphical session is required." >&2
  exit 1
fi
if command -v i3-msg >/dev/null 2>&1; then
  i3-msg "[id=\"$window_id\"] floating enable, resize set 1280 px 800 px, move position center" \
    >/dev/null
else
  xdotool windowsize "$window_id" 1280 800
fi
sleep 0.5
import -window "$window_id" "$iteration_dir/graph.png"

# Start the simulation using the top-right play control and capture a laid-out
# frame as a separate visual regression artifact.
eval "$(xdotool getwindowgeometry --shell "$window_id")"
xdotool windowactivate --sync "$window_id"
xdotool mousemove --window "$window_id" "$((WIDTH - 14))" 14 click 1
sleep "${GPUG_LAYOUT_CAPTURE_DELAY:-1}"
import -window "$window_id" "$iteration_dir/graph-layout.png"
xdotool mousemove --window "$window_id" "$((WIDTH - 14))" 14 click 1
sleep 0.5
import -window "$window_id" "$iteration_dir/graph-layout-full.png"
cleanup
trap - EXIT

previous="$(find "$artifact_root" -mindepth 2 -maxdepth 2 -name benchmark.json \
  -not -path "$iteration_dir/*" -printf '%T@ %h\n' 2>/dev/null | sort -n | tail -1 | cut -d' ' -f2- || true)"
if [[ -n "$previous" ]]; then
  compare -metric RMSE "$previous/graph.png" "$iteration_dir/graph.png" \
    "$iteration_dir/screenshot-diff.png" 2>"$iteration_dir/screenshot-rmse.txt" || true
  if [[ -f "$previous/graph-layout.png" ]]; then
    compare -metric RMSE "$previous/graph-layout.png" "$iteration_dir/graph-layout.png" \
      "$iteration_dir/layout-screenshot-diff.png" \
      2>"$iteration_dir/layout-screenshot-rmse.txt" || true
  fi
  if [[ -f "$previous/graph-layout-full.png" ]]; then
    compare -metric RMSE "$previous/graph-layout-full.png" \
      "$iteration_dir/graph-layout-full.png" \
      "$iteration_dir/full-layout-screenshot-diff.png" \
      2>"$iteration_dir/full-layout-screenshot-rmse.txt" || true
  fi
  cp "$previous/benchmark.json" "$iteration_dir/previous-benchmark.json"
  jq -n --slurpfile previous "$previous/benchmark.json" \
    --slurpfile current "$iteration_dir/benchmark.json" '
      ($previous[0]) as $p | ($current[0]) as $c | {
        previous: ($p | {nodes, edges, probability, frames, layout_current_ms,
          render_optimized_ms}),
        current: ($c | {nodes, edges, probability, frames, layout_current_ms,
          render_optimized_ms}),
        layout_ms_per_frame_previous: ($p.layout_current_ms / $p.frames),
        layout_ms_per_frame_current: ($c.layout_current_ms / $c.frames),
        layout_change_percent: (((($c.layout_current_ms / $c.frames) /
          ($p.layout_current_ms / $p.frames)) - 1) * 100),
        render_ms_per_frame_previous: ($p.render_optimized_ms / $p.frames),
        render_ms_per_frame_current: ($c.render_optimized_ms / $c.frames),
        render_change_percent: (((($c.render_optimized_ms / $c.frames) /
          ($p.render_optimized_ms / $p.frames)) - 1) * 100)
      }' | tee "$iteration_dir/benchmark-comparison.json"
  echo "[compare] previous=$previous"
  echo "[compare] screenshot RMSE=$(cat "$iteration_dir/screenshot-rmse.txt")"
  if [[ -f "$iteration_dir/layout-screenshot-rmse.txt" ]]; then
    echo "[compare] layout screenshot RMSE=$(cat "$iteration_dir/layout-screenshot-rmse.txt")"
  fi
  if [[ -f "$iteration_dir/full-layout-screenshot-rmse.txt" ]]; then
    echo "[compare] full layout screenshot RMSE=$(cat "$iteration_dir/full-layout-screenshot-rmse.txt")"
  fi
else
  echo "[compare] no earlier iteration; this run is the baseline"
fi

git diff --check
git diff --stat | tee "$iteration_dir/code-review.txt"
echo "Artifacts written to $iteration_dir"
