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
# (its last stage out of line, by rows of twiddles, `last_straight_stage`)
# and its last stage one vectorised loop on AArch64 and x86-64, and that
# the sync search's running sums keep their loop there and its column sums
# their 16-bit vectors on the hosts, that min-sum's checks stay written
# out edge by edge on the Cortex-M33 (`min_sum_check_bits!`, one block a
# degree), that a row's levels there take a guess and one step rather
# than the logarithm, in their own loop (`levels_by_steps_out_of_line`),
# that the front end's window there is a ring of quarters, not slid each
# row, that on 32-bit x86 (SSE2, which takes vectors for the transform but
# the scalar forms elsewhere) the front end keeps the vector transform
# and its sliding window, all three of its window's choices together,
# and that the belief-propagation rounds take their variables' totals
# without a bounds check.
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
# for AArch64 and x86-64 also the release profile's 16; no std for the
# microcontrollers) for each target and fails if a function's count is
# past its bound. Each check below has one line on what it guards; the
# counts each bound was set against, and the forms run against it, are in
# planning/cat_ft8 (findings, "The code-generation check's bounds").
# The multiplication by an envelope of 1.0 that the reference leaves out
# (the weight folded by the compiler) is not checked: findings O18b.
# A bound that fails means: re-measure before changing it (planning/
# cat_ft8): on the Cortex-A76 for the AArch64 bounds (findings O12, O17,
# O17c, O18 and O19), by emulated instruction counts on the Cortex-M33 for
# the opt-level "s" bounds (findings O20 and O20b).
#
# Needs: rustup targets aarch64-unknown-linux-gnu, x86_64-unknown-linux-gnu,
# thumbv8m.main-none-eabihf, thumbv7em-none-eabihf and i686-linux-android
# (assembly only; nothing is linked).
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
target_dir="${CARGO_TARGET_DIR:-$here/../target/codegen-check}"
# The builds: a name, the target, cargo's features option, a `--cfg`
# for rustc (`-` for none), an opt-level (`-` for the release
# profile's) and the codegen units (`1`, or `-` for the release
# profile's 16: `a64r` and `x64r` are AArch64 and x86-64 as a release
# build compiles them, whose code the one-unit builds need not be). 16 is
# cargo's default for the release profile; if a profile sets
# `codegen-units`, the `-` here must follow it.
builds=("a64 aarch64-unknown-linux-gnu - - - 1"
  "a64r aarch64-unknown-linux-gnu - - - -"
  "x64 x86_64-unknown-linux-gnu - - - 1"
  "x64r x86_64-unknown-linux-gnu - - - -"
  "m33 thumbv8m.main-none-eabihf --no-default-features cat_ft8_minmaxnm - 1"
  "m33s thumbv8m.main-none-eabihf --no-default-features cat_ft8_minmaxnm s 1"
  "m4 thumbv7em-none-eabihf --no-default-features - - 1"
  "x86 i686-linux-android - - - 1")
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
# bound; `divides` counts `umull` and `udiv` (Thumb), and `mull` and
# `divl` (32-bit x86, where a division by a constant is a widening
# multiply), at most the bound;
# `zerofills` counts calls of `__aeabi_memclr4`, `__aeabi_memclr` and
# `memset`, at most the bound; `memmoves` counts calls of
# `__aeabi_memmove4`, `__aeabi_memmove`, `memmove` and a `copy_within`
# left out of line (`bl` on Thumb, `call` on x86), at most the bound;
# `memmovefloor` counts the same, at least the bound; `sumloopN` counts
# the instructions of the shortest loop with N byte loads, at most the bound, and a function with no
# such loop fails it with its own message; `fploopN` counts
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
# `vec8h` counts instructions with an eight-lane 16-bit `.8h` operand
# (AArch64), at least the bound;
# `paddw` counts `paddw` and `vpaddw` (16-bit vector adds; x86-64), at least
# the bound;
# `uaddw8h` counts `uaddw` and `uaddw2` into `.8h` (bytes widened and added
# to 16-bit lanes; AArch64), at least the bound;
# `add2d` counts two-lane 64-bit vector adds (`add` on `.2d`), at least the
# bound;
# `insns` counts the function's instructions, at most the bound;
# `fpcmps` counts scalar floating-point compares (`fcmp` on AArch64,
# `ucomiss` and `comiss`, with or without the VEX `v`, on x86-64), at
# least the bound;
# `fpbranches` counts floating-point compares (`fcmp` on AArch64, `ucomiss`
# and `comiss` on x86-64) followed at once by a conditional branch
# (`b.cond`, `jcc`), at most the bound: a narrow pattern, which does not
# see a compare whose branch comes later or takes another form (`fccmp`,
# `csel` then `cbnz`), so it guards against the one form the compiler
# gave, and needs the `absent` checks on what it inlines;
# `mulloopN` counts the instructions of the shortest loop with at least N
# four-lane multiplies (`mulps`; x86-64 only), at most the bound, and a
# function with no such loop fails it likewise;
# `vmulloopN` counts the instructions of the longest loop with exactly N
# `f32` multiplies (`vmul.f32`; Thumb), at most the bound, and a function
# with no such loop fails it likewise;
# `mulps` counts four-lane multiplies (`mulps`, with or without the VEX
# `v`; x86), at least the bound;
# `onelane` counts the instructions that name the one-lane transform's
# tables or its last stage (`FIFTH_TWIDDLES`, `LAST_ROWS`,
# `last_straight_stage`), at most the bound;
# `absent` passes only if no function's label holds the name (the bound is
# unused), that is, if it was inlined everywhere; it sees only a type's own
# methods (`10Correction`), not a trait's (`Clone` for it, say) left out of
# line. Otherwise a function whose label is not found, or an unknown
# metric, fails. A count is summed over every function whose label holds
# the name, and a loop metric takes the shortest loop among them; a
# check with a fifth field `one` also fails if more than one label holds
# the name (the subtraction's closures are named only by the function
# that holds them, so another closure in it could pass for the products;
# with 16 units a function copied into two units would be summed). A
# fifth field `each`, for the floors only, takes each function's count
# and checks the least of them, so that one instantiation of a generic
# function cannot pass on another's count (a ceiling is already checked
# most strictly by the sum). It does not check how many functions match,
# so an instantiation inlined away leaves the check on the others.
checks=(
  # The sum-product round's tanh and atanh passes 24 values at a time and its group products in vectors.
  "a64 sum_product_round vec4 500"
  # Its group products as whole vectors, not lane by lane.
  "a64 sum_product_round lanes 25"
  # Min-sum's groups of four checks in the lanes of a vector.
  "a64 min_sum_round vec4 100"
  # The same in the release form, which left the groups scalar while the one-unit build passed.
  "a64r min_sum_round vec4 100 one"
  # Its groups of degree 6 vectorised there too (1 204 instructions with them scalar).
  "a64r min_sum_round insns 1000 one"
  # The correction's lanes out of line, so their loop is two-lane 64-bit adds: each of `blocks`' two instantiations.
  "a64 6blocks add2d 2 each"
  # The same in the release form (the second `blocks` is inlined into `subtract::run` there, unchecked).
  "a64r 6blocks add2d 2 each"
  # The sum-product inputs' blocks of eight without a bounds check or clamp per edge (46-48 clamped, 61-63 checked).
  "a64 sum_product_round sumloop8 40"
  # The same in the release form.
  "a64r sum_product_round sumloop8 40 one"
  # Min-sum check by check, each degree's check written out edge by edge (a loop of 6 and of 7 byte
  # loads), its inputs' totals without a bounds check (the totals padded to any u8 index).
  "m33s min_sum_round sumloop6 204"
  "m33s min_sum_round sumloop7 233"
  # The sum-product inputs likewise, one edge at a time.
  "m33s sum_product_round sumloop1 10"
  # The stopping test's hard decisions without a branch on each value's sign (4 with it); it reads `decode_soft` only, so the two checks after it.
  "a64 decode_soft fpbranches 0"
  # The stopping test inlined into `decode_soft`, so the check above cannot pass with it elsewhere.
  "a64 checks_hold absent 0"
  # The hard decision likewise.
  "a64 hard_bit absent 0"
  # The same three in the release form (3 with the branch).
  "a64r decode_soft fpbranches 0 one"
  "a64r checks_hold absent 0"
  "a64r hard_bit absent 0"
  # The same three on x86-64 (`ucomiss` then a jump; 15 with the branch).
  "x64 decode_soft fpbranches 0"
  "x64 checks_hold absent 0"
  "x64 hard_bit absent 0"
  # The stopping test's compares are in `decode_soft` (9 on AArch64, 15 on x86-64; 4 and 10 with it out of line), so its checks above cannot pass because the test went elsewhere under another name.
  "a64 decode_soft fpcmps 8"
  "a64r decode_soft fpcmps 8 one"
  "x64 decode_soft fpcmps 14"
  # The removal's clamp as four-lane fmaxnm/fminnm.
  "a64 7Removal4refs minmax 2"
  # The time search's clamp likewise.
  "a64 correlation_whole minmax 2"
  # The removal's clamp not as compares and selects.
  "a64 7Removal4refs selects 2"
  # The removal's reference loaded as whole vectors, not interleaved.
  "a64 7Removal4refs interleaved 0"
  # The fit's sums likewise.
  "a64 4Fit410add_all_as interleaved 0"
  # The fit's four-lane step kept inlined (the ceiling above needs this floor).
  "a64 4Fit410add_all_as vec4 12"
  # The reference's block of 16 samples stored without shuffles.
  "a64 6blocks shuffles 12"
  # `scaled_cos_sin` kept inlined into `blocks` (the ceilings on `blocks` need this floor).
  "a64 6blocks vec4 400"
  # The removal's clamp as vmaxnm/vminnm with the cfg.
  "m33 7Removal4refs vminmax 8"
  # The FFT written out butterfly by butterfly: its stages before the last
  # in `finish_row` (118), the last out of line (below).
  "m33s 10finish_row fpmul 118"
  # The last stage by rows of twiddles, range by range: each of its six
  # ranges written out (80 multiplies a butterfly), its shortest loop (204)
  # and the whole stage (1 584), whose ranges' loops grow when the rows'
  # starts are seen as constants (`black_box`).
  "m33s last_straight_stage fpmul 480"
  "m33s last_straight_stage fploop60 210"
  "m33s last_straight_stage insns 1630"
  # The FFT's twiddle index without a division.
  "m33s 10finish_row divides 0"
  # The FFT without zero fills beyond the one.
  "m33s 10finish_row zerofills 1"
  # The row from the window's ring of quarters, each quarter's pair of
  # samples read without an array of the four built a block, the first
  # stage's blocks in the order of their inputs, each later stage's k = 0
  # on its own (1 032; 1 128 in the order of the blocks; 758 without
  # k = 0 on its own; the whole row, so a form that adds work anywhere in
  # it can pass by removing it elsewhere: re-measure on such a change).
  "m33s 10finish_row insns 1040"
  # The same scoped to the first stage's loop, the row's longest loop
  # with exactly eight `vmul.f32`, its window products (115; 147 in the
  # order of the blocks, through the digit-reversal table; the radix-3
  # stage's k = 0 has eight too, in 53 instructions).
  "m33s 10finish_row vmulloop8 120"
  # Each later stage's k = 0 on its own, its twiddles constants: a radix-4
  # stage's butterflies there with no multiply by 1 (84 instructions; the
  # shortest loop of a radix-4 stage at other k, 95).
  "m33s 10finish_row fploop34 90"
  # The window as a ring: no slide of the window (a memmove of 11 520
  # bytes a row) in the front end's feeds.
  "m33s 11Spectrogram4feed memmoves 0"
  "m33s 11Spectrogram8feed_i16 memmoves 0"
  "m33s 11Spectrogram4skip memmoves 0"
  # The sync search's running sums: three levels a bin, a pair of bins a
  # step (25 instructions: six byte loads a pair; with one parity at a
  # time and the eight tones' sum, 15 a bin with three).
  "m33s 6select sumloop6 26"
  # On the hosts its column sums in 16-bit vector lanes (the eight tones
  # added afresh a row, the form before: 13 and 0), the levels widened
  # into them as they are added (18 on AArch64; `vec8h` alone passes the
  # sums in 32-bit lanes), in the one-unit builds and as the release
  # profile compiles them.
  "a64 6select vec8h 24"
  "x64 6select paddw 16"
  "a64 6select uaddw8h 16 one"
  "a64r 6select uaddw8h 16 one"
  "x64r 6select paddw 16 one"
  # The fit one lane at a time with its sums in registers.
  "m33s 4Fit410add_all_as fploop5 15"
  # The frequency correction one sample at a time, not in lanes.
  "m33s 6blocks carries 24"
  # No method of `Correction` out of line.
  "m33s 10Correction absent 0"
  # The reference polynomial's remainder by one shift and quarter by one sign word.
  "a64 6blocks cvtloop4 104"
  # The same, one lane at a time with the quarter's swap as vsel.
  "m33s 6blocks cvtloop1 31"
  # The time search's products in the subtraction's closure: one lane at a time, the sign on the weight.
  "m33s 3run28_\$u7b\$\$u7b\$closure cvtloop1 31 one"
  # The half turn's sign after the weight, not a negation.
  "m33s 6blocks fpneg 0"
  # The time search's whole lanes out of line, its sums not split and spilled.
  "a64 correlation_whole cvtloop4 110"
  # The half turn's sign on the weight, one eor a vector.
  "a64 correlation_whole eors 16"
  # The generator's overlap increments summed as they are taken.
  "m33s fill_phase_only memsums 0"
  # The same in `skip_with`.
  "m33s 9skip_with memsums 0"
  # No library max/min without the cfg: the removal.
  "m4 7Removal4refs libcalls 0"
  # The same: the symbols' correlations.
  "m4 symbol_correlations libcalls 0"
  # The same: the fit.
  "m4 4Fit410add_all_as libcalls 0"
  # The input's check as a vectorised or pass.
  "a64 within_limit vecor 2"
  # The same, four instructions a sample.
  "m33s within_limit shortloop 4"
  # The removal's form without the clamp, the one audio takes.
  "m33s 7Removal4refs fploop4 21"
  # The front end checks each block with `within_limit`.
  "a64 4feed calls_within_limit 2"
  # The same.
  "m33s 4feed calls_within_limit 2"
  # The FFT's last stage one loop, its twiddles as interleaved loads.
  "a64 10finish_row ldn 12"
  # The same loop vectorised on x86-64.
  "x64 10finish_row mulloop80 460"
  # The last stage one loop, not one per segment.
  "a64 10finish_row insns 3000"
  # The same on x86-64.
  "x64 10finish_row insns 3500"
  # A row's level counts in 16 bits, not widened to 64-bit lanes.
  "a64 10finish_row vec2 32"
  # The same transform as the release profile compiles it (16 units), where
  # `finish_row` is inlined into `power_bins`: the last stage one loop with
  # its twiddles as interleaved loads, not one per segment, and a row's
  # level counts not widened to 64-bit lanes on AArch64; on x86-64 the last
  # stage's loop of 80 four-lane multiplies.
  "a64r power_bins ldn 12 one"
  "a64r power_bins insns 3000 one"
  "a64r power_bins vec2 32 one"
  "x64r power_bins mulloop80 470 one"
  "x64r power_bins insns 3500 one"
  # On 32-bit x86 (SSE2: vectors for the transform, the scalar forms
  # elsewhere) the front end keeps the vector transform and its sliding
  # window: no one-lane table or last stage in the row (O31 read 56 of
  # `FIFTH_TWIDDLES` there), and its four-lane multiplies (256; 171 with
  # the one-lane transform).
  "x86 10finish_row onelane 0"
  "x86 10finish_row mulps 240"
  # And its feeds there: the window slid once a row (one memmove; a feed
  # that stopped sliding has none) and written in order (no division by the
  # hop, which the ring's index takes). With the row's checks above, a
  # change to any one of the window's three choices (the feeds' index and
  # slide in `take`, the row's transform in `finish_row`) fails.
  "x86 11Spectrogram4feed memmoves 1"
  "x86 11Spectrogram4feed memmovefloor 1"
  "x86 11Spectrogram4feed divides 0"
  "x86 11Spectrogram8feed_i16 memmoves 1"
  "x86 11Spectrogram8feed_i16 memmovefloor 1"
  "x86 11Spectrogram8feed_i16 divides 0"
  "x86 11Spectrogram4skip memmoves 1"
  "x86 11Spectrogram4skip memmovefloor 1"
  "x86 11Spectrogram4skip divides 0"
  # The bins' powers out of line with indexed slices.
  "m33s bin_powers_out_of_line fploop4 35"
  # The levels of a row's powers by a guess and one step, not the logarithm, out of line.
  "m33s levels_by_steps_out_of_line shortloop 16")

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
    shopt -s nullglob
    unit_files=("$dir/$target/release/deps/"cat_ft8-*.rcgu.s)
    shopt -u nullglob
    if [ ${#unit_files[@]} = 0 ]; then
      echo "FAIL $key: no per-unit assembly (cat_ft8-*.rcgu.s) from the build with $units codegen units" >&2
      exit 1
    fi
    cat "${unit_files[@]}" > "$dir/units.s"
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
    # A function ends at its end label or at the next global label;
    # `fpbranches` forgets a compare there, so it cannot pair with a
    # branch at the start of the next function.
    /^\.Lfunc_end[0-9]+:/ { p = 0; fcmp = 0; next }
    /^[A-Za-z_][^ \t]*:$/ { p = index($0, f) > 0; if (p) found++; delete at; fcmp = 0; next }
    !p { next }
    metric ~ /^(sumloop|fploop|cvtloop|memsums|shortloop|mulloop|vmulloop)/ && /^\.[A-Za-z0-9_$]+:/ {
      sub(/:.*/, ""); at[$0] = k; atb[$0] = bytes; atf[$0] = fps; atc[$0] = cvs
      atl[$0] = lds; atv[$0] = vs; atm[$0] = muls; atu[$0] = vmuls; next
    }
    metric ~ /^(sumloop|fploop|cvtloop|memsums|shortloop|mulloop|vmulloop)/ && /^\t[a-z]/ {
      k++
      if ($1 ~ /^ldrb/) bytes++
      if ($1 ~ /^v(mla|mls|add|sub)\.f32$/) fps++
      if ($1 ~ /^(scvtf|vcvt\.f32\.s32)$/) cvs++
      if ($1 ~ /^ldr/) lds++
      if ($1 ~ /^v/) vs++
      if ($1 == "mulps") muls++
      if ($1 == "vmul.f32") vmuls++
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
      if (back && metric ~ /^vmulloop/ && vmuls - atu[$NF] == substr(metric, 9) + 0 &&
          (best == "" || k - at[$NF] > best)) best = k - at[$NF]
      if (back && metric == "memsums" && lds > atl[$NF] && vs == atv[$NF]) n++
      if (back && metric == "shortloop" && (best == "" || k - at[$NF] < best)) best = k - at[$NF]
    }
    metric == "vec4" && /^\t[a-z][^\t]*\t.*\.4s/ { n++; nf[found]++ }
    metric == "lanes" && /^\t(mov|ins|dup|ld1|st1)\t.*\.s\[/ { n++; nf[found]++ }
    metric == "minmax" && /^\t(fmaxnm|fminnm)\t.*\.4s/ { n++; nf[found]++ }
    metric == "selects" && /^\t(fcmgt|bsl|bit|bif)\t/ { n++; nf[found]++ }
    metric == "vminmax" && /^\t(vmaxnm|vminnm)/ { n++; nf[found]++ }
    metric == "libcalls" && /^\tbl\t(fmaxf|fminf)$/ { n++; nf[found]++ }
    metric == "interleaved" && /^\t(ld[234]|st[234])\t/ { n++; nf[found]++ }
    metric == "shuffles" && /^\t(zip[12]|uzp[12]|trn[12])\t/ { n++; nf[found]++ }
    metric == "fpmul" && /^\tv(mul|mla|mls)\.f32\t/ { n++; nf[found]++ }
    metric == "divides" && /^\t(umull|udiv|mull|divl)\t/ { n++; nf[found]++ }
    metric == "zerofills" && /^\tbl\t(__aeabi_memclr4|__aeabi_memclr|memset)$/ { n++; nf[found]++ }
    metric ~ /^memmove(s|floor)$/ && /^\t(bl|calll?|callq)\t(__aeabi_memmove4|__aeabi_memmove|memmove|.*copy_within.*)(@PLT)?$/ { n++; nf[found]++ }
    metric == "carries" && /^\tadc/ { n++; nf[found]++ }
    metric == "fpneg" && /^\tvneg\.f32\t/ { n++; nf[found]++ }
    metric == "eors" && /^\teor\t/ { n++; nf[found]++ }
    metric == "vecor" && /^\torr\tv[0-9]+\./ { n++; nf[found]++ }
    metric == "calls_within_limit" && /^\tbl\t.*within_limit/ { n++; nf[found]++ }
    metric == "insns" && /^\t[a-z]/ { n++; nf[found]++ }
    metric == "fpbranches" && /^\t[a-z]/ {
      if (fcmp && ($1 ~ /^b\.[a-z]+$/ || ($1 ~ /^j[a-z]+$/ && $1 != "jmp"))) { n++; nf[found]++ }
      fcmp = $1 ~ /^(fcmp|v?u?comiss)$/
    }
    metric == "fpcmps" && /^\t(fcmp|v?u?comiss)\t/ { n++; nf[found]++ }
    metric == "ldn" && /^\tld[234]\t/ { n++; nf[found]++ }
    metric == "vec2" && /^\t[a-z][^\t]*\t.*\.2d/ { n++; nf[found]++ }
    metric == "vec8h" && /^\t[a-z][^\t]*\t.*\.8h/ { n++; nf[found]++ }
    metric == "paddw" && /^\tv?paddw\t/ { n++; nf[found]++ }
    metric == "uaddw8h" && /^\tuaddw2?\tv[0-9]+\.8h/ { n++; nf[found]++ }
    metric == "add2d" && /^\tadd\tv[0-9]+\.2d/ { n++; nf[found]++ }
    metric == "mulps" && /^\tv?mulps\t/ { n++; nf[found]++ }
    metric == "onelane" && /^\t[a-z]/ && /(FIFTH_TWIDDLES|LAST_ROWS|last_straight_stage)/ { n++; nf[found]++ }
    END {
      if (!found) print "missing"
      else if (only == "one" && found > 1) print "many"
      else if (only == "each" && metric ~ /^(sumloop|fploop|cvtloop|shortloop|mulloop|vmulloop|memsums)/) print "noteach"
      else if (metric ~ /^(sumloop|fploop|cvtloop|shortloop|mulloop|vmulloop)/) print (best == "" ? "noloop" : best)
      else if (only == "each") {
        least = nf[1] + 0
        for (i = 2; i <= found; i++) if (nf[i] + 0 < least) least = nf[i] + 0
        print least
      }
      else print n + 0
    }' "${asm[$key]}")
  if [ "$n" = noteach ] || { [ "$only" = each ] && ! [[ "$metric" =~ ^(vec4|minmax|vminmax|fpmul|vecor|calls_within_limit|ldn|add2d|fpcmps|vec8h|paddw|uaddw8h|mulps)$ ]]; }; then
    echo "FAIL $key $name: \`each\` takes a floor, not $metric"
    fail=1
    continue
  fi
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
      vmulloop*) echo "FAIL $key $name: no loop with exactly ${metric#vmulloop} f32 multiplies (moved or unrolled?)" ;;
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
    fpcmps) what="f32 compares (floor $bound)"; [ "$n" -ge "$bound" ] && ok=1 || ok=0 ;;
    divides) what="divisions (ceiling $bound)"; [ "$n" -le "$bound" ] && ok=1 || ok=0 ;;
    zerofills) what="zero-fill calls (ceiling $bound)"; [ "$n" -le "$bound" ] && ok=1 || ok=0 ;;
    memmoves) what="memmove or copy_within calls (ceiling $bound)"; [ "$n" -le "$bound" ] && ok=1 || ok=0 ;;
    memmovefloor) what="memmove or copy_within calls (floor $bound)"; [ "$n" -ge "$bound" ] && ok=1 || ok=0 ;;
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
    fpbranches) what="f32 compares followed by a branch (ceiling $bound)"; [ "$n" -le "$bound" ] && ok=1 || ok=0 ;;
    mulloop*) what="instructions in the loop of ${metric#mulloop} four-lane multiplies (ceiling $bound)"; [ "$n" -le "$bound" ] && ok=1 || ok=0 ;;
    vmulloop*) what="instructions in the loop of ${metric#vmulloop} f32 multiplies (ceiling $bound)"; [ "$n" -le "$bound" ] && ok=1 || ok=0 ;;
    mulps) what="four-lane multiplies (floor $bound)"; [ "$n" -ge "$bound" ] && ok=1 || ok=0 ;;
    onelane) what="references to the one-lane transform (ceiling $bound)"; [ "$n" -le "$bound" ] && ok=1 || ok=0 ;;
    ldn) what="interleaving loads (floor $bound)"; [ "$n" -ge "$bound" ] && ok=1 || ok=0 ;;
    vec2) what="two-lane .2d instructions (ceiling $bound)"; [ "$n" -le "$bound" ] && ok=1 || ok=0 ;;
    vec8h) what=".8h instructions (floor $bound)"; [ "$n" -ge "$bound" ] && ok=1 || ok=0 ;;
    paddw) what="16-bit vector adds (floor $bound)"; [ "$n" -ge "$bound" ] && ok=1 || ok=0 ;;
    uaddw8h) what="bytes widened and added to .8h (floor $bound)"; [ "$n" -ge "$bound" ] && ok=1 || ok=0 ;;
    add2d) what="two-lane 64-bit adds (floor $bound)"; [ "$n" -ge "$bound" ] && ok=1 || ok=0 ;;
    *) what="unknown metric $metric"; ok=0 ;;
  esac
  [ "$only" = each ] && what="$what, the least of each function's"
  if [ $ok = 1 ]; then
    echo "ok   $key $name: $n $what"
  else
    echo "FAIL $key $name: $n $what"
    fail=1
  fi
done
exit $fail
