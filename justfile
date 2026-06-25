set dotenv-load := true

cflags := "-std=c99 -Wall -Wextra -Werror -pedantic -Ishared/include"
sim_platforms := "n64 gba c64 snes gb genesis nes sms"
shared_srcs := "shared/src/control_frame.c shared/src/connection.c shared/src/interface.c"
modern_manifest := "renderers/modern/Cargo.toml"
python_files := "host/bridge/chipviz_bridge.py host/bridge/chipsynth_stream.py host/bridge/chipviz_encode.py host/bridge/live_bridge.py host/bridge/n64_joybus.py host/bridge/usb_hid.py scripts/retroarch_smoke.py shared/tools/build_homebrew.py shared/tools/cvz_to_c.py shared/tools/lint_docs.py tests/test_homebrew.py tests/test_host_bridge.py tests/test_transports.py"

default:
    just --list

check: lint test
    @echo "chipviz check OK"

lint: scaffold-check c-lint python-lint docs-lint rust-lint
    @echo "chipviz lint OK"

test: c-test python-test rust-test build-sim simulate host-sample target-arrays homebrew-artifacts
    @just host-fixtures
    @echo "chipviz tests OK"

scaffold-check:
    @test -f README.md
    @test -f docs/homebrew-targets.md
    @test -f shared/specs/source-adapter-v0.md
    @test -f shared/specs/chipsynth-source-v0.md
    @test -f shared/specs/chipsynth-viz-stream-v0.md
    @test -f shared/specs/control-frame-v0.md
    @test -f shared/specs/control-frame-v0.json
    @test -f shared/include/chipviz/control_frame.h
    @test -f shared/include/chipviz/connection.h
    @test -f shared/include/chipviz/interface.h
    @test -f host/bridge/chipviz_bridge.py
    @test -f host/bridge/chipsynth_stream.py
    @test -f host/bridge/chipviz_encode.py
    @test -f host/bridge/live_bridge.py
    @test -f host/bridge/n64_joybus.py
    @test -f host/bridge/usb_hid.py
    @test -f scripts/retroarch_smoke.py
    @test -f shared/specs/n64-joybus-transport-v0.md
    @test -f shared/specs/usb-hid-transport-v0.md
    @test -f firmware/pico/src/joybus_bridge.c
    @test -f firmware/pico/src/main.c
    @test -f firmware/pico/src/joybus_bridge.pio
    @test -f firmware/pico/CMakeLists.txt
    @test -f firmware/pico/README.md
    @test -f shared/specs/music-source-v0.md
    @test -f host/fixtures/musical/scale.musicsource.json
    @test -f host/fixtures/musical/scale.cvz
    @test -f host/fixtures/chipsynth/groove.csv0
    @test -f host/fixtures/chipsynth/groove.cvz
    @test -f shared/tools/lint_docs.py
    @test -f shared/tools/build_homebrew.py
    @test -f shared/tools/cvz_to_c.py
    @test -f tests/test_homebrew.py
    @test -f tests/control_frame_tests.c
    @test -f tests/connection_tests.c
    @test -f tests/test_host_bridge.py
    @test -f tests/test_transports.py
    @test -f cores/n64/src/main.c
    @test -f cores/n64/homebrew/Makefile
    @test -f cores/n64/homebrew/main.c
    @test -f cores/gba/src/main.c
    @test -f cores/gba/homebrew/Makefile
    @test -f cores/gba/homebrew/source/main.c
    @test -f cores/c64/homebrew/Makefile
    @test -f cores/c64/homebrew/src/main.c
    @test -f cores/snes/homebrew/Makefile
    @test -f cores/snes/homebrew/chipviz-snes.c
    @test -f cores/gb/src/main.c
    @test -f cores/gb/homebrew/Makefile
    @test -f cores/gb/homebrew/main.asm
    @test -f cores/genesis/src/main.c
    @test -f cores/genesis/homebrew/Makefile
    @test -f cores/nes/src/main.c
    @test -f cores/nes/homebrew/Makefile
    @test -f cores/nes/homebrew/src/main.s
    @test -f cores/sms/src/main.c
    @test -f cores/sms/homebrew/Makefile
    @test -f cores/c64/src/main.c
    @test -f cores/snes/src/main.c
    @test -f renderers/modern/Cargo.toml
    @test -f renderers/modern/src/lib.rs
    @echo "chipviz scaffold OK"

