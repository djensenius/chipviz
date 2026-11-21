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
partial pixels; restart begins again at zero. The model uses no heap, display
registers, palette hardware or page flips.

## Host tests

From the repository root, using a C99 compiler:

```sh
mkdir -p build/lines-host
cc -std=c99 -O2 -Wall -Wextra -Werror -pedantic cores/gba/homebrew/lines/lines_tests.c cores/gba/homebrew/lines/lines.c -o build/lines-host/tests && build/lines-host/tests
```

Tests cover deterministic replay, beat edges and fallback, frozen snapshots,
bounded writes, cancellation and restart. The scalar pixel oracle shares the
renderer's formula; it is not an independent golden image.

This subset provides no target fixtures or restore model. It does not prove
page/palette ownership, a hardware handoff, cadence, presented pixels, visual
quality, accessibility, flash safety or physical hardware behavior. The GBA
Mode 4 palette is global to both pages; staging it is not a presentation.
