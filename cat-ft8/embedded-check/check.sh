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
# Lints and formats the bare-metal check program, links it for
# thumbv8m.main-none-eabihf in three images (the small decoder
# configuration, the host configuration, and the encoder alone), prints
# each image's size (flash = .text + .rodata + .data; static RAM = .data +
# .bss), and fails if any image has unresolved symbols or references
# soft-double helpers (`__aeabi_d*`, `__*df*`): the core must use `f32`
# only. Only code reachable from
# `src/main.rs` is linked and so checked. With `--stack`, also prints the
# largest per-function stack frames (`-Z emit-stack-sizes`, enabled on
# stable through RUSTC_BOOTSTRAP for this measurement only).
#
# Needs: rustup target thumbv8m.main-none-eabihf, rustup component llvm-tools.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
target=thumbv8m.main-none-eabihf
bin_dir="$(rustc --print sysroot)/lib/rustlib/$(rustc -vV | sed -n 's/^host: //p')/bin"
nm="$bin_dir/llvm-nm"; size="$bin_dir/llvm-size"; readobj="$bin_dir/llvm-readobj"
[ -x "$nm" ] || { echo "llvm-tools missing: rustup component add llvm-tools" >&2; exit 1; }

# Default: the repository's target directory, never inside the sources.
target_dir="${CARGO_TARGET_DIR:-$here/../../target/embedded-check}"
flags=""
if [ "${1:-}" = "--stack" ]; then
  export RUSTC_BOOTSTRAP=1
  flags="-Z emit-stack-sizes"
  target_dir="$target_dir/stack-sizes"
fi
echo "== fmt (check program)"
rustfmt --edition 2021 --check "$here/src/main.rs" "$here/build.rs"

# One image per decoder configuration, plus the encoder alone as a
# baseline for the decoder's size.
for variant in small host encoder-only; do
  case "$variant" in
    small) features="" ;;
    *) features="--features $variant" ;;
  esac
  echo "==== image: $variant"
  # shellcheck disable=SC2086 # $features is deliberately split
  cargo clippy --quiet --manifest-path "$here/Cargo.toml" --target "$target" \
    --target-dir "$target_dir/$variant" $features -- -D warnings
  # shellcheck disable=SC2086
  RUSTFLAGS="$flags" cargo build --quiet --release --manifest-path "$here/Cargo.toml" \
    --target "$target" --target-dir "$target_dir/$variant" $features
  elf="$target_dir/$variant/$target/release/cat-ft8-embedded-check"

  echo "== size ($elf)"
  "$size" -A "$elf" | grep -E '^(section|\.text|\.rodata|\.data|\.bss)'
  "$size" -A "$elf" | awk '$1==".text"||$1==".rodata"||$1==".data"{f+=$2} $1==".data"||$1==".bss"{r+=$2}
    END {printf "flash %d B, RAM (static) %d B\n", f, r}'
  echo "== undefined symbols (must be none)"
  undef="$("$nm" -u "$elf" || true)"
  echo "${undef:-none}"
  [ -z "$undef" ] || { echo "FAIL: unresolved symbols" >&2; exit 1; }
  echo "== double-precision helpers (must be none)"
  doubles="$("$nm" "$elf" | awk '{print $NF}' \
    | grep -E '^(__aeabi_d|__aeabi_(f|i|ui|l|ul)2d|__[a-z]+df[a-z0-9]*$)' || true)"
  echo "${doubles:-none}"
  [ -z "$doubles" ] || { echo "FAIL: f64 soft-float helpers linked" >&2; exit 1; }

  if [ -n "$flags" ]; then
    echo "== largest stack frames (bytes, per function; precompiled core has none recorded)"
    "$readobj" --stack-sizes "$elf" | awk '/Functions:/{f=$2} /^ *Size: 0x/{print $2, f}' \
      | while read -r hex name; do printf '%6d %s\n' "$((hex))" "$name"; done \
      | sort -rn | head -8
  fi
done
echo "OK"
