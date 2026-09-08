# Build the connector inside the image rather than on the host.
#
# The previous single-stage Dockerfile copied a host-built
# ./target/x86_64-unknown-linux-musl/release/dsp-rs, which needed the musl target and a
# musl linker installed on the developer's machine and always produced an amd64 image.
# Building here needs only Docker, and targets the host architecture — so an arm64
# machine gets an arm64 image instead of one that runs under emulation.
FROM rust:1-alpine AS build

# musl-dev supplies the `cc` the linker invokes; the rest of the toolchain is in the image.
RUN apk add --no-cache musl-dev

# .cargo/config.toml names `x86_64-linux-musl-gcc` — the cross-linker used when building
# from macOS. Inside Alpine the musl linker is plain `cc`, and on an amd64 builder that
# config entry applies to the *host* target, so it has to be overridden or the link fails.
ENV CARGO_TARGET_X86_64_UNKNOWN_LINUX_MUSL_LINKER=cc \
    CARGO_TARGET_AARCH64_UNKNOWN_LINUX_MUSL_LINKER=cc

WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY src ./src
RUN cargo build --release --locked --bin dsp-rs

FROM alpine:latest

WORKDIR /app
COPY --from=build /src/target/release/dsp-rs .

# schemas
COPY third_party/dsp-spec/artifacts/src/main/resources ./third_party/dsp-spec/artifacts/src/main/resources

ENV RUST_BACKTRACE=full
ENV RUST_LOG=debug

CMD ["./dsp-rs"]
