# chipviz colour-field scene v0

Colour fields are two or three large regions of flat colour. They are meant to
be projected onto performers in a dark space. This contract is shared by the
modern renderer (#68) and every console port (#69). It uses
[`control-frame-v0`](control-frame-v0.md) unchanged. The scene's state machine
is deterministic, and the same fixture input gives the same observable trace
on every target.

## Projection rules

- Every visible pixel belongs to exactly one region. There are no borders,
  margins, text, sprites, camera, gradients, dithering, or anti-aliased edges.
- Each region is one solid colour. All region colours and layout changes for a
  step are presented together at the same VBlank or frame present. A partially
  updated frame must never be shown. Targets that cannot rewrite a layout in one
  VBlank prepare an off-screen nametable or buffer and flip to it.
- Cuts are abrupt. There are no forced fades, and modes must not add easing.
- Flash/gate is opt-in only (see [Flash/gate opt-in](#flashgate-opt-in)).

## Selection through existing fields

### Scene index 7 is the colour field

- `scene == 7` selects the colour field on every target. Index 7 fits the
  3-bit HID keyboard scene bits (`8` key), the 4-bit N64 Joybus scene nibble,
  and the full `u8` field.
- Each target keeps its native scenes `0..N-1` (`N <= 7`). Received indices
  `N..6` keep the target's existing interpretation (currently `scene % N`).
  Index 7 is checked before any modulo, including the modern renderer's
  `scene % 3`.
- The controller scene-select ring becomes `0, 1, ..., N-1, 7`. `next` from
  `N-1` goes to `7`, and `next` from `7` goes to `0`. `prev` from `0` goes to
  `7`, and `prev` from `7` goes to `N-1`. This applies to D-pad up/down HID
  pulses, the shared `chipviz_interface` next/prev inputs, and target-local
  rings such as the GBA `GBA_SCENE_COUNT` ring. Auto mode advances through the
  same ring.
- Demo mode on the colour field generates `palette = 0` (held, family A).

### Palette index selects mode and family

While `scene == 7`, the `palette` field is read as `p = palette & 7`. Bits 3..7
are ignored and should be sent as `0`. The 3-bit range matches the HID palette
keys `Q`..`I` and left/right pulses, `GBA_PALETTE_COUNT`, and the N64 nibble.

| `p` | Mode | Family |
| --- | --- | --- |
| 0 | held (default) | A: red/yellow/blue |
| 1 | spectral | A |
| 2 | hard cuts | A |
| 3 | flash/gate (opt-in) | A |
| 4 | held | B: RGB/CMY |
| 5 | spectral | B |
| 6 | hard cuts | B |
| 7 | flash/gate (opt-in) | B |

`mode = p & 3`, `family = p >> 2`. If flash is not armed, mode 3 behaves as
mode 2.

### Flags

Flags keep their v0 meanings, and reserved bits stay clear. The colour field
consumes the rising edges of `beat` (bit 0), `bar` (bit 1), and `drop` (bit 2).
It ignores `fill` and `manual`.

### Mode and family changes

A change to `p` takes effect on the next step. The layout, combo and rotation
are kept. A family change swaps the colour table immediately. Entering spectral
mode re-initialises region levels from the current sample. Entering held mode
sets `combo = dom`. Entering flash/gate starts in the rest state.

Hosts that drive `palette` from other data, such as the chipsynth reducer's
dominant channel, must hold it steady while `scene == 7`. Otherwise the mode
changes every frame.

## Layouts and geometry

| Id | Name | Regions |
| --- | --- | --- |
| 0 | V2 | two columns |
| 1 | H2 | two rows |
| 2 | V3 | three columns |
| 3 | H3 | three rows |

`next layout = (layout + 1) & 3`. Region `0` is leftmost for columns and
**bottom** for rows, so the bass region sits at the floor. Region `i` covers
`[b_i, b_{i+1})` along the split axis. With `n` regions and `S` units along
that axis:

```
b_i = g * floor(i * (S / g) / n),  b_n = S
```

`g` is the snap granularity: `1` for the modern renderer and N64, `8` px for
tile consoles, and `16` px for the NES (attribute blocks, so a layout is just a
64-byte attribute rewrite). Examples:

- 256 px wide with `g=8` and `n=3` gives boundaries at 80 and 168.
- The DMG's 160 px with `g=8` and `n=3` gives 48 and 104.
- The NES's 256 px with `g=16` and `n=3` gives 80 and 160.

## Palettes

### Colours (exact sRGB, modern reference)

| Id | Name | sRGB |
| --- | --- | --- |
| A.R | red | `#E01020` |
| A.Y | yellow | `#FFD000` |
| A.B | blue | `#1030C0` |
| W | white | `#FFFFFF` |
| K | black | `#000000` |
| B.R | red | `#FF0000` |
| B.G | green | `#00FF00` |
| B.B | blue | `#0000FF` |
| B.C | cyan | `#00FFFF` |
| B.M | magenta | `#FF00FF` |
| B.Y | yellow | `#FFFF00` |

### Combos

Each entry lists region colours for regions 0, 1 and 2, before rotation.

| Combo | A, 2 regions | A, 3 regions | B, 2 regions | B, 3 regions |
| --- | --- | --- | --- | --- |
| 0 | R, B | R, Y, B | R, C | R, G, B |
| 1 | Y, B | R, W, B | G, M | C, M, Y |
| 2 | R, Y | Y, K, B | B, Y | R, K, C |
| 3 | B, W | R, K, Y | C, M | B, K, Y |

Region `i` shows combo entry `(i + rot) mod n`. When the region count changes,
`rot = rot mod n`.

### Levels

Held, cuts and the gate's lit state use level 3. Spectral mode and the gate's
rest state use levels 0..3. The modern renderer and N64 scale each sRGB channel
as `c' = (c * L + 1) / 3` using integer division. They write exact sRGB values,
converting to linear first when the surface format is sRGB.

### Per-console hardware colours

Values below are nearest by squared sRGB distance, except where noted. Ports
document their final table and confirm it with an emulator capture.

| Colour | SMS `--BBGGRR` | Genesis `BBB0GGG0RRR0` | SNES/GBA BGR555 | C64 (Pepto) | NES (2C02 ref) |
| --- | --- | --- | --- | --- | --- |
| A.R | `0x03` | `0x20C` | `0x105B` | 10 light red | `$16` |
| A.Y | `0x0B` | `0x0CE` | `0x033F` | 7 yellow | `$28` |
| A.B | `0x24` | `0xA20` | `0x5CC2` | 6 blue | `$02` |
| W | `0x3F` | `0xEEE` | `0x7FFF` | 1 white | `$30` |
| K | `0x00` | `0x000` | `0x0000` | 0 black | `$0F` |
| B.R | `0x03` | `0x00E` | `0x001F` | 2 red | `$16` |
| B.G | `0x0C` | `0x0E0` | `0x03E0` | 5 green | `$19` |
| B.B | `0x30` | `0xE00` | `0x7C00` | 6 blue | `$01` |
| B.C | `0x3C` | `0xEE0` | `0x7FE0` | 3 cyan | `$2C` |
| B.M | `0x33` | `0xE0E` | `0x7C1F` | 4 purple * | `$14` |
| B.Y | `0x0F` | `0x0EE` | `0x03FF` | 7 yellow | `$28` |

\* The nearest C64 colour to magenta by distance is 14 (light blue). Purple is
chosen to keep the hue.

- **SMS, Genesis, SNES and GBA:** dim levels apply the same formula to each
  hardware channel value. SMS A.R and B.R are the same colour; the families
  differ in their combos.
- **N64:** use the exact sRGB values. A 16-bit framebuffer quantises like
  BGR555.
- **C64:** levels 3 and 2 use the base colour. Levels 1 and 0 use `0` (black).
  The fixed palette is muted.
- **NES:** use the common 2C02 reference palette (FCEUX/Nestopia style). Never
  use `$0D`. Level 3 is the base colour. Level 2 is `base - $10` when
  `base >= $10`, otherwise the base colour; `$30` uses `$10`. Levels 1 and 0
  are `$0F`. Each region uses its own background palette.
- **Game Boy DMG (gating variant):** shades `0` (lightest) to `3` (black) are
  the same for both families.

  | Combo | 2 regions | 3 regions |
  | --- | --- | --- |
  | 0 | 0, 3 | 0, 3, 1 |
  | 1 | 1, 3 | 3, 0, 2 |
  | 2 | 0, 2 | 1, 3, 0 |
  | 3 | 3, 1 | 0, 2, 3 |

  In spectral mode, region shade is `3 - level`. Flash/gate alternates the
  combo shades (lit) with all-`3` (dark), or with all-`2` when resting dim. The
  layouts, rules and timings are the same as on other targets.

## Detector (integer reference)

The detector runs one step per new control frame (a new `frame` counter value).
In demo mode it runs one step per locally generated frame. Durations below are
in steps at the nominal 60 Hz. Targets that step at 50 Hz keep the same step
counts, so their timing is 20% slower and still within the rate limits.
Counters are saturating `u16`. Arithmetic is signed 16-bit unless it is marked
`u16`.

```
EMA:     acc_x (u16), avg_x = acc_x >> 3; update acc_x += x - (acc_x >> 3)
         x in {e=energy, b0=bass, b1=mid, b2=treble}
SILENCE: SIL_AVG 12, SIL_EXIT 24, SIL_HOLD 30
TRANS:   T_FLOOR 24, T_E_ON 40, T_E_OFF 16, T_B_ON 56, T_B_OFF 24,
         FLUX_ON 200, FLUX_OFF 64
NOTE:    NOTE_VEL 64
DOM:     DOM_MARGIN 32, DOM_HOLD 30, SECTION_MIN 600
CUT:     CUT_MIN 20
LEVEL:   UP[1..3] = 48,104,168; DOWN[1..3] = 32,88,152; LEVEL_HOLD 5
GATE:    GATE_ON 4, GATE_OFF_MIN 4, REST_DIM_ON 64, REST_DIM_OFF 48
```

**First step.** Set `acc_x = x << 3` and `prev_spec = spectrum`, with previous
flags `0`. Set `armed = 1`, `silent = 0`, and `dom = argmax(b0, b1, b2)` (ties
go to the lowest index). Set `layout = 0`, `combo = 0` (or `dom` in held mode),
and `rot = 0`. Set `since_cut = CUT_MIN`, `since_layout = 0`, and the gate
off-timer to `GATE_OFF_MIN`. Each spectral region level is the count of
`UP[k] <= v`. The gate rests dim if `e >= REST_DIM_ON`, otherwise dark. Render
and stop. The first step produces no events.

**Each following step, in this order:**

1. Compute the deltas against the previous averages:
   - `de = e - avg_e`
   - `db[i] = b_i - avg_b_i`
   - `flux = sum over 8 bands of max(0, spectrum[j] - prev_spec[j])` (`u16`)

   Then update the EMAs and set `prev_spec = spectrum`.
2. Update silence:
   - If silent and `e >= SIL_EXIT`, set `silent = 0` and `quiet = 0`.
   - If not silent, increment `quiet` when the updated `avg_e <= SIL_AVG`,
     otherwise set it to 0. Set `silent = 1` when `quiet >= SIL_HOLD`.
3. Check transients:
   - `hit = e >= T_FLOOR && (de >= T_E_ON || any db[i] >= T_B_ON || flux >= FLUX_ON)`
   - `calm = de < T_E_OFF && all db[i] < T_B_OFF && flux < FLUX_OFF`
   - `T = armed && hit`. If `T`, set `armed = 0`. Otherwise, if `calm`, set
     `armed = 1`.
4. Find note and flag events. `N = note_count > 0` and some
   `notes[0..min(note_count,4)-1].velocity >= NOTE_VEL`. `B` is a rising edge
   of `beat` or `bar`. `X` is a rising edge of `drop`.
   `E = !silent && (T || N || B || X)`.
5. Track dominance using the updated averages. Let `k = argmax(avg_b)`, with
   ties going to the lowest index.
   - If `!silent && k != dom && avg_b[k] >= avg_b[dom] + DOM_MARGIN`, count
     consecutive steps for the same candidate `k`. Otherwise reset the count
     to 0.
   - At `DOM_HOLD`, set `dom = k`, reset the count, and raise `D`.
6. `SECTION = X || (D && since_layout >= SECTION_MIN)`. If not silent, apply
   the mode rules; while silent, all visuals hold. Every layout change resets
   `since_layout`.
7. Increment the timers (`since_cut`, `since_layout`, the gate off-timer, and
   the per-region level holds).

## Modes

The modes are designed for music with no beat. Only hard cuts and flash/gate
use beat and bar flags, and even there they are just one event source among
several.

- **Held (0):** on `D`, set `combo = dom` (bass 0, mid 1, treble 2). On
  `SECTION`, go to the next layout. Transients, notes and beats are ignored.
  Regions use level 3.
- **Spectral (1):** each region follows a band value `v`:
  - With 3 regions, regions 0, 1 and 2 use bass, mid and treble.
  - With 2 regions, region 0 uses bass and region 1 uses `max(mid, treble)`.

  Raw values are used. A region's level rises by 1 when
  `v >= UP[level + 1]` and falls by 1 when `v < DOWN[level]`. It changes at
  most once per `LEVEL_HOLD` steps per region. A full 0-to-3 swing takes at
  least 15 steps. On `X`, go to the next layout and the next combo. On `D`
  with `since_layout >= SECTION_MIN`, go to the next layout.
- **Hard cuts (2):** a cut happens on `E` when `since_cut >= CUT_MIN` (at most
  3 per second at 60 Hz). Each cut resets `since_cut`.
  - If `X`, go to the next layout and the next combo.
  - Otherwise, apply action `k & 3`, then increment `k` (which starts at 0):
    `0` rotate, `1` next combo, `2` rotate, `3` next layout.

  An `E` that is not accepted is dropped, not queued. Regions use level 3.
- **Flash/gate (3, armed only):**
  - **Rest:** the field rests dim (level 1) or dark (level 0). The rest level
    uses hysteresis on `avg_e`: it switches to dim when `avg_e >=
    REST_DIM_ON` and to dark when `avg_e < REST_DIM_OFF`.
  - **Pulse:** an `E` starts a pulse if the field has rested for at least
    `GATE_OFF_MIN` steps since the last pulse. A pulse lights the field at
    level 3 for `GATE_ON` steps and rotates the regions by 1. If the pulse
    starts on `X`, the combo also advances.
  - **Limits:** there is at most 1 pulse per 8 steps (7.5 Hz at 60 Hz, 6.25 Hz
    at 50 Hz). Events during a pulse or the rest minimum are dropped.

### Silence and noise floor

Silence starts when `avg_e <= 12` for 30 consecutive steps. While silent,
everything holds, including spectral levels and the gate's rest state. Silence
ends on the first raw `energy >= 24`. That step may fire `T`, so returning
sound can cut.

`T_FLOOR` and the hysteresis pairs keep a noise floor of 0..12 from creating
events. A ±12 jitter around any level also creates none, because
`flux <= 8 * 24 < FLUX_ON`. If audio starts silent, held and cuts show
combo 0, while spectral and gate show black. This is intentional: no sound
means no light.

## Flash/gate opt-in

- Mode 3 renders as mode 2 until flash is armed.
- **Modern renderer:** the `--allow-flash` CLI flag arms flash for the process
  lifetime. The renderer prints the full warning to stderr at startup.
  Projection output never contains text.
- **Consoles:** when `scene == 7`, mode 3 is selected and flash is unarmed, the
  target shows a warning screen. This is a local interface screen, not a wire
  change.
  - `start` arms flash. Any other input, or 600 steps without input, cancels.
  - After a cancel, the target stays unarmed (mode 2 behaviour) until mode 3 is
    selected again.
  - The armed latch clears on power-on, on reset, and on leaving scene 7.
  - With no operator present (auto mode or a bridge), flash stays unarmed.

Full warning text, used in the modern CLI and docs:

> WARNING: Flash/gate mode produces rapid full-field flashing light at up to
> 7.5 flashes per second. It may trigger seizures in people with
> photosensitive epilepsy and can cause discomfort for others. Do not enable it
> where audience or performers have not been warned.

Console warning screen (at most 20 columns, so it fits the DMG):

```
WARNING: FLASHING
LIGHT. MAY TRIGGER
SEIZURES IN PEOPLE
WITH PHOTOSENSITIVE
EPILEPSY.
START = ENABLE
OTHER = CANCEL
```

## Fixtures

Each fixture runs for 600 steps, with `frame = step` and `scene = 7`. Run it at
`p = 0..7`. For `p = 3` and `p = 7`, run it both armed and unarmed; unarmed
must match `p = 2` and `p = 6`. Unlisted fields are `0`. "Bands" means bass,
mid and treble, and "spectrum" means all 8 bands.

The noise LCG is `x = (x * 25173 + 13849) & 0xFFFF` with `x0 = 1`. It is
advanced once per field per step in wire order. The jitter is
`j = ((x >> 8) % 25) - 12`.

The observable trace per step is
`(layout, combo, rot, family, level[0..2], lit)`. The shared reference
implementation's trace is the oracle. Every target's sim must match it exactly.
Emulator captures must show the boundaries from the geometry formula, and every
pixel of every region must equal the expected hardware colour.

| Id | Input | Expected |
| --- | --- | --- |
| F0 silence | All zero | The trace is constant. Held and cuts show layout V2, combo 0. Spectral and gate show all black. There are 0 cuts and 0 pulses. |
| F1 tone + drop | `e=160`, bands 40/200/40, spectrum `40,40,120,200,200,120,40,40`. `drop` is set for steps 300–305. | Held uses combo 1 from step 0. Spectral levels are 0/3/0, and 2 regions show 0/3. At step 300, there is exactly one `X`: held changes layout to H2; spectral and cuts change layout and combo; gate pulses and advances the combo. Nothing else changes. |
| F2 noise | `e`, bands and spectrum are each `150 + j` | No `T`, `D`, cuts or pulses. Spectral levels stay 2. The gate rests dim. |
| F3 aperiodic | Base `e`, bands and spectrum 40. Spikes to 220 for 2 steps at 37, 101, 113, 250, 400 and 523. | `T` fires at each spike onset. Cuts happen at 37, 101, 250, 400 and 523; 113 is dropped by `CUT_MIN`. Gate pulses at all six. Held never changes. |
| F4 notes | `e`, bands and spectrum 80. Note 60 at velocity 100 at steps 45, 46, 130 and 352, and at velocity 40 at 200. | Cuts and pulses happen at 45, 130 and 352. Step 46 is dropped, and step 200 is ignored. Held never changes. |
| F5 rhythmic | `beat` at `30k` (k = 1..19), and `bar` at `120k`. On a beat step and the next 3 steps, `e=180`, `bass=200`, `spectrum[0..1]=200`. Otherwise everything is 60. | There are 19 cuts, one per beat. Layout changes at 120, 240, 360 and 480: V2→H2→V3→H3→V2. There are 19 pulses. Held never changes. |
| F6 floor | Steps 0–59 are 120. Steps 60–419 are an LCG floor `(x >> 8) % 13` per field. Steps 420–599 are 120, with a spike to 220 at 480–481. | Spectral steps 2→1→0 at 60 and 65. Silence starts before step 130. Nothing changes from step 80 to step 419. Silence ends at 420. There are cuts and pulses at 420 and 480. Spectral steps 0→1→2 at 420 and 425, then 2→3→2 at 480 and 485. |
| F7 sweep | `e=150`. Steps 0–299 have bands 200/60/60 and spectrum 0–3 at 200, the rest at 60. Steps 300–599 have bands 60/60/200 and spectrum 4–7 at 200, the rest at 60. | Held changes from combo 0 to combo 2 exactly once, between steps 330 and 345, with no layout change. Spectral bass goes 3→2→1 at 300 and 305, and treble goes 1→2→3. Cuts and gate fire at 300. |

Fixtures live under `host/fixtures/colour-field/` as `.cvz` streams once the
reference implementation lands. Emulator captures are emulator evidence only,
not hardware or projector validation (#31).
