# Pi Pico Joybus bridge

First N64 live transport firmware target.

The firmware uses a small host-testable packet/buffer core plus Pico SDK build
scaffolding. USB CDC serial feeds one raw 16-byte N64 Joybus transport frame at a
time; each completed frame replaces four controller state buffers. PIO state
machines then read those buffers and answer N64 controller status/poll requests
without reparsing the upstream stream.

## Compile-only build

Use the tools selected in the repository's `mise.toml`, including Arm GNU
Toolchain 15.2.Rel1, CMake and Ninja. From the repository root, fetch Pico SDK
2.3.1 at its full commit pin and initialize its pinned submodules:

```sh
git clone https://github.com/raspberrypi/pico-sdk.git .cache/pico-sdk
git -C .cache/pico-sdk checkout --detach 079c6f39023649b154152db30f1d781e884879bc
git -C .cache/pico-sdk submodule update --init --recursive
PICO_SDK_PATH="$PWD/.cache/pico-sdk" mise exec -- just pico-firmware
```

The recipe requires that SDK commit, clean tracked SDK sources and initialized
submodules at their recorded commits. It builds for the original **Pico /
RP2040**, resets CMake's configuration cache to avoid retaining a host compiler,
and produces nonempty `build/pico/chipviz_pico_joybus.elf` and
`build/pico/chipviz_pico_joybus.uf2`. It also verifies that the ELF machine is ARM.

UF2 conversion builds picotool 2.3.1 from full commit
`2041936441b48a3cc53ae3da9e805229fe8f4e18` through the SDK, rather than selecting
an arbitrary installed tool. This host utility is built without libusb; the
recipe only converts build outputs and does not flash, open USB/Joybus traffic,
or execute firmware on hardware. Initial setup/build requires network access.

The `Pico firmware` CI job uses the same recipe and SDK pin. Normal SDK-free
development and `mise exec -- just c-test` remain unchanged; those host tests
check the packet/buffer core, not the ARM firmware build or physical hardware.

For manual SDK experimentation, the existing CMake entry point still accepts
`PICO_SDK_PATH` as a CMake variable or environment variable:

```sh
cmake -S firmware/pico -B build/pico -DPICO_SDK_PATH=/path/to/pico-sdk
cmake --build build/pico
```

Compile success does not establish working PIO wiring or real-hardware behavior.

## Hardware mapping

| Pico GPIO | N64 controller port |
| --- | --- |
| 0 | Port 1 data |
| 1 | Port 2 data |
| 2 | Port 3 data |
| 3 | Port 4 data |

Pico is powered via USB. N64 controller-port 3.3 V is left unconnected. See
[`../../docs/n64-hardware.md`](../../docs/n64-hardware.md).
