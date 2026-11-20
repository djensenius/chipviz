# SMS emulator validation plan

The frame-level runs below are emulator evidence, not hardware results. Do not
run an emulator, build an instrumented core, or capture frames without the
serialized slot. The candidate's native screen is static white, not a
visualizer; flash remains disabled.

## Installed tooling inspected without loading it

On this machine, `/Applications/RetroArch.app` reports version `1.22.2` and
contains a universal x86_64/arm64 executable. The installed arm64 core is
`~/Library/Application Support/RetroArch/cores/genesis_plus_gx_libretro.dylib`.
The installed `.info` calls it Genesis Plus GX v1.7.4, lists `.sms` support,
deterministic savestates, memory descriptors and license `Non-commercial`;
the `.info` version is metadata, not a verified runtime version. The inspected
core's SHA-256 is
`27bc2ef8ca07c775f022ae90be381b2a2a500245c9c7e362356ba3077b25c268`
and its Mach-O UUID is `96911803-E823-3158-AF1A-1EB46AC8D1EB`. Treat those as
local identifiers, not a pinned upstream revision. Do not distribute or
incorporate the core into the ROM; investigate its rights separately if
redistribution is proposed.

Set `CHIPVIZ_WORKTREE` to the checkout path, then repeat these **read-only**
probes before using it:

```sh
cd "$CHIPVIZ_WORKTREE"
/usr/libexec/PlistBuddy -c 'Print :CFBundleShortVersionString' /Applications/RetroArch.app/Contents/Info.plist
file /Applications/RetroArch.app/Contents/MacOS/RetroArch "$HOME/Library/Application Support/RetroArch/cores/genesis_plus_gx_libretro.dylib"
shasum -a 256 "$HOME/Library/Application Support/RetroArch/cores/genesis_plus_gx_libretro.dylib"
dwarfdump --uuid "$HOME/Library/Application Support/RetroArch/cores/genesis_plus_gx_libretro.dylib"
nm -gU "$HOME/Library/Application Support/RetroArch/cores/genesis_plus_gx_libretro.dylib" | grep -E ' _retro_| _z80_| _vdp_| _vram| _cram| _reg| _work_ram'
```

The binary exports libretro input/video, whole-frame `retro_run`, memory and
serialization functions, and internal Z80/VDP symbols. `/usr/bin/lldb` is
present; no matching core dSYM was found. Symbols are **not** a documented
stable debugger ABI: their presence does not establish their types, CPU
register layout, write-hook coverage, or cycle units. The installed binary
contains `genesis_plus_gx_region_detect` with `ntsc-u`/`pal` strings and
`genesis_plus_gx_vdp_mode` with `60hz`/`50hz` strings. Query and confirm
accepted values and reported video timing at runtime; strings alone are not
proof that a setting took effect.

## What exists and what is missing

- `tests/sms_libretro_capture.py` is a frame-level Genesis Plus GX frontend.
  It scripts joypad input and records frame hashes, selected RGB/PNG images,
  input-poll counts and end-of-frame SMS RAM snapshots. It does **not** expose
  the exact time or number of Z80 polls, VDP writes or instruction cycles.
- `scripts/retroarch_smoke.py` only launches each ROM briefly; it does not
  script SMS input or collect traces, pixels, VRAM or timing. Do not use it
  for this test: it runs other cores and writes shared build output.
- Existing GB/NES libretro frontends demonstrate frame-stepped callbacks,
  video and system-RAM access, but target different cores and are not an SMS
  adapter. A separate SMS frontend could script joypad states and collect
  frame buffers, subject to runtime calibration. Verify SMS button mapping,
  dimensions, pixel format, cropping, and actual memory descriptors; none
  is established by the presence of exported functions.
- `tests/sms_trace_tool.py` exports inputs/oracle traces and compares supplied
  captures. It never loads an emulator. Its mailbox protocol requires calls
  at Z80 entry points; its pad comparator requires one snapshot at each
  `sms_main_finish`; its VBlank comparator requires complete decoded writes
  with independently recorded VBlank boundaries. Whole-frame `retro_run`
  alone supplies none of those guarantees. Standard libretro frame callbacks
  do not provide Z80 PC breakpoints or cycle-stamped VDP writes. Do not label
  frame-only captures as per-poll traces or atomicity evidence.

## Frame-level results (2026-11-20)

The candidate SHA-256 was
`736d1ed5f4b088936f211cbdff9696cb2f798015ae672c5579152fdfecfc6ef6`;
the loaded core reported `Genesis Plus GX v1.7.4 f27655b`. With BIOS and
filters disabled, normal-speed CPU and the region options queried by the
core, the following runs returned:

