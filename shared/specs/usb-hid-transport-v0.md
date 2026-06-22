# USB HID transport v0

This transport maps a reduced `control-frame-v0` into ordinary USB HID
keyboard/gamepad state for devices such as Raspberry Pi USB gadget mode,
Analogue Pocket Dock, C64 Ultimate, and SNES-on-Pocket-Dock cores.

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

For C64 Ultimate keyboard-style input, use ASCII-ish keys:

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
