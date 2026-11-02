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
# Checks that the belief-propagation rounds and the subtraction's input
# clamp are still vectorised on AArch64. Their speed depends on code
# generation that no test sees: the sum-product round's tanh and atanh
# passes 24 values at a time (`PASS_LANES`), its products over four
# checks in the lanes of one vector (`group_products`), min-sum likewise
# (`group_min_sum`), and the clamp of the removal and the time search's
# products as four-lane `fmaxnm`/`fminnm` rather than compares and
# selects (`input_select`). A compiler change can lose any of them with
# every test passing. This emits the release library's AArch64 assembly
# (one codegen unit) and fails if a function's count is past its bound:
# - four-lane `.4s` instructions, a floor: with rustc 1.93
#   sum_product_round has 696 and min_sum_round 156; forms that lost the
#   gain counted 115-452 (passes of 4 to 32 values, 32 not unrolled) and
#   29 (min-sum check by check). The margin under the passes of 4 values
#   is thin: 452 against the floor of 500;
# - single-lane moves in sum_product_round, a ceiling: 15 with rustc
#   1.93; 46 with the group products written in place row by row, 38
#   with the products check by check;
# - four-lane `fmaxnm`/`fminnm` in the removal (`Removal::refs`) and in
#   symbol_correlations, a floor, and compares and selects (`fcmgt`,
#   `bsl`, `bit`, `bif`) in the removal, a ceiling: with rustc 1.93 the
#   removal has 6 and 0, symbol_correlations 8; with the branch form of
#   the clamp 0, 18 and 0.
# The multiplication by an envelope of 1.0 that the reference leaves out
# (the weight folded by the compiler) is not checked: findings O18b.
# A bound that fails means: re-measure on the Cortex-A76 (planning/
# cat_ft8, findings O12, O17, O17c and O18) before changing it.
#
# Needs: rustup target aarch64-unknown-linux-gnu (assembly only; nothing
# is linked).
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
target=aarch64-unknown-linux-gnu
target_dir="${CARGO_TARGET_DIR:-$here/../target/codegen-check}"
# Each check: the function, what is counted, and its bound. The function
# is a name, or a mangled path segment as it appears in the symbol (its
# length first) where the name alone is ambiguous. `vec4` counts
# instructions with a four-lane `.4s` operand, at least the bound; `lanes`
# counts moves into or out of single lanes (`mov`, `ins`, `dup`, `ld1`,
# `st1` on a `.s[i]` element), at most the bound; `minmax` counts
# four-lane `fmaxnm` and `fminnm`, at least the bound; `selects` counts
# `fcmgt`, `bsl`, `bit` and `bif`, at most the bound. A function whose
# label is not found, or an unknown metric, fails.
checks=("sum_product_round vec4 500" "sum_product_round lanes 25" "min_sum_round vec4 100"
  "7Removal4refs minmax 2" "symbol_correlations minmax 2"
  "7Removal4refs selects 2")

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
  case "$name" in [0-9]*) seg=$name ;; *) seg="${#name}$name" ;; esac
  # Prints the count, or "missing" if no function's label holds `seg`.
  n=$(awk -v f="$seg" -v metric="$metric" '
    /^_ZN.*:$/ { p = index($0, f) > 0; if (p) found = 1; next }
    !p { next }
    metric == "vec4" && /^\t[a-z][^\t]*\t.*\.4s/ { n++ }
    metric == "lanes" && /^\t(mov|ins|dup|ld1|st1)\t.*\.s\[/ { n++ }
    metric == "minmax" && /^\t(fmaxnm|fminnm)\t.*\.4s/ { n++ }
    metric == "selects" && /^\t(fcmgt|bsl|bit|bif)\t/ { n++ }
    END { if (found) print n + 0; else print "missing" }' "$asm")
  if [ "$n" = missing ]; then
    echo "FAIL $name: function not found"
    fail=1
    continue
  fi
  case "$metric" in
    vec4) what=".4s instructions (floor $bound)"; [ "$n" -ge "$bound" ] && ok=1 || ok=0 ;;
    lanes) what="single-lane moves (ceiling $bound)"; [ "$n" -le "$bound" ] && ok=1 || ok=0 ;;
    minmax) what="four-lane fmaxnm/fminnm (floor $bound)"; [ "$n" -ge "$bound" ] && ok=1 || ok=0 ;;
    selects) what="compares and selects (ceiling $bound)"; [ "$n" -le "$bound" ] && ok=1 || ok=0 ;;
    *) what="unknown metric $metric"; ok=0 ;;
  esac
  if [ $ok = 1 ]; then
    echo "ok   $name: $n $what"
  else
    echo "FAIL $name: $n $what"
    fail=1
  fi
done
exit $fail