- NTSC: `59.922743 Hz`, 820 candidate frames plus white-seed baseline.
- PAL: `49.701459 Hz`, 820 candidate frames plus white-seed baseline.
- Each: 820 observed pad states, 819 frame hashes matching the preceding
  observed RAM trace, 16 losslessly verified RGB/PNG pairs, matching static
  white native/seed frames, and identical replay JSON from a fresh run.
- Each: observed layouts 0–3, families 0–1, held/spectral/cuts and no flash.
  A held up press enters at observed frame 40; another exits at frame 780.
  Each frame had one libretro input poll and one video callback. That is not
  proof of one Z80 poll per frame.

The core negotiated RGB565. The screenshot checker quantizes the reference
RGB values to RGB565 before comparing; for example blue `$aa` decodes as
`$ad` after 5-bit quantization. It does not silently change the source
palette. The generic libretro video-RAM query returned zero bytes, and no
VDP writes, VBlank boundaries or Z80 instruction cycles were captured.
Thus image matching does not establish atomic publication, timing, complete
per-step target/oracle agreement or photosensitivity safety.

### Independent oracle follow-up (no new emulator run)

The existing JSON omits warm-up RAM and a Z80 poll counter, and a libretro
poll is not a Z80 poll. Its trace is the candidate's own output, **not** an
oracle. Using the previously compiled shared Rust oracle for the first 600
scripted controller values, the observed NTSC and PAL `visible` values
each had zero mismatches. This limited diagnostic treats each recorded frame
as one poll; it does not prove that assumption. With the oracle's procedural
phase starting at zero instead of the unknown target warm-up phase, its
mode/family differed at NTSC frames 141–143 and PAL frames 142–143. These
mismatches are unresolved, not a pass and not proof of their cause.

The capture adapter now records warm-up snapshots and the input bytes,
procedural counter and entry flag, but these are still **frame** snapshots:
they cannot prove that a Z80 poll was missed or that a snapshot was taken
before `sms_finish_step`. Existing evidence lacks even those new fields.
No new captures were run to test the adapter changes.

`tests/sms_capture_oracle.py` prepares an independent comparison for a
future breakpoint adapter. It drives the actual shared `ColourField` with 120
idle **polls** followed by the 820-poll controller schedule, starting at reset,
and compares scene/palette, input bytes, visibility/entry, effective palette
and every field-trace byte. It checks coverage of all safe modes, families,
layouts, unarmed relative palette 3 and field exit. It refuses frame JSON.
The later GX port-hook calibration below rebuilt and exercised this oracle.

For each NTSC and PAL run, a debugger-capable adapter must independently:

1. Boot the unmodified candidate from reset, drive the stated pad value for
   each actual `sms_sample_pad` invocation (not each `retro_run`), and record
   every invocation from the first one. A frame-driven adapter must first
   prove it neither misses nor duplicates any Z80 polls. Verify port mapping
   and that the input value actually sampled matches the scripted controller.
2. Break at `sms_prepare_field` entry for visible steps or
   `sms_prepare_native` entry for native steps. Both occur after
   `sms_begin_step` and, for the field, after `sms_field_make_trace`, but before
   `sms_finish_step` changes the next-step palette and scene. Require exactly
   one such breakpoint between successive pad samples, with no gaps or extras.
3. Emit JSON `{"kind":"sms-z80-poll-v1","start":"reset","records":[...]}`.
   Each record has sequential zero-based `step`; `point` is `"field"` or
   `"native"`; `buttons`, `scene`, `palette`, `visible`, `entry`, `peff`
   are bytes at `$c001`, `$c003`, `$c004`, `$c00a`, `$c00b`, `$c00d`;
   `input` is the 16 bytes `$c020`–`$c02f`; `trace` is the eight bytes
   `$c060`–`$c067`. Capture PC/poll ordering and core/ROM identity separately;
   the JSON label itself is not proof of provenance. Native `peff` and trace
   can be stale and are not compared.
4. Once the serialized slot is assigned, rebuild the changed oracle and run
   the comparison for each independent region capture:

   ```sh
   rustc --edition=2024 tests/sms_shared_oracle.rs -o "$SMS_SCRATCH/sms-shared-oracle"
   python3 tests/sms_capture_oracle.py --oracle "$SMS_SCRATCH/sms-shared-oracle" --capture "$SMS_SCRATCH/ntsc-polls.json"
   python3 tests/sms_capture_oracle.py --oracle "$SMS_SCRATCH/sms-shared-oracle" --capture "$SMS_SCRATCH/pal-polls.json"
   ```

