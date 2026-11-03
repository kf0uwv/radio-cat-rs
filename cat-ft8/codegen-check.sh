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
# that the FFT stays written out butterfly by butterfly on the Cortex-M33
# and its last stage one vectorised loop on AArch64 and x86-64, and that
# the sync search's running sums keep their loop there.
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
# `min`. This emits the release library's assembly (one codegen unit, and
# for AArch64 also the release profile's 16; no std for the
# microcontrollers) for each target and fails if a function's count is
# past its bound:
# - four-lane `.4s` instructions, a floor: with rustc 1.93
#   sum_product_round has 696 and min_sum_round 165 (156 before O27);
#   forms that lost the gain counted 115-452 (passes of 4 to 32 values,
#   32 not unrolled) and 29 (min-sum check by check). The margin under
#   the passes of 4 values is thin: 452 against the floor of 500.
#   min_sum_round also as a release build compiles it (`a64r`, findings
#   O27): with 16 codegen units the local ThinLTO pipeline inlines and
#   unrolls the round again before the vectorisers, and with each row's
#   inputs first in a local array and its gathers bounds-checked it left
#   the groups scalar, 14 with rustc 1.93.0, while the one-unit build
#   counted 156 and passed; with each row's inputs taken in place and the
#   variables' indices taken no further than the last, 162 (a floor of
#   100);
# - single-lane moves in sum_product_round, a ceiling: 15 with rustc
#   1.93; 46 with the group products written in place row by row, 38
#   with the products check by check;
# - four-lane `fmaxnm`/`fminnm` in the removal (`Removal::refs`) and in
#   the time search's whole lanes (`correlation_whole`), a floor, and
#   compares and selects (`fcmgt`, `bsl`, `bit`, `bif`) in the removal, a
#   ceiling: with rustc 1.93.0 the removal has 6 and 0, the time search
#   14; with the branch form of the clamp (`INPUT_SELECT` false) 1, 0 and
#   0 (the floors catch it);
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
#   `Fit4::add_all_as` a floor of 12 `.4s` instructions (28 with rustc
#   1.93.0 over its two forms; 8 with its four-lane step `add4` out of
#   line) and `blocks` 400 (440 with rustc 1.93.0; 0 with
#   `scaled_cos_sin` out of line).
# - thumbv8m.main with the cfg: `vmaxnm`/`vminnm` in the removal, a
#   floor: 20 with rustc 1.93, 0 without the cfg;
# - thumbv7em: calls of `fmaxf`/`fminf` in the removal,
#   symbol_correlations and `Fit4::add_all_as`, a ceiling of 0: with the cfg
#   (wrongly set) they are 20, 6 and 8.
# - thumbv8m.main with the cfg at opt-level "s" (the size the embedded
#   check builds at, though this is the library, not the linked image):
#   the FFT, inlined into `finish_row`, written out butterfly by butterfly
#   with one lane. With rustc 1.93.0: 207 `f32` multiplies (`vmul`, `vmla`,
#   `vmls`; a floor of 199: 238 and 230 before the bins' powers went out
#   of line with their 31, findings O26), 0 divisions (`umull`, `udiv`; a ceiling of 0)
#   and 1 zero-fill call (`__aeabi_memclr4`, `__aeabi_memclr`, `memset`; a
#   ceiling of 1).
#   The forms it was run against are listed in findings O20b.
# - thumbv8m.main with the cfg at opt-level "s": the subtraction's sums
#   where a group is one sample, taken one lane at a time with the sums in
#   registers. With rustc 1.93.0 the shortest loop with five `f32`
#   multiply-accumulates or adds in `Fit4::add_all_as` is the form
#   without the clamp's, 14 instructions (a ceiling of 15; 20 with the
#   clamp, a form this does not bound: see the input's check below; 25
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
#   shortest loop of four conversions, the form without the clamp's, has
#   107 instructions with rustc 1.93.0 (a ceiling of 110; 137 with the
#   clamp), and the half turn's sign on the weight, one `eor` a vector (7
#   in each of the two forms, a ceiling of 16; a form with the sign after
#   the weight had 14, findings O24); on thumbv8m.main with the cfg at
#   opt-level "s" one lane at a time with the sign on the weight, the
#   products' loop in the subtraction's one closure 30 without the clamp
#   (a ceiling of 31; 36 with it; with the clamp, 39 with the products
#   stored and summed after, 42 with the sign after the weight, 69 with
#   the lanes side by side). There
#   too the generator sums its overlap runs' increments as it takes them,
#   so `fill_phase_only` and `skip_with` keep no loop that loads with no
#   floating point (`memsums`, a ceiling of 0; 2 each with the increments
#   stored and summed after).
# - the input checked once per subtraction (findings O25): the loops that
#   read the audio have a form without the clamp, taken when every usable
#   sample is within the limit (`within_limit`), whose bitwise or decides
#   in one pass for audio within ±1. On AArch64 that pass is vectorised
#   (`vecor`, vector `orr`, a floor of 2: 4 with rustc 1.93.0, 0 as a
#   scalar loop). On thumbv8m.main with the cfg at opt-level "s" it is 4
#   instructions a sample (`shortloop`, the function's shortest loop, a
#   ceiling of 4; 7 with the largest magnitude alone). The ceiling is
#   exact because 4 is the least a loop of one sample a turn can have (a
#   load that steps its pointer, the or, the count and the branch), and
#   each instruction more is a quarter more on the pass; a loop unrolled
#   to several samples a turn would be longer and fail it while perhaps
#   faster, and is to be measured then. The removal's form without the
#   clamp, unreported, is 20 instructions a sample (`fploop4`, a ceiling
#   of 21; 26 with the clamp).
#   The loop metrics take the shortest loop, so in the fit, the products,
#   the time search's whole lanes and the removal they bound the forms
#   without the clamp only; the forms with it (20, 36, 137 and 26 above)
#   carry no bound. They run only on audio with a sample beyond ±2¹⁶ or
#   not finite, which a receiver's samples scaled to ±1 never are, so
#   their speed is not on any measured path, and their results are the
#   tests' (each form against the other, bit for bit). The counts above
#   are summed over both forms: a ceiling (the m4 library calls, the
#   removal's selects) binds each form, a floor only the two together.
# - the front end checks each block it is fed with the same function
#   (findings O26): `Spectrogram::feed` calls `within_limit`
#   (`calls_within_limit`, a floor of 2 on AArch64 and on thumbv8m.main with the cfg at opt-level
#   "s": 2 with rustc 1.93.0, the row's first block and the others; 0
#   with the block's own compare fold, 10 instructions a sample on the
#   Cortex-M33 against the or pass's 4).
# - the FFT's last stage (radix 5, one block) as one plain loop over k
#   whose twiddle indices run past one turn of the table rather than wrap
#   (`last_stage`, findings O26 and O26b), which the compiler vectorises
#   with each m's twiddles, m apart in the table, loaded as interleaved
#   vectors. On AArch64 `ld2`-`ld4` in `finish_row` (`ldn`, a floor of 12:
#   12 with rustc 1.93.0, 10 of them the stage's; 2 with the butterflies
#   side by side and their twiddles gathered lane by lane, 9 with the
#   index reduced mod N). On x86-64 the shortest loop in `finish_row` with
#   80 four-lane multiplies (`mulloop80`, a ceiling of 460: 450 with rustc
#   1.93.0; 699 gathered lane by lane, 629 with the segments between the
#   wraps taken at run time, none with the index reduced mod N). And
#   `finish_row`'s instructions (`insns`, a ceiling of 3 000 on AArch64
#   and 3 500 on x86-64: 2 882 and 3 364 with rustc 1.93.0; with the six
#   segments between the wraps compiled each as its own loop, O26's form,
#   5 898 and 7 394; taken at run time 3 703 and 4 296; with the index
#   reduced mod N 3 140 on AArch64), so that the stage stays one loop.
# - a row's levels at the floor and at the top counted in one pass in
#   16-bit counts (`level_counts`, findings O26): two-lane `.2d`
#   instructions in `finish_row` (`vec2`, a ceiling of 32: 16 with rustc
#   1.93.0, from the transform; 170 with two counts in `usize`, which
#   widened each byte to 64-bit lanes).
# - the powers of a row's bins on thumbv8m.main with the cfg at opt-level
#   "s" out of line and with indexed slices (`bin_powers_out_of_line`, findings
#   O26): the shortest loop with four `f32` adds, a ceiling of 35 (34 with
#   rustc 1.93.0; 37 with the zipped iterators out of line, 42 inlined
#   into `finish_row`, where the function is not found).
# The multiplication by an envelope of 1.0 that the reference leaves out
# (the weight folded by the compiler) is not checked: findings O18b.
# A bound that fails means: re-measure before changing it (planning/
# cat_ft8): on the Cortex-A76 for the AArch64 bounds (findings O12, O17,
# O17c, O18 and O19), by emulated instruction counts on the Cortex-M33 for
# the opt-level "s" bounds (findings O20 and O20b).
#
# Needs: rustup targets aarch64-unknown-linux-gnu, x86_64-unknown-linux-gnu,
# thumbv8m.main-none-eabihf and thumbv7em-none-eabihf (assembly only;
# nothing is linked).
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
target_dir="${CARGO_TARGET_DIR:-$here/../target/codegen-check}"
# The builds: a name, the target, cargo's features option, a `--cfg`
# for rustc (`-` for none), an opt-level (`-` for the release
# profile's) and the codegen units (`1`, or `-` for the release
# profile's 16: `a64r` is AArch64 as a release build compiles it, whose
# code the one-unit `a64` need not be).
builds=("a64 aarch64-unknown-linux-gnu - - - 1"
  "a64r aarch64-unknown-linux-gnu - - - -"
  "x64 x86_64-unknown-linux-gnu - - - 1"
  "m33 thumbv8m.main-none-eabihf --no-default-features cat_ft8_minmaxnm - 1"
  "m33s thumbv8m.main-none-eabihf --no-default-features cat_ft8_minmaxnm s 1"
  "m4 thumbv7em-none-eabihf --no-default-features - - 1")
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
# with nine byte loads, have no such loop). The rows passed to
# `costas_scores` by value or `Rows::row` inlined into it leave that loop
# at 15 (findings O25b; the loop of nine loads they lengthened is gone).
# `fploopN` counts
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
# `calls_within_limit` counts calls of `within_limit`, at least the bound;
# `ldn` counts `ld2`, `ld3` and `ld4`, at least the bound;
# `vec2` counts instructions with a two-lane `.2d` operand, at most the bound;
# `insns` counts the function's instructions, at most the bound;
# `mulloopN` counts the instructions of the shortest loop with at least N
# four-lane multiplies (`mulps`; x86-64 only), at most the bound, and a
# function with no such loop fails it likewise;
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
  "a64 min_sum_round vec4 100" "a64r min_sum_round vec4 100"
  "a64 7Removal4refs minmax 2" "a64 correlation_whole minmax 2"
  "a64 7Removal4refs selects 2"
  "a64 7Removal4refs interleaved 0" "a64 4Fit410add_all_as interleaved 0"
  "a64 4Fit410add_all_as vec4 12"
  "a64 6blocks shuffles 12" "a64 6blocks vec4 400"
  "m33 7Removal4refs vminmax 8"
  "m33s 10finish_row fpmul 199" "m33s 10finish_row divides 0" "m33s 10finish_row zerofills 1"
  "m33s 6select sumloop3 16"
  "m33s 4Fit410add_all_as fploop5 15" "m33s 6blocks carries 24"
  "m33s 10Correction absent 0"
  "a64 6blocks cvtloop4 104"
  "m33s 6blocks cvtloop1 31" "m33s 3run28_\$u7b\$\$u7b\$closure cvtloop1 31 one" "m33s 6blocks fpneg 0"
  "a64 correlation_whole cvtloop4 110" "a64 correlation_whole eors 16"
  "m33s fill_phase_only memsums 0" "m33s 9skip_with memsums 0"
  "m4 7Removal4refs libcalls 0" "m4 symbol_correlations libcalls 0" "m4 4Fit410add_all_as libcalls 0"
  "a64 within_limit vecor 2" "m33s within_limit shortloop 4" "m33s 7Removal4refs fploop4 21"
  "a64 4feed calls_within_limit 2" "m33s 4feed calls_within_limit 2"
  "a64 10finish_row ldn 12" "x64 10finish_row mulloop80 460"
  "a64 10finish_row insns 3000" "x64 10finish_row insns 3500"
  "a64 10finish_row vec2 32" "m33s bin_powers_out_of_line fploop4 35")

