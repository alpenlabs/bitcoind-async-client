use bitcoin::{bip32::Xpriv, block::Header, Address, Block, BlockHash, Network, Transaction, Txid};
use std::future::Future;

use crate::{
    client::ClientResult,
    types::{
        CreateRawTransaction, GetBlockchainInfo, GetMempoolInfo, GetRawTransactionVerbosityOne,
        GetRawTransactionVerbosityZero, GetTransaction, GetTxOut, ImportDescriptor,
        ImportDescriptorResult, ListTransactions, ListUnspent, PreviousTransactionOutput,
        SignRawTransactionWithWallet, SubmitPackage, TestMempoolAccept,
    },
};

/// Basic functionality that any Bitcoin client that interacts with the
/// Bitcoin network should provide.
///
/// # Note
///
/// This is a fully `async` trait. The user should be responsible for
/// handling the `async` nature of the trait methods. And if implementing
/// this trait for a specific type that is not `async`, the user should
/// consider wrapping with [`tokio`](https://tokio.rs)'s
/// [`spawn_blocking`](https://docs.rs/tokio/latest/tokio/task/fn.spawn_blocking.html) or any other method.
pub trait Reader {
    /// Estimates the approximate fee per kilobyte needed for a transaction
    /// to begin confirmation within conf_target blocks if possible and return
    /// the number of blocks for which the estimate is valid.
    ///
    /// # Parameters
    ///
    /// - `conf_target`: Confirmation target in blocks.
    ///
    /// # Note
    ///
    /// Uses virtual transaction size as defined in
    /// [BIP 141](https://github.com/bitcoin/bips/blob/master/bip-0141.mediawiki)
    /// (witness data is discounted).
    ///
    /// By default uses the estimate mode of `CONSERVATIVE` which is the
    /// default in Bitcoin Core v27.
    fn estimate_smart_fee(
        &self,
        conf_target: u16,
    ) -> impl Future<Output = ClientResult<u64>> + Send;

    /// Gets a [`Header`] with the given hash.
    fn get_block_header(
        &self,
        hash: &BlockHash,
    ) -> impl Future<Output = ClientResult<Header>> + Send;

    /// Gets a [`Block`] with the given hash.
    fn get_block(&self, hash: &BlockHash) -> impl Future<Output = ClientResult<Block>> + Send;

    /// Gets a block height with the given hash.
    fn get_block_height(&self, hash: &BlockHash) -> impl Future<Output = ClientResult<u64>> + Send;

    /// Gets a [`Header`] at given height.
    fn get_block_header_at(&self, height: u64)
        -> impl Future<Output = ClientResult<Header>> + Send;

    /// Gets a [`Block`] at given height.
    fn get_block_at(&self, height: u64) -> impl Future<Output = ClientResult<Block>> + Send;

    /// Gets the height of the most-work fully-validated chain.
    ///
    /// # Note
    ///
    /// The genesis block has a height of 0.
    fn get_block_count(&self) -> impl Future<Output = ClientResult<u64>> + Send;

    /// Gets the [`BlockHash`] at given height.
    fn get_block_hash(&self, height: u64) -> impl Future<Output = ClientResult<BlockHash>> + Send;

    /// Gets various state info regarding blockchain processing.
    fn get_blockchain_info(&self) -> impl Future<Output = ClientResult<GetBlockchainInfo>> + Send;

    /// Gets the timestamp in the block header of the current best block in bitcoin.
    ///
    /// # Note
    ///
    /// Time is Unix epoch time in seconds.
    fn get_current_timestamp(&self) -> impl Future<Output = ClientResult<u32>> + Send;

    /// Gets all transaction ids in mempool.
    fn get_raw_mempool(&self) -> impl Future<Output = ClientResult<Vec<Txid>>> + Send;

    /// Returns details on the active state of the mempool.
    fn get_mempool_info(&self) -> impl Future<Output = ClientResult<GetMempoolInfo>> + Send;

    /// Gets a raw transaction by its [`Txid`].
    fn get_raw_transaction_verbosity_zero(
        &self,
        txid: &Txid,
    ) -> impl Future<Output = ClientResult<GetRawTransactionVerbosityZero>> + Send;

    /// Gets a raw transaction by its [`Txid`].
    fn get_raw_transaction_verbosity_one(
        &self,
        txid: &Txid,
    ) -> impl Future<Output = ClientResult<GetRawTransactionVerbosityOne>> + Send;

    /// Returns details about an unspent transaction output.
    fn get_tx_out(
        &self,
        txid: &Txid,
        vout: u32,
        include_mempool: bool,
    ) -> impl Future<Output = ClientResult<GetTxOut>> + Send;

    /// Gets the underlying [`Network`] information.
    fn network(&self) -> impl Future<Output = ClientResult<Network>> + Send;
}

