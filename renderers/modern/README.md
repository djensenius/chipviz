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

Without `--udp` or `--mic`, the renderer uses deterministic procedural frames. With UDP,
it consumes `control-frame-v0` packets from the host bridge:

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

```sh
just modern-m1 -- --list-monitors
just modern-m1 -- --fullscreen --monitor "Studio Display"
just modern-m1 -- --fullscreen --monitor 1
```

| Key | Action |
| --- | --- |
| `F`, `Cmd+F`, `Ctrl+Cmd+F` | Toggle fullscreen on the window's current display. |
| `Esc` | Exit fullscreen (does nothing when windowed). |
| `Cmd+Q` | Quit (macOS application menu). |

The cursor is hidden while fullscreen and restored when windowed.

## Microphone input

The renderer can analyse live microphone audio itself, with no bridge
needed. Capture is off by default.

| Option | Effect |
| --- | --- |
| `--mic` | Drive frames from the microphone (the system default input unless `--audio-device` is given). |
| `--audio-device INDEX\|NAME` | Choose the input by the index from `--list-audio-devices` (all-digit values are indices) or by name, matched like `--monitor`: a unique exact name, else a unique case-insensitive substring; ambiguous names are rejected. Requires `--mic`. |
| `--list-audio-devices` | Print each input device's index and name (marking the default), then exit. |

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

Analysis runs on its own thread; the render thread reads the latest result
without waiting. Levels use automatic gain with a noise floor, so quiet and
loud rooms both move the scenes, room hiss stays dark, and silence (or a
device that stops sending audio) fades smoothly to zero.

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

On Linux, building the renderer needs the ALSA headers (`libasound2-dev` on
Debian and Raspberry Pi OS).

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
