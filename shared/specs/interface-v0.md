# chipviz interface v0

Every target exposes the same high-level interface model, even though each
platform renders it with native UI conventions.

## Screens

| Screen | Purpose |
| --- | --- |
| `title` | Boot/title screen and start prompt. |
| `scene_select` | Manual scene selection. |
| `visualizer` | Active visualizer scene. |

## Modes

| Mode | Purpose |
| --- | --- |
| `manual` | Human-controlled scene changes and transport-specific palette controls. |
| `auto` | Host/chipsynth/Pi can advance scenes using a control combination that is deliberately unlikely for a human controller. |
| `demo` | Target generates visualizer-like input locally for store/demo/screensaver use. |

## Inputs

| Input | Meaning |
| --- | --- |
| `start` | Title -> scene select -> visualizer; returns visualizer to scene select. |
| `next_scene` / `prev_scene` | Scene selection pulses. |
| `auto` | Enter auto mode. |
| `demo` | Enter demo mode. |
| `manual` | Exit auto/demo mode and return to human control. |

The concrete button/key combinations live in each transport mapping. USB HID
uses the manual/auto/demo controls in
[`usb-hid-transport-v0.md`](usb-hid-transport-v0.md).
