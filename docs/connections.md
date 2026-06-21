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
2. The bridge reads one source adapter: procedural frames, a recorded
   `control-frame-v0` file, MIDI/audio analysis, or a chipsynth adapter.
3. For chipsynth development, keep chipviz beside the sibling `../chipsynth`
   checkout and add a chipsynth adapter that reads whichever host-visible stream
   is most stable first: MIDI events, ESP32 `AppState`, Daisy voice telemetry, or
   ChipStation SysEx.
4. If a separate ESP32 is doing target-specific transport, send packed frames to
   it over USB serial. Use a simple framed stream: wait for magic byte `0xC7`,
   read 33 bytes, validate version and XOR checksum, then replace the current
   frame.
5. If the Raspberry Pi or ESP32 is directly connected to the target transport,
   it can skip the second bridge and emit N64 controller-port, Bluetooth HID,
   network, or serial output itself.
6. Add Wi-Fi UDP after USB serial works. UDP is a good fit for live visuals
   because fresh frames matter more than retransmitting stale frames.

The ESP32 should also have a procedural fallback mode so each target can run
without the upstream sender connected.

## N64 / Analogue 3D: wired controller-port bridge

Preferred live path: an ESP32 emulates wired N64 controllers and plugs into the
N64 or Analogue 3D controller ports. A Raspberry Pi can perform upstream audio
analysis or chipsynth adaptation, then send frames to the ESP32 over USB serial
or Wi-Fi UDP.

1. Build and run the N64 ROM from a flashcart or Analogue 3D workflow.
2. Wire ESP32 GPIOs to one or more N64 controller data lines, with common ground.
   Use 3.3 V-safe signaling, do not drive 5 V into the ESP32, and keep the GPIO
   behavior close to open-drain/tri-state so the console and bridge do not fight
   the data line.
3. Start with one controller port for scene/palette/intensity controls.
4. Expand to four controller ports for live frame transport. The N64 core polls
   controller state through libdragon; the ESP32 encodes compact slices of
   `control-frame-v0` into button/stick state across those polls.
5. Keep baked frame playback as the fallback for emulator, flashcart, and
   transport debugging.

The N64 visualizer should use real 3D. A good first direction is flat,
graphic-looking objects arranged on 3D planes: from one camera angle they read as
2D motion graphics, then camera orbit/tilt makes the depth obvious. Beat phase,
bass, and note events should drive camera moves, particles, Z-depth, and
palette-lit geometry.

## Analogue Pocket Dock / GBA: Bluetooth controller bridge

Preferred low-bandwidth live path: the Analogue Pocket Dock receives a Bluetooth
controller, and the GBA core treats that controller input as a control surface.
An ESP32 can present as the Bluetooth HID device; a Raspberry Pi can do upstream
audio analysis and forward compact frame/control state to the ESP32.

1. Pair a Bluetooth controller with the Analogue Pocket Dock.
2. Run the `.gba` build through the Pocket's GBA path.
3. Map buttons and D-pad to scene, palette, intensity, and mode flags.
4. For an ESP32 bridge experiment, make the ESP32 present itself as a Bluetooth
   HID gamepad to the Dock, then translate incoming `control-frame-v0` packets
   into button-state patterns.
5. Keep baked `.gba` frame playback as the deterministic fallback.

This path is intentionally low bandwidth. It is best for scene changes,
palette/intensity nudges, and beat/drop triggers. If richer live frame data is
needed later, add a separate ESP32-to-GBA link-cable transport.

## Commodore 64 Ultimate: network bridge

Preferred live path candidate: use the Commodore 64 Ultimate network stack or
network services from the same LAN as the sender. A Raspberry Pi is a good fit
for audio analysis plus LAN output; an ESP32 is a good fit for compact UDP or
serial forwarding.

1. Put the Ultimate and the Raspberry Pi, ESP32, or development computer on the
   same network.
2. Confirm which Ultimate-side network service is practical for a running C64
   program to consume.
3. Start with UDP-style latest-frame delivery if available: the host bridge or
   ESP32 sends packed `control-frame-v0` packets, and the C64 side keeps the most
   recent valid frame.
4. Reduce the frame before it reaches tight C64 code when needed: scene,
   palette, flags, energy, beat phase, and a few spectrum bands may be enough for
   raster/PETSCII effects.
5. Keep `.prg`/`.d64` baked playback as the fallback, and use user-port serial as
   the hardware fallback if the Ultimate network path is not viable.

The C64 target should stay visually native: raster bars, PETSCII, character
animation, sprites, fixed palettes, and SID/VIC-II timing tricks rather than a
scaled-down copy of the N64 or GBA scenes.
