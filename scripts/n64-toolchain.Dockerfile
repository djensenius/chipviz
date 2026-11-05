FROM ubuntu:22.04@sha256:076e7bf82319619f28dd0aa9c02ab34b055ad37100395b4f49a2ffba04fb5e11

ADD --checksum=sha256:fac8e6572493a66468b7d45df41b042f90b1a630ec96aa218bd5fb1f320a0a4d \
    https://github.com/DragonMinded/libdragon/releases/download/toolchain-continuous-prerelease/gcc-toolchain-mips64-x86_64.deb \
    /tmp/libdragon-toolchain.deb

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates g++ gcc git make python3 xxd \
    && rm -rf /var/lib/apt/lists/* \
    && dpkg-deb --extract /tmp/libdragon-toolchain.deb /tmp/toolchain \
    && mv /tmp/toolchain/opt/libdragon /n64_toolchain \
    && rm -rf /tmp/toolchain /tmp/libdragon-toolchain.deb

ENV N64_INST=/n64_toolchain
ENV PATH=/n64_toolchain/bin:$PATH
