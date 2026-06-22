# N64 hardware setup

chipviz's N64 path uses controller ports as a low-bandwidth visualization-data
bus. The N64 does not play audio and does not need a flash-cart USB protocol at
runtime.

```text
Raspberry Pi
  audio/MIDI/chipsynth analysis
  16 bytes @ 60 Hz over USB serial
      |
      v
Pi Pico
  USB CDC receiver
  PIO Joybus on GPIO 0..3
      |
      v
N64 / Analogue 3D
  four controller ports
  libdragon ROM reads controller state
```

## Bill of materials

| Part | Notes |
| --- | --- |
| Raspberry Pi | Audio/MIDI/chipsynth analysis and serial sender. |
| Pi Pico | Basic Pico is enough; Pico W not required. |
| USB sound card | Optional line-in for the Pi. |
| 4x N64 controller extension cables | Cut the female ends off; keep male plugs. |
| Data-capable USB cable | Pi to Pico. |
| EverDrive 64 X7 or compatible flash cart | Loads the `.z64` on N64/Analogue 3D. |

## Inputs

The Pi can combine:

- USB sound card + 3.5 mm line-in
- Bluetooth A2DP
- PipeWire/PulseAudio source
- USB MIDI
- BLE MIDI
- network MIDI / OSC
- chipsynth visualization stream

## Wiring

N64 controller plug, front view:

```text
┌───────────┐
│ 1   2   3 │
└───┬───┬───┘
```

| N64 pin | Connect to |
| --- | --- |
| 1 GND | Pico GND |
| 2 Data | Pico GPIO 0, 1, 2, or 3 |
| 3 3.3 V | Leave unconnected |

The Pico is powered over USB. Do not power the Pico from the N64 controller-port
3.3 V pin.

## Data format and budget

See [`../shared/specs/n64-joybus-transport-v0.md`](../shared/specs/n64-joybus-transport-v0.md).

- 4 bytes per controller poll
- 4 controllers
- 16 bytes/frame
- 60 frames/sec
- 960 bytes/sec

That is enough for compact FFT bands, beat flags, scene/palette hints, overall
energy, and MIDI/chipsynth intensity.

## References

- [libdragon](https://github.com/DragonMinded/libdragon)
- [libjoybus / Joybus PIO](https://github.com/loopj/libjoybus)
- [Pi Pico SDK](https://github.com/raspberrypi/pico-sdk)
- [N64-UNFLoader](https://github.com/buu342/N64-UNFLoader) as a reference only;
  it is not required for this transport.
- [PCM1802 I2S ADC for Pico](https://github.com/NeimadG/raspberry_pico_pcm1802)
- [BLE MIDI on Pico W](https://github.com/rppicomidi/ble-midi2usbhost)
- [Pico W A2DP sink](https://github.com/joba-1/PicoW_A2DP)
