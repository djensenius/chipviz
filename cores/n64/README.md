# chipviz-n64

N64 visualizer core for original Nintendo 64 hardware and Analogue 3D.

## Planned stack

- libdragon for toolchain, controllers, video, filesystem, and debugging.
- tiny3d for approachable hardware-accelerated geometry.
- Flashcart deployment, with SummerCart64 as the preferred development path if USB I/O is needed.

## First demo

- Self-running procedural scene.
- Controller input for scene, palette, and camera changes.
- Palette cycling, particles, and simple beat simulation.
- Later: controller-port data transport from an ESP32 or host bridge.

## Constraints

Analogue 3D HDMI output can scale to 4K, but this core renders as N64 software. Design scenes around bold shapes, color, motion, and scanline/CRT-friendly composition rather than native 4K detail.