build: build-sim

c-lint: build-sim
    @echo "chipviz C lint OK"

build-sim:
    @mkdir -p build/sim
    @for target in {{ sim_platforms }}; do \
      ${CC:-cc} {{ cflags }} -DCHIPVIZ_SIMULATOR cores/$target/src/main.c {{ shared_srcs }} -o build/sim/$target; \
    done

simulate: build-sim
    @for target in {{ sim_platforms }}; do build/sim/$target >/dev/null; done
    @echo "chipviz platform simulations OK"

c-test:
    @mkdir -p build/tests
    @${CC:-cc} {{ cflags }} tests/control_frame_tests.c shared/src/control_frame.c -o build/tests/control_frame_tests
    @${CC:-cc} {{ cflags }} tests/connection_tests.c {{ shared_srcs }} -o build/tests/connection_tests
    @${CC:-cc} {{ cflags }} tests/interface_tests.c shared/src/control_frame.c shared/src/interface.c -o build/tests/interface_tests
    @${CC:-cc} {{ cflags }} -Ifirmware/pico/include tests/pico_joybus_tests.c firmware/pico/src/joybus_bridge.c -o build/tests/pico_joybus_tests
    @build/tests/control_frame_tests
    @build/tests/connection_tests
    @build/tests/interface_tests
    @build/tests/pico_joybus_tests
    @echo "chipviz C tests OK"

python-lint:
    @${PYTHON:-python3} -m py_compile {{ python_files }}
    @echo "chipviz Python lint OK"

python-test:
    @${PYTHON:-python3} -m unittest discover -s tests -p 'test_*.py'
    @echo "chipviz Python tests OK"

docs-lint:
    @${PYTHON:-python3} -m json.tool shared/specs/control-frame-v0.json >/dev/null
    @${PYTHON:-python3} shared/tools/lint_docs.py
    @echo "chipviz docs lint OK"

host-sample:
    @mkdir -p build
    @${PYTHON:-python3} host/bridge/chipviz_bridge.py --frames 8 --output build/control-frame-v0.cvz
    @${PYTHON:-python3} host/bridge/chipsynth_stream.py --demo-packet build/chipsynth-viz-stream-v0.bin --output build/chipsynth-derived.cvz
    @test -s build/control-frame-v0.cvz
    @test -s build/chipsynth-viz-stream-v0.bin
    @test -s build/chipsynth-derived.cvz
    @echo "chipviz host bridge sample OK"

host-fixtures:
    @${PYTHON:-python3} host/bridge/chipviz_encode.py --verify-fixtures
    @echo "chipviz host fixtures OK"

target-arrays:
    @mkdir -p build/generated
    @${PYTHON:-python3} shared/tools/cvz_to_c.py --input host/fixtures/musical/scale.cvz --symbol chipviz_scale_frames --output build/generated/scale_frames.h
    @test -s build/generated/scale_frames.h
    @echo "chipviz target arrays OK"

homebrew-artifacts:
    @mkdir -p build/homebrew
    @${PYTHON:-python3} shared/tools/build_homebrew.py --output build/homebrew
    @test -s build/homebrew/chipviz-c64.prg
    @test -s build/homebrew/chipviz-snes.sfc
    @test -s build/homebrew/chipviz-genesis.md
    @test -s build/homebrew/chipviz-nes.nes
    @test -s build/homebrew/chipviz-sms.sms
    @test -s build/homebrew/homebrew-status.json
    @echo "chipviz homebrew artifacts OK"

gba-rom:
    @make -C cores/gba/homebrew

gba-rom-docker:
    @command -v docker >/dev/null 2>&1 || (echo "docker is required for gba-rom-docker" >&2; exit 1)
    @docker run --rm -v "$PWD":/workspace -w /workspace/cores/gba/homebrew devkitpro/devkitarm:latest make

n64-rom:
    @test -n "${N64_INST:-}" || (echo "N64_INST is required; install libdragon" >&2; exit 1)
    @make -C cores/n64/homebrew

n64-rom-docker:
    @command -v docker >/dev/null 2>&1 || (echo "docker is required for n64-rom-docker" >&2; exit 1)
    @scripts/n64-docker-build.sh

