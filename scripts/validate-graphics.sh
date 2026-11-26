#!/bin/bash
# SPDX-License-Identifier: GPL-3.0-or-later
set -euo pipefail

usage() {
    cat <<'HELP'
Usage: scripts/validate-graphics.sh [quick|full]

quick (default): focused CPU and native Metal checks for produced geometry,
                 views, samplers, guest-backed copies, R8 sampling, mip/slice transfers, array-slice/range views, typed formats/roles and compute caching.
full:            all nine repository validation gates, including the GPU suite.

Run on native Apple Silicon with the repository build dependencies installed.
Tests compare offscreen GPU pixels automatically; no application window opens.
Logs are saved under artifacts/validation-*. Run only one GPU workload at a time.
CARGO_BUILD_JOBS may be set to 1, 2, 3 or 4 (default: 4).
HELP
}

if [[ $# -gt 1 ]]; then usage >&2; exit 2; fi
mode=${1:-quick}
case "$mode" in
    -h|--help) usage; exit 0 ;;
    quick|full) ;;
    *) usage >&2; exit 2 ;;
esac
root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
cd "$root"
if [[ $(uname -s) != Darwin || $(uname -m) != arm64 ]]; then
    echo "Native arm64 macOS is required; do not run through Rosetta." >&2
    exit 2
fi
command -v cargo >/dev/null || { echo "Install the repository Rust toolchain first." >&2; exit 2; }
export CARGO_BUILD_JOBS=${CARGO_BUILD_JOBS:-4}
case "$CARGO_BUILD_JOBS" in
    1|2|3|4) ;;
    *) echo "CARGO_BUILD_JOBS must be between 1 and 4." >&2; exit 2 ;;
esac
if [[ "$mode" == full ]]; then
    for dependency in python3 timeout git; do
        command -v "$dependency" >/dev/null || {
            echo "Missing dependency: $dependency (see scripts/bootstrap.sh)." >&2
            exit 2
        }
    done
fi
export MTL_DEBUG_LAYER=1
export MTL_SHADER_VALIDATION=1
export MTL_SHADER_VALIDATION_REPORT_TO_STDERR=1
export MTL_SHADER_VALIDATION_ABORT_ON_FAULT=1
mkdir -p artifacts
lock=artifacts/graphics-validation.lock
if ! mkdir "$lock" 2>/dev/null; then
    echo "Another validation run may be active: $root/$lock" >&2
    echo "If a prior run was killed, remove that empty directory only after it stops." >&2
    exit 2
