#!/usr/bin/env bash
# Copyright 2026 Matt Franklin
#
# Licensed under the Apache License, Version 2.0 (the "License");
# you may not use this file except in compliance with the License.
# You may obtain a copy of the License at
#
#     http://www.apache.org/licenses/LICENSE-2.0
#
# Unless required by applicable law or agreed to in writing, software
# distributed under the License is distributed on an "AS IS" BASIS,
# WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
# See the License for the specific language governing permissions and
# limitations under the License.
#
# Checks that the belief-propagation rounds are still vectorised on
# AArch64. Their speed depends on code generation that no test sees: the
# sum-product round's tanh and atanh passes 24 values at a time
# (`PASS_LANES`), its products over four checks in the lanes of one
# vector (`group_products`), and min-sum likewise (`group_min_sum`). A
# compiler change can lose any of them with every test passing. This
# emits the release library's AArch64 assembly (one codegen unit) and
# fails if a function's count is past its bound:
# - four-lane `.4s` instructions, a floor: with rustc 1.93
#   sum_product_round has 696 and min_sum_round 156; forms that lost the
#   gain counted 115-452 (passes of 4 to 32 values, 32 not unrolled) and
#   29 (min-sum check by check);
# - single-lane moves in sum_product_round, a ceiling: 15 with rustc
#   1.93; 46 with the group products written in place row by row, 38
#   with the products check by check.
# A bound that fails means: re-measure the rounds on the Cortex-A76
# (planning/cat_ft8, findings O12, O17 and O17c) before changing it.
#
# Needs: rustup target aarch64-unknown-linux-gnu (assembly only; nothing
# is linked).
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
target=aarch64-unknown-linux-gnu
target_dir="${CARGO_TARGET_DIR:-$here/../target/codegen-check}"
# Each check: the function, what is counted, and its bound. `vec4` counts
# instructions with a four-lane `.4s` operand, at least the bound; `lanes`
# counts moves into or out of single lanes (`mov`, `ins`, `dup`, `ld1`,
# `st1` on a `.s[i]` element), at most the bound.
checks=("sum_product_round vec4 500" "sum_product_round lanes 25" "min_sum_round vec4 100")

# Clean the library first: cargo would otherwise see it as fresh, run no
# compiler and write no assembly.
CARGO_TARGET_DIR="$target_dir" cargo clean -q --release -p cat-ft8 --target "$target" \
  --manifest-path "$here/Cargo.toml"
CARGO_TARGET_DIR="$target_dir" cargo rustc -q --release -p cat-ft8 --lib --target "$target" \
  --manifest-path "$here/Cargo.toml" -- --emit=asm -C codegen-units=1
asm=$(ls "$target_dir/$target/release/deps/"cat_ft8-*.s)

fail=0
for entry in "${checks[@]}"; do
  set -- $entry
  name=$1 metric=$2 bound=$3
  # The function's body runs from its mangled label to the next
  # function's; count its instructions of the metric's kind.
  n=$(awk -v f="${#name}$name" -v metric="$metric" '
    /^_ZN.*:$/ { p = index($0, f) > 0; next }
    !p { next }
    metric == "vec4" && /^\t[a-z][^\t]*\t.*\.4s/ { n++ }
    metric == "lanes" && /^\t(mov|ins|dup|ld1|st1)\t.*\.s\[/ { n++ }
    END { print n + 0 }' "$asm")
  if [ "$metric" = vec4 ]; then
    what=".4s instructions (floor $bound)"
    [ "$n" -ge "$bound" ] && ok=1 || ok=0
  else
    what="single-lane moves (ceiling $bound)"
    [ "$n" -le "$bound" ] && ok=1 || ok=0
  fi
  if [ $ok = 1 ]; then
    echo "ok   $name: $n $what"
  else
    echo "FAIL $name: $n $what"
    fail=1
  fi
done
exit $fail
