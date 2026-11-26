# chipviz-sms

Sega Master System / SN76489 PSG visualizer for the ChipStation SN76489 core.

## Homebrew stack

- The current `.sms` artifact is an SDK-free, generated white VDP seed. It is
  not an interactive colour-field ROM.
- An opt-in WLA-DX candidate links independently written Z80 startup, DATA
  input, detector, safe modes and VDP presentation. It has booted in an SMS
  emulator, but is not a hardware-validated visualizer or release artifact.

### WLA-DX bootstrap

The following build-only tool revision was tested with CMake 4.3.4 and Apple
clang 21. Use a private, empty directory; this does not install anything:

```sh
git clone --no-checkout https://github.com/vhelin/wla-dx.git wla-dx
git -C wla-dx checkout --detach 01cfa6dcabc286dcee975ef58e02797e160866b3
cmake -S wla-dx -B wla-build -DGEN_DOC=OFF -DCMAKE_BUILD_TYPE=Release
cmake --build wla-build --target wla-z80 wlalink --parallel 2
```

WLA-DX is GPL-2.0-or-later; its assembler and linker are build tools, not
linked runtime. The independently written bootstrap fixture used no SDK or
runtime. A future ROM must separately account for any runtime it includes; in
particular, devkitSMS's startup code is GPL with a linking exception and
SMSlib has component-specific notices. No devkitSMS code is included here.

### Colour-field assets (work in progress)

`homebrew/field_assets.py` generates solid mode-4 tile patterns, four
2 KiB-padded name maps, CRAM levels, colour-combo and pad-edge lookups, and
one period of procedural detector inputs. Run
`python3 cores/sms/homebrew/field_assets.py OUTPUT_DIRECTORY` from the
repository root. These assets are not used by the seed. The maps cover
256 × 192 pixels in 8-pixel cells; horizontal
region 0 is at the bottom. The combo table is indexed by
`family * 32 + (regions - 2) * 16 + combo * 4`, and its entries index the
colour-major, four-level CRAM table. Host checks are in
`tests/test_sms_field_assets.py`.

### Data-role control module (work in progress)

`homebrew/data_control.asm` samples the SMS pad, applies edge and opposing
direction rules, and implements scene debounce and the relative palette gate
with change-budget counters. It maps unarmed 3/7 to 2/6 and has no flash
arming. `data_input.asm` supplies procedural detector inputs until either
button is first held; thereafter it uses the digital-only pad mapping:
energy and bands 64, zero spectrum, button 1 beat and button 2 bar. The SMS
pad has no shoulder for the high-energy portion of F19. Flags are held
levels, so the detector must detect rising edges and seed previous
inputs on scene entry. `field_data.asm` supplies labels for the generated
field assets. None of these modules is included in the seed. The hidden
bring-up loop polls input and computes traces with the display disabled; it
never presents a field.

`data_detector.asm` calculates averages, silence, transient/beat/note edges
and dominance for this reduced input (at most one note pair). `safe_modes.asm`
and `spectral_levels.asm` implement held, spectral and unarmed hard cuts.
These modules are linked to the hidden bring-up loop and candidate ROM but
unverified on target. There is no flash mode.

The modules reserve `$C000`–`$C015`, `$C020`–`$C033`, `$C040`–`$C050`,
`$C060`–`$C067` and `$C070`–`$C095`. Their caller must
initialise the stack and input state, then sample the pad, make the input,
begin the step, run the detector and field rule, snapshot the trace and finish
the step in that order. After `sms_finish_step`, initialise any requested mode
with that same step's inputs and acknowledge `sms_mode_changed`. See the
assembly comments for entry points and counter ownership.

### Experimental candidate ROM

The normal `make` still builds the SDK-free white seed, including when WLA-DX
is unavailable. To build the separate, **unverified** candidate with the
pinned tools above:

```sh
make -C cores/sms/homebrew candidate WLA_Z80=/path/to/wla-z80 WLALINK=/path/to/wlalink
```

The candidate uses its own startup, no SDK/runtime. Its native fallback is
static white, not a visualizer. It prepares that map and all four field maps
before enabling the display; up/down switches
between native and field after the DATA-role scene debounce, and left/right
request the relative palette. Buttons 1/2 switch from procedural input to
digital beat/bar levels. Palette 3/7 remains unarmed (2/6). It stages colours
in RAM, then waits for a new VBlank before writing three CRAM entries and
selecting a preloaded name table; it never blanks the display on a transition.
Frame-level NTSC and PAL emulator captures exercise input, the four layouts,
both families and non-flash modes; see [EMULATOR_VALIDATION.md](EMULATOR_VALIDATION.md).
The candidate's controller-poll trace and a separate F18 fixture match the
shared oracle in NTSC and PAL; VDP write timing has not been tested.
From the VBlank status read through the last VDP register write, the nominal
path is 225 Z80 T-states, excluding possible NMI or hardware wait states.
No target or hardware safety claim follows from the host checks.

