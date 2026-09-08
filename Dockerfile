# syntax=docker/dockerfile:1

# Builds a fully static power-agent binary using Alpine's native musl
# toolchain (all 7 target nodes are x86_64, so no cross-compilation matrix
# is needed) and ships it on `scratch`: smallest possible attack surface,
# no shell available for interactive debugging inside the container. That
# trade-off is deliberate -- use `pstate-cli` (built and run directly on a
# node, not containerized) or `kubectl debug node/<name>` for on-node
# investigation instead of `kubectl exec` into this image.
FROM rust:1-alpine AS builder
RUN apk add --no-cache musl-dev build-base perl
WORKDIR /build

# The workspace manifest lists all three crates as members, so Cargo needs
# their Cargo.toml files present to resolve even though only power-agent
# gets built below.
COPY Cargo.toml Cargo.lock ./
COPY crates/cpu-power-hal ./crates/cpu-power-hal
COPY crates/power-agent ./crates/power-agent
COPY crates/pstate-cli ./crates/pstate-cli

RUN cargo build --release -p power-agent

# No USER directive: this binary genuinely needs to run as root to write
# to sysfs (see deploy/manifests/04-daemonset.yaml's securityContext,
# which is the authoritative setting -- Kubernetes' runAsUser always
# overrides whatever the image itself defaults to).
FROM scratch
COPY --from=builder /build/target/release/power-agent /power-agent
ENTRYPOINT ["/power-agent"]
