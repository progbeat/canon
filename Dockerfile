# syntax=docker/dockerfile:1

ARG ALPINE_VERSION=3.22
ARG RUST_VERSION=1

FROM rust:${RUST_VERSION}-alpine${ALPINE_VERSION} AS canon-builder
RUN apk add --no-cache musl-dev
WORKDIR /build

COPY Cargo.toml Cargo.lock ./
COPY src ./src
COPY resources ./resources
COPY .canon/templates/default/check.yml ./.canon/templates/default/check.yml

RUN cargo build --release --locked

FROM alpine:${ALPINE_VERSION} AS codex-downloader
ARG TARGETARCH
ARG CODEX_RELEASE=rust-v0.142.5
RUN apk add --no-cache ca-certificates curl gzip tar
RUN set -eux; \
    target_arch="${TARGETARCH:-}"; \
    if [ -z "${target_arch}" ]; then \
      case "$(apk --print-arch)" in \
        x86_64) target_arch="amd64" ;; \
        aarch64) target_arch="arm64" ;; \
        *) echo "unsupported Alpine arch: $(apk --print-arch)" >&2; exit 1 ;; \
      esac; \
    fi; \
    case "${target_arch}" in \
      amd64) codex_target="x86_64-unknown-linux-musl" ;; \
      arm64) codex_target="aarch64-unknown-linux-musl" ;; \
      *) echo "unsupported TARGETARCH: ${target_arch}" >&2; exit 1 ;; \
    esac; \
    if [ "${CODEX_RELEASE}" = "latest" ]; then \
      codex_url="https://github.com/openai/codex/releases/latest/download/codex-${codex_target}.tar.gz"; \
    else \
      codex_url="https://github.com/openai/codex/releases/download/${CODEX_RELEASE}/codex-${codex_target}.tar.gz"; \
    fi; \
    curl -fsSL "${codex_url}" -o /tmp/codex.tar.gz; \
    mkdir -p /tmp/codex; \
    tar -xzf /tmp/codex.tar.gz -C /tmp/codex; \
    install -m 0755 "/tmp/codex/codex-${codex_target}" /usr/local/bin/codex

FROM alpine:${ALPINE_VERSION} AS runtime-base
RUN apk add --no-cache \
    bash \
    ca-certificates \
    coreutils \
    curl \
    diffutils \
    file \
    findutils \
    git \
    gzip \
    jq \
    less \
    patch \
    python3 \
    ripgrep \
    tar

COPY --from=canon-builder /build/target/release/canon /usr/local/bin/canon
COPY --from=codex-downloader /usr/local/bin/codex /usr/local/bin/codex
COPY docker/entrypoint /usr/local/bin/docker-canon-entrypoint

WORKDIR /scratch/secret/repository

RUN set -eux; \
    chmod 0755 /usr/local/bin/docker-canon-entrypoint; \
    canon help >/dev/null; \
    codex --version >/dev/null; \
    docker-canon-entrypoint help >/dev/null

ENV HOME=/scratch/home \
    TMPDIR=/scratch/tmp \
    TEMP=/scratch/tmp \
    TMP=/scratch/tmp \
    CANON_SECRET_DIR=/scratch/secret/ \
    CANON_SANDBOX_DIR=/scratch/sandbox \
    CODEX_HOME=/scratch/codex-home \
    GIT_CONFIG_COUNT=1 \
    GIT_CONFIG_KEY_0=safe.directory \
    GIT_CONFIG_VALUE_0=*

ENTRYPOINT ["docker-canon-entrypoint"]
CMD ["help"]

FROM runtime-base AS runtime
RUN set -eux; \
    ln -sf /usr/bin/python3 /usr/local/bin/python; \
    python3 --version >/dev/null
