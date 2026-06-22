# N64 Joybus transport v0

This transport packs one `control-frame-v0` into four emulated N64 controller
states. A Raspberry Pi sends 16 bytes per frame at 60 Hz over USB serial to a Pi
Pico. The Pico exposes those bytes as four Joybus controller replies.

## Serial frame

The Pi-to-Pico frame is exactly 16 bytes:

| Byte | Controller | N64 field | Meaning |
| --- | --- | --- | --- |
| 0 | 1 | buttons high | beat flags for sub-bass/bass |
| 1 | 1 | buttons low | reserved, must be `0` |
| 2 | 1 | stick X | sub-bass energy (`spectrum[0]`) |
| 3 | 1 | stick Y | bass energy (`spectrum[1]`) |
| 4 | 2 | buttons high | beat flags for low-mid/mid |
| 5 | 2 | buttons low | reserved, must be `0` |
| 6 | 2 | stick X | low-mid energy (`spectrum[2]`) |
| 7 | 2 | stick Y | mid energy (`spectrum[3]`) |
| 8 | 3 | buttons high | beat flags for upper-mid/treble |
| 9 | 3 | buttons low | reserved, must be `0` |
| 10 | 3 | stick X | upper-mid energy (`spectrum[4]`) |
| 11 | 3 | stick Y | treble energy (`spectrum[5]`) |
| 12 | 4 | buttons high | global beat, bar, fill, mode/palette bits |
| 13 | 4 | buttons low | scene/palette low bits |
| 14 | 4 | stick X | overall energy |
| 15 | 4 | stick Y | MIDI/chipsynth intensity (`max(notes.velocity)`) |

The first implementation treats stick bytes as unsigned visual values even
though native N64 sticks are signed. The ROM-side decoder should read raw
controller bytes or remap signed values back to `0..255`.

## Pico behavior

- USB CDC receives one 16-byte packet at a time.
- The newest valid packet replaces four controller-state buffers.
- GPIO 0..3 map to controller ports 1..4.
- Joybus timing is handled in PIO state machines; the CPU only updates buffers.
- N64 controller-port 3.3 V is left unconnected. The Pico is USB powered and
  shares ground with the console.
