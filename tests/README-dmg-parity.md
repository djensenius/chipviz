# Opt-in DMG emulator parity

This test builds `cores/gb/homebrew/main.asm` with `CHIPVIZ_FIXTURE` in a
temporary directory using RGBDS (`rgbasm`, `rgblink`, `rgbfix` on `PATH`).
Set `GB_LIBRETRO_CORE` to a local SameBoy libretro shared library, or use the
default from `scripts/gb_libretro_capture.py`. Python 3 and a headless WGPU
PRIMARY adapter are also required. Once the serialized heavy-test slot is
available, run from the repository root:

```sh
python3 -m unittest discover -s tests -p test_gb_dmg_parity_helpers.py
cargo test --manifest-path renderers/modern/Cargo.toml --test dmg_sameboy_parity
cargo test --manifest-path renderers/modern/Cargo.toml --test dmg_sameboy_parity -- --ignored --nocapture
```

The ignored test fails rather than skips on missing prerequisites. It prints
SHA-256 identities of both source files, the built ROM and the loaded core,
and the WGPU adapter. These identify local evidence, not provenance or
shared-machine integrity; local executables and libraries are trusted.
Only temporary ROM/symbol files and process-memory frame bytes are used; no
window, recording, or production ROM is produced.
The core's system directory is temporary and contains no external boot ROM;
SameBoy may report that it could not open one. Fixture readiness is checked
after reset, not used as evidence about hardware boot behavior.

The fixture supplies scene 0 and no pad input. Every sequence is acknowledged
and checked against symbol-derived `wStep`, `wFrame`, native mode, and shadow
registers. An acknowledgement follows Step, not its presentation: the
mailbox is held unchanged for three further emulator frames, and the final
two must be fresh, non-null, complete, equal callbacks. Consecutive selected
states must not return identical images. A clone of the model receives one
additional tick to expose the prepared output without advancing the model for
the next acknowledged step. The capture process sends binary records on
stdout: a two-byte little-endian `wStep` read after acknowledgement and
freeze, followed by exactly 160×144×3 RGB bytes. Before GPU use, Rust requires
the complete expected record count, exact total length and exact ordered IDs,
including across the 8-bit mailbox counter wrap; stderr carries diagnostics.
The test compares every native RGB pixel to the
model and both UNORM and sRGB offscreen GPU readbacks (allowing one code of
GPU rounding for intermediate greys). It covers selected scroll positions,
palette boundaries, and wrap, not all possible states or cores.

This is frozen emulator/offscreen evidence only, not live cadence or timing,
presented/composited output, physical Game Boy or Pi behavior, flash safety,
certification, or complete coverage. Palette or core differences are reported
as failures; no palette normalization or emulator provenance is assumed.
