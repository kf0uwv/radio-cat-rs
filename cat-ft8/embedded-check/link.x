/* Copyright 2026 Matt Franklin
 *
 * Licensed under the Apache License, Version 2.0 (the "License");
 * you may not use this file except in compliance with the License.
 * You may obtain a copy of the License at
 *
 *     http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */

/* Minimal memory map for the cat-ft8 link check: a Cortex-M33-class part
 * with 1 MiB of flash and 512 KiB of RAM (the small image's statics fit in
 * 256 KiB; the host image's, with the subtraction's working storage, do
 * not). Only sizes matter; the program is linked, measured and inspected,
 * never run. check.sh prints each image's static RAM and fails above each
 * image's own ceiling (256 KiB small and encoder-only, 512 KiB host). */
MEMORY
{
  FLASH : ORIGIN = 0x00000000, LENGTH = 1M
  RAM   : ORIGIN = 0x20000000, LENGTH = 512K
}

ENTRY(_start);

SECTIONS
{
  .text : { KEEP(*(.text._start)) *(.text .text.*) } > FLASH
  .rodata : { *(.rodata .rodata.*) } > FLASH
  .data : { *(.data .data.*) } > RAM AT > FLASH
  .bss (NOLOAD) : { *(.bss .bss.*) *(COMMON) } > RAM
  .stack_sizes (INFO) : { KEEP(*(.stack_sizes)) }
}