declare -A asm
for b in "${builds[@]}"; do
  set -- $b
  key=$1 target=$2 cargo_opt=$3 rustc_opt=""
  [ "$cargo_opt" = - ] && cargo_opt=""
  [ "$4" = - ] || rustc_opt="--cfg $4"
  [ "$5" = - ] || rustc_opt="$rustc_opt -C opt-level=$5"
  # One unit's assembly is one file; with 16, named here because rustc
  # takes one unit for `--emit=asm` unless told, one file a unit, read
  # as one.
  units=$6
  [ "$units" = - ] && units=16
  dir="$target_dir/$key"
  # Clean the library first: cargo would otherwise see it as fresh, run
  # no compiler and write no assembly.
  CARGO_TARGET_DIR="$dir" cargo clean -q --release -p cat-ft8 --target "$target" \
    --manifest-path "$here/Cargo.toml"
  # shellcheck disable=SC2086 # the options are deliberately split
  CARGO_TARGET_DIR="$dir" cargo rustc -q --release -p cat-ft8 --lib --target "$target" \
    $cargo_opt --manifest-path "$here/Cargo.toml" -- --emit=asm -C codegen-units="$units" $rustc_opt
  if [ "$units" = 1 ]; then
    asm[$key]=$(ls "$dir/$target/release/deps/"cat_ft8-*.s)
  else
    cat "$dir/$target/release/deps/"cat_ft8-*.rcgu.s > "$dir/units.s"
    asm[$key]="$dir/units.s"
  fi
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
    metric ~ /^(sumloop|fploop|cvtloop|memsums|shortloop|mulloop)/ && /^\.[A-Za-z0-9_$]+:/ {
      sub(/:.*/, ""); at[$0] = k; atb[$0] = bytes; atf[$0] = fps; atc[$0] = cvs
      atl[$0] = lds; atv[$0] = vs; atm[$0] = muls; next
    }
    metric ~ /^(sumloop|fploop|cvtloop|memsums|shortloop|mulloop)/ && /^\t[a-z]/ {
      k++
      if ($1 ~ /^ldrb/) bytes++
      if ($1 ~ /^v(mla|mls|add|sub)\.f32$/) fps++
      if ($1 ~ /^(scvtf|vcvt\.f32\.s32)$/) cvs++
      if ($1 ~ /^ldr/) lds++
      if ($1 ~ /^v/) vs++
      if ($1 == "mulps") muls++
      # A back branch: AArch64 and Thumb, or x86-64 (`j` and a condition).
      back = $1 ~ /^(b[a-z]*(\.[nw])?|b\.[a-z]+|cbn?z|tbn?z|j[a-z]+)$/ && ($NF in at)
      if (back && metric ~ /^sumloop/ && bytes - atb[$NF] == substr(metric, 8) + 0 &&
          (best == "" || k - at[$NF] < best)) best = k - at[$NF]
      if (back && metric ~ /^fploop/ && fps - atf[$NF] >= substr(metric, 7) + 0 &&
          (best == "" || k - at[$NF] < best)) best = k - at[$NF]
      if (back && metric ~ /^cvtloop/ && cvs - atc[$NF] >= substr(metric, 8) + 0 &&
          (best == "" || k - at[$NF] < best)) best = k - at[$NF]
      if (back && metric ~ /^mulloop/ && muls - atm[$NF] >= substr(metric, 8) + 0 &&
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
    metric == "calls_within_limit" && /^\tbl\t.*within_limit/ { n++ }
    metric == "insns" && /^\t[a-z]/ { n++ }
    metric == "ldn" && /^\tld[234]\t/ { n++ }
    metric == "vec2" && /^\t[a-z][^\t]*\t.*\.2d/ { n++ }
    END {
      if (!found) print "missing"
      else if (only == "one" && found > 1) print "many"
      else if (metric ~ /^(sumloop|fploop|cvtloop|shortloop|mulloop)/) print (best == "" ? "noloop" : best)
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
      mulloop*) echo "FAIL $key $name: no loop with ${metric#mulloop} four-lane multiplies (not vectorised?)" ;;
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
    calls_within_limit) what="calls of within_limit (floor $bound)"; [ "$n" -ge "$bound" ] && ok=1 || ok=0 ;;
    insns) what="instructions (ceiling $bound)"; [ "$n" -le "$bound" ] && ok=1 || ok=0 ;;
    mulloop*) what="instructions in the loop of ${metric#mulloop} four-lane multiplies (ceiling $bound)"; [ "$n" -le "$bound" ] && ok=1 || ok=0 ;;
    ldn) what="interleaving loads (floor $bound)"; [ "$n" -ge "$bound" ] && ok=1 || ok=0 ;;
    vec2) what="two-lane .2d instructions (ceiling $bound)"; [ "$n" -le "$bound" ] && ok=1 || ok=0 ;;
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
