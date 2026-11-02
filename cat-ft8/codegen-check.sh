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
# Checks that the belief-propagation rounds, the subtraction's input
# clamp and its reference's loads and stores are still vectorised on
# AArch64, that the clamp keeps its form on the microcontroller targets,
# and that the FFT stays written out butterfly by butterfly on the
# Cortex-M33. Their speed depends on code generation that no test sees: the
# sum-product round's tanh and atanh passes 24 values at a time
# (`PASS_LANES`), its products over four checks in the lanes of one
# vector (`group_products`), min-sum likewise (`group_min_sum`), the
# clamp of the removal and the time search's products as four-lane
# `fmaxnm`/`fminnm` rather than compares and selects (`input_select`),
# and the reference stored in groups of four (`GROUP`) loaded and stored
# as whole vectors, without interleaving loads and stores or shuffles. A
# compiler change can lose any of them with every test passing. On
# thumbv8m.main built with `--cfg cat_ft8_minmaxnm` (the Cortex-M33
# measured) the clamp is `vmaxnm`/`vminnm`; on thumbv7em
# (no such cfg) it must not call `fmaxf`/`fminf`, the library's `max` and
# `min`. This emits the release library's assembly (one codegen unit; no
# std for the microcontrollers) for each target and fails if a
# function's count is past its bound:
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
#   removal has 5 and 0, symbol_correlations 8; with the branch form of
#   the clamp 0, 18 and 0;
# - interleaving loads and stores (`ld2`-`ld4`, `st2`-`st4`) in the
#   removal and in `Fit4::add_all`, a ceiling of 0: with rustc 1.93 both
#   0; with the reference's parts interleaved sample by sample (a group
#   of one) 3 and 1, and with the removal's ramp index carried as an
#   integer (the loop vectoriser takes four groups at a time) 2 and 0;
# - four-lane `.4s` instructions in `whole_lane_sums`, a floor of 2: 2
#   with rustc 1.93; inlined into `lane_sums` (the sums kept as scalars)
#   the function is not found;
# - shuffles (`zip`, `uzp`, `trn`) in `blocks` (the reference's block
#   of 16 samples), a ceiling of 12: 3 with rustc 1.93; with the parts
#   interleaved 19, and with each sample stored by itself 75;
# - a ceiling counts only the code in its function, so it would pass if
#   the loop it guards moved out (into a function of its own, or inlined
#   elsewhere): on AArch64 each function with a ceiling also has a floor
#   on its vector instructions (the thumbv7em `fmaxf`/`fminf` ceilings
#   have none). `Removal::refs` has its `fmaxnm`/`fminnm` floor;
#   `Fit4::add_all` a floor of 12 `.4s` instructions (18 with rustc
#   1.93; 8 with its four-lane step `add4` out of line) and `blocks` 400
#   (549; 0 with `scaled_cos_sin` out of line).
# - thumbv8m.main with the cfg: `vmaxnm`/`vminnm` in the removal, a
#   floor: 20 with rustc 1.93, 0 without the cfg;
# - thumbv7em: calls of `fmaxf`/`fminf` in the removal,
#   symbol_correlations and `Fit4::add_all`, a ceiling of 0: with the cfg
#   (wrongly set) they are 20, 6 and 8.
# - thumbv8m.main with the cfg at opt-level "s" (the size the embedded
#   check builds at, though this is the library, not the linked image):
#   the FFT, inlined into `finish_row`, written out butterfly by butterfly
#   with one lane. With rustc 1.93.0: 238 `f32` multiplies (`vmul`, `vmla`,
#   `vmls`; a floor of 230), 0 divisions (`umull`, `udiv`; a ceiling of 0)
#   and 1 zero-fill call (`__aeabi_memclr4`, `__aeabi_memclr`, `memset`; a
#   ceiling of 1).
#   It was run against these forms (findings O20b), and fails on each: a
#   radix-4 stage as loops (202 multiplies), the radix-3 stage (218), the
#   radix-5 stage (171, 1 division), all the later stages as loops (142),
#   `stage` in their place (66, 4, 7) and `first_stage::<1>` for the first
#   stage (243, 1, 2: the ceilings, not the floor).
#   It misses: the radix-2 stage alone as loops (234 passes), the form of
#   the twiddle index (`% N` and a subtraction both pass), a zero fill by
#   `__aeabi_memclr8` or a tail call, and what link-time optimisation does
#   to the image. A first stage as loops with no division and no zero fill
#   was not run, and the floor would not be expected to catch it.
# The multiplication by an envelope of 1.0 that the reference leaves out
# (the weight folded by the compiler) is not checked: findings O18b.
# A bound that fails means: re-measure before changing it (planning/
# cat_ft8): on the Cortex-A76 for the AArch64 bounds (findings O12, O17,
# O17c, O18 and O19), by emulated instruction counts on the Cortex-M33 for
# the opt-level "s" bounds (findings O20 and O20b).
#
# Needs: rustup targets aarch64-unknown-linux-gnu, thumbv8m.main-none-eabihf
# and thumbv7em-none-eabihf (assembly only; nothing is linked).
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
target_dir="${CARGO_TARGET_DIR:-$here/../target/codegen-check}"
# The builds: a name, the target, cargo's features option, a `--cfg`
# for rustc (`-` for none) and an opt-level (`-` for the release
# profile's).
builds=("a64 aarch64-unknown-linux-gnu - - -"
  "m33 thumbv8m.main-none-eabihf --no-default-features cat_ft8_minmaxnm -"
  "m33s thumbv8m.main-none-eabihf --no-default-features cat_ft8_minmaxnm s"
  "m4 thumbv7em-none-eabihf --no-default-features - -")
