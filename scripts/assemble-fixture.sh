#!/bin/sh
# SPDX-License-Identifier: GPL-3.0-or-later
set -eu

cd "$(dirname "$0")/.."
llvm_bin=${LLVM_BIN:-$(brew --prefix llvm)/bin}
scratch=$(mktemp -d)
trap 'rm -rf "$scratch"' EXIT
case "${1:-}" in
  --check|"") ;;
  *) printf '%s\n' 'usage: scripts/assemble-fixture.sh [--check]' >&2; exit 2 ;;
esac

for name in lane-read-e64-spec wave-bpermute wave-swizzle wave-permlane16 wave-permlanex16 scalar-compare-immediate x9-scalar-cmpk-ps x9-scalar-mul-ps scalar-immediate x9-scalar-movk-ps x9-scalar-addk-ps x9-scalar-mulk-ps scalar-pack x9-scalar-pack-ll-ps x9-scalar-pack-lh-ps x9-scalar-pack-hh-ps x9-descriptor-cross-ps x9-descriptor-dynamic-ps x9-storage-branch-ps x9-storage-loop-ps x9-storage-uint-sampled-ps x9-array-slot1-ps x9-array-branch-ps x9-array-uniform-ps x9-loop-signed-count-ps x9-loop-signed-reversed-ps x9-loop-signed-negative-ps x9-loop-signed-break-ps x9-loop-signed-nested-ps x9-loop-signed-sampled-ps x9-loop-signed-dead-sample-ps \
    x9-loop-break-after-ps x9-loop-break-before-ps x9-loop-break-dead-sample-ps x9-loop-break-geometry-vs x9-loop-break-recurring-ps x9-loop-break-sampled-ps x9-loop-break-typed-ps \
    x9-nested-loop-reject-crossing-ps x9-nested-loop-reject-unguarded-counter-step-ps x9-nested-loop-reject-increasing-retained-counter-ps \
    x9-nested-loop-sampled-accum-ps x9-nested-loop-geometry-vs x9-nested-loop-budget-one-over-ps x9-nested-loop-alias-scc-ps x9-nested-loop-changing-bound-ps x9-nested-loop-scalar-siblings-ps x9-nested-loop-swap-ps x9-nested-loop-depth-three-ps x9-nested-loop-budget-accept-ps x9-nested-loop-budget-reject-ps x9-nested-loop-zero-outer-unbounded-child-ps x9-nested-loop-reject-runtime-unbounded-child-ps x9-nested-loop-retained-child-counter-ps x9-nested-loop-reject-bypass-reset-ps x9-nested-loop-reject-child-writes-outer-ps x9-nested-loop-reject-outer-decrement-in-child-ps x9-nested-loop-reject-shared-counter-ps x9-nested-loop-reject-bypassed-decrement-ps \
    x9-nested-loop-vgpr-ps x9-loop-sampled-accum-ps x9-loop-limit-sequential-9-ps x9-loop-limit-sequential-8-ps x9-loop-preheader-merge-ps x9-loop-diamond-swap-ps \
    x9-loop-limit-trip-256-ps x9-loop-limit-trip-257-ps x9-loop-limit-stream-1024-ps x9-loop-limit-stream-1028-ps \
    x9-loop-geometry-vs x9-loop-scalar-ps x9-loop-scalar-reversed-min-ps x9-loop-swap-ps x9-loop-diamond-ps x9-loop-sampled-ps x9-loop-zero-sampled-ps x9-loop-zero-dead-ps x9-loop-reject-header-reset-ps x9-loop-reject-stale-decrement-ps x9-loop-reject-stale-compare-ps x9-loop-reject-missing-entry-ps x9-loop-reject-missing-zero-trip-ps x9-loop-reject-unbounded-ps x9-loop-reject-signed-ps x9-loop-reject-wrap-step-ps x9-loop-reject-counter-overwrite-ps x9-loop-reject-export-body-ps x9-loop-reject-interpolation-entry-ps \
    x9-loop-vgpr-ps x9-scalar-add-chain-ps x9-scalar-add-chain-alias-ps x9-scalar-sub-chain-ps x9-scalar-sub-chain-alias-ps x9-scalar-direct-phi-mrt-ps \
    x9-scalar-add-u32-ps x9-scalar-sub-u32-ps x9-scalar-add-i32-ps x9-scalar-sub-i32-ps x9-scalar-addc-u32-ps x9-scalar-subb-u32-ps x9-scalar-min-i32-ps x9-scalar-min-u32-ps x9-scalar-max-i32-ps x9-scalar-max-u32-ps x9-scalar-cselect-b32-ps x9-scalar-mov-b32-ps x9-scalar-add-vs x9-scalar-nested-mrt-ps \
    x9-runtime-sampled-ps x9-runtime-eq-i32-ps x9-runtime-lg-i32-ps x9-runtime-gt-i32-ps x9-runtime-ge-i32-ps x9-runtime-lt-i32-ps x9-runtime-le-i32-ps x9-runtime-eq-u32-ps x9-runtime-lg-u32-ps x9-runtime-gt-u32-ps x9-runtime-ge-u32-ps x9-runtime-lt-u32-ps x9-runtime-le-u32-ps x9-runtime-nested-mrt-ps \
    x9-runtime-uniform-vs x9-runtime-uniform-ps x9-view-sample-ps x9-multislot-ps add bitwise-branch scc1-branch wait-vmcnt mbcnt lane-read lane-write mask-nonzero-branch buffer-narrow-load float-exact or shifts ashr \
    x9-scene-varying-mrt-ps x9-scene-sampled-mrt-ps x9-conditional-vs x9-conditional-right-vs x9-conditional-ps x9-forward-vs x9-forward-ps x9-constant-vs x9-constant-ps x9-varying-vs x9-varying-ps x9-transform-vs x9-pixel-arithmetic-ps x9-sampled-arithmetic-ps x9-mrt-ps scalar-alu vector-compare scalar-logic scalar-compare scalar-unary scalar-arith vector-integer vector-unary scalar-conditional vop3-integer vop3-bitfield vop3-arithmetic vop3-minmax vop3-aliases vop3-clamp vop3-mad-sad; do
    "$llvm_bin/llvm-mc" -triple=amdgcn-amd-amdhsa -mcpu=gfx1030 -mattr=+wavefrontsize32 \
        -filetype=obj "fixtures/rdna2/$name.s" -o "$scratch/$name.o"
    "$llvm_bin/llvm-objcopy" -O binary --only-section=.text "$scratch/$name.o" "$scratch/$name.bin"
    if [ "${1:-}" = --check ]; then
        cmp "$scratch/$name.bin" "fixtures/rdna2/$name.bin"
    else
        cp "$scratch/$name.bin" "fixtures/rdna2/$name.bin"
    fi
done
