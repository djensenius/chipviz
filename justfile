cflags := "-std=c99 -Wall -Wextra -Werror -pedantic -Ishared/include"
sim_platforms := "n64 gba c64"
shared_srcs := "shared/src/control_frame.c shared/src/connection.c"
modern_manifest := "renderers/modern/Cargo.toml"

default:
    just --list

check: scaffold-check build-sim simulate host-sample modern-check
    @echo "chipviz check OK"

scaffold-check:
    @test -f README.md
    @test -f shared/specs/control-frame-v0.md
    @test -f shared/specs/control-frame-v0.json
    @test -f shared/include/chipviz/control_frame.h
    @test -f shared/include/chipviz/connection.h
    @test -f host/bridge/chipviz_bridge.py
    @test -f cores/n64/src/main.c
    @test -f cores/gba/src/main.c
    @test -f cores/c64/src/main.c
    @test -f renderers/modern/Cargo.toml
    @test -f renderers/modern/src/lib.rs
    @echo "chipviz scaffold OK"

build: build-sim

build-sim:
    @mkdir -p build/sim
    @for target in {{ sim_platforms }}; do \
      ${CC:-cc} {{ cflags }} -DCHIPVIZ_SIMULATOR cores/$target/src/main.c {{ shared_srcs }} -o build/sim/$target; \
    done

simulate: build-sim
    @for target in {{ sim_platforms }}; do build/sim/$target >/dev/null; done
    @echo "chipviz platform simulations OK"

host-sample:
    @mkdir -p build
    @${PYTHON:-python3} host/bridge/chipviz_bridge.py --frames 8 --output build/control-frame-v0.cvz
    @test -s build/control-frame-v0.cvz
    @echo "chipviz host bridge sample OK"

modern-check:
    @cargo check --manifest-path {{ modern_manifest }} --bins

modern-pi5 *args:
    @cargo run --manifest-path {{ modern_manifest }} --bin chipviz-pi5 -- {{ args }}

modern-m1 *args:
    @cargo run --manifest-path {{ modern_manifest }} --bin chipviz-m1 -- {{ args }}

clean:
    @rm -rf build
