# **Unofficial** Online Match Protocol Analysis of [5D Chess With Multiverse Time Travel](https://store.steampowered.com/app/1349230/5D_Chess_With_Multiverse_Time_Travel/)

- Analysis of messages involved in the protocol is located in `analysis/message.h`.

- Analysis of passcode and its internal representation is located in `analysis/passcode.py`.

- An unofficial online match server written in Rust is located in `5dcserver/`.

# **Unofficial** Online Match Server

**Highlights**

- Written in Rust

- Asynchronous network and inter-thread communication

**Supported game features**

- Query public match list and server match history

- Create, join and play public and private matches

- Standard and Standard - Turn Zero variants

**Unsupported game features** (work in progress)

- Variants other than Standard and Standard - Turn Zero

- Random variant

- Clock

## Build

Requires the latest Rust toolchain.

```sh
cd 5dcserver

# Debug build
cargo build
cargo run

# Release build
cargo build -r
cargo run -r
```

Binaries are located in `5dcserver/target/debug/` or `5dcserver/target/release/`.
