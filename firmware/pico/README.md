# Pi Pico Joybus bridge

First N64 live transport firmware target.

The firmware uses a small host-testable packet/buffer core plus Pico SDK build
scaffolding. USB CDC serial feeds one raw 16-byte N64 Joybus transport frame at a
time; each completed frame replaces four controller state buffers. PIO state
machines then read those buffers and answer N64 controller status/poll requests
without reparsing the upstream stream.

Build with a normal Pico SDK checkout:

```sh
cmake -S firmware/pico -B build/pico -DPICO_SDK_PATH=/path/to/pico-sdk
cmake --build build/pico
```

Hardware mapping:

| Pico GPIO | N64 controller port |
| --- | --- |
| 0 | Port 1 data |
| 1 | Port 2 data |
| 2 | Port 3 data |
| 3 | Port 4 data |

Pico is powered via USB. N64 controller-port 3.3 V is left unconnected. See
[`../../docs/n64-hardware.md`](../../docs/n64-hardware.md).