c64-rom:
    @command -v cl65 >/dev/null 2>&1 || (echo "cl65 is required; install cc65" >&2; exit 1)
    @make -C cores/c64/homebrew

snes-rom:
    @test -n "${PVSNESLIB_HOME:-}" || (echo "PVSNESLIB_HOME is required; install PVSnesLib" >&2; exit 1)
    @sed_bin="${GNU_SED_BIN:-}"; \
      if [ -z "$sed_bin" ] && command -v brew >/dev/null 2>&1; then sed_bin="$(brew --prefix)/opt/gnu-sed/libexec/gnubin"; fi; \
      if [ -d "$sed_bin" ]; then PATH="$sed_bin:$PATH" make -C cores/snes/homebrew; else make -C cores/snes/homebrew; fi

gb-rom:
    @command -v rgbasm >/dev/null 2>&1 || (echo "rgbasm is required; install RGBDS" >&2; exit 1)
    @command -v rgblink >/dev/null 2>&1 || (echo "rgblink is required; install RGBDS" >&2; exit 1)
    @command -v rgbfix >/dev/null 2>&1 || (echo "rgbfix is required; install RGBDS" >&2; exit 1)
    @make -C cores/gb/homebrew

genesis-rom:
    @make -C cores/genesis/homebrew

nes-rom:
    @command -v ca65 >/dev/null 2>&1 || (echo "ca65 is required; install cc65" >&2; exit 1)
    @command -v ld65 >/dev/null 2>&1 || (echo "ld65 is required; install cc65" >&2; exit 1)
    @make -C cores/nes/homebrew

sms-rom:
    @make -C cores/sms/homebrew

sdk-roms:
    @just gba-rom
    @just n64-rom
    @just c64-rom
    @just snes-rom
    @just gb-rom
    @just genesis-rom
    @just nes-rom
    @just sms-rom

debug-env:
    @ra="${RETROARCH_BIN:-/Applications/RetroArch.app/Contents/MacOS/RetroArch}"; \
      cores="${RETROARCH_CORES:-$HOME/Library/Application Support/RetroArch/cores}"; \
      echo "RETROARCH_BIN=$ra"; test -x "$ra" || (echo "RetroArch binary not found or not executable" >&2; exit 1); \
      echo "RETROARCH_CORES=$cores"; test -d "$cores" || (echo "RetroArch cores directory not found" >&2; exit 1); \
      for core in mgba_libretro.dylib mupen64plus_next_libretro.dylib snes9x_libretro.dylib vice_x64sc_libretro.dylib sameboy_libretro.dylib genesis_plus_gx_libretro.dylib mesen_libretro.dylib; do test -f "$cores/$core" || (echo "Missing RetroArch core: $core" >&2; exit 1); done; \
      echo "RetroArch debug environment OK"

gba-debug:
    @if [ -n "${DEVKITARM:-}" ]; then just gba-rom; else just gba-rom-docker; fi
    @mkdir -p build/retroarch-smoke
    @ra="${RETROARCH_BIN:-/Applications/RetroArch.app/Contents/MacOS/RetroArch}"; cores="${RETROARCH_CORES:-$HOME/Library/Application Support/RetroArch/cores}"; \
      "$ra" -L "$cores/mgba_libretro.dylib" cores/gba/homebrew/chipviz-gba.gba --verbose 2>&1 | tee build/retroarch-smoke/gba.log

n64-debug:
    @if [ -n "${N64_INST:-}" ]; then just n64-rom; else just n64-rom-docker; fi
    @ares="${ARES_BIN:-}"; \
      if [ -z "$ares" ] && [ -x /Applications/ares.app/Contents/MacOS/ares ]; then ares=/Applications/ares.app/Contents/MacOS/ares; fi; \
      if [ -z "$ares" ]; then ares="$(command -v ares 2>/dev/null || true)"; fi; \
      if [ -n "$ares" ] && [ -x "$ares" ]; then "$ares" cores/n64/homebrew/chipviz-n64.z64; else just _n64-launch-retroarch; fi

n64-debug-retroarch:
    @if [ -n "${N64_INST:-}" ]; then just n64-rom; else just n64-rom-docker; fi
    @just _n64-launch-retroarch

