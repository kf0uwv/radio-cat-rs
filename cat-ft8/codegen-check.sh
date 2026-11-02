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
# (`PASS_LANES`), and min-sum over four checks in the lanes of one vector
# (`group_min_sum`). A compiler change can lose either with every test
# passing. This emits the release library's AArch64 assembly (one codegen
# unit) and fails if either function has fewer four-lane `.4s`
# instructions than its floor. With rustc 1.93 the counts are 696 and
# 156; forms that lost the gain counted 115-452 (passes of 4 to 32
# values, 32 not unrolled) and 29 (min-sum check by check). A floor that
# fails means: re-measure the rounds on the Cortex-A76 (planning/cat_ft8,
# findings O12 and O17) before changing it. The sum-product group
# products are not separately visible in these counts.
#
# Needs: rustup target aarch64-unknown-linux-gnu (assembly only; nothing
# is linked).
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
target=aarch64-unknown-linux-gnu
target_dir="${CARGO_TARGET_DIR:-$here/../target/codegen-check}"
# Each function, and its floor of `.4s` instructions.
floors=("sum_product_round 500" "min_sum_round 100")

# Clean the library first: cargo would otherwise see it as fresh, run no
# compiler and write no assembly.
CARGO_TARGET_DIR="$target_dir" cargo clean -q --release -p cat-ft8 --target "$target" \
  --manifest-path "$here/Cargo.toml"
CARGO_TARGET_DIR="$target_dir" cargo rustc -q --release -p cat-ft8 --lib --target "$target" \
  --manifest-path "$here/Cargo.toml" -- --emit=asm -C codegen-units=1
asm=$(ls "$target_dir/$target/release/deps/"cat_ft8-*.s)

fail=0
for entry in "${floors[@]}"; do
  set -- $entry
  name=$1 floor=$2
  # The function's body: from its mangled label to the next function's.
  n=$(awk -v f="${#name}$name" '/^_ZN.*:$/ { p = index($0, f) > 0 } p' "$asm" \
    | grep -cP '^\t[a-z][^\t]*\t.*\.4s' || true)
  if [ "$n" -ge "$floor" ]; then
    echo "ok   $name: $n .4s instructions (floor $floor)"
  else
    echo "FAIL $name: $n .4s instructions (floor $floor)"
    fail=1
  fi
done
exit $fail
