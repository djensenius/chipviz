cflags := "-std=c99 -Wall -Wextra -Werror -pedantic -Ishared/include"
sim_platforms := "n64 gba c64 snes"
shared_srcs := "shared/src/control_frame.c shared/src/connection.c"
modern_manifest := "renderers/modern/Cargo.toml"
python_files := "host/bridge/chipviz_bridge.py host/bridge/chipsynth_stream.py host/bridge/chipviz_encode.py host/bridge/live_bridge.py host/bridge/n64_joybus.py host/bridge/usb_hid.py shared/tools/cvz_to_c.py shared/tools/lint_docs.py tests/test_host_bridge.py tests/test_transports.py"

default:
    just --list

check: lint test
    @echo "chipviz check OK"

lint: scaffold-check c-lint python-lint docs-lint rust-lint
    @echo "chipviz lint OK"

test: c-test python-test rust-test build-sim simulate host-sample target-arrays
    @just host-fixtures
    @echo "chipviz tests OK"

scaffold-check:
    @test -f README.md
    @test -f shared/specs/source-adapter-v0.md
    @test -f shared/specs/chipsynth-source-v0.md
    @test -f shared/specs/chipsynth-viz-stream-v0.md
    @test -f shared/specs/control-frame-v0.md
    @test -f shared/specs/control-frame-v0.json
    @test -f shared/include/chipviz/control_frame.h
    @test -f shared/include/chipviz/connection.h
    @test -f host/bridge/chipviz_bridge.py
    @test -f host/bridge/chipsynth_stream.py
    @test -f host/bridge/chipviz_encode.py
    @test -f host/bridge/live_bridge.py
    @test -f host/bridge/n64_joybus.py
    @test -f host/bridge/usb_hid.py
    @test -f shared/specs/n64-joybus-transport-v0.md
    @test -f shared/specs/usb-hid-transport-v0.md
    @test -f firmware/pico/src/joybus_bridge.c
    @test -f firmware/pico/README.md
    @test -f shared/specs/music-source-v0.md
    @test -f host/fixtures/musical/scale.musicsource.json
    @test -f host/fixtures/musical/scale.cvz
    @test -f host/fixtures/chipsynth/groove.csv0
    @test -f host/fixtures/chipsynth/groove.cvz
    @test -f shared/tools/lint_docs.py
    @test -f shared/tools/cvz_to_c.py
    @test -f tests/control_frame_tests.c
    @test -f tests/connection_tests.c
    @test -f tests/test_host_bridge.py
    @test -f tests/test_transports.py
    @test -f cores/n64/src/main.c
    @test -f cores/gba/src/main.c
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
    @${CC:-cc} {{ cflags }} -Ifirmware/pico/include tests/pico_joybus_tests.c firmware/pico/src/joybus_bridge.c -o build/tests/pico_joybus_tests
    @build/tests/control_frame_tests
    @build/tests/connection_tests
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