The native table occupies VRAM `$1800`–`$1eff` with an end-of-sprites marker
at `$1f00`; maps occupy `$2000`–`$3fff`. Candidate scratch is `$c100`–`$c106`.
`tests/test_sms_presenter.py` checks the host arithmetic and VRAM allocation.
`tests/sms_candidate_smoke.py`, with the same `--wla`, `--linker` and
`--workdir` arguments as the control smoke below, builds the candidate and
checks linked bytes and header; it does not execute Z80 code.

`detector_fixture.asm` supplies a test-only callable mailbox adapter at
`$C0A0`–`$C0CF` for target trace comparisons. The host fixtures in
`tests/sms_detector_fixtures.py` cover F18–F22 and F24; F23 is a host transport
check, not an SMS controller input. F18 has been executed through a test-only
ROM on an emulator; F19–F22 and F24 remain unexecuted.

To check assembly and embedded asset bytes without running a console emulator:

```sh
python3 tests/sms_control_smoke.py --wla /path/to/wla-z80 \
  --linker /path/to/wlalink --workdir /path/to/private/scratch
```

The host tests exercise the pad-edge table, including the F24 relative input
sequence. `tests/sms_procedural_oracle.c` compares the generated lookup and a
host model of its note counter against all 65,536 frames from the shared
procedural generator. With `SMS_SCRATCH` set to a private scratch directory:

```sh
python3 cores/sms/homebrew/field_assets.py "$SMS_SCRATCH"
cc -std=c99 -Wall -Wextra -Werror -Ishared/include \
  tests/sms_procedural_oracle.c shared/src/control_frame.c \
  -o "$SMS_SCRATCH/sms-procedural-oracle"
"$SMS_SCRATCH/sms-procedural-oracle" "$SMS_SCRATCH/field-procedural.bin"
```

These host checks do **not** execute the Z80 state machine:
F19–F24 receiver traces, VBlank timing and hardware remain unverified.

`tests/sms_shared_oracle.rs` checks the fixture expectations against the
actual shared Rust reference without building the renderer. Its SMS pad path
uses the wrapping 16-bit procedural counter and note calculation from the
shared C generator; it does not change the renderer's Rust generator. Long
host traces compare those inputs against C and the SMS lookup/counter model,
not executed Z80 instructions:

```sh
rustc --edition=2024 tests/sms_shared_oracle.rs -o "$SMS_SCRATCH/sms-shared-oracle"
"$SMS_SCRATCH/sms-shared-oracle"
```

### Preparing target validation (no emulator included)

See [EMULATOR_VALIDATION.md](EMULATOR_VALIDATION.md) for the installed-core
inspection, adapter limitations and serialized-slot acceptance plan.

The following build-only commands prepare deterministic vectors in private
scratch. The Rust stream path independently verifies **every** fixture input
row against the reference setup before producing expected traces; F18 uses
palette 2 (hard cuts), not 0 (held).

```sh
rustc --edition=2024 tests/sms_shared_oracle.rs -o "$SMS_SCRATCH/sms-shared-oracle"
python3 tests/sms_trace_tool.py export --oracle "$SMS_SCRATCH/sms-shared-oracle" --output-dir "$SMS_SCRATCH/vectors"
python3 tests/sms_trace_tool.py export-pad --oracle "$SMS_SCRATCH/sms-shared-oracle" --output "$SMS_SCRATCH/vectors/pad.input.bin" --expected "$SMS_SCRATCH/vectors/pad.expected.bin"
python3 tests/sms_trace_tool.py export-render --output "$SMS_SCRATCH/vectors/render.expected.bin"
python3 tests/sms_fixture_build.py --wla /path/to/wla-z80 --linker /path/to/wlalink --output-dir "$SMS_SCRATCH/fixture"
python3 cores/sms/homebrew/build_candidate.py --wla /path/to/wla-z80 --linker /path/to/wlalink --output "$SMS_SCRATCH/candidate.sms" --symbols "$SMS_SCRATCH/candidate.sym"
```

These commands do not launch an emulator. The GX adapter in
[EMULATOR_VALIDATION.md](EMULATOR_VALIDATION.md) executes F18; an adapter for
the other scenarios is still required. For each F18/F19/F20/F21/F22/F24
`*.input.bin`, load `fixture.bin` at `$0000`, supply RAM at `$c000`, and use
the addresses in `fixture.sym`. Reset the CPU for each scenario, disable
interrupts, set SP to `$dff0`, and call `sms_fixture_init` with A equal to
the first row's scene. For each of 600 18-byte input rows (scene, palette,
16 detector bytes), copy the last 16 bytes to `$c0a0`, the first two to
`$c0b0`/`$c0b1`, call `sms_fixture_step`, then append `$c0c0`–`$c0cf`
to a capture. Calls must actually run through `ret`; a debugger can put a
sentinel return PC on the stack and break there. Do not reset RAM between
steps. Compare the 600 16-byte captures with:

