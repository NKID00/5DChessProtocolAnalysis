# 5dcserver

An **Unofficial** Online Match Server of [5D Chess With Multiverse Time Travel](https://store.steampowered.com/app/1349230/5D_Chess_With_Multiverse_Time_Travel/).

Highlights:

- Written in Rust
- Asynchronous network and inter-thread communication
- Configurable variant ban
- Defense against hackers that may reset your game

Supports all game features including:

- Query public match list and server match history
- Create, join and play public and private matches
- All variants and random variant
- Clock

## Try it out

```sh
./5dchesswithmultiversetimetravel --server-hostname 5d.nkid00.name
```

For Chinese mainland:

```sh
./5dchesswithmultiversetimetravel --server-hostname 5dc.nkid00.name
```

(you'll probably have the very same experience as the official server, sadly)

## Usage

Server side:

```sh
docker run -p 39005:39005 docker.io/nkid00/5dcserver:latest
```

or

```sh
./5dcserver <CONFIG FILE>
```

See the [default configuration file](./5dcserver.toml) for available options.

Client side:

```sh
./5dchesswithmultiversetimetravel --server-hostname <HOSTNAME> [--server-port <PORT>]
```

The default port is 39005.

## Build

Build with docker:

```sh
docker bulid .
```

Build with the latest Rust toolchain:

```sh
# Debug build
cargo build

# Release build
cargo build -r
```

Binaries are located in `target/debug/` or `target/release/`.

## License

Copyright (C) 2022-2023 NKID00

This program is free software: you can redistribute it and/or modify it under the terms of the GNU Affero General Public License as published by the Free Software Foundation, version 3.

This program is distributed in the hope that it will be useful, but WITHOUT ANY WARRANTY; without even the implied warranty of MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the GNU Affero General Public License for more details.

You should have received a copy of the GNU Affero General Public License along with this program. If not, see <https://www.gnu.org/licenses/>
