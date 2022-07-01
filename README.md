# **Unofficial** Online Match Protocol Analysis of [5D Chess With Multiverse Time Travel](https://store.steampowered.com/app/1349230/5D_Chess_With_Multiverse_Time_Travel/).

## TOC

- Analysis of messages involved in the protocol is located in `analysis/message.h`.

- Analysis of passcode and its internal representation is located in `analysis/passcode.py`.

- An unofficial online match server written in Rust is located in `5dcserver/` (unfinished yet).

## Build the unofficial online match server

Requires the latest Rust toolchain.

```sh
cd 5dcserver

# Debug build
cargo build
cargo run

# Release build
cargo build --release
cargo run --release
```

Binaries are located in `5dcserver/target/debug/` or `5dcserver/target/release/`.
