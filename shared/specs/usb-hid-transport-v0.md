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
every selection. A source change across the scene-7 boundary must hold the
same decoded scene for 30 consecutive **input records** before it becomes the
destination. Native-to-native changes are immediate. A new source scene
restarts this count, even while a ring move is pending. Synthetic tail states
do not count as input: a recording ending with an unconfirmed change does not
cause it. Once a crossing has been accepted, the target's 30-step debounce
still applies, so a later reversal can arrive after that crossing is visible.
Other buttons and axes follow each input record. After the last record, the
converter appends states until the last *accepted* destination and its 60-poll
interval are complete. Padding clears beat, bar and drop onsets, but retains
fill, manual and shoulder levels and the last axes. West remains held if fill
is held, even if the last record also had drop. If any buttons remain held,
one final state releases them (axes stay at their last values). A streaming
mapper caller must mark synthetic states with `padding=True` and arrange its
own final release.
Playback must deliver every JSON state in order, exactly one per receiver
step, at no more than 60 polls/s; neither the JSON nor this converter provides
timing or enforces the rate. Transient input selections may never be reached,
and reaching a destination can take multiple seconds.

The `--initial-*` values are the actual receiver ring values, not the desired
input values. Before starting or explicitly resynchronizing, independently
establish those values, release all D-pad directions, and let the receiver
settle for at least 60 steps. Also establish that flash is not already armed;
this gamepad-only data path cannot verify an unknown latch from an earlier
absolute or operator input on a mixed-input receiver. The stateful Python
mapper's `disconnect()`
invalidates both estimates and refuses further conversion until `resync()`
is called with newly established values. A JSON file cannot detect a
disconnect or perform this resynchronization itself. Estimates assume every
state is delivered and no other input changes the rings; loss, reset, replay
from the wrong point, or external button presses invalidate that assumption.
There is no receiver feedback and no alignment guarantee.

Without `--allow-flash`, the usual HID scene-7 guard maps palette 3/7 to 2/6.
Every absolute input is sanitized before conversion, and the guard also
applies to the accepted scene while a source exit is still debouncing. The
mapper requires a safe initial scene-7 palette. When moving a palette, it
emits only relative D-pad pulses, never absolute scene or palette keys. Its
ring may pass through 3/7 while scene 7 stays shown; this is not an absolute
3/7 report. A conforming `data` receiver treats those positions as unarmed
requests, effective as 2/6, and cannot arm flash through them (F24). The
receiver's stability gate and change budget apply; intermediate safe mode or
family changes may be visible before the destination is reached. The output
must never be fed to an `operator` build or a receiver without these safety
rules. `--allow-flash` disables the host guard and prints the warning, but
does not make relative inputs capable of arming a conforming `data` receiver.
The GBA receiver
implementation and actual USB/Pocket Dock operation require separate
validation; hardware validation is tracked in #31.
