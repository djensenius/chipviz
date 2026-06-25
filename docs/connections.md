# chipviz connection guide

chipviz starts with baked playback and deterministic procedural frames, then adds
live transport one platform at a time. The common path is:

```text
music source
  chipsynth, MIDI, audio analysis, recorded frames, or procedural source
      |
      v
sender / analysis device
  ESP32, Raspberry Pi, or desktop development host
  reduces source data into control-frame-v0
      |
      +--> raw playback files for ROM builds
      +--> USB serial / Wi-Fi / BLE to ESP32 bridge
      +--> direct network transport where hardware supports it
```

The sender/audio-processing device should be an ESP32 or Raspberry Pi for live
setups. A desktop computer is still useful while developing adapters and baked
playback files, but the hardware goal is a small box that can sit with the retro
hardware.

The retro platform targets are homebrew programs. Analogue 3D, Pocket Dock, C64
Ultimate, flash carts, and emulators are loading/display/input paths for those
homebrew artifacts, not replacements for native target builds.

Use an ESP32 when the job is mostly transport, GPIO timing, Bluetooth HID, or
simple MIDI/control reduction. Use a Raspberry Pi when the job needs richer
audio analysis, multiple USB MIDI/audio devices, filesystem-backed recordings,
or heavier chipsynth integration. Either device should emit the same
`control-frame-v0` packets so the platform transports stay interchangeable.

## Sender devices and chipsynth

Use USB serial first because it is easiest to debug and reliable enough for the
current 33-byte control frames.

1. Run the chipviz host bridge on an ESP32, Raspberry Pi, or desktop development
   computer.
2. The bridge reads one or more source adapters: procedural frames, a recorded
   `control-frame-v0` file, audio analysis, generic MIDI, or the chipsynth
   visualization stream.
3. For chipsynth development, keep chipviz beside the sibling `../chipsynth`
   checkout. chipsynth should publish a continuous, latest-state visualization
   stream that chipviz can connect to. That stream mirrors MIDI/control data sent
   to Daisy and adds Daisy telemetry when available: active voices, channel/chip
   assignment, note, velocity, level, parameter changes, transport, BPM, and
   performance gestures.
4. Keep audio routing separate from this stream. The chipviz Raspberry Pi can
   capture audio from line-in, Bluetooth, PipeWire/PulseAudio, or another source
   and combine that FFT/beat analysis with chipsynth's musical/control telemetry.
5. If a separate ESP32 is doing target-specific transport, send packed frames to
   it over USB serial. Use a simple framed stream: wait for magic byte `0xC7`,
   read 33 bytes, validate version and XOR checksum, then replace the current
   frame.
6. If the Raspberry Pi or ESP32 is directly connected to the target transport,
   it can skip the second bridge and emit N64 controller-port, Bluetooth HID,
   network, or serial output itself.
7. Add Wi-Fi UDP after USB serial works. UDP is a good fit for live visuals
   because fresh frames matter more than retransmitting stale frames.

The ESP32 should also have a procedural fallback mode so each target can run
without the upstream sender connected.

## N64 / Analogue 3D: wired controller-port bridge

Preferred live path: a Raspberry Pi performs upstream audio analysis or
chipsynth adaptation, then sends 16-byte frames to a Pi Pico over USB serial. The
Pico emulates wired N64 controllers and plugs into the N64 or Analogue 3D
controller ports.

1. Build and run the N64 ROM from a flashcart or Analogue 3D workflow.
2. Wire Pico GPIOs to one or more N64 controller data lines, with common ground.
   Use 3.3 V-safe signaling, do not drive 5 V into the Pico, and keep the GPIO
   behavior close to open-drain/tri-state so the console and bridge do not fight
   the data line.
3. Start with one controller port for scene/palette/intensity controls.
4. Expand to four controller ports for live frame transport. The N64 core polls
   controller state through libdragon; the Pico exposes compact slices of
   `control-frame-v0` into button/stick state across those polls.
5. Keep baked frame playback as the fallback for emulator, flashcart, and
   transport debugging.

The N64 visualizer should use real 3D. A good first direction is flat,
graphic-looking objects arranged on 3D planes: from one camera angle they read as
2D motion graphics, then camera orbit/tilt makes the depth obvious. Beat phase,
bass, and note events should drive camera moves, particles, Z-depth, and
palette-lit geometry.

## Analogue Pocket Dock / GBA: USB HID controller bridge

Preferred low-bandwidth live path: the Raspberry Pi presents directly to the
Analogue Pocket Dock as a USB HID controller, and the GBA core treats that
controller input as a control surface.

1. Plug the Pi USB gadget controller into the Analogue Pocket Dock.
2. Run the `.gba` build through the Pocket's GBA path.
3. Map buttons and D-pad to scene, palette, intensity, and mode flags.
4. Translate incoming `control-frame-v0` packets into the reduced HID mapping in
   [`../shared/specs/usb-hid-transport-v0.md`](../shared/specs/usb-hid-transport-v0.md).
5. Keep baked `.gba` frame playback as the deterministic fallback.

This path is intentionally low bandwidth. It is best for scene changes,
palette/intensity nudges, and beat/drop triggers. If richer live frame data is
needed later, add a separate ESP32-to-GBA link-cable transport.

