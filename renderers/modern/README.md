# chipviz modern renderers

Rust + wgpu renderers for the modern 4K chipviz target.

The two binaries share the same scene code and control-frame decoder, but use
different quality budgets:

| Binary | Target | Default profile |
| --- | --- | --- |
| `chipviz-pi5` | Raspberry Pi 5 HDMI output | 4K-capable, lower instance count, simpler layering. |
| `chipviz-m1` | Apple Silicon Mac with large memory/GPU headroom | 4K60-oriented, denser 3D fields, deeper layering. |

The visual language should match the retro targets at the signal level: beat,
energy, palette, scene, flags, and spectrum all map to recognizable behavior.
Each platform then renders those controls using its native strengths.

## Run

```sh
just modern-pi5
just modern-m1 -- --udp 127.0.0.1:6464
```

Explicitly selecting scene 6 (`Starfield`) reads a user-supplied
HYG-compatible CSV; no catalog is bundled. It requires both an observer
location and the catalog path. The observation time defaults to current UTC:

```sh
just modern-m1 -- --udp 127.0.0.1:6464 --scene 6 \
  --star-catalog /path/to/stars.csv --location 45.5,-73.25
just modern-m1 -- --scene 6 --star-catalog /path/to/stars.csv \
  --location 45.5,-73.25
just modern-pi5 -- --scene 6 --star-catalog /path/to/stars.csv \
  --location 45.5,-73.25 --time 946728000
```

The CSV must have `ra` (hours), `dec` (degrees) and `mag` columns; `ci` and
`proper` are optional. Standard CSV quoting is supported. Explicit `--scene 6`
without both setup options exits with an actionable error before opening the
window. `--catalog` is an alias for `--star-catalog`; both load the file at
runtime and never embed it in the executable. Supplying a catalog alone does
not opt into Starfield. Scene 7 stays the ColourField. `--scene 6` opts into
Starfield for procedural or microphone
frames without a UDP sender and overrides UDP scene bytes when explicitly set.
Without that explicit option, every wire byte keeps its original modulo-six
mapping (byte 7 remains the ColourField); in particular, bytes 6 and 13 do not
silently become Starfield. Scene 6 requires its catalog and location at startup
when selected with `--scene`; unrelated scenes do not require a catalog.

Without `--udp` or `--mic`, the renderer uses deterministic procedural frames.
With UDP, it consumes `control-frame-v0` packets from the host bridge:

```sh
python3 host/bridge/chipviz_bridge.py --frames 600 --udp 127.0.0.1:6464
```

## Controls

The renderer starts windowed by default.

