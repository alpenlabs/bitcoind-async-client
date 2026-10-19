# BitcoinD JSON-RPC Async Client

[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](https://opensource.org/licenses/MIT)
[![License: Apache-2.0](https://img.shields.io/badge/License-Apache-blue.svg)](https://opensource.org/licenses/apache-2-0)
[![ci](https://github.com/alpenlabs/bitcoind-async-client/actions/workflows/lint.yml/badge.svg?event=push)](https://github.com/alpenlabs/bitcoind-async-client/actions)
[![docs](https://img.shields.io/badge/docs-docs.rs-orange)](https://docs.rs/bitcoind-async-client)

## Features

- Async bitcoin Client based on [`bitreq`](https://crates.io/crates/bitreq)
- Supports Bitcoin Core 29, 30 and 31 via mutually exclusive Cargo features (see below)
- PSBT and wallet RPC methods for advanced transaction handling

## Usage

```rust
// NOTE: in production code, don't glob all trait imports.
use bitcoind_async_client::{traits::*, Auth, Client};

let client = Client::new(
    "http://localhost:8332".to_string(),
    Auth::UserPass("username".to_string(), "password".to_string()),
    None,
    None,
    None,
)?;

let blockchain_info = client.get_blockchain_info().await?;
```

## Bitcoin Core versions

| Feature | Bitcoin Core | Tested against | Default |
| --- | --- | --- | --- |
| `29_0` | 29 | 29.0 | |
| `30_2` | 30 | 30.2 | |
| `31_1` | 31 | 31.1 | yes |

For typed RPC methods, enable exactly one version feature matching your daemon's
major version. Clients using only `call_raw` (the `raw_rpc` feature) can disable
all version features.
Every crate in a dependency graph that enables `bitcoind-async-client` features
must agree on that version feature. Cargo features are additive; enabling two
version features fails to compile.

For example, to use Bitcoin Core 30:

```toml
bitcoind-async-client = { version = "0.16", default-features = false, features = ["30_2"] }
```

A version feature does not support other Core majors. Newer daemons may add fields
that older features ignore, while older daemons lack fields that newer features require.

## Contributing

Contributions are generally welcome.
If you intend to make larger changes please discuss them in an issue
before opening a PR to avoid duplicate work and architectural mismatches.

For more information please see [`CONTRIBUTING.md`](/CONTRIBUTING.md).

## License

This work is dual-licensed under MIT and Apache 2.0.
You can choose between one of them if you use this work.