_n64-launch-retroarch:
    @mkdir -p build/retroarch-smoke
    @ra="${RETROARCH_BIN:-/Applications/RetroArch.app/Contents/MacOS/RetroArch}"; cores="${RETROARCH_CORES:-$HOME/Library/Application Support/RetroArch/cores}"; \
      "$ra" -L "$cores/mupen64plus_next_libretro.dylib" cores/n64/homebrew/chipviz-n64.z64 --verbose 2>&1 | tee build/retroarch-smoke/n64.log

c64-debug: c64-rom
    @mkdir -p build/retroarch-smoke
    @ra="${RETROARCH_BIN:-/Applications/RetroArch.app/Contents/MacOS/RetroArch}"; cores="${RETROARCH_CORES:-$HOME/Library/Application Support/RetroArch/cores}"; rom="$(pwd)/cores/c64/homebrew/chipviz-c64.prg"; \
      "$ra" -L "$cores/vice_x64sc_libretro.dylib" "$rom" --verbose 2>&1 | tee build/retroarch-smoke/c64.log

snes-debug: snes-rom
    @mkdir -p build/retroarch-smoke
    @ra="${RETROARCH_BIN:-/Applications/RetroArch.app/Contents/MacOS/RetroArch}"; cores="${RETROARCH_CORES:-$HOME/Library/Application Support/RetroArch/cores}"; \
      "$ra" -L "$cores/snes9x_libretro.dylib" cores/snes/homebrew/chipviz-snes.sfc --verbose 2>&1 | tee build/retroarch-smoke/snes.log

gb-debug: gb-rom
    @mkdir -p build/retroarch-smoke
    @ra="${RETROARCH_BIN:-/Applications/RetroArch.app/Contents/MacOS/RetroArch}"; cores="${RETROARCH_CORES:-$HOME/Library/Application Support/RetroArch/cores}"; \
      "$ra" -L "$cores/sameboy_libretro.dylib" cores/gb/homebrew/chipviz-gb.gb --verbose 2>&1 | tee build/retroarch-smoke/gb.log

genesis-debug: genesis-rom
    @mkdir -p build/retroarch-smoke
    @ra="${RETROARCH_BIN:-/Applications/RetroArch.app/Contents/MacOS/RetroArch}"; cores="${RETROARCH_CORES:-$HOME/Library/Application Support/RetroArch/cores}"; \
      "$ra" -L "$cores/genesis_plus_gx_libretro.dylib" cores/genesis/homebrew/chipviz-genesis.md --verbose 2>&1 | tee build/retroarch-smoke/genesis.log

nes-debug: nes-rom
    @mkdir -p build/retroarch-smoke
    @ra="${RETROARCH_BIN:-/Applications/RetroArch.app/Contents/MacOS/RetroArch}"; cores="${RETROARCH_CORES:-$HOME/Library/Application Support/RetroArch/cores}"; \
      "$ra" -L "$cores/mesen_libretro.dylib" cores/nes/homebrew/chipviz-nes.nes --verbose 2>&1 | tee build/retroarch-smoke/nes.log

sms-debug: sms-rom
    @mkdir -p build/retroarch-smoke
    @ra="${RETROARCH_BIN:-/Applications/RetroArch.app/Contents/MacOS/RetroArch}"; cores="${RETROARCH_CORES:-$HOME/Library/Application Support/RetroArch/cores}"; \
      "$ra" -L "$cores/genesis_plus_gx_libretro.dylib" cores/sms/homebrew/chipviz-sms.sms --verbose 2>&1 | tee build/retroarch-smoke/sms.log

retroarch-smoke:
    @just debug-env
    @mkdir -p build/retroarch-smoke
    @python3 scripts/retroarch_smoke.py

rust-lint:
    @cargo fmt --manifest-path {{ modern_manifest }} -- --check
    @cargo clippy --manifest-path {{ modern_manifest }} --bins --tests -- -D warnings
    @echo "chipviz Rust lint OK"

rust-test:
    @cargo test --manifest-path {{ modern_manifest }} --bins --lib
    @echo "chipviz Rust tests OK"
modern-check: rust-lint
    @cargo check --manifest-path {{ modern_manifest }} --bins

modern-pi5 *args:
    @cargo run --manifest-path {{ modern_manifest }} --bin chipviz-pi5 -- {{ args }}

modern-m1 *args:
    @cargo run --manifest-path {{ modern_manifest }} --bin chipviz-m1 -- {{ args }}

clean:
    @rm -rf build dist
