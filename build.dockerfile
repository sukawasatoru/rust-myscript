FROM ubuntu:20.04
SHELL ["/bin/bash", "-c"]
ARG DEBIAN_FRONTEND=noninteractive
ARG RUST_VERSION
ENV RUSTUP_HOME=/usr/local/rustup
ENV CARGO_HOME=/usr/local/cargo
ENV PATH=/usr/local/cargo/bin:$PATH
RUN apt-get update && apt-get install -y --no-install-recommends \
  build-essential \
  ca-certificates \
  curl \
  # rust-openssl
  libssl-dev \
  # rust-openssl
  pkg-config && \
  rm -rf /var/lib/apt/lists/*
RUN set -o pipefail && \
  curl -sSf https://sh.rustup.rs | bash -s -- -v -y --no-modify-path --default-toolchain $RUST_VERSION
