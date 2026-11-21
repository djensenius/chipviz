# GBA Lines host prototype (unlinked)

This pure-C model is not linked into the GBA ROM or selectable as a scene.
It is outside the production `source/*.c` wildcard. Scene selection and
integration require a separate design; scene 3 in the legacy ring is Blockscape.

The reduced data-role inputs are sampled A/B and L keys, not spectrum, notes or
BPM. A rising edges count once even with B held; a held A at entry is not a
beat. L gently adjusts a palette level from the supported energy values (200
held, 64 released). Sixteen A edges make a phrase; a 192-step minimum section
and a 512-step stalled-input fallback allow the mirrored plane to fold into a
hallway and back. These step counts are prototype assumptions, not measured
time or music synchronization.

The caller supplies an unsigned step counter. The first sample records held
keys; duplicate samples and gaps greater than eight steps are rejected without
changing state. Counter wrap is supported. Reinitializing after a rejected gap
resets the phrase and fold. For an accepted gap, the newly sampled L level
applies to every elapsed step; an A rising edge applies to the final step.
Unsampled edges cannot be recovered. The visual phase wraps every 48 steps,
the period of the cross-line pattern.

A snapshot freezes state and all 256 staged palette entries. Rendering writes
at most 240 packed halfwords per batch into a caller-owned, explicitly sized
240x160 page; a complete render replaces the whole page. The caller must keep
the same aligned destination until completion or restart. Cancellation leaves
partial pixels; restart begins again at zero. Zero-initialize a fresh render
before calling batch or cancel. A valid batch returns 0 for a fresh or
cancelled partial render and 1 for a completed render, including later calls;
cancelling after completion preserves completion. An invalid destination or
size returns -1 even after completion. The model uses no heap, display registers,
palette hardware or page flips.

## Host tests

From the repository root, using a C99 compiler:

```sh
mkdir -p build/lines-host
cc -std=c99 -O2 -Wall -Wextra -Werror -pedantic cores/gba/homebrew/lines/lines_tests.c cores/gba/homebrew/lines/lines.c -o build/lines-host/tests && build/lines-host/tests
```

Tests cover deterministic replay, beat edges and fallback, frozen snapshots,
bounded writes, cancellation and restart. The scalar pixel oracle shares the
renderer's formula; it is not an independent golden image.

## Isolated raw-page fixture (test only)

`fixture/` links this model into a separate ARM ROM, not the production ROM.
Its regular and stalled synthetic inputs produce the original 13 ordered
checkpoints, including a partial render, cancellation and restart. Each record
includes the entire rendered page, adjacent canaries, state, render progress,
batch counts and the complete staged palette. An independent Python scalar
oracle compares all records and bytes, including the unfinished sentinel
portion. The host tests also exercise phase 48 and steps 65535/65536, render
polling, and failure cases in the trace protocol.

Page 1 is the render destination; page 0 carries the trace in three chunks
per checkpoint (two raw page chunks and one palette chunk). The global hardware
palette encodes byte values as RGB565 transport colors, **not displayed scene
colors**. Headers are invalidated during copying; the harness requires two
identical fresh non-NULL observations on consecutive libretro runs per ordered
chunk and a completion record. A run with a NULL or absent video callback
resets stability and counts toward the timeout. Only a byte-identical repeat of
the latest accepted chunk is allowed; older replays and corrupt repeats fail.
A missing, corrupt or incomplete trace, or a timeout, is a failure, not a
successful comparison. The libretro core must expose 240x160 RGB565 frames.

With a C99 compiler, from the repository root, the focused host checks are:

```sh
mkdir -p build/lines-fixture
cc -std=c99 -Wall -Wextra -Werror -pedantic -O2 -shared -fPIC cores/gba/homebrew/lines/fixture/fixture.c cores/gba/homebrew/lines/lines.c -o build/lines-fixture/fixture.so
python3 cores/gba/homebrew/lines/fixture/host_tests.py build/lines-fixture/fixture.so
cc -std=c99 -Wall -Wextra -Werror -pedantic -fsyntax-only cores/gba/homebrew/lines/fixture/main.c cores/gba/homebrew/lines/fixture/fixture.c cores/gba/homebrew/lines/lines.c
```

With devkitARM configured (`DEVKITARM`, its `bin` on `PATH`, and `gbafix`)
and a local mGBA libretro core, build and run the matching fixture:

```sh
make -C cores/gba/homebrew/lines/fixture
python3 cores/gba/homebrew/lines/fixture/harness.py --rom cores/gba/homebrew/lines/fixture/build/gba-lines-fixture.gba --core /absolute/path/to/mgba_libretro.dylib --max-frames 12000
```

Use the platform's core extension as appropriate. Record the exact head,
tool versions, results and any earlier failures. The old #167 mGBA trace
used the pre-#195 model and is not evidence for this corrected model; this
migration requires a fresh build and run.

The fixture does not install the staged scene palette or present the rendered
scene. Synthetic steps are independent of VBlank, and transport copies and
waits dominate its frame count. It proves neither live page/palette ownership
or atomicity, production VBlank timing, hardware behavior, visual quality,
accessibility nor photosensitivity/flash safety. The GBA Mode 4 palette is
global to both pages; staging it is not a presentation.
