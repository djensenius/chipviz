cflags := "-std=c99 -Wall -Wextra -Werror -pedantic -Ishared/include"
sim_platforms := "n64 gba c64"
shared_srcs := "shared/src/control_frame.c shared/src/connection.c"
modern_manifest := "renderers/modern/Cargo.toml"
python_files := "host/bridge/chipviz_bridge.py host/bridge/chipsynth_stream.py shared/tools/lint_docs.py tests/test_host_bridge.py"

default:
    just --list

check: lint test
    @echo "chipviz check OK"

lint: scaffold-check c-lint python-lint docs-lint rust-lint
    @echo "chipviz lint OK"

test: c-test python-test rust-test build-sim simulate host-sample
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
    @test -f shared/tools/lint_docs.py
    @test -f tests/control_frame_tests.c
    @test -f tests/connection_tests.c
    @test -f tests/test_host_bridge.py
    @test -f cores/n64/src/main.c
    @test -f cores/gba/src/main.c
    @test -f cores/c64/src/main.c
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
    @build/tests/control_frame_tests
    @build/tests/connection_tests
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
    @rm -rf build