# Each check: the build, the function, what is counted, and its bound. The function
# is a name, or a mangled path segment as it appears in the symbol (its
# length first) where the name alone is ambiguous. `vec4` counts
# instructions with a four-lane `.4s` operand, at least the bound; `lanes`
# counts moves into or out of single lanes (`mov`, `ins`, `dup`, `ld1`,
# `st1` on a `.s[i]` element), at most the bound; `minmax` counts
# four-lane `fmaxnm` and `fminnm`, at least the bound; `selects` counts
# `fcmgt`, `bsl`, `bit` and `bif`, at most the bound; `vminmax` counts
# `vmaxnm` and `vminnm`, at least the bound; `libcalls` counts calls of
# `fmaxf` and `fminf`, at most the bound; `interleaved` counts `ld2`,
# `ld3`, `ld4`, `st2`, `st3` and `st4`, at most the bound; `shuffles`
# counts `zip1`, `zip2`, `uzp1`, `uzp2`, `trn1` and `trn2`, at most the
# bound; `fpmul` counts `vmul`, `vmla` and `vmls` on `f32`, at least the
# bound; `divides` counts `umull` and `udiv`, at most the bound;
# `zerofills` counts calls of `__aeabi_memclr4`, `__aeabi_memclr` and
# `memset`, at most the bound. A function whose label is not
# found, or an unknown metric, fails.
checks=("a64 sum_product_round vec4 500" "a64 sum_product_round lanes 25"
  "a64 min_sum_round vec4 100"
  "a64 7Removal4refs minmax 2" "a64 symbol_correlations minmax 2"
  "a64 7Removal4refs selects 2"
  "a64 7Removal4refs interleaved 0" "a64 4Fit47add_all interleaved 0"
  "a64 4Fit47add_all vec4 12" "a64 whole_lane_sums vec4 2"
  "a64 6blocks shuffles 12" "a64 6blocks vec4 400"
  "m33 7Removal4refs vminmax 8"
  "m33s 10finish_row fpmul 230" "m33s 10finish_row divides 0" "m33s 10finish_row zerofills 1"
  "m4 7Removal4refs libcalls 0" "m4 symbol_correlations libcalls 0" "m4 4Fit47add_all libcalls 0")

