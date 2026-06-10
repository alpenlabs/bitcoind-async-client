#[cfg(test)]
pub mod corepc_node_helpers {
    use std::env;

    use bitcoin::{Address, BlockHash};
    use corepc_node::{Conf, Node};

    use crate::{error::ClientError, Auth, Client, ClientResult};

    /// Get the authentication credentials for a given `bitcoind` instance.
    fn get_auth(bitcoind: &Node) -> Auth {
        Auth::CookieFile(bitcoind.params.cookie_file.clone())
    }

    /// Mine a number of blocks of a given size `count`, which may be specified to a given coinbase
    /// `address`.
    pub fn mine_blocks(
        bitcoind: &Node,
        count: usize,
        address: Option<Address>,
    ) -> anyhow::Result<Vec<BlockHash>> {
        let coinbase_address = match address {
            Some(address) => address,
            None => bitcoind.client.new_address()?,
        };
        let block_hashes = bitcoind
            .client
            .generate_to_address(count as _, &coinbase_address)?
            .0
            .iter()
            .map(|hash| hash.parse::<BlockHash>())
            .collect::<Result<Vec<_>, _>>()?;
        Ok(block_hashes)
    }

    pub fn get_bitcoind_and_client() -> (Node, Client) {
        // setting the ENV variable `BITCOIN_XPRIV_RETRIEVABLE` to retrieve the xpriv
        unsafe {
            env::set_var("BITCOIN_XPRIV_RETRIEVABLE", "true");
        }
        let mut conf = Conf::default();
        conf.args.push("-txindex=1");
        let bitcoind = Node::from_downloaded_with_conf(&conf).unwrap();

        let url = bitcoind.rpc_url();
        let client = Client::new(url, get_auth(&bitcoind), None, None, None).unwrap();
        (bitcoind, client)
    }

    pub fn assert_max_burn_amount_rejected<T>(result: ClientResult<T>, method: &str) {
        match result {
            Err(ClientError::Server(_, message)) => assert!(
                message.contains("maxburnamount") || message.contains("max-burn-amount"),
                "{method} should reject the default maxburnamount, got: {message}"
            ),
            Err(error) => panic!("{method} returned an unexpected error: {error:?}"),
            Ok(_) => panic!("{method} unexpectedly accepted a nonzero OP_RETURN output"),
        }
    }
}
