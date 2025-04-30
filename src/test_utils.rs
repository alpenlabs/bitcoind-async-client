#[cfg(test)]
pub mod corepc_node_helpers {
    use std::env;

    use bitcoin::{Address, BlockHash};
    use corepc_node::Node;

    use crate::Client;

    /// Get the authentication credentials for a given `bitcoind` instance.
    fn get_auth(bitcoind: &Node) -> (String, String) {
        let params = &bitcoind.params;
        let cookie_values = params.get_cookie_values().unwrap().unwrap();
        (cookie_values.user, cookie_values.password)
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
        let bitcoind = Node::from_downloaded().unwrap();
        let url = bitcoind.rpc_url();
        let (user, password) = get_auth(&bitcoind);
        let client = Client::new(url, user, password).unwrap();
        (bitcoind, client)
    }
}
