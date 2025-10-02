use bitcoin::{bip32::Xpriv, block::Header, Address, Block, BlockHash, Network, Transaction, Txid};
use corepc_types::model::{
    GetBlockchainInfo, GetMempoolInfo, GetRawMempool, GetRawMempoolVerbose, GetRawTransaction,
    GetRawTransactionVerbose, GetTransaction, GetTxOut, ListTransactions, ListUnspent,
};
use std::future::Future;

use crate::{
    client::ClientResult,
    types::{
        CreateRawTransactionArguments, CreateRawTransactionInput, CreateRawTransactionOutput,
        GetAddressInfo, ImportDescriptor, ImportDescriptorResult, ListUnspentQueryOptions,
        PreviousTransactionOutput, PsbtBumpFee, PsbtBumpFeeOptions, SignRawTransactionWithWallet,
        SubmitPackage, TestMempoolAccept, WalletCreateFundedPsbt, WalletCreateFundedPsbtOptions,
        WalletProcessPsbtResult,
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
    fn get_raw_mempool(&self) -> impl Future<Output = ClientResult<GetRawMempool>> + Send;

    /// Gets verbose representation of transactions in mempool.
    fn get_raw_mempool_verbose(
        &self,
    ) -> impl Future<Output = ClientResult<GetRawMempoolVerbose>> + Send;

    /// Returns details on the active state of the mempool.
    fn get_mempool_info(&self) -> impl Future<Output = ClientResult<GetMempoolInfo>> + Send;

    /// Gets a raw transaction by its [`Txid`].
    fn get_raw_transaction_verbosity_zero(
        &self,
        txid: &Txid,
    ) -> impl Future<Output = ClientResult<GetRawTransaction>> + Send;

    /// Gets a raw transaction by its [`Txid`].
    fn get_raw_transaction_verbosity_one(
        &self,
        txid: &Txid,
    ) -> impl Future<Output = ClientResult<GetRawTransactionVerbose>> + Send;

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

    /// Lists transactions in the underlying Bitcoin client's wallet.
    ///
    /// # Parameters
    ///
    /// - `count`: The number of transactions to list. If `None`, assumes a maximum of 10
    ///   transactions.
    fn list_transactions(
        &self,
        count: Option<usize>,
    ) -> impl Future<Output = ClientResult<ListTransactions>> + Send;

    /// Lists all wallets in the underlying Bitcoin client.
    fn list_wallets(&self) -> impl Future<Output = ClientResult<Vec<String>>> + Send;

    /// Creates a raw transaction.
    fn create_raw_transaction(
        &self,
        raw_tx: CreateRawTransactionArguments,
    ) -> impl Future<Output = ClientResult<Transaction>> + Send;

    /// Creates and funds a PSBT with inputs and outputs from the wallet.
    ///
    /// Uses the wallet's UTXOs to automatically fund the specified outputs, creating
    /// a partially signed Bitcoin transaction that can be further processed or signed.
    ///
    /// # Parameters
    ///
    /// - `inputs`: Array of specific transaction inputs to include (can be empty for automatic selection).
    /// - `outputs`: Array of transaction outputs, supporting both address-amount pairs and OP_RETURN data.
    /// - `locktime`: Optional locktime for the transaction (0 = no locktime).
    /// - `options`: Optional funding options including fee rate, change address, and confirmation targets.
    /// - `bip32_derivs`: Whether to include BIP32 derivation paths in the PSBT for signing.
    ///
    /// # Returns
    ///
    /// Returns a [`WalletCreateFundedPsbt`] containing the funded PSBT, calculated fee, and change output position.
    ///
    /// # Note
    ///
    /// The returned PSBT is not signed and requires further processing with `wallet_process_psbt`
    /// or `finalize_psbt` before it can be broadcast to the network.
    fn wallet_create_funded_psbt(
        &self,
        inputs: &[CreateRawTransactionInput],
        outputs: &[CreateRawTransactionOutput],
        locktime: Option<u32>,
        options: Option<WalletCreateFundedPsbtOptions>,
        bip32_derivs: Option<bool>,
    ) -> impl Future<Output = ClientResult<WalletCreateFundedPsbt>> + Send;

    /// Returns detailed information about the given address.
    ///
    /// Queries the wallet for comprehensive information about a Bitcoin address,
    /// including ownership status, spending capabilities, and script details.
    /// This is useful for determining if an address belongs to the wallet and
    /// how it can be used for transactions.
    ///
    /// # Parameters
    ///
    /// - `address`: The Bitcoin address to analyze (any valid Bitcoin address format).
    ///
    /// # Returns
    ///
    /// Returns a [`GetAddressInfo`] containing:
    /// - Address ownership status (`is_mine`)
    /// - Watch-only status (`is_watchonly`)
    /// - Spending capability (`solvable`)
    /// - The queried address for confirmation
    ///
    /// # Note
    ///
    /// The address doesn't need to belong to the wallet to query information about it.
    /// However, detailed ownership and spending information will only be available
    /// for addresses that the wallet knows about.
    fn get_address_info(
        &self,
        address: &Address,
    ) -> impl Future<Output = ClientResult<GetAddressInfo>> + Send;

    /// Lists unspent transaction outputs with filtering options.
    ///
    /// Queries the wallet for unspent transaction outputs (UTXOs) with comprehensive
    /// filtering capabilities. This is essential for coin selection, balance calculation,
    /// and preparing transaction inputs. Provides fine-grained control over which
    /// UTXOs are returned based on confirmations, addresses, safety, and amounts.
    ///
    /// # Parameters
    ///
    /// - `min_conf`: Minimum number of confirmations required (default: 1). Use 0 for unconfirmed outputs.
    /// - `max_conf`: Maximum number of confirmations to include (default: 9,999,999). Limits how old UTXOs can be.
    /// - `addresses`: Optional list of specific addresses to filter by. If provided, only UTXOs from these addresses are returned.
    /// - `include_unsafe`: Whether to include outputs that are not safe to spend (default: true). Unsafe outputs include unconfirmed transactions from external keys.
    /// - `query_options`: Additional filtering options for amount ranges and result limits via [`ListUnspentQueryOptions`].
    ///
    /// # Returns
    ///
    /// Returns a vector of [`ListUnspent`] containing:
    /// - Transaction ID and output index (`txid`, `vout`)
    /// - Bitcoin address and amount (`address`, `amount`)
    /// - Confirmation count and safety status (`confirmations`, `safe`)
    /// - Spendability information (`spendable`, `solvable`)
    /// - Script details (`script_pubkey`, `label`)
    ///
    /// # Note
    ///
    /// UTXOs must satisfy ALL specified criteria to be included in results.
    /// This method is commonly used for wallet balance calculation and transaction
    /// preparation. Consider using `query_options` for amount-based filtering
    /// to optimize coin selection strategies.
    fn list_unspent(
        &self,
        min_conf: Option<u32>,
        max_conf: Option<u32>,
        addresses: Option<&[Address]>,
        include_unsafe: Option<bool>,
        query_options: Option<ListUnspentQueryOptions>,
    ) -> impl Future<Output = ClientResult<ListUnspent>> + Send;
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

    /// Updates a PSBT with input information from the wallet and optionally signs it.
    ///
    /// # Parameters
    ///
    /// - `psbt`: The PSBT to process as a base64 string.
    /// - `sign`: Whether to sign the transaction (default: true).
    /// - `sighashtype`: Optional signature hash type to use.
    /// - `bip32_derivs`: Whether to include BIP32 derivation paths.
    ///
    /// # Returns
    ///
    /// Returns a [`WalletProcessPsbtResult`] with the processed PSBT and completion status.
    fn wallet_process_psbt(
        &self,
        psbt: &str,
        sign: Option<bool>,
        sighashtype: Option<crate::types::SighashType>,
        bip32_derivs: Option<bool>,
    ) -> impl Future<Output = ClientResult<WalletProcessPsbtResult>> + Send;

    /// Bumps the fee of an opt-in-RBF transaction, replacing it with a new transaction.
    ///
    /// # Parameters
    ///
    /// - `txid`: The transaction ID to be bumped.
    /// - `options`: Optional fee bumping options including:
    ///   - `conf_target`: Confirmation target in blocks
    ///   - `fee_rate`: Fee rate in sat/vB (overrides conf_target)
    ///   - `replaceable`: Whether the new transaction should be BIP-125 replaceable
    ///   - `estimate_mode`: Fee estimate mode ("unset", "economical", "conservative")
    ///   - `outputs`: New transaction outputs to replace existing ones
    ///   - `original_change_index`: Index of change output to recycle from original transaction
    ///
    /// # Returns
    ///
    /// Returns a [`PsbtBumpFee`] containing the new PSBT and fee information.
    ///
    /// # Note
    ///
    /// The transaction must be BIP-125 opt-in replaceable and the new fee rate must be
    /// higher than the original.
    fn psbt_bump_fee(
        &self,
        txid: &Txid,
        options: Option<PsbtBumpFeeOptions>,
    ) -> impl Future<Output = ClientResult<PsbtBumpFee>> + Send;
}
