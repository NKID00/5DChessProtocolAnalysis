# 5dcserver

An **Unofficial** Online Match Server of [5D Chess With Multiverse Time Travel](https://store.steampowered.com/app/1349230/5D_Chess_With_Multiverse_Time_Travel/).

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

## Copyright

Copyright (C) 2022 NKID00

This program is free software: you can redistribute it and/or modify it under the terms of the GNU Affero General Public License as published by the Free Software Foundation, version 3.

This program is distributed in the hope that it will be useful, but WITHOUT ANY WARRANTY; without even the implied warranty of MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the GNU Affero General Public License for more details.

You should have received a copy of the GNU Affero General Public License along with this program. If not, see <https://www.gnu.org/licenses/>