declare -A asm
for b in "${builds[@]}"; do
  set -- $b
  key=$1 target=$2 cargo_opt=$3 rustc_opt=""
  [ "$cargo_opt" = - ] && cargo_opt=""
  [ "$4" = - ] || rustc_opt="--cfg $4"
  [ "$5" = - ] || rustc_opt="$rustc_opt -C opt-level=$5"
  dir="$target_dir/$key"
  # Clean the library first: cargo would otherwise see it as fresh, run
  # no compiler and write no assembly.
  CARGO_TARGET_DIR="$dir" cargo clean -q --release -p cat-ft8 --target "$target" \
    --manifest-path "$here/Cargo.toml"
  # shellcheck disable=SC2086 # the options are deliberately split
  CARGO_TARGET_DIR="$dir" cargo rustc -q --release -p cat-ft8 --lib --target "$target" \
    $cargo_opt --manifest-path "$here/Cargo.toml" -- --emit=asm -C codegen-units=1 $rustc_opt
  asm[$key]=$(ls "$dir/$target/release/deps/"cat_ft8-*.s)
done

fail=0
for entry in "${checks[@]}"; do
  set -- $entry
  key=$1 name=$2 metric=$3 bound=$4
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
    metric == "vminmax" && /^\t(vmaxnm|vminnm)/ { n++ }
    metric == "libcalls" && /^\tbl\t(fmaxf|fminf)$/ { n++ }
    metric == "interleaved" && /^\t(ld[234]|st[234])\t/ { n++ }
    metric == "shuffles" && /^\t(zip[12]|uzp[12]|trn[12])\t/ { n++ }
    metric == "fpmul" && /^\tv(mul|mla|mls)\.f32\t/ { n++ }
    metric == "divides" && /^\t(umull|udiv)\t/ { n++ }
    metric == "zerofills" && /^\tbl\t(__aeabi_memclr4|__aeabi_memclr|memset)$/ { n++ }
    END { if (found) print n + 0; else print "missing" }' "${asm[$key]}")
  if [ "$n" = missing ]; then
    echo "FAIL $key $name: function not found"
    fail=1
    continue
  fi
  case "$metric" in
    vec4) what=".4s instructions (floor $bound)"; [ "$n" -ge "$bound" ] && ok=1 || ok=0 ;;
    lanes) what="single-lane moves (ceiling $bound)"; [ "$n" -le "$bound" ] && ok=1 || ok=0 ;;
    minmax) what="four-lane fmaxnm/fminnm (floor $bound)"; [ "$n" -ge "$bound" ] && ok=1 || ok=0 ;;
    selects) what="compares and selects (ceiling $bound)"; [ "$n" -le "$bound" ] && ok=1 || ok=0 ;;
    vminmax) what="vmaxnm/vminnm (floor $bound)"; [ "$n" -ge "$bound" ] && ok=1 || ok=0 ;;
    libcalls) what="fmaxf/fminf calls (ceiling $bound)"; [ "$n" -le "$bound" ] && ok=1 || ok=0 ;;
    interleaved) what="interleaving loads and stores (ceiling $bound)"; [ "$n" -le "$bound" ] && ok=1 || ok=0 ;;
    shuffles) what="shuffles (ceiling $bound)"; [ "$n" -le "$bound" ] && ok=1 || ok=0 ;;
    fpmul) what="f32 multiplies (floor $bound)"; [ "$n" -ge "$bound" ] && ok=1 || ok=0 ;;
    divides) what="divisions (ceiling $bound)"; [ "$n" -le "$bound" ] && ok=1 || ok=0 ;;
    zerofills) what="zero-fill calls (ceiling $bound)"; [ "$n" -le "$bound" ] && ok=1 || ok=0 ;;
    *) what="unknown metric $metric"; ok=0 ;;
  esac
  if [ $ok = 1 ]; then
    echo "ok   $key $name: $n $what"
  else
    echo "FAIL $key $name: $n $what"
    fail=1
  fi
done
exit $fail
