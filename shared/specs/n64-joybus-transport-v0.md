# N64 Joybus transport v0 and v1

This transport packs one `control-frame-v0` into four emulated N64 controller
states. A Raspberry Pi sends 16 bytes per frame at 60 Hz over USB serial to a Pi
Pico. The Pico exposes those bytes as four Joybus controller replies.

## Serial frame

The Pi-to-Pico frame is exactly 16 bytes:

| Byte | Controller | N64 field | Meaning |
| --- | --- | --- | --- |
| 0 | 1 | buttons high | beat flags for sub-bass/bass |
| 1 | 1 | buttons low | v0: reserved `0`; v1: frame marker magic |
| 2 | 1 | stick X | sub-bass energy (`spectrum[0]`) |
| 3 | 1 | stick Y | bass energy (`spectrum[1]`) |
| 4 | 2 | buttons high | beat flags for low-mid/mid |
| 5 | 2 | buttons low | v0: reserved `0`; v1: frame marker sequence |
| 6 | 2 | stick X | low-mid energy (`spectrum[2]`) |
| 7 | 2 | stick Y | mid energy (`spectrum[3]`) |
| 8 | 3 | buttons high | beat flags for upper-mid/treble |
| 9 | 3 | buttons low | v0: reserved `0`; v1: frame marker checksum |
| 10 | 3 | stick X | upper-mid energy (`spectrum[4]`) |
| 11 | 3 | stick Y | treble energy (`spectrum[5]`) |
| 12 | 4 | buttons high | global beat, bar, fill, mode/palette bits |
| 13 | 4 | buttons low | scene/palette low bits |
| 14 | 4 | stick X | overall energy |
| 15 | 4 | stick Y | MIDI/chipsynth intensity (`max(notes.velocity)`) |

The first implementation treats stick bytes as unsigned visual values even
though native N64 sticks are signed. The ROM-side decoder should read raw
controller bytes or remap signed values back to `0..255`.

## v1 frame marker

An ordinary idle N64 pad and a v0 frame of silence are byte-identical, so the
ROM cannot tell a real controller from live chipviz data. Version 1 keeps the
same 16-byte layout and payload but fills the reserved buttons-low bytes 1, 5
and 9 with a marker. libdragon drops bits `0x80` (reset) and `0x40` (unused) of
buttons-low, so each marker field is six bits and the top two bits must be `0`.

| Byte | Field | Value |
| --- | --- | --- |
| 1 | magic | `0x2D` |
| 5 | sequence | sender's per-packet counter `& 0x3F` |
| 9 | checksum | six-bit checksum below |

Checksum: start with `c = 0xA5`. For every byte `b` at index `0..15` except 9
(so including the magic and sequence), set `c = rotl8(c, 1) ^ b`. The checksum
is `(c ^ (c >> 6)) & 0x3F`. A frame is marked when byte 1 is the magic, byte 5
has no bits above `0x3F`, and byte 9 equals the checksum.

The sequence must advance once per emitted packet, whatever the control-frame
`frame` counter does. A sequence that stops advancing makes the ROM hold and
then fall back to procedural. `convert_stream` numbers packets from
`first_sequence`. The live bridge serial sink (`N64SerialSink`) keeps its own
counter across sends.

The host encoder is `host/bridge/n64_joybus.py`. The ROM state machine is
`cores/n64/homebrew/joybus_link.h`. Both are tested against the same golden
vectors.

### ROM source selection

The ROM chooses one of three sources every frame:

- **Procedural** unless all four controller ports report a connected device.
  A partial disconnect counts as no device: the ROM switches immediately and
  resets its state. It also
  stays procedural while a connected pad sends no valid marker, which covers an
  idle pad, a pad in use, a v0 host or a corrupt marker.
- **Live** after a valid frame followed by 4 valid frames with an advancing
  (changed) sequence. The ROM then uses each advancing valid frame, even if
  every data byte is zero. A valid frame that repeats its sequence neither
  advances nor resets that count. An unmarked or corrupt frame resets it.
- Live also tolerates up to 2 consecutive valid repeats of the same sequence,
  for example a host polled faster than it sends. Those repeats stay live.
- **Hold** while live, when a frame is not an advancing valid frame (a lost
  marker or a repeat beyond the 2-frame grace). The ROM keeps drawing the last
  live frame. After 45 frames without an advance it returns to procedural and
  needs the 4-frame recovery again. Any advancing valid frame during hold
  resumes live at once.

The 30-frame start-up warm-up still forces procedural output.

### Compatibility

- v1 host with a v0 ROM: v0 ROMs ignore bytes 1, 5 and 9, so the frames decode
  unchanged.
- v0 host with a v1 ROM: v0 frames carry no marker, so the ROM treats them as an
  ordinary pad and stays procedural. Update the host to show live data.
- The Pico firmware copies the 16 bytes unchanged and needs no change.
- Live mode needs all four controller ports wired to the Pico. Ports 1, 2 and
  3 carry the marker in their buttons-low bytes. Port 4 carries scene, palette,
  beat, energy and MIDI data, and the checksum covers those bytes. A missing
  port reads as zeros, and the marker alone cannot always detect that. For
  example, zeroing port 4 bytes `00 00 20 40` keeps the checksum. The ROM
  therefore requires all four ports to be connected and stays procedural if any
  is missing.
- The Pico's serial framing has no resync. If the byte stream slips (a dropped
  or extra byte), every packet straddles two frames and fails the marker. The
  link then stays procedural until the Pico is reset. Hardware validation is
  tracked in #31.
- A real pad can only forge a marker by pressing the exact L, R and C-button
  pattern on ports 1 to 3 with a matching checksum and a changing sequence for
  several frames in a row.

## Pico behavior

- USB CDC receives one 16-byte packet at a time.
- The newest valid packet replaces four controller-state buffers.
- GPIO 0..3 map to controller ports 1..4.
- Joybus timing is handled in PIO state machines; the CPU only updates buffers.
- N64 controller-port 3.3 V is left unconnected. The Pico is USB powered and
  shares ground with the console.
