# BitcoinD JSON-RPC Async Client

[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](https://opensource.org/licenses/MIT)
[![License: Apache-2.0](https://img.shields.io/badge/License-Apache-blue.svg)](https://opensource.org/licenses/apache-2-0)
[![ci](https://github.com/alpenlabs/bitcoind-async-client/actions/workflows/lint.yml/badge.svg?event=push)](https://github.com/alpenlabs/bitcoind-async-client/actions)
[![docs](https://img.shields.io/badge/docs-docs.rs-orange)](https://docs.rs/bitcoind-async-client)

## Features

- Async bitcoin Client based on [`reqwest`](https://crates.io/crates/reqwest)
- Supports bitcoin core versions `28.0` and above
- PSBT and wallet RPC methods for advanced transaction handling

## Usage

```rust
// NOTE: in production code, don't glob all trait imports.
use bitcoind_async_client::{Client, traits::* }; 

let client = Client::new("http://localhost:8332", "username", "password", None, None).await?;

let blockchain_info = client.get_blockchain_info().await?;
```

## Contributing

Contributions are generally welcome.
If you intend to make larger changes please discuss them in an issue
before opening a PR to avoid duplicate work and architectural mismatches.

For more information please see [`CONTRIBUTING.md`](/CONTRIBUTING.md).

## License

This work is dual-licensed under MIT and Apache 2.0.
You can choose between one of them if you use this work.
