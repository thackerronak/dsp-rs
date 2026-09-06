FROM alpine:latest AS builder
RUN apk add --no-cache rust cargo gcc musl-dev
WORKDIR /app
COPY Cargo.toml Cargo.lock ./
COPY src ./src
COPY third_party ./third_party
RUN cargo build --release

FROM alpine:latest
RUN apk add --no-cache libgcc
WORKDIR /app
COPY --from=builder /app/target/release/dsp-rs .
COPY third_party/dsp-spec/artifacts/src/main/resources ./third_party/dsp-spec/artifacts/src/main/resources
ENV RUST_BACKTRACE=full
ENV RUST_LOG=debug
CMD ["./dsp-rs"]