```sh
python3 tests/sms_trace_tool.py compare-trace f18 --expected "$SMS_SCRATCH/vectors/f18.expected.bin" --capture /path/to/f18.capture.bin
```

Repeat for all scenarios. The oracle files have 600 nine-byte records:
shown, then the eight-byte trace. The comparator also checks F18's complete
T/N/B/X/D/E/silence event bytes, including only a beat and E at step 300.
F19's shoulder energy is synthetic and cannot be supplied by an SMS pad.

For the booted candidate, feed `pad.input.bin` as active-high pad bits
(invert bits 0–5 for port `$dc`) at each `sms_sample_pad` call; do not
repeat or omit polls. Break at `sms_main_finish` before it executes and
capture a JSON array of 600 objects with integer `step`, `buttons` (`$c001`),
`scene_ring` (`$c003`), `palette_ring` (`$c004`), `visible` (`$c00a`),
`peff` (`$c00d`), and `trace` (eight bytes `$c060`–`$c067` as an array).
Then run:

```sh
python3 tests/sms_trace_tool.py compare-pad --expected "$SMS_SCRATCH/vectors/pad.expected.bin" --capture /path/to/pad.json
```

This schedule includes scene entry/exit, held buttons, release, simultaneous
opposing directions and relative palette 3. It tests actual candidate traces
against the reference if the adapter captures them faithfully.

For pixel testing, capture the **active 256×192 area** as unfiltered RGB888
(SMS channels decoded as 0/85/170/255), with no border, scaling or colour
correction. For each candidate pad step, capture the frame published after
that step and compare against `pad.expected.bin`:

```sh
python3 tests/sms_trace_tool.py compare-pixels --expected "$SMS_SCRATCH/vectors/pad.expected.bin" --step 110 --capture /path/to/step110.rgb
```

For exhaustive presenter-only coverage, `render.expected.bin` contains 320
synthetic field traces covering both families, four layouts, four combos,
each valid rotation and four level combinations, followed by 280 native
traces. With the candidate already booted, a debugger can inject each
eight-byte field trace into `$c060`, call `sms_prepare_field`, resume at
`sms_main_publish`, break at the next `sms_main_loop` and capture the
resulting active frame. For native, stage `$f7,$3f,0,0` at `$c100`–`$c103`
instead. Use `compare-pixels` on each step. This bypasses the state machine,
so it cannot establish controller or temporal safety.

To check presentation timing, an adapter must log **all** decoded VRAM, CRAM
and register writes after startup, not just writes it expects to see, and
derive VBlank boundaries independently from the emulated VDP. For each of
600 candidate pad steps, record one JSON object with `step`, independently
observed monotonic `vdp_frame`, absolute Z80 cycle `start` (VBlank begins),
`active` (next active display begins), and
`writes`: ordered objects `{ "cycle": n, "kind": "cram"|"reg"|"vram",
"address": n, "value": n }`. Include writes outside VBlank as well; do not
omit display-enable register writes. Associate each step with its publication
and capture consecutive frames, including missed or extra frames. Independently
calibrate an exact frame period in the same cycle units; then the diagnostic
can screen the data, with `SMS_FRAME_PERIOD` set to that integer:

```sh
python3 tests/sms_trace_tool.py compare-vblank --expected "$SMS_SCRATCH/vectors/pad.expected.bin" --capture /path/to/vblank.json --frame-period "$SMS_FRAME_PERIOD"
```

This rejects gaps in the supplied frame indices or period, and checks the
three CRAM entries, name-table register, order and reported intervals. It
always exits **INCONCLUSIVE** (status 2) after screening: supplied indices and
period alone do not prove their provenance, complete capture or synchronization.
No target timing or atomicity is established. PAL/NTSC, real hardware,
missed-frame handling and pixel/control evidence need independent validation.

## First demo

- PSG tone/noise channel meters.
- TMS9918/SMS VDP tile and CRAM palette cycling.
- Controller input can later select scene/palette/intensity.

## Current scaffold

`src/main.c` consumes the shared chipviz connection layer and maps each frame to
PSG tone/noise and tile-phase values for host simulation.
`cores/sms/homebrew` normally generates `chipviz-sms.sms` through the shared
SDK-free homebrew artifact builder; the candidate is an explicit separate build.

## Host-only field assets

`homebrew/field_assets.py` generates deterministic candidate field patterns,
maps and lookup tables into a supplied directory. These files are not used by
the static seed ROM. Python host-model tests are discovered by
`just python-test`; `just sms-procedural-test` generates the assets in a
temporary directory and compares all 65,536 procedural frames against the
shared C control-frame implementation. It also runs as part of `just c-test`.
These checks do not execute SMS code or establish visualization or hardware
coverage.

## Connection path

Start with baked/procedural playback and controller input. A later hardware
bridge can map ChipStation PSG telemetry to a tiny controller-port protocol.

## Constraints

Use tile maps, simple CRAM changes, and channel meters. Avoid framebuffer-style
assumptions.