/// Broadcasting functionality that any Bitcoin client that interacts with the
/// Bitcoin network should provide.
///
/// # Note
///
/// This is a fully `async` trait. The user should be responsible for
/// handling the `async` nature of the trait methods. And if implementing
/// this trait for a specific type that is not `async`, the user should
/// consider wrapping with [`tokio`](https://tokio.rs)'s
/// [`spawn_blocking`](https://docs.rs/tokio/latest/tokio/task/fn.spawn_blocking.html)
/// or any other method.
pub trait Broadcaster {
    /// Sends a raw transaction to the network.
    ///
    /// # Parameters
    ///
    /// - `tx`: The raw transaction to send. This should be a byte array containing the serialized
    ///   raw transaction data.
    fn send_raw_transaction(
        &self,
        tx: &Transaction,
    ) -> impl Future<Output = ClientResult<Txid>> + Send;

    /// Tests if a raw transaction is valid.
    fn test_mempool_accept(
        &self,
        tx: &Transaction,
    ) -> impl Future<Output = ClientResult<Vec<TestMempoolAccept>>> + Send;

    /// Submit a package of raw transactions (serialized, hex-encoded) to local node.
    ///
    /// The package will be validated according to consensus and mempool policy rules. If any
    /// transaction passes, it will be accepted to mempool. This RPC is experimental and the
    /// interface may be unstable. Refer to doc/policy/packages.md for documentation on package
    /// policies.
    ///
    /// # Warning
    ///
    /// Successful submission does not mean the transactions will propagate throughout the network.
    fn submit_package(
        &self,
        txs: &[Transaction],
    ) -> impl Future<Output = ClientResult<SubmitPackage>> + Send;
}

/// Wallet functionality that any Bitcoin client **without private keys** that
/// interacts with the Bitcoin network should provide.
///
/// For signing transactions, see [`Signer`].
///
/// # Note
///
/// This is a fully `async` trait. The user should be responsible for
/// handling the `async` nature of the trait methods. And if implementing
/// this trait for a specific type that is not `async`, the user should
/// consider wrapping with [`tokio`](https://tokio.rs)'s
/// [`spawn_blocking`](https://docs.rs/tokio/latest/tokio/task/fn.spawn_blocking.html)
/// or any other method.
pub trait Wallet {
    /// Generates new address under own control for the underlying Bitcoin
    /// client's wallet.
    fn get_new_address(&self) -> impl Future<Output = ClientResult<Address>> + Send;

    /// Gets information related to a transaction.
    ///
    /// # Note
    ///
    /// This assumes that the transaction is present in the underlying Bitcoin
    /// client's wallet.
    fn get_transaction(
        &self,
        txid: &Txid,
    ) -> impl Future<Output = ClientResult<GetTransaction>> + Send;

    /// Gets all Unspent Transaction Outputs (UTXOs) for the underlying Bitcoin
    /// client's wallet.
    fn get_utxos(&self) -> impl Future<Output = ClientResult<Vec<ListUnspent>>> + Send;

    /// Lists transactions in the underlying Bitcoin client's wallet.
    ///
    /// # Parameters
    ///
    /// - `count`: The number of transactions to list. If `None`, assumes a maximum of 10
    ///   transactions.
    fn list_transactions(
        &self,
        count: Option<usize>,
    ) -> impl Future<Output = ClientResult<Vec<ListTransactions>>> + Send;

    /// Lists all wallets in the underlying Bitcoin client.
    fn list_wallets(&self) -> impl Future<Output = ClientResult<Vec<String>>> + Send;

    /// Creates a raw transaction.
    fn create_raw_transaction(
        &self,
        raw_tx: CreateRawTransaction,
    ) -> impl Future<Output = ClientResult<Transaction>> + Send;
}

/// Signing functionality that any Bitcoin client **with private keys** that
/// interacts with the Bitcoin network should provide.
///
/// # Note
///
/// This is a fully `async` trait. The user should be responsible for
/// handling the `async` nature of the trait methods. And if implementing
/// this trait for a specific type that is not `async`, the user should
/// consider wrapping with [`tokio`](https://tokio.rs)'s
/// [`spawn_blocking`](https://docs.rs/tokio/latest/tokio/task/fn.spawn_blocking.html)
/// or any other method.
pub trait Signer {
    /// Signs a transaction using the keys available in the underlying Bitcoin
    /// client's wallet and returns a signed transaction.
    ///
    /// # Note
    ///
    /// The returned signed transaction might not be consensus-valid if it
    /// requires additional signatures, such as in a multisignature context.
    fn sign_raw_transaction_with_wallet(
        &self,
        tx: &Transaction,
        prev_outputs: Option<Vec<PreviousTransactionOutput>>,
    ) -> impl Future<Output = ClientResult<SignRawTransactionWithWallet>> + Send;

    /// Gets the underlying [`Xpriv`] from the wallet.
    fn get_xpriv(&self) -> impl Future<Output = ClientResult<Option<Xpriv>>> + Send;

    /// Imports the descriptors into the wallet.
    fn import_descriptors(
        &self,
        descriptors: Vec<ImportDescriptor>,
        wallet_name: String,
    ) -> impl Future<Output = ClientResult<Vec<ImportDescriptorResult>>> + Send;
}