The frame-only frontend has no breakpoint facility and cannot produce these
breakpoint JSON records. Neither a poll comparison nor the fixture alone
establishes which pixels were published for each poll; independently associate
frames to polls and compare to oracle pixels.

### GX port-hook calibration (2026-11-20)

The pinned arm64 GX core was observed in-process using the independently
written host shim `tests/sms_gx_port_hook.c`, not an altered candidate ROM.
The shim replaces the exported `z80_readport` function pointer, forwards every
read to the original `z80_ms_port_r`, and reports `$dc` reads and their real
returned values without modifying them. It is not a stable libretro API.
The adapter checks the pinned core and ROM hashes, verifies the original
pointer and Z80 PC `$02e4` (after the candidate's `IN A,($dc)`), and rejects
any unexpected pad, extra/missing poll or multiple polls in a frontend frame.

At each read after the first, the hook reads the preceding step's state from
`work_ram`. This is a **post-finish** snapshot: the candidate's fixed loop
finishes and publishes before returning to its next pad read; no pre-finish
instruction breakpoint was observed. Consequently the comparator checks
post-finish `shown` and effective palette against the next oracle trace, and
requires a settled palette at exit. It compares all 940 actual button, input,
procedural counter/digital latch, scene, entry/exit and visible trace records
against the independent shared `ColourField`, including held, spectral and
cuts, both families, four layouts and unarmed relative palette 3. The 941st
read obtains the final snapshot.
The first actual read was frontend frame 10 (NTSC) or 8 (PAL); the script is
indexed by actual reads, not by these startup frames.

- NTSC and PAL each: `941 actual port reads, 940 completed steps`; both:
  `PASS: 940 reset-to-finish Z80 polls match shared ColourField oracle`.
- A separate **test-only** ROM runs the unchanged callable fixture with all
  600 F18 input rows. The shim observes the fixture's marker read, verifies
  its PC, monotonic mailbox counter and each actual input row, and captures
  `$c0c0`–`$c0cf`. NTSC and PAL each:
  `PASS: 600 target F18 fixture traces and events match shared oracle`.
  This synthetic F18 ROM is not the production candidate.
- In-memory mutations of each real region capture's field trace and F18
  event were rejected by their independent comparators.

The production ROM SHA-256 remained
`736d1ed5f4b088936f211cbdff9696cb2f798015ae672c5579152fdfecfc6ef6`.
The F18 test ROM SHA-256 was
`ce6736868f095b7140c849173a53597cf51fdacf18c5561ac4b7cd24ce5df7dc`.
Evidence in the private `sms-emulator` scratch directory:

| File | SHA-256 |
|---|---|
| `gx-poll-ntsc.json` | `0e879fd540e029580298baf713daec750079b1f0249b17ad9210142ed43ec84e` |
| `gx-poll-pal.json` | `50d99c841aceaa1295855d4b73d37dbc09f0ffc7367de3ef88c969b8b1b0aac8` |
| `f18-gx-ntsc/f18.capture.bin` | `2591c443cd496ac40cb9da2591d961310aa2697a83a8b982ab13a318292a65ed` |
| `f18-gx-pal/f18.capture.bin` | `2591c443cd496ac40cb9da2591d961310aa2697a83a8b982ab13a318292a65ed` |

With the slot assigned, set `SMS_SCRATCH`, `SMS_CORE`, `SMS_WLA` and
`SMS_LINKER` to private scratch and the pinned tools, and use fresh output
paths. Build once:

```sh
cc -std=c99 -O2 -Wall -Wextra -Werror -dynamiclib tests/sms_gx_port_hook.c -o "$SMS_SCRATCH/sms-gx-poll-hook.dylib"
rustc --edition=2024 tests/sms_shared_oracle.rs -o "$SMS_SCRATCH/sms-shared-oracle"
python3 tests/sms_f18_rom_build.py --wla "$SMS_WLA" --linker "$SMS_LINKER" --output-dir "$SMS_SCRATCH/f18-target-rom"
```

Then for each `REGION` in `ntsc` and `pal`:

```sh
python3 tests/sms_gx_poll_capture.py --rom "$SMS_SCRATCH/candidate.sms" --core "$SMS_CORE" --shim "$SMS_SCRATCH/sms-gx-poll-hook.dylib" --region "$REGION" --output "$SMS_SCRATCH/gx-poll-$REGION.json"
python3 tests/sms_capture_oracle.py --oracle "$SMS_SCRATCH/sms-shared-oracle" --capture "$SMS_SCRATCH/gx-poll-$REGION.json"
python3 tests/sms_gx_f18_capture.py --rom "$SMS_SCRATCH/f18-target-rom/f18.sms" --symbols "$SMS_SCRATCH/f18-target-rom/f18.sym" --core "$SMS_CORE" --shim "$SMS_SCRATCH/sms-gx-poll-hook.dylib" --oracle "$SMS_SCRATCH/sms-shared-oracle" --region "$REGION" --output "$SMS_SCRATCH/f18-gx-$REGION"
```

Hooking the core changes its host execution and is not timing-neutral proof. These runs
did not trace VDP writes or independently associate published pixels with
oracle states. VBlank atomicity, hardware timing and photosensitivity safety
remain unverified.

To reproduce once the slot is assigned, first build `$SMS_SCRATCH/candidate.sms`
using `build_candidate.py` as described in the SMS README and generate the SDK-free seed in
`$SMS_SCRATCH/seed` with `python3 shared/tools/build_homebrew.py --target sms
--output "$SMS_SCRATCH/seed"`. Set `SMS_CORE` to the installed core and use
a **fresh**, nonexistent output directory for each run. For each `REGION` in
`ntsc` and `pal`, run:

```sh
python3 tests/sms_libretro_capture.py --rom "$SMS_SCRATCH/seed/chipviz-sms.sms" --core "$SMS_CORE" --region "$REGION" --output "$SMS_SCRATCH/seed-$REGION" --scenario seed
python3 tests/sms_libretro_capture.py --rom "$SMS_SCRATCH/candidate.sms" --core "$SMS_CORE" --region "$REGION" --output "$SMS_SCRATCH/$REGION"
python3 tests/sms_libretro_capture.py --rom "$SMS_SCRATCH/candidate.sms" --core "$SMS_CORE" --region "$REGION" --output "$SMS_SCRATCH/$REGION-repeat"
python3 tests/sms_capture_check.py --candidate "$SMS_SCRATCH/$REGION" --seed "$SMS_SCRATCH/seed-$REGION" --repeat "$SMS_SCRATCH/$REGION-repeat"
```

Each directory contains `capture.json` and selected raw `.rgb` and `.png`
images. The checker verifies control snapshots, previous-trace frame hashes,
PNG CRCs and RGB payloads, native/seed pixels and deterministic replay. It
does not convert frame snapshots into instruction-level trace evidence.

## Slot execution and acceptance

1. In private scratch, use the commands in the SMS README to build the
   candidate, callable fixture, six 600-step oracle streams, pad schedule and
   presenter vectors. Record their hashes, core hash, actual core/runtime
   version, frontend configuration and test ROM header. Do not overwrite
   shared RetroArch configuration, caches or other worktrees.
2. Provide an SMS-specific adapter. First calibrate boot, each libretro
   button, `retro_run`/input-poll relationship, RAM mapping and raw active
   video dimensions. Disable BIOS, borders, filters and colour correction
   only after confirming the core accepts those options. Repeat identical
   captures from a fresh reset and compare bytes. Require evidence of one
   candidate poll per captured step; reject missing/duplicate polls and do
   not assume frame index equals Z80 poll index.
3. Implement a real target mailbox driver before claiming F18–F22/F24: either
   a debugger/instrumented core that can call `sms_fixture_init` and
   `sms_fixture_step` with persistent CPU/RAM, or a separately built test ROM
   that runs one fixture row per VBlank and exposes a monotonic step counter
   plus `$c0c0`–`$c0cf`. A frame-level frontend using the latter must reject
   every missing, repeated or skipped step. `fixture.bin` is callable code,
   **not** a bootable cartridge. Compare all six 600-step captures to the
   shared oracle; F18 also checks the event bytes. F19's high-energy input
   is synthetic, not an SMS controller capability.
4. On the candidate, compare all 600 pad snapshots and their actual published
   active-area RGB888 frames. Cover scene entry/exit, release, opposing
   directions and relative palette/no-arming. Use the documented
   presenter-only injections for both families, four layouts and rotations;
   they bypass the state machine and are not safety evidence. If an adapter
   cannot synchronize a snapshot with the published frame, report a blocker
   rather than guessing an offset.
5. Run separately in verified NTSC and PAL. An instrumented adapter must
   record **all** VRAM/CRAM/register writes, Z80-relative timestamps and
   VDP-derived VBlank/active boundaries, including unexpected writes and
   missed or extra frames. Validate the capture path with deliberately
   late, missing and extra synthetic records before comparing the actual
   600-step logs using `compare-vblank`. Record counter units, poll counts,
   transition frames and full logs. If only frame-level snapshots are
   available, report timing/atomic publication as unverified.

Never infer physical SMS timing from emulator timing. Neither this plan nor
emulator results establish photosensitivity safety, PEAT/Harding results, or
hardware validation; those require separate evidence and authorization.
