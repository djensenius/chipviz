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

Without `--udp`, the renderer uses deterministic procedural frames. With UDP,
it consumes `control-frame-v0` packets from the host bridge:

```sh
python3 host/bridge/chipviz_bridge.py --frames 600 --udp 127.0.0.1:6464
```

## Scene plan

- `SignalField`: thousands of thin high-contrast data planes in a rotating 3D
  field.
- `SpectrumVolume`: layered spectrum slices forming an audio volume.
- `PulseTunnel`: beat-driven planes and bars moving through depth.

The Pi 5 profile favors crisp geometry and predictable frame pacing. The M1
profile favors density, deeper Z spacing, and more aggressive camera motion.
