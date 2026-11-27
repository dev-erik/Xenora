#!/bin/sh
# SPDX-License-Identifier: GPL-3.0-or-later
set -eu
cd "$(dirname "$0")/.."

export MTL_DEBUG_LAYER=1
export MTL_SHADER_VALIDATION=1
export MTL_SHADER_VALIDATION_REPORT_TO_STDERR=1
export MTL_SHADER_VALIDATION_ABORT_ON_FAULT=1
export CARGO_BUILD_JOBS=${CARGO_BUILD_JOBS:-4}

# Native view identity plus shared production state-machine unit checks.
cargo test --locked -p xenora-metal --features gpu-tests --lib --no-run
timeout 30s cargo test --locked -p xenora-metal --features gpu-tests --lib -- --nocapture --test-threads=1

cargo test --locked -p xenora-metal --features gpu-tests --test native_compute --no-run
# Additional independently gated format/input-role cases extend the serial suite.
timeout 60s cargo test --locked -p xenora-metal --features gpu-tests --test native_compute -- \
    --nocapture --test-threads=1

# Original SCC1 compute and runtime graphics branch readback.
cargo test --locked -p xenora-metal --features gpu-tests --test scc1_branch --no-run
timeout 30s cargo test --locked -p xenora-metal --features gpu-tests --test scc1_branch -- --nocapture --test-threads=1

cargo test --locked -p xenora-loader --features gpu-tests --test dbt_render --no-run
timeout 30s cargo test --locked -p xenora-loader --features gpu-tests --test dbt_render -- \
    --nocapture --test-threads=1

cargo test --locked -p xenora-loader --features gpu-tests --test runtime_x15 --no-run
timeout 30s cargo test --locked -p xenora-loader --features gpu-tests --test runtime_x15 -- \
    test_fixture_c_service_to_metal_framebuffer --exact --nocapture --test-threads=1

cargo test --locked -p xenora-loader --features gpu-tests --test graphics_d3d12 --no-run
timeout 30s cargo test --locked -p xenora-loader --features gpu-tests --test graphics_d3d12 -- \
    --nocapture --test-threads=1

# Reuse loader command lists and guest buffers across changing frames and failed submissions.
cargo test --locked -p xenora-loader --features gpu-tests --test repeated_frames --no-run
timeout 30s cargo test --locked -p xenora-loader --features gpu-tests --test repeated_frames -- \
    --nocapture --test-threads=1

# The native app interface runs the trusted fixture's guest draw on Metal.
cargo test --locked -p xenora-ffi --features gpu-tests --test native_frame --no-run
timeout 30s cargo test --locked -p xenora-ffi --features gpu-tests --test native_frame -- \
    --nocapture --test-threads=1

# Selected original shader pipelines, textures and retained depth through production adapter.
cargo test --locked -p xenora-ffi --features gpu-tests --test large_scene --no-run
timeout 60s cargo test --locked -p xenora-ffi --features gpu-tests --test large_scene -- --nocapture --test-threads=1

cargo test --locked -p xenora-ffi --features gpu-tests --test pipeline --no-run
timeout 30s cargo test --locked -p xenora-ffi --features gpu-tests --test pipeline -- --nocapture --test-threads=1

# Combined indexed, mixed-program, sampled MRT scene with retained depth and recovery.
cargo test --locked -p xenora-ffi --features gpu-tests --test integrated_scene --no-run
timeout 30s cargo test --locked -p xenora-ffi --features gpu-tests --test integrated_scene -- --nocapture --test-threads=1

# Original PE guest calls the versioned pipeline/resource service through import dispatch.
cargo test --locked -p xenora-ffi --features gpu-tests --lib --no-run
timeout 30s cargo test --locked -p xenora-ffi --features gpu-tests --lib async_tests -- --nocapture --test-threads=1
# Descriptor rejection must precede attachment host-seed admission.
timeout 30s cargo test --locked -p xenora-ffi --features gpu-tests --lib queued_descriptor_ -- --nocapture --test-threads=1
# Real two-queue producer/consumer: both commits precede a controlled GPU gate.
timeout 30s cargo test --locked -p xenora-ffi --features gpu-tests --lib queued::tests -- --nocapture --test-threads=1

cargo test --locked -p xenora-ffi --features gpu-tests --test guest_graphics --no-run
timeout 30s cargo test --locked -p xenora-ffi --features gpu-tests --test guest_graphics -- --nocapture --test-threads=1

# The persistent frame loop runs the guest on its own thread and renders on Metal.
cargo test --locked -p xenora-ffi --features gpu-tests --test frame_loop --no-run
timeout 30s cargo test --locked -p xenora-ffi --features gpu-tests --test frame_loop -- \
    --nocapture --test-threads=1

# Interactive guest execution with dynamic input handling and rendering (Gate 4, #276).
cargo test --locked -p xenora-ffi --features gpu-tests --test interactive --no-run
timeout 30s cargo test --locked -p xenora-ffi --features gpu-tests --test interactive -- \
    --nocapture --test-threads=1

# Persistent GPU resources, dynamic texture updates, and resource retirement (Gate 5, #279).
cargo test --locked -p xenora-ffi --features gpu-tests --test resources --no-run
timeout 30s cargo test --locked -p xenora-ffi --features gpu-tests --test resources -- \
    --nocapture --test-threads=1

# Audio PCM streaming and sandboxed virtual filesystem integration (Gate 6, #284).
cargo test --locked -p xenora-ffi --features gpu-tests --test audio_vfs --no-run
timeout 30s cargo test --locked -p xenora-ffi --features gpu-tests --test audio_vfs -- \
    --nocapture --test-threads=1

# Native Metal known answers of the independent semantic review, including SOP2 scalar ALU
# and VOPC compares.
cargo test --locked -p xenora-semantic-review --no-run
timeout 120s cargo test --locked -p xenora-semantic-review -- --ignored --nocapture --test-threads=1
