# GBA Blockscape offscreen prototype

This prototype is not registered, selectable, captured, or approved for
presentation. The native beat and key overlays can flash; the tests are not a
photosensitivity assessment. It adds no platform or visualization coverage.

The CPU model takes an explicit 16-bit native frame counter, one of the four
native background variants, palette `0..=7`, eight spectrum bytes, energy,
beat, note, and key bits. It reproduces the indexed background, palette,
halfword-aligned overlays, and overlay ordering from
`cores/gba/homebrew/source/main.c`; other key bits are ignored. It does not
reproduce native state generation, input acquisition, page flips, Colour
Field, or request/commit behavior. The native demo normally presents no more
often than once per four VBlank polls, subject to rendering time; the model has
no clock and its caller chooses when to advance the counter.

The host-only oracle compiles the repository's actual GBA C functions with a
host C compiler and compares every palette entry and index for selected inputs.
It does not run an emulator or establish hardware timing or physical-display
pixel parity. The renderer expands each RGB15 channel by bit replication to
illustrative 8-bit sRGB bytes; this is not calibrated GBA screen colour. It
uses an integral full-target viewport, nearest-neighbour sampling, centered
fractional downscaling, integer upscaling, and opaque black letterboxing.
Targets and allocations are bounded; malformed frames and unsupported target
formats, shapes, usage, or device limits are rejected before drawing.

Portable tests and Clippy can be run with the repository's normal Rust
toolchain:

```sh
cargo test --locked --manifest-path renderers/modern/Cargo.toml --no-default-features \
  --test gba_blockscape_model --test gba_blockscape_render --test gba_blockscape_gpu
cargo clippy --locked --manifest-path renderers/modern/Cargo.toml --no-default-features \
  --test gba_blockscape_model --test gba_blockscape_render --test gba_blockscape_gpu -- -D warnings
```

The three ignored GPU tests must be run only while holding the shared exclusive
heavy-work lock. They also share an in-process mutex when run together. Select
the Metal, Vulkan, or GL test explicitly, for example:

```sh
cargo test --locked --manifest-path renderers/modern/Cargo.toml --no-default-features \
  --test gba_blockscape_gpu gpu_metal_matches_native_source_and_fractional_scaling -- --ignored --nocapture
```

The test prints the actual adapter, compares both UNORM and sRGB offscreen
readbacks to the native C oracle, and checks tiny and letterboxed targets.
Record the actual adapter with any result. An opt-in test is not evidence of
backend support: GL, Vulkan, physical Pi hardware, timing, display colour, and
safety remain unverified until separately tested. No image or video file is
produced.
