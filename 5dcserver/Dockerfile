FROM docker.io/rust:1.71.0-alpine as builder
RUN apk add musl-dev
ADD . /app
WORKDIR /app
RUN cargo build --release

FROM gcr.io/distroless/static:latest
COPY --from=builder /app/target/release/5dcserver /5dcserver
COPY --from=builder /app/5dcserver.toml /5dcserver.toml
ENTRYPOINT ["/5dcserver"]
CMD ["/5dcserver.toml"]
