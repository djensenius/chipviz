# chipviz colour-field scene v0

Colour fields are two or three large regions of flat colour. They are meant to
be projected onto performers in a dark space. This contract is shared by the
modern renderer (#68) and every console port (#69). It uses
[`control-frame-v0`](control-frame-v0.md) unchanged. The state machine is
deterministic. On the full control-frame path, the same fixture input gives the
same trace on every target. Reduced transports have their own oracle (see
[Reduced transports](#reduced-transports)).

## Projection rules

- Every visible pixel belongs to exactly one region. There are no margins,
  text, sprites, camera, gradients, dithering, or anti-aliased edges. Hardware
  borders that cannot be filled are exceptions (see the C64 note).
- Each region is one solid colour. All of a step's colour and layout changes
  are presented together at one VBlank or frame present. A partially updated
  layout is never shown. Tile consoles change colours with palette writes at
  VBlank. They change layouts either by flipping between two prepared
  nametables or screens, or by a rewrite that fits in one VBlank. On the NES,
  that is the 64-byte attribute table.
- **C64:** colour RAM (`$D800`) cannot be double-buffered, so it is not used.
  Enable extended colour mode (ECM) and point every cell at a blank glyph. The
  top two bits of each screen code then select `$D021`/`$D022`/`$D023` as that
  cell's colour, one register per region. Colours change through those
  registers at VBlank. Layouts flip between two prepared screen matrices via
  `$D018`. The border (`$D020`) stays black.
- Cuts are abrupt, with no forced fades or easing.
- Modes 0–2 must stay within 3 flashes per second, following WCAG 2.3.1 and the
  Harding test. Flash/gate exceeds that limit and is opt-in only.

## Selection through existing fields

### Scene index 7 is the colour field

- `scene == 7` selects the colour field on every target. It fits the 3-bit HID
  keyboard scene bits (key `8`), the 4-bit N64 Joybus scene nibble, and the
  `u8` field. Every target checks for 7 before applying its existing scene
  mapping.
- Existing scene mappings stay the same for other indices:
  - The modern renderer checks `scene == 7` first, then uses
    `REGISTRY[scene % REGISTRY.len()]`. The registry length may grow; scene 7
    still means the colour field whatever that length is.
  - The N64 homebrew uses the raw 0..15 scene as a drawing parameter.
  - The GBA homebrew displays `(phrase + override) & 3`, which changes over
    time.

  When the effective scene is 7, ports must stop deriving the displayed scene
  from time.
- Controller scene-select rings become `native scenes..., 7`. `next` from the
  last native scene goes to 7, and `next` from 7 goes to the first. `prev` works
  the same way in reverse. This applies to HID D-pad up/down, the shared
  `chipviz_interface` next/prev inputs, and local rings such as the GBA
  `GBA_SCENE_COUNT`. Auto mode advances through the same ring.
- **Requirements for scene sources:**
  - Procedural and demo generators never emit scene 7. That includes the N64
    `procedural_packet`, which currently can.
  - `chipsynth_stream.py` never maps a chip ID to scene 7. When the operator
    forces scene 7, it sends a fixed, operator-chosen palette instead of the
    dominant channel.
  - `live_bridge.py` audio currently cycles `scene = (i // 240) & 7` and
    `palette = (i // 60) & 7`. It must skip scene 7, or hold a fixed palette
    there that is not 3 or 7.
  - `live_bridge.py` MIDI uses CC1 to select the scene and CC2 the palette.
    Selecting 3 or 7 through CC2 while in scene 7 needs `--allow-flash`.
  - **Every host path** follows one rule: it never sends `p` of 3 or 7 while in
    scene 7 unless that host process was started with `--allow-flash`. Host
    paths include live audio, MIDI, chipsynth, the procedural bridge, and file
    playback of `.cvz` streams from `chipviz_encode`. Without the flag, the
    host rewrites such a frame's palette to 2 or 6. This covers recorded files
    as well as live sources.

### Palette index selects mode and family

In scene 7, `p = palette & 7` and bits 3..7 are ignored. `mode = p & 3` and
`family = p >> 2`.

| `p` | Mode | Family |
| --- | --- | --- |
| 0 | held (default) | A: red/yellow/blue |
| 1 | spectral | A |
| 2 | hard cuts | A |
| 3 | flash/gate (opt-in) | A |
| 4–7 | same as 0–3 | B: RGB/CMY |

The effective palette `p_eff` comes from one of three sources:

- **Local:** procedural fallback, demo or attract. `p_eff` is the locally
  selected value, starting at 0. The frame's `palette` field is ignored, so
  procedural `(frame/32)%8` and the GBA demo cycling never drive the mode.
  Demo and attract never use flash/gate: a local 3 or 7 runs as 2 or 6.
- **Operator:** only in an `operator` input role (see
  [Input roles](#input-roles)). A pulse sets the requested palette `p_req`,
  which is applied under the [change budget](#change-budget).
- **Wire:** control frames, file playback, Joybus and HID palette bits.
  - On scene entry, `p_eff` is the wire value. If that value is 3 or 7, it runs
    as 2 or 6.
  - The stability counter starts at scene entry. The entry value counts as
    step 1.
  - A wire value, including one present at entry, becomes `p_req` once it has
    been received unchanged for `P_STABLE = 30` consecutive steps. A wire 3 or
    7 held for 30 steps from entry therefore arms on its 30th step and, when
    the change budget allows (as in F10), runs from the 31st.
  - A stable 3 or 7 becomes an *armed request* (see
    [arming](#arming-and-operator-inputs)). The flash latch is set only when
    that request is applied under the change budget, never before.
- **Precedence:** a later event overrides an earlier one, whatever its source.
  - An operator pulse sets `p_req` and consumes the current wire value: that
    value never becomes `p_req` again, however long it stays.
  - The stability counter restarts only when a new wire value (different from
    the consumed one) arrives. That new value overrides the operator choice
    once it has stayed unchanged for 30 steps.

If `p_eff` selects mode 3 and flash is not armed, the target runs mode 2.

### Flags

Flag meanings are unchanged, and the reserved bits stay clear. The colour field
uses the rising edges of `beat`, `bar` and `drop`, and ignores `fill` and
`manual`.

## Layouts and geometry

| Id | Name | Regions |
| --- | --- | --- |
| 0 | V2 | 2 columns |
| 1 | H2 | 2 rows |
| 2 | V3 | 3 columns |
| 3 | H3 | 3 rows |

- **Region order:** region 0 is leftmost for columns and bottom for rows. Region
  `i` covers `[b_i, b_(i+1))`.
  - For columns, `b_i` is measured from the left edge.
  - For rows, `b_i` is measured from the bottom edge. Screen row
    `y = S - 1 - y_from_bottom`.
- **Boundaries:** `b_i = g * ((i * (S / g)) / n)` with integer division, and
  `b_n = S`. `S` is the size along the split axis, which must be a multiple of
  `g`.
- **Granularity `g`:** 1 on modern and N64, 8 on tile consoles, 16 on NES
  attributes. On the C64, `g = 8` with `S` = 320 by 200.
- **Examples:**
  - 256 px, `g=8`, `n=3`: boundaries at 80 and 168.
  - DMG 160 px: 48 and 104.
  - NES 256 px, `g=16`: 80 and 160.
- **Operations:**
  - `next_layout`: `layout = (layout + 1) & 3`.
  - `next_combo`: `combo = (combo + 1) & 3`.
  - `rotate`: `rot = (rot + 1) mod n`.
  - After any layout change, `rot = rot mod n` for the new `n`.

### Layout pinning (local, no wire change)

`pin` is `auto` or a fixed layout id, and it starts as `auto` at power-on. The
pin control cycles the requested pin `pin_req` through `auto → V2 → H2 → V3 →
H3 → auto`. Only the latest request is kept. A request that would change the
displayed layout is applied under the [change budget](#change-budget); setting
`auto`, or pinning the layout already shown, applies at the end of the step
without using the budget.

When the pin is set, layout equals the pinned layout:

- Event-driven `next_layout` does nothing, except that hard-cut action 3 rotates
  instead (see [Modes](#modes)).
- Applying a pin that changes the layout counts as a layout change: it sets
  `since_layout = 0`, and in spectral mode it re-initialises `lvl` as
  `next_layout` does.

Setting the pin back to `auto` resumes event-driven changes from the current
layout.

### Input roles

ROMs cannot tell a physical pad from a USB-HID-driven one, so the safety
default treats all pad input as data.

Every build or config declares one input role:

- **`data` (default):** pad and keyboard input is a data channel. There is no
  local pin, no physical arming, and no warning-screen confirmation from pad
  input. Scene and palette pulses are wire values, so they go through
  `P_STABLE`.
- **`operator`:** explicitly declared for standalone builds with a human at the
  controls. It enables the pin control and physical arming. An operator build
  must never be fed by USB HID.

| Target | Role mechanism | Pin control (operator role only) |
| --- | --- | --- |
| modern | CLI flags are operator by nature | `--layout auto\|v2\|h2\|v3\|h3`, and the `L` key cycles |
| GBA | `make INPUT_ROLE=operator` (`-DCHIPVIZ_INPUT_ROLE_OPERATOR`) | `L` button |
| SNES, NES, GB | same build flag | `Select` |
| Genesis | same build flag | `C` |
| SMS | same build flag | button 2 |
| C64 | same build flag | `L` key (`RETURN` confirms the warning screen) |
| N64 | none: always `data` | no pin |

Why `data` is the default:

- **USB HID collisions:** the shoulders carry the high-energy and treble
  buckets, which collide with the GBA `L` pin. Select is part of the
  auto/demo/manual combos, and Start is part of the auto combo. A HID feed
  could therefore pin or arm by accident.
- **N64:** all four Joybus ports are data. Controller 4 Start/Z carry scene
  bits. The ROM falls back to procedural only when no controller is connected
  (`main.c`), so a local controller 1 is never an operator input. The N64 has
  no console-side pin or arming: its layout is always event-driven, and flash
  arms only by host selection. The fallback-detection marker in #43 may change
  this later; this spec does not depend on it.
- **Clashes:** ports confirm the pin control does not clash with an existing
  binding.

## Palettes

| Id | sRGB | | Id | sRGB |
| --- | --- | --- | --- | --- |
| A.R red | `#E01020` | | B.R red | `#FF0000` |
| A.Y yellow | `#FFD000` | | B.G green | `#00FF00` |
| A.B blue | `#1030C0` | | B.B blue | `#0000FF` |
| W white | `#FFFFFF` | | B.C cyan | `#00FFFF` |
| K black | `#000000` | | B.M magenta | `#FF00FF` |
| | | | B.Y yellow | `#FFFF00` |

The combos below list colours for regions 0, 1 and 2. Region `i` shows entry
`(i + rot) mod n`.

| Combo | A, 2 regions | A, 3 regions | B, 2 regions | B, 3 regions |
| --- | --- | --- | --- | --- |
| 0 | R, B | R, Y, B | R, C | R, G, B |
| 1 | Y, B | R, W, B | G, M | C, M, Y |
| 2 | R, Y | Y, K, B | B, Y | R, K, C |
| 3 | B, W | R, K, Y | C, M | B, K, Y |

**Levels:** each region is shown at level `L` (0..3). Modern and N64 scale each
sRGB channel as `c' = (c * L + 1) / 3`, using integer division. When the surface
is sRGB, convert to linear before writing.

### Per-console hardware colours

Values are nearest by squared sRGB distance unless marked **chosen**. Ports
confirm them with emulator captures.

| Colour | SMS `--BBGGRR` | Genesis `BBB0GGG0RRR0` | SNES/GBA BGR555 | C64 (Pepto) | NES | NES L2 |
| --- | --- | --- | --- | --- | --- | --- |
| A.R | `0x03` | `0x20C` | `0x105B` | 10 light red | `$16` | `$06` |
| A.Y | `0x0B` | `0x0CE` | `0x033F` | 7 yellow | `$28` | `$18` |
| A.B | `0x24` | `0xA20` | `0x5CC2` | 6 blue | `$12` **chosen** | `$02` |
| W | `0x3F` | `0xEEE` | `0x7FFF` | 1 white | `$30` | `$10` |
| K | `0x00` | `0x000` | `0x0000` | 0 black | `$0F` | `$0F` |
| B.R | `0x03` | `0x00E` | `0x001F` | 2 red | `$16` | `$06` |
| B.G | `0x0C` | `0x0E0` | `0x03E0` | 5 green | `$19` | `$09` |
| B.B | `0x30` | `0xE00` | `0x7C00` | 6 blue | `$12` **chosen** | `$02` |
| B.C | `0x3C` | `0xEE0` | `0x7FE0` | 3 cyan | `$3C` | `$2C` |
| B.M | `0x33` | `0xE0E` | `0x7C1F` | 4 purple **chosen** | `$14` | `$04` |
| B.Y | `0x0F` | `0x0EE` | `0x03FF` | 7 yellow | `$28` | `$18` |

- **SMS, Genesis, SNES, GBA:** dim levels apply `(v * L + 1) / 3` to each
  hardware channel. On the SMS, A.R and B.R are identical.
- **N64:** uses exact sRGB. A 16-bit framebuffer quantises like BGR555.
- **C64:**
  - Levels 3 and 2 use the base colour; levels 1 and 0 use black.
  - Magenta's nearest colour by distance is 14 (light blue), so purple is used
    to keep the hue.
- **NES:**
  - The reference palette is the 2C02 table in Wikipedia's "List of video game
    console palettes". For example, `$16` is `#F83800`, `$12` is `#0058F8` and
    `$19` is `#00B800`.
  - The owner chose `$12` for both blues because it is brighter than the
    nearest values (`$02` and `$01`). Against this table, the nearest green is
    `$19` (`$1A` is second), and the nearest cyan is `$3C` (`#00FCFC`).
  - Level 3 is the base colour, level 2 uses the L2 column, and levels 1 and 0
    are `$0F`. Never use `$0D`.
  - Each region uses its own background palette.
- **Game Boy DMG (gating variant):** shades run from 0 (lightest) to 3 (black),
  and both families use the same shades.

  | Combo | 2 regions | 3 regions |
  | --- | --- | --- |
  | 0 | 0, 3 | 0, 3, 1 |
  | 1 | 1, 3 | 3, 0, 2 |
  | 2 | 0, 2 | 1, 3, 0 |
  | 3 | 3, 1 | 0, 2, 3 |

  In spectral mode, the shade is `3 - L`. In flash/gate, a lit field shows the
  combo shades, a dim rest shows all regions at shade 2, and a dark rest shows
  all regions at shade 3.

## Step clock and inputs

**Full control-frame path:** one step runs for each new `frame` counter value,
subject to the 60 steps per second cap below.

**Local sources** (procedural, demo): one step runs per generated frame.

<a id="reduced-transports"></a>**Reduced transports:** one step runs per VBlank
poll. Unavailable fields are 0.

- **N64 Joybus** (bytes as packed by `n64_joybus.py`):
  - `spectrum[0..5]` are bytes 2, 3, 6, 7, 10 and 11, and `spectrum[6..7]` are
    0.
  - `bass` is the larger of `spectrum[0..1]`, `mid` of `spectrum[2..3]`, and
    `treble` of `spectrum[4..5]`.
  - `energy` is byte 14.
  - Byte 12 bits 0..3 carry `flags & 0x0F`: bit 0 is `beat`, bit 1 is `bar`, and
    bit 2 is `drop`. Bit 3, `fill`, is ignored.
  - Scene is `((b12 >> 4) & 3) | (((b13 >> 4) & 3) << 2)`, and palette is
    `b13 & 0x0F`.
  - A note event is a rising edge of byte 15 crossing `NOTE_VEL`.
- **USB HID** (as packed by `usb_hid.py`):
  - `energy` is the left stick X plus 128. `signed_bucket` clamps to -127, so a
    sent energy of 0 arrives as 1. That is still below every threshold.
  - `bass`, `mid` and `treble` all equal `energy`, and the spectrum is 0.
  - `beat` is South and `bar` is East.
  - West carries `flags & 0x0C`, which is `drop` or the held `fill`. HID cannot
    tell them apart, so West is ignored and HID has no `X` events.
  - The shoulders and the left stick Y (`beat_phase`) are ignored, and there are
    no notes.
  - Scene and palette come from keys or pulses, through the wire stability gate.

  With no separate bands, dominance never changes on HID.

On reduced transports, the oracle is the reference implementation fed the
mapped frames, stepped once per poll.

**Step rate (normative):** every rate limit in this spec assumes at most 60
steps per second.

- Renderers and ROMs must advance colour-field logic (detector, counters and
  mode rules) at most 60 times per second of wall-clock time. Extra display
  frames repeat the previous step's output and do not advance any counter.
- Hosts must not send more than 60 colour-field frames per second, for example
  `live_bridge --rate 120` must drop to 60 while scene is 7. A target that
  receives frames faster must resample to at most 60 steps per second, using
  the newest frame and OR-ing flag rising edges and note onsets since the last
  step, rather than stepping once per frame.
- At 50 Hz the step counts are the same, which only lowers the rates. The flash
  limits therefore hold whatever the transport rate.

Counters are saturating `u16`. A counter that is reset during
a step reads 1 on the next step, so "at least N steps" means the next action can
happen at `t + N`. Arithmetic is signed 16-bit.

## Detector (integer reference)

```
EMA:     acc_x (u16), avg_x = acc_x >> 3, update acc_x += x - (acc_x >> 3)
         x in {e=energy, b0=bass, b1=mid, b2=treble}
SILENCE: SIL_AVG 12, SIL_EXIT 24, SIL_HOLD 30
TRANS:   T_FLOOR 24, T_E_ON 40, T_E_OFF 16, T_B_ON 56, T_B_OFF 24,
         FLUX_ON 200, FLUX_OFF 64
NOTE:    NOTE_VEL 64
DOM:     DOM_MARGIN 32, DOM_HOLD 30, SECTION_MIN 600
CUT:     CUT_MIN 20                       (<= 3 cuts/s)
LEVEL:   UP[1..3] = 48,104,168  DOWN[1..3] = 32,88,152
         LEVEL_OPP 10   field-wide (<= 3 flashes/s for the whole field)
GATE:    GATE_ON 4, GATE_OFF_MIN 4, REST_DIM_ON 64, REST_DIM_OFF 48
WIRE:    P_STABLE 30
```

### Entry

The first step after entering scene 7 initialises the state, renders, and raises
no events. This happens at power-on and every time scene 7 is entered again.

- **Averages and previous input:** `acc_x = x << 3`, `prev_spec = spectrum`,
  previous flags and notes are empty.
- **Detector state:** `armed = 1`, `silent = 0`, `quiet = 0`.
- **Dominance:** `dom = argmax(b0, b1, b2)`, with ties going to the lowest
  index. `cand = none` and `cand_n = 0`.
- **Layout and colours:** `layout = pin` (or `0` when the pin is `auto`), and
  `rot = 0`. `combo` is `dom` in held mode, otherwise `0`.
- **Mode state:** apply mode entry (below).
- `since_layout = 0`.

**Mode entry** runs on scene entry and whenever the effective mode (after the
unarmed 3→2 rule) or the family of `p_eff` changes. `layout`, `combo` and `rot` are kept, except that held mode sets
`combo = dom`.

- **Counters:** `act = 0`. On scene entry, `since_cut = CUT_MIN` and `lg =
  LEVEL_OPP`. On a mode or family change, `since_cut = 0` and `lg = 0`: the
  change is itself a visible full-field change and uses the budget.
- **Spectral levels:** each region gets `lvl[i] = count(UP[k] <= v_i)`.
  `lvl[2] = 0` in 2-region layouts.
- **Gate:** `pulse = 0`, `gate_off = GATE_OFF_MIN`, and `dim = avg_e >=
  REST_DIM_ON`, where `avg_e` is the current average.
- **Silence:** in spectral or flash/gate mode, if `silent`, all regions are
  black.

### Each later step

1. Compute the deltas against the previous averages:
   - `de = e - avg_e`
   - `db[i] = b_i - avg_b_i`
   - `flux = Σ_j max(0, spectrum[j] - prev_spec[j])` (`u16`)

   Then update every `acc_x` and set `prev_spec`.
2. Update silence:
   - If `silent`: when `e >= SIL_EXIT`, set `silent = 0` and `quiet = 0`.
   - Otherwise: set `quiet` to `quiet + 1` when the new `avg_e <= SIL_AVG`, else
     to 0. When `quiet >= SIL_HOLD`, set `silent = 1`. On that entry,
     flash/gate goes black at once: the gate rests dark and `pulse = 0`. Held
     and cuts hold their state.
   - **Spectral blackout:** in spectral mode, on every step where `silent` is
     set and any `lvl > 0`, set all `lvl = 0` and `lg = 0` if `lg >=
     LEVEL_OPP`. Otherwise wait for a later silent step. The blackout is a
     visible change and uses the budget; if silence ends first, it is
     cancelled.
3. Check for transients:
   - `hit = e >= T_FLOOR && (de >= T_E_ON || any db[i] >= T_B_ON || flux >=
     FLUX_ON)`
   - `calm = de < T_E_OFF && all db[i] < T_B_OFF && flux < FLUX_OFF`
   - If `armed && hit`, set `T = 1` and `armed = 0`. Otherwise, if `calm`, set
     `armed = 1`.
4. Find events:
   - `N` (note onset): some pair `(note, velocity)` in
     `notes[0..min(note_count,4)-1]` has `velocity >= NOTE_VEL` and does not
     appear in the previous frame's notes.
   - `B`: rising edge of `beat` or `bar`.
   - `X`: rising edge of `drop`.
   - `E = !silent && (T || N || B || X)`.
5. Update dominance with the new averages. Let `c = argmax(avg_b)`, with ties
   going to the lowest index.
   - If `!silent && c != dom && avg_b[c] >= avg_b[dom] + DOM_MARGIN`, then
     `cand_n` becomes `cand_n + 1` if `cand == c`, otherwise `1`. Set
     `cand = c`.
   - Otherwise, set `cand = none` and `cand_n = 0`.
   - When `cand_n >= DOM_HOLD`, set `dom = c`, `cand_n = 0`, and `D = 1`.
6. If not silent, apply the mode rule. `SECTION = X || (D && since_layout >=
   SECTION_MIN)`. Any `next_layout` sets `since_layout = 0`. In spectral mode,
   it also re-initialises `lvl` from the current values for the new region
   count, as in mode entry, and sets `lg = 0`.
7. Increment `since_cut`, `since_layout`, `gate_off` and `lg`. Then update
   `p_req` (wire stability gate, operator pulses) and `pin_req`, and apply at
   most one pending request under the change budget, so it takes effect from
   the next step.

### Change budget

`lg` is a field-wide count of steps since the last visible change, kept in every
mode. Outside armed flash/gate, every visible change sets `lg = 0`: a spectral
level change, any `next_layout`, `next_combo` or `rotate`, a held-mode combo
change on `D`, an applied pin, and a mode or family change. Flash/gate pulses do
not touch `lg`, so a request to leave mode 3 is never starved.

At the end of step 7, a pending request is applied only when `lg >= LEVEL_OPP`
and `since_cut >= CUT_MIN`:

- A request is a pair `(p, armed)`. Its effective mode is `p & 3`, except
  that 3 counts as 2 unless the request is armed. Requests are compared by
  effective mode and family, never by raw `p`.
- A `p_req` whose effective mode or family differs from the current one is
  applied: `p_eff = p`, the flash latch is set if the request is armed and its
  effective mode is 3 (cleared otherwise), and mode entry runs, which sets
  `lg = 0` and `since_cut = 0`.
- Otherwise, a `pin_req` that changes the layout applies the pin and sets `lg =
  0` and `since_cut = 0`.
- Otherwise, the request stays pending. Only the latest `p_req` and `pin_req`
  are kept; a newer request replaces an older one, and a request whose
  effective mode and family equal the current ones is cleared without
  effect. When an unapplied armed request is replaced or cleared, its arming
  is discarded with it, so a later unarmed 3 or 7 still runs as 2 or 6 (and
  shows the warning screen in `operator` builds).

Operator- and host-driven mode, family and pin changes are therefore at most 3
per second, the same as hard cuts. The flash latch is only ever set by applying
a request, so arming always goes through the budget.

## Modes

- **Held (0):** level 3.
  - On `D`, set `combo = dom` (bass 0, mid 1, treble 2).
  - On `SECTION` with `since_cut >= CUT_MIN`, call `next_layout` and set
    `since_cut = 0`.
- **Spectral (1):** each region follows a raw band value `v_i`.
  - In 3-region layouts, the regions follow bass, mid and treble.
  - In 2-region layouts, they follow bass and `max(mid, treble)`.
  - **Order within the step:**
    1. On `X` with `since_cut >= CUT_MIN` and `lg >= LEVEL_OPP`:
       `next_layout`, `next_combo`, and `since_cut = 0`.
    2. Else on `D` with `since_layout >= SECTION_MIN` and
       `lg >= LEVEL_OPP`: `next_layout`.
    3. Only then, if `lg >= LEVEL_OPP`, update every region's level. A layout
       change in steps 1 or 2 set `lg = 0`, so it blocks this.
    An `X` or `D` that arrives while `lg < LEVEL_OPP` is dropped, not queued,
    so layout changes and level changes together stay 10 or more steps apart.
  - **Level up:** `lvl += 1` when `v >= UP[lvl + 1]`.
  - **Level down:** `lvl -= 1` when `v < DOWN[lvl]`.
  - **Field-wide rate limit:** any region may change in the same step. If any
    did, set `lg = 0`. Level changes therefore happen at most once per 10 steps
    across the whole field, which keeps it at or below 3 flashes per second.
    There are no per-region timers.
- **Hard cuts (2):** level 3. On `E` with `since_cut >= CUT_MIN`, set
  `since_cut = 0` and `lg = 0`, then:
  - If `X`: `next_layout` and `next_combo`.
  - Otherwise, apply action `act` and then set `act = (act + 1) & 3`:
    - 0: rotate
    - 1: `next_combo`
    - 2: rotate
    - 3: `next_layout`, or rotate when pinned

  An `E` that is not accepted is dropped, not queued.
- **Flash/gate (3, armed only):**
  - **Rest:** the field rests at level 1 when `dim`, otherwise level 0. `dim`
    is updated first in this mode's step, from the average updated in step 1.
    It becomes true when `avg_e >= REST_DIM_ON` and false when `avg_e <
    REST_DIM_OFF`.
  - **Pulse:** if `pulse > 0`, decrement it; when it reaches 0, set
    `gate_off = 0`. Otherwise, an `E` with `gate_off >= GATE_OFF_MIN` starts a
    pulse: `pulse = GATE_ON`, rotate, and on `X` also `next_combo`.
  - **Lit:** the field shows level 3 while `pulse > 0`.
  - **Limit:** at most 1 pulse per 8 steps (7.5 Hz).

**Trace:** each step records
`(mode, family, layout, combo, rot, lvl0, lvl1, lvl2)`, where `lvl` is the
displayed level for each region and unused regions are 0. If audio starts
silent, held and cuts show combo 0 at level 3. Spectral and flash/gate show
black.

## Arming and operator inputs

Flash is armed only by a deliberate source.

- **(a) Operator arming:** only in the `operator` input role (see
  [Input roles](#input-roles)).
  - **modern:** the `--allow-flash` CLI flag prints the full warning to stderr
    when the renderer starts. While the process runs, every operator request
    for 3 or 7 is an armed request, so it re-arms each time mode 3 is applied
    again after the latch has cleared.
  - **Consoles built with the `operator` role:** use the warning screen below,
    confirmed with the target's confirm input: controller 1 `start` as the port
    maps it for interface-v0, or the `RETURN` key on C64, which has no Start
    button.
  - **Consoles in the default `data` role and N64:** no operator arming at all.
    Pad input there may be a HID feed, and the N64's Joybus ports carry
    controller 4 Start/Z as scene bits.
- **(b) Deliberate host selection:** a wire `p` of 3 or 7 that passes the
  `P_STABLE` gate while scene is 7 becomes an armed request. This includes a 3
  or 7 present at scene entry: it runs as 2 or 6, and after 30 stable steps its
  armed request differs in effective mode, so it is applied under the budget.
  - Hosts emit 3 or 7 only when that host process was started with
    `--allow-flash` (see [Requirements for scene sources](#scene-index-7-is-the-colour-field)).
    A host with that flag prints the full warning when it starts.
  - Auto mode on the host may arm flash this way.
  - After the latch clears, a wire 3 or 7 re-arms once it is again stable for
    `P_STABLE` steps.

Edges, pulses, or values held for fewer than `P_STABLE` steps on a data channel
never arm. Demo, attract and procedural sources never arm. The armed latch
clears at power-on and reset, when leaving scene 7, and when `p_eff` leaves
mode 3.

**Console warning screen:** this is a local screen added to the interface-v0
flow, with no wire change. It exists only in `operator` builds.

- **When it appears:** an operator palette pulse selects mode 3 while flash is
  unarmed.
- **While it is shown:**
  - The detector is paused and the projection shows the text below.
  - The confirm input (`start`, or `RETURN` on C64) turns the pending
    request into an armed request and returns to the visualizer; the latch is
    set when the change budget applies it. It is consumed there, so
    it does not trigger interface-v0's visualizer-to-`scene_select`
    transition.
  - Any other input, or 600 steps without input, cancels. The target then
    returns to the visualizer, still unarmed, in mode 2.
- **Wire arming (b)** never shows this screen.

Full warning (CLI, bridge and docs):

> WARNING: Flash/gate mode produces full-field flashing light at up to 7.5
> flashes per second. This exceeds the WCAG 2.3.1 and Harding limits of 3
> flashes per second. Saturated red flashes are the highest-risk case. It may
> trigger seizures in people with photosensitive epilepsy. Enable it only where
> performers and audience have been warned.

Console screen (at most 20 columns):

```
WARNING: FLASHING
LIGHT. MAY TRIGGER
SEIZURES IN PEOPLE
WITH PHOTOSENSITIVE
EPILEPSY.
START = ENABLE
OTHER = CANCEL
```

## Fixtures (normative)

**Setup:**

- Each fixture runs for 600 steps on the full path, with `frame = step` and
  `scene = 7`.
- `p` is selected locally at `p = 0..7`. For `p = 3` and `p = 7`, flash is
  pre-armed by operator arming (the harness acts as an `operator` build). Unarmed
  3 and 7 must equal 2 and 6.
- Unlisted fields are 0. "Bands" means bass, mid and treble. "Spectrum" means
  all 8 bands.
- **LCG:** `x = (x * 25173 + 13849) & 0xFFFF`, with `x0 = 1`. It advances once
  per field per step, in wire order. Jitter is `j = ((x >> 8) % 25) - 12`.

**Verification:**

- The Expected column is normative. Traces from the shared reference
  implementation must match it, and every full-path sim must match the
  reference trace exactly.
- Emulator captures must show the formula boundaries, and every pixel of a
  region must be its expected hardware colour.

| Id | Input | Expected |
| --- | --- | --- |
| F0 silence | All 0 | Constant trace. Held and cuts: V2, combo 0, level 3. Spectral and flash/gate: black. No cuts or pulses. |
| F1 tone + drop | `e=160`, bands 40/200/40, spectrum `40,40,120,200,200,120,40,40`. `drop` is set for steps 300–305. | Held is combo 1 from step 0. Spectral levels are 0/3. At step 300 only: held goes to H2; spectral and cuts go to H2 and combo 1; flash/gate pulses with combo 1 and rot 1. |
| F2 noise | `e`, bands and spectrum each `150 + j`. | No events. Spectral stays at 2/2. Flash/gate rests dim. |
| F3 aperiodic | `e`, bands and spectrum 40, spiking to 220 for 2 steps at 37, 101, 113, 250, 400 and 523. | Cuts at 37, 101, 250, 400 and 523 (113 is dropped). Pulses at all six. Held never changes. |
| F4 notes | `e`, bands and spectrum 80. Note 60 at velocity 100 at steps 45, 46, 130 and 352. Note 60 at velocity 40 at 200. | Onsets at 45, 130 and 352 (46 is a repeat and 200 is too quiet). Cuts and pulses at those three steps. |
| F5 rhythmic | `beat` at `30k` (`k = 1..19`) and `bar` at `120k`. A beat step and the next 3 steps have `e=180`, `bass=200` and `spectrum[0..1]=200`; everything else is 60. | 19 cuts. Layout changes at 120, 240, 360 and 480 (V2→H2→V3→H3→V2). 19 pulses. Held never changes. |
| F6 floor | Steps 0–59 are 120. Steps 60–419 are a floor of `(x >> 8) % 13` per field. Steps 420–599 are 120, with 220 at 480–481. | Spectral steps 2→1→0 at 60 and 70. Silence starts before step 130. No change from 80 to 419. Cuts and pulses at 420 and 480. Spectral steps 0→1→2 at 420 and 430, 2→3 at 480, and 3→2 at 490 (field-wide `LEVEL_OPP`). |
| F7 sweep | `e=150`. Steps 0–299 have bands 200/60/60 and spectrum 0–3 at 200. Steps 300–599 have bands 60/60/200 and spectrum 4–7 at 200. Everything else is 60. | Held changes combo 0→2 once, between steps 330 and 345, with no layout change. Spectral (V2) goes from 3/1 to 2/2 at 300 and to 1/3 at 310. Cuts and pulses at 300. |
| F8 held note | `e`, bands and spectrum 80. Note 60 at velocity 100 for steps 100–299, and note 64 at velocity 100 added for steps 200–299. | Onsets at 100 and 200 only. Cuts and pulses at 100 and 200. |
| F9 beat/bar | `e`, bands and spectrum 100. `beat` at `20 + 40k` (`k = 0..14`) and `bar` at `40 + 120k` (`k = 0..4`). | 20 cuts and 20 pulses, one at every beat and bar step. Held never changes. |
| F10 wire gate | F1 input without `drop`. Wire `p` is 0 for steps 0–99, 2 for 100–109, 0 for 110–199, and 3 from 200 on. | `p_eff` is 0 through step 229 and 3 from step 230, when mode 3 enters resting dim (armed by host selection). Steps 100–109 have no effect. |
| F11 pinned | F5 input, pinned to H3. | Layout is always H3. 19 cuts, and every fourth cut rotates instead of changing layout. |
| F12 mode toggle | F0 input, starting at `p = 0`. Operator pulses request `p = 1` at steps `60 + 12k` and `p = 0` at `66 + 12k`, through step 594. | Requests apply at the end of steps 60, 80, 100, 126, 146, 166, 192, … (20 or 26 steps apart; a request equal to the current `p_eff` is cleared). The field alternates held level 3 and spectral black, visible from the next step. Never more than 3 changes in any 60 steps. |
| F13 silence spikes | `e = 0` except `e = 30` at steps `31k` (`k = 1..19`). Bands 250, spectrum 0. | Spectral (`p = 1` and 5): black at 30; levels step up at 40, 50 and 60 (the spike at 31 waits for `lg`); the silence at 61 does not black out until `lg` allows, and silence ends at 62 first; black again at 92, up at 102, 112 and 122, and so on every 62 steps. Never more than 4 changes in any 60 steps. Other modes: no change. |
