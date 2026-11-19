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

Scenes live in `src/scenes/`, one module per scene, behind the `REGISTRY`
table in `src/scenes/mod.rs`. The control frame's `scene` byte selects
`REGISTRY[scene % REGISTRY.len()]`:

| `scene` byte | Scene | Camera |
| --- | --- | --- |
| 0, 3, 6, ... | `SignalField` | `Orbit3D` |
| 1, 4, 7, ... | `SpectrumVolume` | `Orbit3D` |
| 2, 5, 8, ... | `PulseTunnel` | `Orbit3D` |

- `SignalField`: thousands of thin high-contrast data planes in a rotating 3D
  field.
- `SpectrumVolume`: layered spectrum slices forming an audio volume.
- `PulseTunnel`: beat-driven planes and bars moving through depth.

To add a scene, copy one scene module, give it a unique `ENTRY` name and camera
mode, and append that entry to `REGISTRY`. Appending keeps every byte below the
old registry length on the same scene; wrapped bytes follow the new length.
`Orbit3D` scenes use world coordinates and the smoothed orbit camera.
`ScreenSpace` scenes draw flat: `center.xy` is NDC (`-1..1` spans the viewport,
`+y` up), depth is ignored, and instances draw in order with their colour
unchanged (no quad-edge darkening or beat/energy pulse). Builders receive the
viewport aspect (width / height) for laying out screen-space fields.

The Pi 5 profile favors crisp geometry and predictable frame pacing. The M1
profile favors density, deeper Z spacing, and more aggressive camera motion.

## Capture

`chipviz-capture` renders frames through the same pipeline and shader into an
offscreen texture, then writes PNGs plus `manifest.json` (profile, input, size,
and per-frame file, index, elapsed time and scene). It opens no window and
needs no screen-recording permission, only a GPU adapter.

```sh
just modern-capture --profile m1 --scene 1 --frames 0,120,240 --size 1920x1080 --out build/shots
just modern-capture --input host/fixtures/musical/groove.cvz --frames 0,30 --out build/groove
```

Frame `N` renders at `elapsed = N / 60` s, with camera smoothing replayed from
frame 0, so very large frame indices take longer to capture.
A `.cvz` input does not wrap: indices past its last frame are an error.
`--scene` overrides each frame's scene byte; without it the input's
scene byte is used. `--input` is `procedural` (default) or a `.cvz` file of
concatenated control-frame packets, such as the bridge's `--output`.
`--size` defaults to the profile's target size. Run `just modern-capture --help`
for every option. The GPU smoke test is ignored by default; run it with
`cargo test --manifest-path renderers/modern/Cargo.toml --lib -- --ignored`.
