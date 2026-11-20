# USB HID transport v0

This transport maps a reduced `control-frame-v0` into ordinary USB HID
keyboard/gamepad state for devices such as Raspberry Pi USB gadget mode,
Analogue Pocket Dock, C64 Ultimate, SNES-on-Pocket-Dock cores, and other
controller-oriented retro loading paths.

## Reduced fields

The HID path is low bandwidth and stateful. It carries:

- beat/bar/fill/drop triggers
- scene and palette low bits
- coarse energy buckets
- manual override flag
- optional spectrum strobe bits

## Gamepad mapping

| HID control | chipviz meaning |
| --- | --- |
| D-pad up/down | scene increment/decrement pulse |
| D-pad left/right | palette decrement/increment pulse |
| South button | beat |
| East button | bar |
| West button | fill/drop |
| North button | manual override |
| Left shoulder | high energy bucket |
| Right shoulder | treble/spectrum strobe |
| Select + Start + both shoulders | enter auto mode |
| Select + South + East + West | enter demo mode |
| Select + North | manual override / exit auto or demo mode |
| Left stick X | signed energy bucket |
| Left stick Y | signed beat phase bucket |

## Keyboard mapping

For C64 Ultimate and other keyboard-style input paths, use ASCII-ish keys:

| Key | chipviz meaning |
| --- | --- |
| `1`..`8` | scene low bits |
| `Q`..`I` | palette low bits |
| Space | beat |
| Return | bar |
| `F` | fill/drop |
| `M` | manual override |

Receivers should treat HID state as latest-state and use edge detection for
button/key pulses.

## GBA gamepad-only JSON conversion

`host/bridge/usb_hid.py` defaults to the existing keyboard mapping and emits
one JSON state per input record. `--gba-gamepad --initial-scene N
--initial-palette N` instead emits gamepad-only states: `buttons`, `left_x`,
and `left_y` retain their existing meanings, `keys` is empty, and `dpad` is
an array containing at most one of `up`, `down`, `left`, `right` (empty means
release). These are abstract JSON states, not a HID report descriptor or USB
gadget driver.

The GBA ring is `0, 1, 2, 3, 7`; the palette ring is `0..7`. The low three
scene bits select 7 directly, otherwise they select a native scene modulo 4
(thus 4, 5, 6 alias 0, 1, 2). Palette uses its low three bits. Each pulse
chooses the shortest permitted route, with increment winning a tie. Native
scene-to-scene routes avoid 7. Only one axis moves per poll. Every press is
followed by 59 release states before the next press: this holds each ring
value for 60 polls, covering the 30-step scene debounce and, after entry,
30 steps of palette stability. It can expose intermediate scenes and palettes;
these are not atomic changes.

Input records are sampled once per output poll; selection changes arriving
during a wait replace the pending destination (latest wins), not a FIFO of
every selection. Other buttons and axes follow each input record. After the
last record, the converter appends states until the last destination and its
60-poll interval are complete, repeating the last record's non-D-pad values.
If any buttons remain held, it appends one final state releasing them (axes
stay at their last values). The streaming mapper's caller must arrange its own
final release.
Playback must deliver every JSON state in order, exactly one per receiver
step, at no more than 60 polls/s; neither the JSON nor this converter provides
timing or enforces the rate. Tail repetition can hold other buttons until the
final release. Transient input selections may
never be reached, and reaching a destination can take multiple seconds.

The `--initial-*` values are the actual receiver ring values, not the desired
input values. Before starting or explicitly resynchronizing, independently
establish those values, release all D-pad directions, and let the receiver
settle for at least 60 steps. The stateful Python mapper's `disconnect()`
invalidates both estimates and refuses further conversion until `resync()`
is called with newly established values. A JSON file cannot detect a
disconnect or perform this resynchronization itself. Estimates assume every
state is delivered and no other input changes the rings; loss, reset, replay
from the wrong point, or external button presses invalidate that assumption.
There is no receiver feedback and no alignment guarantee.

Without `--allow-flash`, the usual HID scene-7 guard maps palette 3/7 to 2/6.
The mapper also refuses to pass through 3/7 while its scene ring is 7, and
requires a safe initial scene-7 palette. Since these forbidden ring positions
separate the palette ring, some changes (for example 0 to 4 while in scene
7) fail explicitly rather than violating the guard. Move out of scene 7,
select the desired palette, then return, or independently resynchronize.
With `--allow-flash` the guard is disabled and the warning is printed. Even
then gamepad-only relative inputs cannot arm flash in a conforming `data`
receiver (F24); they must never be fed to an `operator` build. The target's
own stability, change budget and safety rules still apply. The GBA receiver
implementation and actual USB/Pocket Dock operation require separate
validation; hardware validation is tracked in #31.
