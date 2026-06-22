# Pi Pico Joybus bridge

First N64 live transport firmware target.

The production firmware will use the Pico SDK plus PIO Joybus state machines.
The checked-in C module is the host-testable packet/buffer core: it accepts one
16-byte N64 Joybus transport frame and exposes four controller state buffers
that the PIO IRQ handlers can read without parsing.

Hardware mapping:

| Pico GPIO | N64 controller port |
| --- | --- |
| 0 | Port 1 data |
| 1 | Port 2 data |
| 2 | Port 3 data |
| 3 | Port 4 data |

Pico is powered via USB. N64 controller-port 3.3 V is left unconnected. See
[`../../docs/n64-hardware.md`](../../docs/n64-hardware.md).