fi
log_dir=""
finish() {
    result=$?
    trap - EXIT
    rmdir "$lock" || true
    if [[ $result -ne 0 ]]; then
        echo "Validation stopped (exit $result). Logs: ${log_dir:-not created}" >&2
    fi
    exit "$result"
}
trap finish EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
log_dir=$(mktemp -d "$root/artifacts/validation-${mode}.XXXXXX")
step=0
last_log=""
run() {
    step=$((step + 1))
    local label=$1
    shift
    last_log="$log_dir/$(printf '%02d' "$step")-$label.log"
    printf '\n[%s] %s\n' "$step" "$label"
    printf '  %q' "$@"
    printf '\n'
    "$@" 2>&1 | tee "$last_log"
}
run_tests() {
    run "$@"
    if ! grep -Eq 'test result: ok\. [1-9][0-9]* passed;' "$last_log"; then
        echo "No passing tests selected; check the test filter. Log: $last_log" >&2
        exit 1
    fi
}
printf 'Running %s validation. Logs: %s\n' "$mode" "$log_dir"
if [[ "$mode" == quick ]]; then
    run_tests shader-array-decoder cargo test --locked -p xenora-decoder --test graphics_arrays
    run_tests shader-array-compiler cargo test --locked -p xenora-shader --test graphics_arrays
    run_tests shader-array-loader cargo test --locked -p xenora-loader --test array_sampling
    run_tests shader-array-native cargo test --locked -p xenora-metal --features gpu-tests --test native_compute shader_array_views:: -- --nocapture --test-threads=1
    run_tests shader-array-budget cargo test --locked -p xenora-metal --features gpu-tests --lib native_shader_array_views_ -- --nocapture --test-threads=1
    run_tests shader-array-guest cargo test --locked -p xenora-ffi --features gpu-tests --test guest_graphics original_guest_array_sampling_ -- --nocapture --test-threads=1
    run_tests attachment-composition-loader cargo test --locked -p xenora-loader --test attachment_composition -- --nocapture
    run_tests attachment-composition-boundaries cargo test --locked -p xenora-loader --test attachment_composition_boundaries -- --nocapture
    run_tests attachment-composition-guest cargo test --locked -p xenora-ffi --features gpu-tests --test guest_graphics original_guest_attachment_composition_ -- --nocapture --test-threads=1
    run_tests attachment-transactions-loader cargo test --locked -p xenora-loader --test attachment_transactions -- --nocapture
    run_tests attachment-transactions-failure cargo test --locked -p xenora-ffi --features gpu-tests --lib queued_attachment_transaction_ -- --nocapture --test-threads=1
    run_tests attachment-transactions-guest cargo test --locked -p xenora-ffi --features gpu-tests --test guest_graphics original_guest_attachment_transactions_ -- --nocapture --test-threads=1
    run_tests depth-attachment-view-native cargo test --locked -p xenora-metal --features gpu-tests --test native_compute depth_attachment_views:: -- --nocapture --test-threads=1
    run_tests depth-attachment-view-budget cargo test --locked -p xenora-metal --features gpu-tests --lib native_depth_attachment_view_load_ -- --nocapture --test-threads=1
    run_tests attachment-view-native cargo test --locked -p xenora-metal --features gpu-tests --test native_compute attachment_views:: -- --nocapture --test-threads=1
    run_tests attachment-view-budget cargo test --locked -p xenora-metal --features gpu-tests --lib native_attachment_view_load_ -- --nocapture --test-threads=1
    run_tests typed-layout cargo test --locked -p xenora-texture --test packed_layout ten_format_ -- --nocapture
    run_tests typed-shader cargo test --locked -p xenora-shader --test graphics_typed -- --nocapture
    run_tests typed-role-loader cargo test --locked -p xenora-loader --test typed_roles -- --nocapture
    run_tests typed-transfer-loader cargo test --locked -p xenora-loader --test typed_transfers -- --nocapture
    run_tests typed-copy-budget cargo test --locked -p xenora-metal --features gpu-tests --lib native_typed_copy_ -- --nocapture --test-threads=1
    run_tests typed-transfer-native cargo test --locked -p xenora-metal --features gpu-tests --test native_compute typed_transfers:: -- --nocapture --test-threads=1
    run_tests typed-attachment-native cargo test --locked -p xenora-metal --features gpu-tests --test native_compute typed_attachments:: -- --nocapture --test-threads=1
    run_tests typed-role-guest cargo test --locked -p xenora-ffi --features gpu-tests --test guest_graphics original_guest_typed_roles_ -- --nocapture --test-threads=1
    run_tests typed-transfer-guest cargo test --locked -p xenora-ffi --features gpu-tests --test guest_graphics original_guest_typed_transfers_ -- --nocapture --test-threads=1
    run_tests geometry-cpu cargo test --locked -p xenora-loader --test produced_geometry -- --nocapture
    run_tests nested-cpu cargo test --locked -p xenora-loader --test pipeline cpu_nested_view -- --nocapture
    run_tests nested-abi cargo test --locked -p xenora-loader --test graphics_service cpu_guest_child_view -- --nocapture
    run_tests samplers-cpu cargo test --locked -p xenora-loader --test pipeline cpu_sampler -- --nocapture
    run_tests samplers-abi cargo test --locked -p xenora-loader --test graphics_service cpu_guest_sampler -- --nocapture
    run_tests guest-copy-budget cargo test --locked -p xenora-loader --lib guest_copy_budget_rejection -- --nocapture
    run_tests guest-copy-cpu cargo test --locked -p xenora-loader --test queued_copies cpu_guest_copy -- --nocapture
    run_tests guest-region-cpu cargo test --locked -p xenora-loader --test queued_copies cpu_guest_region -- --nocapture
    run_tests guest-copy-abi cargo test --locked -p xenora-loader --test graphics_service cpu_guest_backed -- --nocapture
    run_tests array-layout cargo test --locked -p xenora-texture --test packed_layout array_ -- --nocapture --skip ten_format_
    run_tests mip-loader cargo test --locked -p xenora-loader --test mip_transfers -- --nocapture
    run_tests array-loader cargo test --locked -p xenora-loader --test array_transfers -- --nocapture
    run_tests array-ranges-loader cargo test --locked -p xenora-loader --test array_ranges -- --nocapture
    run_tests array-views-loader cargo test --locked -p xenora-loader --test array_views -- --nocapture
    run_tests r8-layout cargo test --locked -p xenora-texture --test packed_layout r8_ -- --nocapture
    run_tests r8-upload cargo test --locked -p xenora-metal --lib r8_validation_tests:: -- --nocapture
    run_tests r8-loader cargo test --locked -p xenora-loader --test pipeline cpu_r8_ -- --nocapture
    run_tests subresource-budget cargo test --locked -p xenora-metal --features gpu-tests --lib native_mip_array_ -- --nocapture --test-threads=1
    run_tests array-native cargo test --locked -p xenora-metal --features gpu-tests --test native_compute array_transfers:: -- --nocapture --test-threads=1
    run_tests array-guest cargo test --locked -p xenora-ffi --features gpu-tests --test guest_graphics original_guest_array_transfers_ -- --nocapture --test-threads=1
    run_tests array-views-budget cargo test --locked -p xenora-metal --features gpu-tests --lib native_array_view_ -- --nocapture --test-threads=1
    run_tests array-views-native cargo test --locked -p xenora-metal --features gpu-tests --test native_compute array_views:: -- --nocapture --test-threads=1
    run_tests array-ranges-guest cargo test --locked -p xenora-ffi --features gpu-tests --test guest_graphics original_guest_array_ranges_ -- --nocapture --test-threads=1
    run_tests array-views-guest cargo test --locked -p xenora-ffi --features gpu-tests --test guest_graphics original_guest_array_views_ -- --nocapture --test-threads=1
    run_tests mip-native cargo test --locked -p xenora-metal --features gpu-tests --test native_compute mip_transfers:: -- --nocapture --test-threads=1
    run_tests mip-guest cargo test --locked -p xenora-ffi --features gpu-tests --test guest_graphics original_guest_mip_transfers_ -- --nocapture --test-threads=1
    run_tests geometry-gpu cargo test --locked -p xenora-metal --features gpu-tests --test native_compute produced_geometry:: -- --nocapture --test-threads=1
    run_tests geometry-guest cargo test --locked -p xenora-ffi --features gpu-tests --test guest_graphics original_guest_produced_geometry_ -- --nocapture --test-threads=1
    run_tests nested-guest cargo test --locked -p xenora-ffi --features gpu-tests --test guest_graphics original_guest_nested_views -- --nocapture --test-threads=1
    run_tests samplers-guest cargo test --locked -p xenora-ffi --features gpu-tests --test guest_graphics original_guest_sampler_handles -- --nocapture --test-threads=1
    run_tests guest-copy-guest cargo test --locked -p xenora-ffi --features gpu-tests --test guest_graphics original_guest_backed_copy_ -- --nocapture --test-threads=1
    run_tests guest-region-guest cargo test --locked -p xenora-ffi --features gpu-tests --test guest_graphics original_guest_backed_region_ -- --nocapture --test-threads=1
    run_tests r8-native cargo test --locked -p xenora-metal --features gpu-tests --test native_compute r8_sampled:: -- --nocapture --test-threads=1
    run_tests r8-guest cargo test --locked -p xenora-ffi --features gpu-tests --test guest_graphics original_guest_r8_ -- --nocapture --test-threads=1
    run_tests pipeline-cache cargo test --locked -p xenora-metal --features gpu-tests --test native_compute pipeline_cache:: -- --nocapture --test-threads=1
else
    run format cargo fmt --all -- --check
    run clippy cargo clippy --locked --workspace --all-targets -- -D warnings
    run workspace cargo test --locked --workspace
    run gpu scripts/test-gpu.sh
    run fixtures scripts/assemble-fixture.sh --check
    run status python3 scripts/audit/status.py --check
    run claims scripts/audit/claims.sh
    run dependencies scripts/audit/dependencies.sh
    run diff git diff --check
fi
printf '\nPASS: all %s %s steps completed. Logs: %s\n' "$step" "$mode" "$log_dir"
