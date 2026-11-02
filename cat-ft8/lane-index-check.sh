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
# Fails if any source file of the crate (`src`, tests included) indexes an
# array by an expression taken modulo, or masked to, a lane count: `% LANES`,
# `% GROUP`, `% 2`..`% 64` (powers of two), `& 1`..`& 63`, `& 0x..`,
# `& (LANES - 1)`. Rust has no way to see that such an index wraps, and
# LLVM's loop vectoriser (LLVM 20 and 21, rustc 1.93.0 among them) has
# widened a loop over `acc[(i + a) % 4] += …` as if the index ran on
# without wrapping: it wrote lanes past the array and never lane 0, with
# no unsafe code involved (planning/cat_ft8, findings O19b). The crate
# sums by lane with selects (`add_lane`) or with a loop per lane instead.
# The check is textual: it sees the index written inside the brackets,
# the form that was miscompiled. An index first bound to a variable
# (`let l = (i + a) % LANES; acc[l] += …`) is not seen and needs review;
# a table read at a non-affine modulus (`% 1920`, `% 38`) is not flagged.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
pattern='\[[^]]*(%[[:space:]]*(LANES|GROUP|2|4|8|16|32|64)\b|&[[:space:]]*\(?[[:space:]]*(LANES[[:space:]]*-[[:space:]]*1|GROUP[[:space:]]*-[[:space:]]*1|0x[0-9a-fA-F]+|1|3|7|15|31|63)\b)[^]]*\]'
# Comment lines are not code.
hits=$(grep -rnE "$pattern" --include='*.rs' "$here/src" | grep -vE '^[^:]+:[0-9]+:[[:space:]]*//' || true)
if [ -n "$hits" ]; then
  echo "FAIL array indexed by a lane-wrapping expression (see this script's header):"
  echo "$hits"
  exit 1
fi
echo "ok   no array indexed by a lane-wrapping expression in $here/src"
