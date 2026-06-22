# chipviz-n64

N64 visualizer core for original Nintendo 64 hardware and Analogue 3D.

## Planned stack

- libdragon for toolchain, controllers, video, filesystem, and debugging.
- tiny3d for approachable hardware-accelerated geometry.
- Flashcart deployment, with SummerCart64 as the preferred development path if USB I/O is needed.

## First demo

- Self-running procedural scene.
- Controller input for scene, palette, and camera changes.
- Palette cycling, particles, 3D depth, and simple beat simulation.
- Later: controller-port data transport from an ESP32 or host bridge.

## Current scaffold

`src/main.c` consumes the shared chipviz connection layer and maps each frame to
a minimal N64-style visualization model: beat phase rotates the scene, energy
sets particle density, and bass drives depth pulse. The native simulator build
prints those values until the libdragon/tiny3d renderer replaces the stub.

The first real renderer should be unmistakably 3D. A good direction is
graphic/2D-looking planes and sprites arranged in 3D space, with camera angle,
orbit, and tilt changes that reveal the objects live on a 3D plane.

## Connection path

Use a Raspberry Pi plus Pi Pico wired controller-port bridge. The Pi sends the
16-byte N64 Joybus transport over USB serial, and the Pico exposes the current
frame as emulated N64 controller state across one to four controller ports. See
[`../../docs/n64-hardware.md`](../../docs/n64-hardware.md) and
[`../../shared/specs/n64-joybus-transport-v0.md`](../../shared/specs/n64-joybus-transport-v0.md).

## Constraints

Analogue 3D HDMI output can scale to 4K, but this core renders as N64 software. Design scenes around bold shapes, color, motion, and scanline/CRT-friendly composition rather than native 4K detail.
