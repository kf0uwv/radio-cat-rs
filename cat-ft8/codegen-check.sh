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
# that the FFT stays written out butterfly by butterfly on the Cortex-M33,
# and that the sync search's running sums keep their loop there.
# Their speed depends on code generation that no test sees: the
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
#   the time search's whole lanes (`correlation_whole`), a floor, and
#   compares and selects (`fcmgt`, `bsl`, `bit`, `bif`) in the removal, a
#   ceiling: with rustc 1.93 the removal has 5 (6 with O25's second
#   form) and 0, the time search 14; with the branch form of the clamp 0,
#   18 and 0;
# - interleaving loads and stores (`ld2`-`ld4`, `st2`-`st4`) in the
#   removal and in `Fit4::add_all`, a ceiling of 0: with rustc 1.93 both
#   0; with the reference's parts interleaved sample by sample (a group
#   of one) 3 and 1, and with the removal's ramp index carried as an
#   integer (the loop vectoriser takes four groups at a time) 2 and 0;
# - shuffles (`zip`, `uzp`, `trn`) in `blocks` (the reference's block
#   of 16 samples), a ceiling of 12: 3 with rustc 1.93; with the parts
#   interleaved 19, and with each sample stored by itself 75;
# - a ceiling counts only the code in its function, so it would pass if
#   the loop it guards moved out (into a function of its own, or inlined
#   elsewhere): on AArch64 each function with a ceiling also has a floor
#   on its vector instructions (the thumbv7em `fmaxf`/`fminf` ceilings
#   have none). `Removal::refs` has its `fmaxnm`/`fminnm` floor;
#   `Fit4::add_all_as` a floor of 12 `.4s` instructions (18 with rustc
#   1.93 in one form, 28 in its two since O25; 8 with its four-lane step
#   `add4` out of line) and `blocks` 400 (443 with rustc 1.93 since O23's
#   cheaper polynomial, 440 since O25, 549 before O23; 0 with
#   `scaled_cos_sin` out of line).
# - thumbv8m.main with the cfg: `vmaxnm`/`vminnm` in the removal, a
#   floor: 20 with rustc 1.93, 0 without the cfg;
# - thumbv7em: calls of `fmaxf`/`fminf` in the removal,
#   symbol_correlations and `Fit4::add_all_as`, a ceiling of 0: with the cfg
#   (wrongly set) they are 20, 6 and 8.
# - thumbv8m.main with the cfg at opt-level "s" (the size the embedded
#   check builds at, though this is the library, not the linked image):
#   the FFT, inlined into `finish_row`, written out butterfly by butterfly
#   with one lane. With rustc 1.93.0: 238 `f32` multiplies (`vmul`, `vmla`,
#   `vmls`; a floor of 230), 0 divisions (`umull`, `udiv`; a ceiling of 0)
#   and 1 zero-fill call (`__aeabi_memclr4`, `__aeabi_memclr`, `memset`; a
#   ceiling of 1).
#   The forms it was run against are listed in findings O20b.
# - thumbv8m.main with the cfg at opt-level "s": the subtraction's sums
#   where a group is one sample, taken one lane at a time with the sums in
#   registers. With rustc 1.93.0 the shortest loop with five `f32`
#   multiply-accumulates or adds in `Fit4::add_all` has 20 instructions
#   (a ceiling of 22, now 15 for the form without the clamp: below; 25
#   with the four lanes side by side); the frequency
#   correction one sample at a time leaves 19 carry
#   adds (`adc`) in the two `blocks` (a ceiling of 24; 37 with the
#   correction in lanes), and no method of `Correction` out of line (with
#   the correction's `apply` out of line, `blocks` keeps 15 carry adds,
#   under the ceiling, so the count alone would pass). The forms were run
#   in findings O22 and O22b.
#   It misses: the radix-2 stage alone as loops (234 passes), the form of
#   the twiddle index (`% N` and a subtraction both pass), a zero fill by
#   `__aeabi_memclr8` or a tail call, and what link-time optimisation does
#   to the image. A first stage as loops with no division and no zero fill
#   was not run, and the floor would not be expected to catch it.
# - the polynomial of the subtraction's reference and of the time search's
#   products (`cos_sin_turns_times`): on AArch64 its remainder by one shift
#   and quarter by one sign word (`cvtloop4` in `blocks` and
#   `correlation_whole`); on thumbv8m.main with the cfg at opt-level "s"
#   also the quarter's swap as `vsel` and the half turn's sign after the
#   weight (`cvtloop1` in `blocks` and in the subtraction's one closure,
#   which holds the products; `fpneg` in `blocks`). The ceilings sit one
#   to five instructions above rustc 1.93.0's code; the forms they catch,
#   and what they miss, are in findings O23 and O23b (the closure's
#   ceiling, 40 there, is O24's, below).
# - the time search's sums taken as its products are (findings O24): on
#   AArch64 its whole lanes out of line (`correlation_whole`; inlined, the
#   compiler split the sums across half vectors and spilled them), whose
#   loop of four conversions has 137 instructions with rustc 1.93.0 (a
#   ceiling of 140, now 110: below), and the half turn's sign on the
#   weight, one `eor` a vector (7 in the function, a ceiling of 8, now 16
#   for its two forms; 14 with the sign after the weight, whose loop is
#   138, under the loop's ceiling); on thumbv8m.main
#   with the cfg at opt-level "s" one lane at a time with the sign on the
#   weight, the products' loop in the subtraction's one closure 36 (a
#   ceiling of 37, now 31: below; 39 with the products stored and summed after, 42
#   with the sign after the weight, 69 with the lanes side by side). There
#   too the generator sums its overlap runs' increments as it takes them,
#   so `fill_phase_only` and `skip_with` keep no loop that loads with no
#   floating point (`memsums`, a ceiling of 0; 2 each with the increments
#   stored and summed after).
# - the input checked once per subtraction (findings O25): the loops that
#   read the audio have a second form without the clamp, taken when every
#   usable sample is within the limit (`within_limit`), whose bitwise or
#   decides in one pass for audio within ±1. On AArch64 that pass is
#   vectorised (`vecor`, vector `orr`, a floor of 2: 4 with rustc 1.93.0, 0
#   as a scalar loop), and the whole lanes without the clamp are 107
#   instructions a loop of four conversions (the shortest of the two
#   forms: a ceiling of 110; 137 with the clamp only). On thumbv8m.main
#   with the cfg at opt-level "s" the pass is 4 instructions a sample
#   (`shortloop`, the function's shortest loop, a ceiling of 4; 7 with the
#   largest magnitude alone), and the loops without the clamp are the
#   products 30 (the closure's ceiling of 31 above; 36 with the clamp),
#   the fit's 14 (`fploop5`, a ceiling of 15; 20 with the clamp) and the
#   removal's, unreported, 20 (`fploop4`, a ceiling of 21; 26 with the
#   clamp). The time search's whole lanes now have two forms of 7 `eor`
#   each (the ceiling above is 16 for both).
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
# `memset`, at most the bound; `sumloopN` counts the instructions of the
# shortest loop with N byte loads, at most the bound, and a function with no
# such loop fails it with its own message: the sync search's running sums
# (findings O25) read three levels a bin (the expected tone's, and tone 7's
# and tone 0's entering and leaving the sum), 15 instructions with rustc
# 1.93.0 (a ceiling of 16; the eight tones added afresh, 24 instructions
# with nine byte loads, have no such loop). The sum's speed depends on what
# else is live around that loop, so it failed if the rows were passed to
# `costas_scores` by value or `Rows::row` inlined into it (findings O21, for
# the nine-load loop). `fploopN` counts
# the instructions of the shortest loop with at least N `f32`
# multiply-accumulates or adds (`vmla`, `vmls`, `vadd`, `vsub`), at most the
# bound, and a function with no such loop fails it likewise; `cvtloopN`
# likewise with at least N conversions from integers (`scvtf`,
# `vcvt.f32.s32`); `fpneg` counts `vneg.f32`, at most the bound; `carries`
# counts `adc` (the high word of a 64-bit add), at most the bound;
# `memsums` counts the loops with a load (`ldr`, `ldrb` and the like;
# not `ldm`) and no floating-point or vector instruction (an `f32`
# value's moves included), at most the bound: it reads "no mnemonic that
# starts with v" as "no floating point", which holds on Thumb only, so it
# is valid for the m33 builds only; `eors`
# counts `eor` (exclusive or), at most the bound;
# `shortloop` counts the instructions of the function's shortest loop, at
# most the bound, and a function with no loop fails it; `vecor` counts
# `orr` on vector registers, at least the bound;
# `absent` passes only if no function's label holds the name (the bound is
# unused), that is, if it was inlined everywhere; it sees only a type's own
# methods (`10Correction`), not a trait's (`Clone` for it, say) left out of
# line. Otherwise a function whose label is not found, or an unknown
# metric, fails. A count is summed over every function whose label holds
# the name, and a loop metric takes the shortest loop among them; a
# check with a fifth field `one` also fails if more than one label holds
# the name (the subtraction's closures are named only by the function
# that holds them, so another closure in it could pass for the products).
checks=("a64 sum_product_round vec4 500" "a64 sum_product_round lanes 25"
  "a64 min_sum_round vec4 100"
  "a64 7Removal4refs minmax 2" "a64 correlation_whole minmax 2"
  "a64 7Removal4refs selects 2"
  "a64 7Removal4refs interleaved 0" "a64 4Fit410add_all_as interleaved 0"
  "a64 4Fit410add_all_as vec4 12"
  "a64 6blocks shuffles 12" "a64 6blocks vec4 400"
  "m33 7Removal4refs vminmax 8"
  "m33s 10finish_row fpmul 230" "m33s 10finish_row divides 0" "m33s 10finish_row zerofills 1"
  "m33s 6select sumloop3 16"
  "m33s 4Fit410add_all_as fploop5 15" "m33s 6blocks carries 24"
  "m33s 10Correction absent 0"
  "a64 6blocks cvtloop4 104"
  "m33s 6blocks cvtloop1 31" "m33s 3run28_\$u7b\$\$u7b\$closure cvtloop1 31 one" "m33s 6blocks fpneg 0"
  "a64 correlation_whole cvtloop4 110" "a64 correlation_whole eors 16"
  "m33s fill_phase_only memsums 0" "m33s 9skip_with memsums 0"
  "m4 7Removal4refs libcalls 0" "m4 symbol_correlations libcalls 0" "m4 4Fit410add_all_as libcalls 0"
  "a64 within_limit vecor 2" "m33s within_limit shortloop 4" "m33s 7Removal4refs fploop4 21")

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
  key=$1 name=$2 metric=$3 bound=$4 only=${5:-}
  # The function's body runs from its mangled label to the next
  # function's; count its instructions of the metric's kind.
  case "$name" in [0-9]*) seg=$name ;; *) seg="${#name}$name" ;; esac
  # Prints the count, "missing" if no function's label holds `seg`, or,
  # for a check marked `one`, "many" if more than one does.
  n=$(awk -v f="$seg" -v metric="$metric" -v only="$only" '
    # A function ends at its end label or at the next global label.
    /^\.Lfunc_end[0-9]+:/ { p = 0; next }
    /^[A-Za-z_][^ \t]*:$/ { p = index($0, f) > 0; if (p) found++; delete at; next }
    !p { next }
    metric ~ /^(sumloop|fploop|cvtloop|memsums|shortloop)/ && /^\.[A-Za-z0-9_$]+:/ {
      sub(/:.*/, ""); at[$0] = k; atb[$0] = bytes; atf[$0] = fps; atc[$0] = cvs
      atl[$0] = lds; atv[$0] = vs; next
    }
    metric ~ /^(sumloop|fploop|cvtloop|memsums|shortloop)/ && /^\t[a-z]/ {
      k++
      if ($1 ~ /^ldrb/) bytes++
      if ($1 ~ /^v(mla|mls|add|sub)\.f32$/) fps++
      if ($1 ~ /^(scvtf|vcvt\.f32\.s32)$/) cvs++
      if ($1 ~ /^ldr/) lds++
      if ($1 ~ /^v/) vs++
      back = $1 ~ /^(b[a-z]*(\.[nw])?|b\.[a-z]+|cbn?z|tbn?z)$/ && ($NF in at)
      if (back && metric ~ /^sumloop/ && bytes - atb[$NF] == substr(metric, 8) + 0 &&
          (best == "" || k - at[$NF] < best)) best = k - at[$NF]
      if (back && metric ~ /^fploop/ && fps - atf[$NF] >= substr(metric, 7) + 0 &&
          (best == "" || k - at[$NF] < best)) best = k - at[$NF]
      if (back && metric ~ /^cvtloop/ && cvs - atc[$NF] >= substr(metric, 8) + 0 &&
          (best == "" || k - at[$NF] < best)) best = k - at[$NF]
      if (back && metric == "memsums" && lds > atl[$NF] && vs == atv[$NF]) n++
      if (back && metric == "shortloop" && (best == "" || k - at[$NF] < best)) best = k - at[$NF]
    }
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
    metric == "carries" && /^\tadc/ { n++ }
    metric == "fpneg" && /^\tvneg\.f32\t/ { n++ }
    metric == "eors" && /^\teor\t/ { n++ }
    metric == "vecor" && /^\torr\tv[0-9]+\./ { n++ }
    END {
      if (!found) print "missing"
      else if (only == "one" && found > 1) print "many"
      else if (metric ~ /^(sumloop|fploop|cvtloop|shortloop)/) print (best == "" ? "noloop" : best)
      else print n + 0
    }' "${asm[$key]}")
  if [ "$metric" = absent ]; then
    if [ "$n" = missing ]; then
      echo "ok   $key $name: no function out of line"
    else
      echo "FAIL $key $name: a function out of line"
      fail=1
    fi
    continue
  fi
  if [ "$n" = many ]; then
    echo "FAIL $key $name: more than one function matches"
    fail=1
    continue
  fi
  if [ "$n" = missing ]; then
    echo "FAIL $key $name: function not found"
    fail=1
    continue
  fi
  if [ "$n" = noloop ]; then
    case "$metric" in
      sumloop*) echo "FAIL $key $name: no loop with ${metric#sumloop} byte loads (moved or unrolled?)" ;;
      cvtloop*) echo "FAIL $key $name: no loop with ${metric#cvtloop} conversions (moved or unrolled?)" ;;
      shortloop) echo "FAIL $key $name: no loop" ;;
      *) echo "FAIL $key $name: no loop with ${metric#fploop} f32 adds (moved or unrolled?)" ;;
    esac
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
    sumloop*) what="instructions in the summing loop (ceiling $bound)"; [ "$n" -le "$bound" ] && ok=1 || ok=0 ;;
    fploop*) what="instructions in the loop of ${metric#fploop} f32 adds (ceiling $bound)"; [ "$n" -le "$bound" ] && ok=1 || ok=0 ;;
    cvtloop*) what="instructions in the loop of ${metric#cvtloop} conversions (ceiling $bound)"; [ "$n" -le "$bound" ] && ok=1 || ok=0 ;;
    carries) what="carry adds (ceiling $bound)"; [ "$n" -le "$bound" ] && ok=1 || ok=0 ;;
    fpneg) what="f32 negations (ceiling $bound)"; [ "$n" -le "$bound" ] && ok=1 || ok=0 ;;
    memsums) what="loops that load with no floating point (ceiling $bound)"; [ "$n" -le "$bound" ] && ok=1 || ok=0 ;;
    eors) what="exclusive ors (ceiling $bound)"; [ "$n" -le "$bound" ] && ok=1 || ok=0 ;;
    shortloop) what="instructions in the shortest loop (ceiling $bound)"; [ "$n" -le "$bound" ] && ok=1 || ok=0 ;;
    vecor) what="vector ors (floor $bound)"; [ "$n" -ge "$bound" ] && ok=1 || ok=0 ;;
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
