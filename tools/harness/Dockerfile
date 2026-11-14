# Host side of the emulator: QEMU (x86, full system, TCG) on arm64 Debian.
# No game data in this image.
FROM --platform=linux/arm64 debian:bookworm-slim
RUN apt-get update && DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends \
    qemu-system-x86 e2fsprogs perl-base ca-certificates \
    && rm -rf /var/lib/apt/lists/*
WORKDIR /research