## Commodore 64 Ultimate: USB HID first, network fallback

Preferred live path candidate: present the Raspberry Pi as a USB HID controller
to the Commodore 64 Ultimate, matching the Pocket Dock reduced-control mapping
where possible. A Raspberry Pi is a good fit for audio analysis plus USB gadget
output; an ESP32 is a good fit for compact HID, GPIO, UDP, or serial forwarding.

1. Plug the Pi USB gadget controller into the C64 Ultimate USB input path.
2. Translate incoming `control-frame-v0` packets into the reduced HID mapping in
   [`../shared/specs/usb-hid-transport-v0.md`](../shared/specs/usb-hid-transport-v0.md).
3. Reduce the frame before it reaches tight C64 code when needed: scene,
   palette, flags, energy, beat phase, and a few spectrum bands may be enough for
   raster/PETSCII effects.
4. Keep `.prg`/`.d64` baked playback as the fallback.
5. Use Ultimate network services or user-port serial only if the USB HID path is
   not viable for a live setup.

The C64 target should stay visually native: raster bars, PETSCII, character
animation, sprites, fixed palettes, and SID/VIC-II timing tricks rather than a
scaled-down copy of the N64 or GBA scenes.

## SNES / Analogue Pocket Dock: USB HID controller bridge

Preferred first path: run the SNES target through a Pocket Dock SNES core and
use the same Raspberry Pi USB HID mapping as the GBA path. Real SNES hardware
later uses a controller-port adapter and SNES controller extension cable.

1. Run the PVSnesLib-built `.sfc`/`.smc`; use the native
   simulator target.
2. Map buttons to beat/bar/fill/manual and scene/palette controls.
3. Keep baked `control-frame-v0` arrays as the deterministic fallback.
4. For real SNES, emulate the controller serial protocol instead of sending raw
   audio or high-bandwidth data.

## Game Boy / Analogue Pocket: Pocket Dock first, link cable later

Preferred first path: run the RGBDS-built `.gb` through the Pocket's Game Boy
core or a flashcart, and use the Pocket Dock controller path for low-bandwidth
scene, palette, beat, and mode controls.

1. Build `chipviz-gb.gb` with RGBDS/rgbfix; rgbfix writes the required Game Boy
   header/logo during the build.
2. Map Dock controller buttons through the reduced HID mapping when running on
   Pocket.
3. Keep baked `control-frame-v0` playback as the deterministic fallback for
   emulator, flashcart, and core testing.
4. Add a Game Boy link-cable bridge later if richer live data is needed. That
   bridge should still reduce incoming telemetry to the same scene/palette/beat
   model before it reaches tight DMG code.

The Game Boy target should stay DMG-native: four shades, tile maps, cheap
scrolling, sparse sprites, and Game Boy APU channel hints rather than a generic
framebuffer visualizer.

## Genesis / Mega Drive: YM2612 planes plus PSG accents

Preferred first path: baked playback in an emulator or flashcart/devcart, with a
later controller-port or devcart-side bridge for live ChipStation telemetry.

1. Use the generated `chipviz-genesis.md` seed for smoke tests until an SGDK or
   68000/Z80 assembly SDK path is selected.
2. Map YM2612 channels to six FM voice planes/meters and SN76489 activity to PSG
   flash accents.
3. Keep the generated ROM and host simulator as the deterministic fallback.
4. For live hardware, prefer a transport that behaves like controller/devcart
   state and never requires raw audio-rate data on the Genesis side.

The Genesis target should lean on VDP planes, CRAM palette changes, sprites, and
the distinct YM2612/SN76489 channel layout.

## NES: NROM/PPU visualizer for 2A03 telemetry

Preferred first path: the ca65-built `chipviz-nes.nes` NROM demo in an emulator,
flashcart, or NES-compatible core. Live input comes later through controller-port
or devcart-style reduced controls.

1. Build the `.nes` with cc65/ca65.
2. Map 2A03 pulse/triangle/noise activity to nametable bars, palette pulses, and
   sparse sprite/OAM-style effects.
3. Keep the generated iNES fallback for local packaging when ca65 is unavailable.
4. For live hardware, reduce incoming telemetry to scene, palette, beat, energy,
   and a few channel buckets before it reaches the NES program.

The NES target should stay inside NROM-style limits first: fixed CHR/tile data,
small palette changes, nametable animation, and no framebuffer assumptions.

## Master System / Game Gear: SN76489 VDP channel meters

Preferred first path: baked playback in Genesis Plus GX or an SMS-compatible
flashcart/core, with a later controller-port/devcart bridge for live PSG
telemetry.

1. Use the generated `chipviz-sms.sms` seed for smoke tests until a devkitSMS,
   SDCC, or small Z80 assembly SDK path is selected.
2. Map SN76489 tone/noise channel levels to VDP tile/channel meters and CRAM
   palette pulses.
3. Keep the generated ROM and host simulator as the deterministic fallback.
4. For live hardware, use reduced latest-state input; do not attempt to stream
   raw audio or full-resolution frames.

The SMS target should share the SN76489 visual vocabulary with Genesis PSG
accents while staying native to SMS VDP tile and palette constraints.