| Option | Effect |
| --- | --- |
| `--fullscreen` | Start in borderless fullscreen (primary display unless `--monitor` is given). |
| `--monitor INDEX\|NAME` | Choose the display by the index from `--list-monitors` (all-digit values are indices) or by a case-insensitive name substring. Names matching several displays, including identical models, are rejected as ambiguous; use the index. Without `--fullscreen`, the window opens on that display. |
| `--list-monitors` | Print each display's index, name, size, and scale, then exit. |
| `--layout auto\|v2\|h2\|v3\|h3` | Pin the colour-field (scene 7) layout: two vertical or horizontal regions, or three. Default `auto` lets events change it. |
| `--star-catalog PATH` | Read a user-supplied HYG-compatible CSV for scene 6; catalogs are not bundled or embedded. `--catalog` is an alias. |
| `--location LAT,LON` | Observer latitude and longitude in decimal degrees, longitude east positive. Required with `--star-catalog`. |
| `--time UNIX_SECONDS` | Observation time in UTC seconds since the Unix epoch (1800–2200); defaults to current UTC, advances while live, and requires `--star-catalog` plus `--location`. Capture manifests record the resolved epoch and whether it advances. |
| `--scene N` | Override the input scene byte (0-255), including UDP frames; absent by default. Byte 7 is ColourField, byte 6 selects Starfield when its catalog and location are configured, and other bytes keep the original modulo-six mapping. |
| `--mic-layers off\|scope\|bursts\|both` | Draw the [mic layers](#mic-layers) over eligible native scenes (not ColourField or Starfield). Default `off`. |
| `--allow-flash` | Only builds with the `unmeasured-flash` Cargo feature can flash (see [Flash/gate is off by default](#flashgate-is-off-by-default)). In those builds it prints the photosensitivity warning to stderr at start-up and arms operator requests for flash/gate (palettes 3 and 7); the opt-in for wire arming is the host's own `--allow-flash` (hosts send 3 or 7 only with it), so a stable absolute wire 3 or 7 in scene 7 arms with or without this flag. Default builds accept the flag, print a notice that it has no effect, and run 3 and 7 as hard cuts. Procedural and mic frames never arm. |
| `--lines-drop-cuts` | Let Lines (scene 3) cut straight between plane and hallway on a rising drop flag (0x04), at most once per bar. The cut refolds the same lines; it is not a full-screen flash. Default: the drop flag is ignored. |
| `--tunnel-variant original\|hallway\|abstract` | Choose the PulseTunnel look at startup, for every scene byte selecting it. Default `original` preserves the orbiting tunnel. Also supported by `chipviz-capture`. |

```sh
just modern-m1 -- --list-monitors
just modern-m1 -- --fullscreen --monitor "Studio Display"
just modern-m1 -- --fullscreen --monitor 1
```

| Key | Action |
| --- | --- |
| `F`, `Cmd+F`, `Ctrl+Cmd+F` | Toggle fullscreen on the window's current display. |
| `Esc` | Exit fullscreen (does nothing when windowed). |
| `L` | Cycle the colour-field layout pin: auto, V2, H2, V3, H3. A pin that changes the layout waits for the change budget. |
| `M` | Cycle the mic layers: off, scope, bursts, both. |
| `Cmd+Q` | Quit (macOS application menu). |

The cursor is hidden while fullscreen and restored when windowed.

## Microphone input

The renderer can analyse live microphone audio itself, with no bridge
needed. Capture is off by default.

| Option | Effect |
| --- | --- |
| `--mic` | Drive frames from the microphone (the system default input unless `--audio-device` is given). |
| `--audio-device INDEX\|NAME` | Choose the input by the index from `--list-audio-devices` (all-digit values are indices) or by name, matched like `--monitor`: a unique exact name, else a unique case-insensitive substring; ambiguous names are rejected. Requires `--mic`. |
| `--list-audio-devices` | Print each input device's index and name (marking the default), then exit. Combined with `--list-monitors`, both lists are printed under headings. |

```sh
just modern-m1 -- --list-audio-devices
just modern-m1 -- --mic
just modern-m1 -- --mic --audio-device "MacBook Pro Microphone" --fullscreen
```

Source precedence, highest first:

1. UDP packets (`--udp`). After a packet arrives, UDP keeps priority for
   500 ms, so a bridge streaming at the frame rate never flickers with mic
   frames.
2. The microphone (`--mic`), which replaces the procedural fallback.
3. Procedural frames, used only when neither `--mic` nor `--udp` is given.

Without `--mic`, behaviour is unchanged. Mic frames fill `energy`, `bass`,
`mid`, `treble`, the eight `spectrum` bands (log-spaced from 40 Hz to 12 kHz,
as in the host bridge), `bpm` (`0` until a tempo is detected), `beat_phase`,
`beat_count`, and the beat and bar flags. `frame`, `scene`, and `palette`
follow the same timeline as procedural frames, so scenes still cycle.

The beat clock only moves forward: `beat_count` increments exactly when
`beat_phase` wraps, and `beat_count * 256 + beat_phase` never decreases. Onsets
near the expected beat nudge the clock (jumping forward, or briefly slowing
down) instead of rewinding it; off-beat onsets are ignored. Tempo comes from
onset gaps folded into 60-200 BPM, so eighths faster than 200 per minute read as
the quarter-note tempo. Known limits: tempos outside 60-200 BPM are folded into
that range (tempos within about one analysis hop of either end are pinned to
it), very fast tempos (about 180 BPM and up) with loose timing can read at half
tempo, 3/4
and swung feels are not distinguished from straight time, and music with only
off-beat accents is tracked on those accents.

Analysis runs on its own thread; the render thread reads the latest result
without waiting. Levels use automatic gain with a noise floor, so quiet and
loud rooms both move the scenes, room hiss stays dark, and silence (or a
device that stops sending audio) fades smoothly to zero. Audio dropped while
analysis catches up still advances the beat clock, so it keeps real time, and
so do short gaps where the device itself loses audio (they count as silence).
Deliveries that run late for a while and then catch up, at once or
gradually, are not counted as lost: any silence added for them is taken back
(once a drift has been measured, a gradual catch-up takes back only silence
from the last 30 seconds or so).
A device clock running slightly slow or fast (up to about 0.5%) is followed as
a steady drift once it has been measured confidently, not mistaken for lost
audio or delivery jitter. Losses during it are still filled, but losses too
small to fill on their own (under about 20 ms) can be partly missed: in
simulations up to about a fifth of them at 0.1% drift and up to about 60% at
0.5%. A device that regularly loses about 5 ms of audio delivers exactly what
a slightly slower clock would, so those losses are followed as drift and not
filled; larger regular losses are filled, except, rarely, 10 ms losses whose
timing exactly matches a fast clock's drift. Rarely, with heavy delivery
jitter on a drifting clock, a fill is taken back as if it had covered a delay
(in simulations, 1 run in about 3,000). Heavy delivery jitter, or frequent
delays or losses, can keep the drift from being measured; the capture then
behaves as if the clock kept time. If the drift changes (a clock that starts or
stops drifting, or whose rate wanders), the measured drift is dropped once it
no longer fits, without filling or taking back anything for the change, and
the capture behaves as if the clock kept time until the new drift is measured.
While it does, a clock that has started running slow is filled as before, and
fills older than about 30 seconds are taken back only when late deliveries
catch up at once. Analysis polls every 5 ms
or so; if it is held up so that polls come regularly no more often than audio
blocks arrive, a drifting clock delivers exactly what lost blocks would, and
the drift is not measured. When polls are held up to about half a block or
more apart, a fast clock that regularly loses whole blocks can likewise be
followed as a slower drift, and those losses are not filled.

The stream is not reopened if the device is unplugged or fails: the renderer
prints a stream error (at most once every five seconds) and the levels fade to
silence. Restart the renderer to pick up a reconnected device.

If no input device exists, the name or index matches nothing, or the stream
cannot be opened, the renderer prints an error and exits with a non-zero
status before opening a window.

### macOS microphone permission

The unbundled binary has no app bundle of its own, so macOS asks the app that
launched it (Terminal, iTerm2, VS Code, ...) for microphone access, and the
permission is stored for that app. When access is denied, macOS sends silent
audio instead of an error, so after three seconds of pure silence the renderer
prints a warning that points to System Settings > Privacy & Security >
Microphone. To be asked again, reset the permission (for every app, or only for
one bundle ID), then restart the terminal app:

```sh
tccutil reset Microphone
tccutil reset Microphone com.apple.Terminal
```

On other platforms the silence warning instead suggests checking that the
input is not muted and is the expected device.

### Linux (including Raspberry Pi 5)

All renderer binaries, including `chipviz-pi5` and `chipviz-capture`, link
against ALSA even when `--mic` is not used. Building needs the headers
(`libasound2-dev` on Debian and Raspberry Pi OS) and running needs the runtime
library `libasound2` (e.g. `sudo apt install libasound2`).

## Mic layers

Two overlay layers can be drawn on eligible native scenes. They are suppressed
over the colour field (scene 7), whose colours depend only on its trace, and
over Starfield to preserve its luminance bound. They are off by default; choose
them with `--mic-layers` or cycle them with `M`. They do not change the
control-frame wire format. Their red flashes are not yet bounded
(see below), so like flash/gate they are drawn only in builds with the
`unmeasured-flash` Cargo feature, for measurement and development. Default
builds (including release packages) accept `--mic-layers` and `M`, print a
notice, and draw nothing.

- `scope`: an oscilloscope ribbon across the lower part of the screen. With
  `--mic`, it shows a 2,048-sample window aligned on a rising zero crossing,
  so steady tones hold still; the crossing is searched for in the newest
  period of a 30 Hz tone (at least 1,024 samples; 1,600 at 48 kHz), so the
  window can end up to about 33 ms (at 44.1 kHz and above) before the newest
  sample. Its 256 points each cover 8 samples, and it is drawn as two thin
  lines through each point's highest and lowest sample. For slow sound (a
  440 Hz tone, say) the two lines coincide and draw the waveform. Faster
  tones (from about 1 kHz at 48 kHz; 2 kHz at 96 kHz) and noise would make
  each line jump between extremes from point to point and fill a band, so
  the lines blend, by one weight for the whole window, into an envelope (the
  highest and lowest values within 4 points either side): a loud tone of a
  few kHz draws as two near-flat lines at its peaks. Tones in between keep
  their waveform until the lines would cover more height than a loud 440 Hz
  tone, then blend just enough to stay within that. It is peak-normalised
  with a floor so room hiss stays flat, and silence flattens it smoothly.
  Whatever its source (live, procedural or UDP), the ribbon is limited once
  more just before drawing: if the lines drawn in one frame, strokes
  included, would cover more than 13% of the screen, they blend towards
  their envelope and then shrink towards a flat centre line until they fit.
  This limits each frame's ribbon, not its afterglow (see below). A loud
  440 Hz tone is drawn unchanged. This limit tightens at once but relaxes
  over 0.5 s, so input hovering at the limit does not jitter, and just after
  a loud burst even a 440 Hz tone stays briefly reduced. In portrait windows
  the whole ribbon (amplitude and stroke widths as well as length) scales
  with the width. Each line is drawn as a dimmed palette-tinted halo and a
  near-white tinted core, with a gap between them where the scene shows
  through (the layers only add light), so the core is its brightest part.
- `bursts`: a soft glow with a ring of sparks at a detected onset. A burst
  rises over 80 ms and decays to nothing within 1.6 s. Onsets are rate-capped:
  one arriving less than 0.5 s after the last burst started is skipped, and
  onsets while bursts are hidden (layers `off` or `scope`, scene 7, or the
  hold after a resize or scene 7, below) start nothing, so none first appears
  part way up its rise. Hiding bursts drops any still fading, so none
  reappear mid-fade.
  In portrait windows bursts shrink with the width, as the ribbon does.

With `--mic`, the analysis thread publishes the samples, the onset count and
the newest onset's age (in analysed audio) through a lock-free side channel
(atomics only), which the render thread reads without waiting. An onset older
than 0.25 s when the renderer sees it, as after rendering was paused, starts
no burst. When frames come from the procedural source or over UDP, the ribbon
is synthesised from the eight spectrum bands and the energy, and bursts fire
when `beat_count` changes.

The layers glow like a long-persistence oscilloscope. They are drawn into
their own target, and a per-pixel phosphor follows it in linear light: each
channel rises at once to what is drawn but falls by at most 0.15 per second of
full brightness, and the result is added onto the scene. So the ribbon leaves
a fading trail wherever it has been, and a moving ribbon's trails can cover
far more of the screen than the 13% one frame draws; a burst lingers for a few
seconds after it ends; and turning the layers off fades them out over up to
about 6.7 s rather than cutting. Entering the colour field (scene 7), which is
already a full-screen change, clears the afterglow at once, so nothing is ever
drawn over it, and a window resize clears it too. Either sudden fall is one
the decay would have allowed, given time, so the layers then stay dark for as
long as the decay would have taken to fade what was cleared. The hold counts
from the layers' last drawn frame, so while the ribbon is shown (it draws
every frame) it is usually the full 6.7 s; only after the layers have drawn
nothing for a while is it shorter. After a resize the layers also wait until
the size has held still for 0.15 s, so a drag does not reallocate anything
each frame. Resizing in steps, or switching in and out of scene 7, therefore
makes the layers vanish and return, with their trails restarted, but no faster
than they could fade. Being added light, the layers brighten the scene beneath
them and never darken it, so they cannot be seen where the scene is white
(Static's white sections, for example); darkening the scene beneath them would
make their falls depend on the scene, outside the bound below. Red layer
colours are drawn slightly pink (see below).

While the layers glow, the scene is drawn into a texture of its own and a
final pass writes it with the layers added, in linear light; on a surface that
is not sRGB, that pass decodes and encodes sRGB itself. The phosphor needs
four 8-bit RGBA textures at the window size (the layers, two for the
afterglow, and the scene copy): about 33 MB at 1920x1080 and 133 MB at
3840x2160. Nothing is allocated, and no pass runs, until the layers first
draw, and the textures are freed again once the afterglow has faded out. On an
M1 Max, with scene 0 and both layers, a frame took about 0.3 ms longer at
1920x1080 (2.09 ms against 1.80 ms with layers off) and 1.8 ms longer at
3840x2160 (3.64 ms against 1.80 ms). The Pi 5 has not been measured; check its
frame rate with the layers on before relying on them there, especially at 4K.

The layers' added light (the output minus the scene) depends on the scene
only where a channel saturates at full brightness, so a scene change alters
it by at most the same amount in each linear channel; nothing about the
layers switches on the scene's colour. Luminance flashes are bounded for any
input, frame rate or geometry. A flash (WCAG 2.3.1) is a pair of opposing
changes of at least 0.1 relative luminance, and relative luminance falls no
faster than the fastest channel. Three flashes in 1 s would need three falls
of 0.1 ending in that second, the last two wholly inside it, and 0.2 of fall
takes at least 1.28 s at 0.15 per second (counting the decay's rounding), so
the light the layers add to any point flashes at most twice in any 1 s, under
the limit of three, with about 28% to spare for output rounding and frame
timing. Where the scene moves, the added light also moves by up to the
scene's own change.

That bound is on the light the layers add, not on what reaches the screen.
A scene that animates in step with the layers can combine with them into
flashes neither makes alone: a grey scene point alternating between black
and 0.091 at 5 Hz, under layers alternating between black and 10/255 in
step, changes by more than 0.1 five times a second, at 120 fps. This is one
reason the layers are gated (below).

Red flashes are not bounded. A red flash needs a saturated red, at least 80%
of a point's light, at one end of a change of its redness, and green or blue
rising over a red point lowers its redness as much as red falling. The
afterglow keeps green plus blue at least a third of its red (at most 75%
red), so where the scene is not a saturated red, the layers never make one.
But over a scene point that is already a saturated red, small green blips,
relit as soon as they fade, make three red flashes in 1 s. Fading the layers
out over red scene points would avoid that, but only with a steep switch on
the scene's colour, which turns small, harmless scene changes (one level of
noise near black, say) into large changes of the layers. Until there is a
design with a stated bound for red and for the combined output, and the
layers have been measured, they stay behind `unmeasured-flash`.

Unit tests run a CPU model of the phosphor against adversarial per-pixel input
at 60, 120 and 240 fps, steady and jittered. For luminance (alternating every
frame, square waves of every period, random colours, and input that relights a
pixel as soon as it has dimmed by 0.1) they find at most two flashes in any 1
s; at 0.25 per second the relighting input reaches three. For red (red against
white, red and green squares, relighting in red, green or white, a red and
green staircase, and random colours) they find no saturated red over black,
grey, orange, duller red and blue scenes, where per-channel decay with no
floor allows three red flashes in 1 s, and they find three over saturated red
scenes. Over animated scenes (one-level noise near black, a dark red changing
every frame, and red and grey alternating at two flashes per second), the
output never has more luminance or red flashes in 1 s than the larger of two
and the scene's own for those inputs; the earlier switch on the scene's colour
turned the dark red into over 100 flashes in 1 s. A regression test keeps the
in-step counterexample above (five flashes in 1 s), and another pins the
output to the scene over white. Windows alternating between two sizes, or
between scene 7 and a native scene, every 0.16 to 8 s give at most two flashes
in 1 s; without the hold, every 0.2 s gives five for sizes and three for scene
7. GPU tests check that the composite matches its model on sRGB and non-sRGB
targets; that the light added at any unsaturated pixel of an animated scene
falls no faster than the decay; that a replayed still matches a live run; that
the colour field is never drawn over; and that layers `off` render exactly as
before the layers existed.

The design keeps the layers calm well within the luminance bound. Each burst
is a single smooth, partly transparent rise and fall (glow at most 30% and
sparks at most 50% opacity), with its colour and peak fixed when it starts,
new bursts start at most once every 0.5 s, and at most four are alive at
once.
Unit tests rasterise the ribbon for a loud 4 kHz tone and white noise gated
at 3, 5 and 10 Hz, and for loud tones every 100 Hz from 0.5 to 8 kHz (plus
those that divide the point spacing) at 44.1, 48 and 96 kHz, steady and
gated. They check that it never draws more area than a steady full-scale
440 Hz ribbon, nor more than 13% of the screen. Further tests pass a sparse
pulse train, mixed snapshots (a read racing a publish), the densest
synthesised ribbon, portrait windows (9:16 and 1:10) and a thousand seeded
random outlines through the final limit, and check each stays within 13% of
the screen. Overlay timing is kept in f64 seconds, so burst timing and the
0.5 s cap hold however long the renderer runs (a test checks after 30 days).
These are models, not photometric measurements: measure the actual display
before relying on this for a photosensitive audience.

```sh
cargo run --manifest-path renderers/modern/Cargo.toml \
  --features unmeasured-flash --bin chipviz-m1 -- --mic --mic-layers both
```

## Scene plan

Scenes live in `src/scenes/`, one module per scene, behind the `REGISTRY`
table in `src/scenes/mod.rs`. Scene byte 7 is checked first and always selects
the colour field. To preserve all existing wire behavior, bytes other than 7
keep the original modulo-six mapping. For the live app, Starfield is selected
with `--scene 6` and both catalog and location options. Capture can also use
`--starfield` with those options to enable Starfield only on recorded scene-6
frames. Bare or wrapped wire bytes and catalog options alone do not select it.

| `scene` byte | Scene | Camera |
| --- | --- | --- |
| 7 | `ColourField` | `ScreenSpace` |
| 0, 6, 12, ... (unless explicit scene-6 opt-in) | `SignalField` | `Orbit3D` |
| 1, 7, 13, 19, ... (except byte 7) | `SpectrumVolume` | `Orbit3D` |
| 2, 8, 14, ... | `PulseTunnel` | `Orbit3D` |
| 3, 9, 15, ... | `Lines` | `ScreenSpace` |
| 4, 10, 16, ... | `Static` | `ScreenSpace` |
| 5, 11, 17, ... | `Object` | `ScreenSpace` |
| Live `--scene 6` with catalog/location, or capture `--starfield` | `Starfield` | `ScreenSpace` |

- `ColourField` (scene 7, `src/scenes/colour_field/`): two or three large
  regions of flat colour, meant to be projected onto performers in the dark.
  It follows [`shared/specs/colour-field-v0.md`](../../shared/specs/colour-field-v0.md),
  which is shared with the console ports. A pure, deterministic step function
  (`state.rs`) runs once per new control frame, at most 60 times per
  second. Its fixtures F0 to F22 and F24 are unit tests (F23 is host-only).
  Rendering only draws the step's trace. In scene 7, `palette & 3` picks the
  mode (0 held, 1 spectral levels, 2 hard cuts, 3 flash/gate) and
  `palette & 4` the family (red/yellow/blue or RGB/CMY). Beats, bars, drops and
  note onsets change the layout (V2, H2, V3, H3), the colour combination and
  the rotation, within the spec's limit of 3 flashes per second. Wire palette
  and scene values must hold for 30 steps before they apply, and changes share
  a budget of one per 20 steps. Without `--udp` the palette is local 0 (held).
  Scene 7 arrives over UDP (procedural and mic frames use scenes 0 to 2). The
  field steps once per new UDP, mic or procedural frame. A held UDP frame
  repeats the last step and its cleared one-off flags never count as edges, so
  when mic frames take over, the debounce returns to the native scene after
  about 30 steps.
  Flash/gate runs only when armed, and default builds never latch it (see
  [Flash/gate is off by default](#flashgate-is-off-by-default)).
  With the `unmeasured-flash` feature, a stable absolute wire 3 or 7 (held 30
  steps with incoming scene 7 throughout) arms it, since hosts send those
  only when started with `--allow-flash`. Relative palette inputs, procedural
  and mic frames never arm, and unarmed 3 and 7 run as hard cuts (2 and 6).
- `Starfield` (scene 6, `src/scenes/starfield.rs`): reads a user-supplied CSV
  with HYG-compatible `ra` hours, `dec` degrees, and `mag` columns (optional
  `ci` and quoted `proper` name). Select it with `--scene 6` plus both catalog
  and location options, or with `--starfield` plus those options to preserve
  recorded scene transitions. No catalog is distributed while the data
  licensing research is unresolved. The coordinates are treated as HYG-style
  J2000; a UTC timestamp and east-positive longitude feed an approximate GMST
  rotation and local horizon conversion. Visible stars are mapped to a
  rectangular azimuth/altitude panorama (north wraps at both horizontal
  edges; east, south and west map across the middle; horizon at the bottom).
  IAU 1976 precession transforms J2000 coordinates to date for supported UTC
  dates from 1800 through 2200. Proper motion, parallax and atmospheric
  refraction are not modeled, so this is a useful sky view rather than a
  precision planetarium. Magnitude controls compressed, monotonically ordered
  brightness and point size; colour index controls approximate hue. The
  existing square-root linear-light magnitude scalar `B` preserves the
  point-size response; emitted luminance is `L = 0.02 * tanh(B / 0.02)`.
  A neutral magnitude-6.5 star stays near 0.00514 linear luminance (about 16
  sRGB codes), while very bright white stars top out near 0.01974 (about 38
  codes). Each approximate black-body tint is scaled by `L / Y`, where `Y` is
  its Rec.709 linear luminance, keeping different hues in the same magnitude
  order. Over the complete 1000–40000 K range, `Y` is at least 0.403096 at the
  warm endpoint (the cool endpoint is about 0.719); thus the worst channel is
  below `0.01974 / 0.403 < 0.049`, leaving margin under the 0.05 linear
  per-channel cap even with rounding. The tint values are linear RGB, not
  sRGB-encoded inputs. This is deliberate tone shaping, not an sRGB correction;
  the renderer's output encoding is unchanged. Magnitude ordering holds for
  the pre-quantization luminance, but output encoding and 8-bit quantization
  can tie or slightly invert displayed luminance across hues for close
  magnitudes, particularly in the compressed bright range.
  Point footprints retain the
  existing approximately 2.8–3.5 output-pixel curve over the rendered
  magnitude range. At most
  200 stars are rendered from a spatial index that retains every in-limit
  star, so brighter below-horizon records cannot hide a fainter visible one.
  At most 131,072 CSV rows, 64 MiB total input, and 1 MiB per CSV record are
  accepted; larger inputs fail with an actionable error instead of silently
  dropping stars. Projection and ordering are cached for each effective
  ten-second sky snapshot, observer, viewport and profile budget. Catalog
  scanning remains bounded by the row limit, with wholly below-horizon bins
  skipped; benchmark the largest catalogs on target hardware.
  Each star's linear-RGB channel is capped at
  0.05; stars are opaque and drawn in ascending brightness (faintest first,
  brightest last), so a
  coincident faint star cannot overwrite a brighter one. Changing membership
  or overlap changes a Starfield pixel by at most 0.05 linear luminance.
  Transitions into or out of Starfield are separately gated to at most two
  crossings per rolling second, using monotonic timestamps after the window
  surface's `present()` call returns. Acquisition, rendering and present-call
  stalls therefore count against the gate; wgpu does not expose physical
  scanout timestamps, so this is not a physical presentation-time guarantee.
  While the gate is full, it continues showing the last committed scene and
  retries the latest request; native-to-native changes are immediate.
  Mic-layer overlays are suppressed while Starfield is displayed, then resume
  on other eligible scenes. The gate survives resize and capture replay uses
  deterministic frame time.
  This bounds Starfield crossings, not native-to-native or ColourField-internal
  flashing, which remain separate safety work (#119/#124); it is not
  independent photosensitivity certification.
  Live wall-clock corrections are synchronized at the next ten-second
  snapshot; the full star snapshot (positions, membership, ordering and
  brightness) is held between updates.
  The renderer writes sRGB-encoded star colors explicitly on non-sRGB targets
  so faint linear-light stars survive attachment quantization. Stars fainter
  than magnitude 6.5 are excluded. Below-horizon stars are culled; the
  visible candidates are selected by brightness with a screen-quarter coverage
  reserve, and output respects each profile's instance budget. Seam-crossing
  stars use two instance slots atomically; with only one slot remaining, that
  star is skipped rather than drawn at just one edge.
- `SignalField`: thousands of thin high-contrast data planes in a rotating 3D
  field.
- `SpectrumVolume`: layered spectrum slices forming an audio volume.
- `PulseTunnel`: beat-driven planes and bars moving through depth by default.
  `--tunnel-variant hallway` instead draws receding rectangular line frames
  and corner rails; `abstract` draws spiralling radial dashes. These are
  PulseTunnel looks, not the separate Lines scene. Both use a screen-space
  projection, slowly moving depth and time-smoothed energy, bass and treble
  (hallway uses energy and bass); palette still chooses the line colour.
  The envelope starts at zero and follows the newest displayed audio once per
  redraw, including a held UDP packet, with a 0.8 s response. A redraw after
  a stall advances it by at most 1/30 s and discards the excess, avoiding a
  large one-frame audio jump; equal or backward timestamps do not advance it.
  Capture replay advances it once per replayed frame on the same clock.
  Selection does not consume any wire, palette or layout bits. It applies
  whenever scene 2, 8, 14, ... is selected; live procedural input cycles
  scenes 0 to 2, or a host can send scene 2. Capture can force `--scene 2`.
  The new looks use dim thin lines, faded depth wraps and no beat-triggered
  full-screen flashes. Palette changes and upstream control changes still
  matter; this is not physical-display or photosensitivity certification.
  Frame-based safety/timing dependencies #119 and #124 remain open.
- `Lines`: lines of varied direction, speed, length and thickness travel
  across a mirrored plane; each spectrum band drives its own group and energy
  sets density (up to an eighth of the instance budget per half, fading lines
  at the cutoff). The plane alternates with a hallway whose two walls mirror
  each other. The scene keeps its section between frames: a new section
  starts when `beat_count` crosses a multiple of 16 (a four-bar phrase), and
  the fold eases in over one bar at the frame's `bpm` (60 to 240 control
  frames), counted in observed frames so it never moves backwards; every
  fourth section cuts straight to the hallway. A backward or large jump in
  `beat_count` (a restart, loop or source handover) resyncs without a section
  change, and if the counter stalls for 4 s sections free-run every 512
  frames. `beat_phase` is not used, because procedural input and
  `live_bridge` wrap it before the count ticks. Section changes are at least
  one bar apart. The timings in seconds assume 60 control frames per second.
  The drop flag is ignored unless the renderer runs with `--lines-drop-cuts`;
  then a rising drop flag cuts straight to the next section (a refold of the
  same lines, not a screen flash). A drop (or phrase boundary) that arrives
  within one bar of the last section change is ignored, not delayed. The camera
  swings around the plane and eases into the hallway entrance. The scene projects its own camera on the CPU, so
  it uses `ScreenSpace`. Procedural input never selects it; use `--scene 3`
  (procedural frames 250, 316 and 400 show plane, mid-fold and hallway).
- `Static`: a grid of noise cells, each a random glyph, solid block or
  bitmap, or blank. Energy sets both the change rate (each cell changes every
  1 to 12 frames, faster when loud) and the chance of "nothing" (blank cells,
  80% when quiet down to 20% when loud); each beat reshuffles the field.
  Palettes 0 to 2 tint the field, palette 3 uses random hues, and every fifth
  section swaps to a white background with inverted colours. Each section
  resolves into content and dissolves back over 512 control frames: an embedded
  multilingual phrase, a constant (π, e, φ or √2 to 15 places), or a rotating
  3D torus knot traced in the digits of its own coordinates. Cells cycle
  through random glyphs for 40 frames before landing (frames 160 to 256 of a
  section), hold, then cycle away from frame 400; every fourth section is
  plain noise. Odd groups of four sections use pixel noise instead of glyphs:
  the fragment shader gives each screen pixel its own value. Within a section
  the camera stays flat, slowly zooms out and tilts, or orbits once around the
  plane, showing mirrored text from behind; it is projected on the CPU, so the
  scene uses `ScreenSpace`. Sections follow the beat: the scene keeps its
  schedule between frames, and once a section's 512 frames have run the next
  starts on the first bar line (`beat_count` crossing a multiple of 4), so
  sections stay at least 512 frames apart (every 600 at 120 bpm live; at
  slow tempos, about 30 bpm, a section can run up to about 480 extra frames
  while it waits for a bar). Every
  bar line at least 60 frames after the last one reframes the camera, easing
  over one beat (15 to 30 frames) to the next of six framings (head on every
  other bar, either side, closer, from above, pulled back). In a text
  section, such a bar during the hold flips the phrase to another language
  like a split-flap board: cells switch at staggered frames over 24 frames,
  cycle for 12, then land. Backward or large `beat_count` jumps are not bars;
  if the counter stalls for 4 s, sections free-run every 512 frames. Before
  any frame is observed the schedule is the frame counter's (`frame / 512`,
  head on), and a state joins it on its first frame, so with procedural
  input sections start on the same frames as before. Flash safety: beats
  change only geometry and characters, never the background or brightness,
  and each change eases or staggers over at least 15 frames; a CPU luminance
  test over all eight section types, up to racing beats, finds at most one
  flash in any second at 60 fps. The noise follows energy through a slew
  limit: it moves at most 1/48 of the range per 1/60 s of the renderer's
  clock (a gap counts at most 8/60 s, time is credited from the latest
  seen, packets queued for one redraw share one step toward the newest,
  and frame-counter jumps add nothing), so silence to full energy
  takes 0.8 s at any frame rate, and 60 fps output is as with a per-frame
  limit. Without it, energy strobed the noise density (4 flashes in 1 s in
  a screen tile with procedural input, up to 30 with a square wave; a
  per-frame limit still allowed 4 at 120 fps). With it, CPU tests at 60,
  120 and 240 fps find the energy path adds at most 2 flashes in 1 s with
  procedural input and for square waves into full energy (from silence or
  energy 180) at periods from 2/60 to 1 s, under WCAG 2.3.1's three. That
  bound covers the energy path only: section, bar, flip and dissolve
  timing still counts frames, so it runs faster at high redraw rates (3
  flashes in 1 s overall measured at 480 fps, at the limit; #119).
  Recorded golden fingerprints cover both the frame-timed path (frames
  built before any is observed) and the beat-synced replay from frame 0 that the renderer and captures use (the
  replay's were re-recorded for the slew limit).
  Procedural input never selects it; use
  `--scene 4` (frames 100, 200 and 300 show glyph noise, a mid-transition and
  resolved text; 3604 shows pixel noise, 4908 a constant on white, 3422 the
  knot and 4366 the orbit; captures replay from frame 0, so these are the
  beat-synced frames, reframed on bars).
  Limitations: the text is a small embedded phrase list written for chipviz,
  with no live headlines yet (a `TextSource` trait is the seam for them); the
  glyphs are a hand-drawn 5x8 uppercase font covering Latin with accents
  (one-row accents, no cedilla), Greek, Cyrillic, digits and some symbols, and
  characters it lacks (CJK, emoji) render as glyph-like noise. One quad draws
  each cell, so glyph density is limited by the instance budget (about 208x78
  cells on `m1`, 104x39 on `pi5`). The glyph grid density does not change
  with the beat.
- `Object`: an object in space, presented like a 1980s/90s infomercial or
  camcorder tape. Eleven original procedural models (a CRT television, a VHS
  tape, a CD with a rainbow sheen, a sunglasses-wearing sun, a tree, a beige
  desktop computer, a red phone box with a lit sign, a tricycle with turning
  wheels and streamers, a gable-top milk carton, a neon jukebox whose grille
  follows the spectrum, and a tin of beans with loose beans that hop on the
  beat) are built in code from boxes, cylinders and spheres
  sampled into lit, back-face-culled splats; no downloaded assets are used.
  Every 512 control frames the object spins in, swings back and forth on a
  turntable around a three-quarter front view (the CD and sun rock instead),
  and spins out. Sections cycle the object (11, the newer five appended
  after the first six),
  the background (5: procedural starfield, lava goop, a vaporwave sunset over
  a grid floor, a rainbow sweep, and a chroma-key colour field), the palette
  morph (3: a hue swing, a rainbow sweep, or a hue jump every beat; glowing
  screens and the sun keep their colours) and the overlay (2: camcorder
  scanlines, rolling tracking noise, tearing, "REC" and a date stamp, or an
  infomercial caption banner, taglines and a spinning "NEW!" burst), so all
  55 object/background pairs appear within 55 sections. Beats punch the zoom
  and drive the screens and spectrum; the drop flag, every eighth beat and
  each reveal fire a lens flare and sparkles. Flash safety: beat effects only
  touch the object, the flare and small sparkles; the chroma-key field cuts
  between hues of equal luminance at most every 32 frames, so there are no
  full-screen brightness flips. The mesh step coarsens to fit the instance budget
  (at most about 15,000 object splats on `m1` and 3,500 on `pi5`), and the object is projected on
  the CPU, so the scene uses `ScreenSpace`. Text reuses Static's 5x8 font.
  Procedural input never selects it; use `--scene 5` (frames 200, 712, 1224,
  1736, 2248, 2760, 3272, 3784, 4296, 4808 and 5320 show each object; 48
  past a section start is the reveal flare).
  Limitations: sections are frame-timed, not beat-synced; there is no depth
  buffer, so splats are sorted far to near on the CPU.

To add a scene, copy one scene module, give it a unique `ENTRY` name and camera
mode, and append that entry to `REGISTRY`. Do not change the legacy modulo-six
mapping for control-frame bytes: Starfield is selected only by explicit local
opt-in to scene 6, and byte 7 remains the ColourField.
`Orbit3D` scenes use world coordinates and the smoothed orbit camera.
`ScreenSpace` scenes draw flat: `center.xy` is NDC (`-1..1` spans the viewport,
`+y` up), depth is ignored, and instances draw in order with their colour
unchanged (no quad-edge darkening or beat/energy pulse). Builders receive the
viewport aspect (width / height) for laying out screen-space fields.
A `ScreenSpace` instance's `pattern` is `[0, 0]` for a solid quad; positive
values carry a 5x8 glyph bitmap (bits 0 to 19 and 20 to 39) whose unlit pixels
are discarded, a negative `pattern.x` asks for per-pixel noise (see
`src/scenes/static_noise/mod.rs`), and `[0, -(1 + shape)]` draws one of
Object's shapes: 0 an antialiased disc, 1 a soft glow, 2 a four-pointed
sparkle, 3 scanlines (see `src/scenes/object/mod.rs`).

The Pi 5 profile favors crisp geometry and predictable frame pacing. The M1
profile favors density, deeper Z spacing, and more aggressive camera motion.

## Flash/gate is off by default

Colour-field modes 3 and 7 (flash/gate) flash the whole field at up to 7.5
flashes per second, above the WCAG 2.3.1 and Harding limit of 3. Issue #68
requires an independent photosensitivity analyser (PEAT, or a Harding-style
general and red flash check) to measure them before any build ships them.
Until then they sit behind the `unmeasured-flash` Cargo feature, which is off
by default and must not be enabled for releases:

- **Default builds** (including release packages) never latch flash, whatever
  arms it: wire selection, operator requests or a local pre-arm. Palettes 3
  and 7 run as the spec's unarmed requests, hard cuts 2 and 6, and
  `--allow-flash` only prints a notice.
- **`--features unmeasured-flash`** runs flash/gate as the spec describes, for
  measurement and development only.

The same feature gates the mic layers, whose red flashes are not yet bounded
(see [Mic layers](#mic-layers)): default builds never draw them.

`just rust-lint` and `just rust-test` run clippy and the tests for both
builds. The default build's tests check that every armed fixture equals hard
cuts, and count WCAG 2.3.1 general and red flashes in its rendered sequences
(at most 3 per second). That count is supporting evidence, not a substitute
for the analyser.

## Capture

`chipviz-capture` renders frames through the same pipeline and shader into an
offscreen texture, then writes PNGs plus `manifest.json` (profile, input, size, resolved Starfield epoch/clock mode,
and per-frame file, index, elapsed time, resolved Starfield epoch, requested
scene byte and debounced `displayed_scene`).
It opens no window and
needs no screen-recording permission, only a GPU adapter.

```sh
just modern-capture --profile m1 --scene 1 --frames 0,120,240 --size 1920x1080 --out build/shots
just modern-capture --input host/fixtures/musical/groove.cvz --frames 0,30 --out build/groove
```

By default capture overwrites existing outputs. The CLI's `--exclusive-output`
instead atomically creates each PNG and manifest, refusing existing files or
symlinks (including dangling symlinks) and duplicate frames. It does not secure
an untrusted or mutable parent directory. On failure, newly created partial
files may remain; existing outputs are never deleted. To use a literal path
named `--exclusive-output`, prefix it with `./`.

Frame `N` renders at `elapsed = N / 60` s, with camera smoothing replayed from
frame 0, so very large frame indices take longer to capture.
Starfield captures without `--time` advance from the resolved current UTC epoch
in ten-second snapshot steps. That epoch and advancing mode are recorded in the
manifest. To reproduce a moving-sky set, pass the recorded
`starfield_epoch_unix_seconds` as `--time` together with `--follow-time`.
An explicit `--time` remains fixed unless `--follow-time` is supplied.
The capture CLI preserves legacy mapping by default: bare `--scene 6` remains
`SignalField`, and raw/wrapped scene bytes keep their existing mappings. To
force Starfield in every captured frame, use the paired form
`--scene 6 --catalog PATH --location LAT,LON`; this preserves the original
personal-bundle invocation. Catalog/location options without either `--scene
6` or `--starfield` do not enable Starfield. Alternatively, `--starfield --catalog PATH
--location LAT,LON` opts in to Starfield for recorded scene-6 frames without
forcing the scene, and is useful for replaying recordings with scene
transitions. `--starfield` requires both catalog and location before output or
GPU setup.
The stable programmatic `capture::run` and `capture::run_with_options` APIs
also keep legacy scene-byte mapping (byte 6 is `SignalField`); opt into
Starfield through `capture::run_with_starfield_options`. That explicit API
replays recorded scene-6 frames with Starfield even when `CaptureArgs.scene` is
`None`.
A `.cvz` input does not wrap: indices past its last frame are an error.
`--scene` overrides each frame's scene byte; without it the input's
scene byte is used. `--scene 7` captures the colour field, which is replayed
from frame 0 like the camera. `--lines-drop-cuts` turns on Lines drop cuts
as in the renderer, replayed from frame 0; only a `.cvz` input carries drop
flags. Procedural input never arms flash/gate; with
the `unmeasured-flash` feature, a recording arms it only as a target would,
from a stable absolute wire 3 or 7 held in scene 7, and default builds never
flash. Recorded inputs use
their wire palettes; procedural input uses palette 0. `--input` is `procedural` (default) or a `.cvz` file of
concatenated control-frame packets, such as the bridge's `--output`, or
`synth-mic`, a deterministic synthetic microphone (110 Hz and 330 Hz tones with
a click every 0.5 s, silent from 6 s). It uses the same analyser and scope tap
as `--mic`, but is fed one capture frame of samples at a time in the capture
thread, with no device, analysis thread, dropped audio, or wall-clock timing,
so its results can differ slightly from a live microphone.
`--mic-layers` draws the [mic layers](#mic-layers) as in the renderer.
`--size` defaults to the profile's target size. Run `just modern-capture --help`
for every option. The GPU smoke test is ignored by default; run it with
`cargo test --manifest-path renderers/modern/Cargo.toml --lib -- --ignored`.

Library callers can keep constructing `capture::CaptureArgs` and calling
`capture::run` for the original tunnel. To select a look programmatically,
pass `capture::CaptureOptions { tunnel_variant: capture::TunnelVariant::Hallway }`
to `capture::run_with_options`. For command-line parsing in a library caller,
`capture::parse_args_with_options` returns both arguments and options;
the original `capture::parse_args` rejects the new flag rather than losing it.

### PulseTunnel evaluation

After the serialized build slot is available, a catalog-free GPU check is:

```sh
cargo test --manifest-path renderers/modern/Cargo.toml --lib tunnel_variants_render_synthetic_input_at_representative_aspects -- --ignored
cargo test --manifest-path renderers/modern/Cargo.toml --lib tunnel_variants_bound_gpu_luminance_changes -- --ignored --nocapture
```

The second check samples 120 frames with energy and palette alternating on
every frame, at 320x180 and 180x320 for both profile budgets. It reports
whole-frame luminance changes; this is supporting evidence, not a WCAG
viewing-area test or real-display assessment.

For reproducible stills, run the following in this worktree. It covers both
profile budgets, landscape and portrait, all three looks and two synthetic
inputs. The capture manifest names the selected look and camera; inspect the
images before selecting safe screenshots for a PR.

```sh
for profile in m1 pi5; do
  for variant in original hallway abstract; do
    for input in procedural synth-mic; do
      for size in 960x540 540x960; do
        just modern-capture --profile "$profile" --scene 2 --tunnel-variant "$variant" \
          --input "$input" --frames 60,120,240 --size "$size" \
          --out "build/tunnel-evidence/$profile/$variant/$input/$size"
      done
    done
  done
done
```

For #123, make local playable videos of all three looks under both profile
budgets with at least two meaningful synthetic input patterns; record the
input, dimensions, profile (not native hardware), settings and limitations in
the local index, validate with `ffprobe`, and visually spot-check them. Do not
upload the videos. Console adaptation in #138 should preserve all three
selectable identities, the receding/moving geometry and audio response using
each target's own controls and budgets; a static picture is not a finished
port. Timing and flash behaviour need target-specific validation.
