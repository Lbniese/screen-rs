FROM rust:bookworm

ENV DEBIAN_FRONTEND=noninteractive

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    curl \
    libncurses-dev \
    libpam0g-dev \
    locales \
    pkg-config \
    python3 \
    && rm -rf /var/lib/apt/lists/*

RUN sed -i 's/^# *en_US.UTF-8 UTF-8/en_US.UTF-8 UTF-8/' /etc/locale.gen && locale-gen
ENV LANG=C.UTF-8 LC_ALL=C.UTF-8
ENV PATH=/usr/local/cargo/bin:/usr/local/rustup/toolchains/stable-aarch64-unknown-linux-gnu/bin:/usr/local/rustup/toolchains/stable-x86_64-unknown-linux-gnu/bin:$PATH

WORKDIR /workspace
CMD ["bash"]
