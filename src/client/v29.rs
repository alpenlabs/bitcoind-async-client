//! This module contains the implementation of the [`Client`] for Bitcoin Core v29.

use std::env::var;

use bitcoin::{
    bip32::Xpriv,
    block::Header,
    consensus::{self, encode::serialize_hex},
    Address, Block, BlockHash, Network, Transaction, Txid,
};
use corepc_types::model;
use corepc_types::v29::{
    CreateWallet, GetAddressInfo, GetBlockHeaderVerbose, GetBlockVerboseOne, GetBlockVerboseZero,
    GetBlockchainInfo, GetMempoolInfo, GetNewAddress, GetRawMempool, GetRawMempoolVerbose,
    GetRawTransaction, GetRawTransactionVerbose, GetTransaction, GetTxOut, ImportDescriptors,
    ListDescriptors, ListTransactions, ListUnspent, PsbtBumpFee, SignRawTransactionWithWallet,
    SubmitPackage, TestMempoolAccept, WalletCreateFundedPsbt, WalletProcessPsbt,
};
use serde_json::value::{RawValue, Value};
use tracing::*;

use crate::{
    client::Client,
    error::ClientError,
    to_value,
    traits::{Broadcaster, Reader, Signer, Wallet},
    types::{
        CreateRawTransactionArguments, CreateRawTransactionInput, CreateRawTransactionOutput,
        CreateWalletArguments, ImportDescriptorInput, ListUnspentQueryOptions,
        PreviousTransactionOutput, PsbtBumpFeeOptions, SighashType, WalletCreateFundedPsbtOptions,
    },
    ClientResult,
};

impl Reader for Client {
    async fn estimate_smart_fee(&self, conf_target: u16) -> ClientResult<u64> {
        let result = self
            .call::<Box<RawValue>>("estimatesmartfee", &[to_value(conf_target)?])
            .await?
            .to_string();

        let result_map: Value = result.parse::<Value>()?;

        let btc_vkb = result_map
            .get("feerate")
            .unwrap_or(&"0.00001".parse::<Value>().unwrap())
            .as_f64()
            .unwrap();

        // convert to sat/vB and round up
        Ok((btc_vkb * 100_000_000.0 / 1000.0) as u64)
    }

    async fn get_block_header(&self, hash: &BlockHash) -> ClientResult<Header> {
        let get_block_header = self
            .call::<GetBlockHeaderVerbose>(
                "getblockheader",
                &[to_value(hash.to_string())?, to_value(false)?],
            )
            .await?;
        let header = get_block_header
            .block_header()
            .map_err(|err| ClientError::Other(format!("header decode: {err}")))?;
        Ok(header)
    }

    async fn get_block(&self, hash: &BlockHash) -> ClientResult<Block> {
        let get_block = self
            .call::<GetBlockVerboseZero>("getblock", &[to_value(hash.to_string())?, to_value(0)?])
            .await?;
        let block = get_block
            .into_model()
            .map_err(|e| ClientError::Parse(e.to_string()))?
            .0;
        Ok(block)
    }

    async fn get_block_height(&self, hash: &BlockHash) -> ClientResult<u64> {
        let block_verobose = self
            .call::<GetBlockVerboseOne>("getblock", &[to_value(hash.to_string())?])
            .await?;

        let block_height = block_verobose.height as u64;
        Ok(block_height)
    }

    async fn get_block_header_at(&self, height: u64) -> ClientResult<Header> {
        let hash = self.get_block_hash(height).await?;
        self.get_block_header(&hash).await
    }

    async fn get_block_at(&self, height: u64) -> ClientResult<Block> {
        let hash = self.get_block_hash(height).await?;
        self.get_block(&hash).await
    }

    async fn get_block_count(&self) -> ClientResult<u64> {
        self.call::<u64>("getblockcount", &[]).await
    }

    async fn get_block_hash(&self, height: u64) -> ClientResult<BlockHash> {
        self.call::<BlockHash>("getblockhash", &[to_value(height)?])
            .await
    }

    async fn get_blockchain_info(&self) -> ClientResult<model::GetBlockchainInfo> {
        let res = self
            .call::<GetBlockchainInfo>("getblockchaininfo", &[])
            .await?;
        res.into_model()
            .map_err(|e| ClientError::Parse(e.to_string()))
    }

    async fn get_current_timestamp(&self) -> ClientResult<u32> {
        let best_block_hash = self.call::<BlockHash>("getbestblockhash", &[]).await?;
        let block = self.get_block(&best_block_hash).await?;
        Ok(block.header.time)
    }

    async fn get_raw_mempool(&self) -> ClientResult<model::GetRawMempool> {
        let resp = self.call::<GetRawMempool>("getrawmempool", &[]).await?;
        resp.into_model()
            .map_err(|e| ClientError::Parse(e.to_string()))
    }

    async fn get_raw_mempool_verbose(&self) -> ClientResult<model::GetRawMempoolVerbose> {
        let resp = self
            .call::<GetRawMempoolVerbose>("getrawmempool", &[to_value(true)?])
            .await?;

        resp.into_model()
            .map_err(|e| ClientError::Parse(e.to_string()))
    }

    async fn get_mempool_info(&self) -> ClientResult<model::GetMempoolInfo> {
        let resp = self.call::<GetMempoolInfo>("getmempoolinfo", &[]).await?;
        resp.into_model()
            .map_err(|e| ClientError::Parse(e.to_string()))
    }

    async fn get_raw_transaction_verbosity_zero(
        &self,
        txid: &Txid,
    ) -> ClientResult<model::GetRawTransaction> {
        let resp = self
            .call::<GetRawTransaction>(
                "getrawtransaction",
                &[to_value(txid.to_string())?, to_value(0)?],
            )
            .await
            .map_err(|e| ClientError::Parse(e.to_string()))?;
        resp.into_model()
            .map_err(|e| ClientError::Parse(e.to_string()))
    }

    async fn get_raw_transaction_verbosity_one(
        &self,
        txid: &Txid,
    ) -> ClientResult<model::GetRawTransactionVerbose> {
        let resp = self
            .call::<GetRawTransactionVerbose>(
                "getrawtransaction",
                &[to_value(txid.to_string())?, to_value(1)?],
            )
            .await
            .map_err(|e| ClientError::Parse(e.to_string()))?;
        resp.into_model()
            .map_err(|e| ClientError::Parse(e.to_string()))
    }

    async fn get_tx_out(
        &self,
        txid: &Txid,
        vout: u32,
        include_mempool: bool,
    ) -> ClientResult<model::GetTxOut> {
        let resp = self
            .call::<GetTxOut>(
                "gettxout",
                &[
                    to_value(txid.to_string())?,
                    to_value(vout)?,
                    to_value(include_mempool)?,
                ],
            )
            .await
            .map_err(|e| ClientError::Parse(e.to_string()))?;
        resp.into_model()
            .map_err(|e| ClientError::Parse(e.to_string()))
    }

    async fn network(&self) -> ClientResult<Network> {
        self.call::<GetBlockchainInfo>("getblockchaininfo", &[])
            .await?
            .chain
            .parse::<Network>()
            .map_err(|e| ClientError::Parse(e.to_string()))
    }
}

impl Broadcaster for Client {
    async fn send_raw_transaction(&self, tx: &Transaction) -> ClientResult<Txid> {
        let txstr = serialize_hex(tx);
        trace!(txstr = %txstr, "Sending raw transaction");
        match self
            .call::<Txid>("sendrawtransaction", &[to_value(txstr)?])
            .await
        {
            Ok(txid) => {
                trace!(?txid, "Transaction sent");
                Ok(txid)
            }
            Err(ClientError::Server(i, s)) => match i {
                // Dealing with known and common errors
                -27 => Ok(tx.compute_txid()), // Tx already in chain
                _ => Err(ClientError::Server(i, s)),
            },
            Err(e) => Err(ClientError::Other(e.to_string())),
        }
    }

    async fn test_mempool_accept(
        &self,
        tx: &Transaction,
    ) -> ClientResult<model::TestMempoolAccept> {
        let txstr = serialize_hex(tx);
        trace!(%txstr, "Testing mempool accept");
        let resp = self
            .call::<TestMempoolAccept>("testmempoolaccept", &[to_value([txstr])?])
            .await?;
        resp.into_model()
            .map_err(|e| ClientError::Parse(e.to_string()))
    }

    async fn submit_package(&self, txs: &[Transaction]) -> ClientResult<model::SubmitPackage> {
        let txstrs: Vec<String> = txs.iter().map(serialize_hex).collect();
        let resp = self
            .call::<SubmitPackage>("submitpackage", &[to_value(txstrs)?])
            .await?;
        trace!(?resp, "Got submit package response");

        resp.into_model()
            .map_err(|e| ClientError::Parse(e.to_string()))
    }
}

impl Wallet for Client {
    async fn get_new_address(&self) -> ClientResult<Address> {
        let address_unchecked = self
            .call::<GetNewAddress>("getnewaddress", &[])
            .await?
            .0
            .parse::<Address<_>>()
            .map_err(|e| ClientError::Parse(e.to_string()))?
            .assume_checked();
        Ok(address_unchecked)
    }
    async fn get_transaction(&self, txid: &Txid) -> ClientResult<model::GetTransaction> {
        let resp = self
            .call::<GetTransaction>("gettransaction", &[to_value(txid.to_string())?])
            .await?;
        resp.into_model()
            .map_err(|e| ClientError::Parse(e.to_string()))
    }

    async fn list_transactions(
        &self,
        count: Option<usize>,
    ) -> ClientResult<model::ListTransactions> {
        let resp = self
            .call::<ListTransactions>("listtransactions", &[to_value(count)?])
            .await?;
        resp.into_model()
            .map_err(|e| ClientError::Parse(e.to_string()))
    }

    async fn list_wallets(&self) -> ClientResult<Vec<String>> {
        self.call::<Vec<String>>("listwallets", &[]).await
    }

    async fn create_raw_transaction(
        &self,
        raw_tx: CreateRawTransactionArguments,
    ) -> ClientResult<Transaction> {
        let raw_tx = self
            .call::<String>(
                "createrawtransaction",
                &[to_value(raw_tx.inputs)?, to_value(raw_tx.outputs)?],
            )
            .await?;
        trace!(%raw_tx, "Created raw transaction");
        consensus::encode::deserialize_hex(&raw_tx)
            .map_err(|e| ClientError::Other(format!("Failed to deserialize raw transaction: {e}")))
    }

    async fn wallet_create_funded_psbt(
        &self,
        inputs: &[CreateRawTransactionInput],
        outputs: &[CreateRawTransactionOutput],
        locktime: Option<u32>,
        options: Option<WalletCreateFundedPsbtOptions>,
        bip32_derivs: Option<bool>,
    ) -> ClientResult<model::WalletCreateFundedPsbt> {
        let resp = self
            .call::<WalletCreateFundedPsbt>(
                "walletcreatefundedpsbt",
                &[
                    to_value(inputs)?,
                    to_value(outputs)?,
                    to_value(locktime.unwrap_or(0))?,
                    to_value(options.unwrap_or_default())?,
                    to_value(bip32_derivs)?,
                ],
            )
            .await?;
        resp.into_model()
            .map_err(|e| ClientError::Parse(e.to_string()))
    }

    async fn get_address_info(&self, address: &Address) -> ClientResult<model::GetAddressInfo> {
        trace!(address = %address, "Getting address info");
        let resp = self
            .call::<GetAddressInfo>("getaddressinfo", &[to_value(address.to_string())?])
            .await?;
        resp.into_model()
            .map_err(|e| ClientError::Parse(e.to_string()))
    }

    async fn list_unspent(
        &self,
        min_conf: Option<u32>,
        max_conf: Option<u32>,
        addresses: Option<&[Address]>,
        include_unsafe: Option<bool>,
        query_options: Option<ListUnspentQueryOptions>,
    ) -> ClientResult<model::ListUnspent> {
        let addr_strings: Vec<String> = addresses
            .map(|addrs| addrs.iter().map(|a| a.to_string()).collect())
            .unwrap_or_default();

        let mut params = vec![
            to_value(min_conf.unwrap_or(1))?,
            to_value(max_conf.unwrap_or(9_999_999))?,
            to_value(addr_strings)?,
            to_value(include_unsafe.unwrap_or(true))?,
        ];

        if let Some(query_options) = query_options {
            params.push(to_value(query_options)?);
        }

        let resp = self.call::<ListUnspent>("listunspent", &params).await?;
        trace!(?resp, "Got UTXOs");

        resp.into_model()
            .map_err(|e| ClientError::Parse(e.to_string()))
    }
}

impl Signer for Client {
    async fn sign_raw_transaction_with_wallet(
        &self,
        tx: &Transaction,
        prev_outputs: Option<Vec<PreviousTransactionOutput>>,
    ) -> ClientResult<model::SignRawTransactionWithWallet> {
        let tx_hex = serialize_hex(tx);
        trace!(tx_hex = %tx_hex, "Signing transaction");
        trace!(?prev_outputs, "Signing transaction with previous outputs");
        let resp = self
            .call::<SignRawTransactionWithWallet>(
                "signrawtransactionwithwallet",
                &[to_value(tx_hex)?, to_value(prev_outputs)?],
            )
            .await?;
        resp.into_model()
            .map_err(|e| ClientError::Parse(e.to_string()))
    }

    async fn get_xpriv(&self) -> ClientResult<Option<Xpriv>> {
        // If the ENV variable `BITCOIN_XPRIV_RETRIEVABLE` is not set, we return `None`
        if var("BITCOIN_XPRIV_RETRIEVABLE").is_err() {
            return Ok(None);
        }

        let descriptors = self
            .call::<ListDescriptors>("listdescriptors", &[to_value(true)?]) // true is the xpriv, false is the xpub
            .await?
            .descriptors;
        if descriptors.is_empty() {
            return Err(ClientError::Other("No descriptors found".to_string()));
        }

        // We are only interested in the one that contains `tr(`
        let descriptor = descriptors
            .iter()
            .find(|d| d.descriptor.contains("tr("))
            .map(|d| d.descriptor.clone())
            .ok_or(ClientError::Xpriv)?;

        // Now we extract the xpriv from the `tr()` up to the first `/`
        let xpriv_str = descriptor
            .split("tr(")
            .nth(1)
            .ok_or(ClientError::Xpriv)?
            .split("/")
            .next()
            .ok_or(ClientError::Xpriv)?;

        let xpriv = xpriv_str.parse::<Xpriv>().map_err(|_| ClientError::Xpriv)?;
        Ok(Some(xpriv))
    }

    async fn import_descriptors(
        &self,
        descriptors: Vec<ImportDescriptorInput>,
        wallet_name: String,
    ) -> ClientResult<ImportDescriptors> {
        let wallet_args = CreateWalletArguments {
            name: wallet_name,
            load_on_startup: Some(true),
        };

        // TODO: this should check for -35 error code which is good,
        //       means that is already created
        let _wallet_create = self
            .call::<CreateWallet>("createwallet", &[to_value(wallet_args.clone())?])
            .await;
        // TODO: this should check for -35 error code which is good, -18 is bad.
        let _wallet_load = self
            .call::<CreateWallet>("loadwallet", &[to_value(wallet_args)?])
            .await;

        let result = self
            .call::<ImportDescriptors>("importdescriptors", &[to_value(descriptors)?])
            .await?;
        Ok(result)
    }

    async fn wallet_process_psbt(
        &self,
        psbt: &str,
        sign: Option<bool>,
        sighashtype: Option<SighashType>,
        bip32_derivs: Option<bool>,
    ) -> ClientResult<model::WalletProcessPsbt> {
        let mut params = vec![to_value(psbt)?, to_value(sign.unwrap_or(true))?];

        if let Some(sighashtype) = sighashtype {
            params.push(to_value(sighashtype)?);
        }

        if let Some(bip32_derivs) = bip32_derivs {
            params.push(to_value(bip32_derivs)?);
        }

        let resp = self
            .call::<WalletProcessPsbt>("walletprocesspsbt", &params)
            .await?;
        resp.into_model()
            .map_err(|e| ClientError::Parse(e.to_string()))
    }

    async fn psbt_bump_fee(
        &self,
        txid: &Txid,
        options: Option<PsbtBumpFeeOptions>,
    ) -> ClientResult<model::PsbtBumpFee> {
        let mut params = vec![to_value(txid.to_string())?];

        if let Some(options) = options {
            params.push(to_value(options)?);
        }

        let resp = self.call::<PsbtBumpFee>("psbtbumpfee", &params).await?;
        resp.into_model()
            .map_err(|e| ClientError::Parse(e.to_string()))
    }
}

#[cfg(test)]
mod test {

    use std::sync::Once;

    use bitcoin::{hashes::Hash, transaction, Amount, FeeRate, NetworkKind};
    use corepc_types::v29::ImportDescriptorsResult;
    use tracing_subscriber::{fmt, layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

    use super::*;
    use crate::{
        test_utils::corepc_node_helpers::{get_bitcoind_and_client, mine_blocks},
        types::{CreateRawTransactionInput, CreateRawTransactionOutput},
    };

    /// 50 BTC in [`Network::Regtest`].
    const COINBASE_AMOUNT: Amount = Amount::from_sat(50 * 100_000_000);

    /// Only attempts to start tracing once.
    fn init_tracing() {
        static INIT: Once = Once::new();

        INIT.call_once(|| {
            tracing_subscriber::registry()
                .with(fmt::layer())
                .with(EnvFilter::from_default_env())
                .try_init()
                .ok();
        });
    }

    #[tokio::test()]
    async fn client_works() {
        init_tracing();

        let (bitcoind, client) = get_bitcoind_and_client();

        // network
        let got = client.network().await.unwrap();
        let expected = Network::Regtest;

        assert_eq!(expected, got);
        // get_blockchain_info
        let get_blockchain_info = client.get_blockchain_info().await.unwrap();
        assert_eq!(get_blockchain_info.blocks, 0);

        // get_current_timestamp
        let _ = client
            .get_current_timestamp()
            .await
            .expect("must be able to get current timestamp");

        let blocks = mine_blocks(&bitcoind, 101, None).unwrap();

        // get_block
        let expected = blocks.last().unwrap();
        let got = client.get_block(expected).await.unwrap().block_hash();
        assert_eq!(*expected, got);

        // get_block_at
        let target_height = blocks.len() as u64;
        let expected = blocks.last().unwrap();
        let got = client
            .get_block_at(target_height)
            .await
            .unwrap()
            .block_hash();
        assert_eq!(*expected, got);

        // get_block_count
        let expected = blocks.len() as u64;
        let got = client.get_block_count().await.unwrap();
        assert_eq!(expected, got);

        // get_block_hash
        let target_height = blocks.len() as u64;
        let expected = blocks.last().unwrap();
        let got = client.get_block_hash(target_height).await.unwrap();
        assert_eq!(*expected, got);

        // get_new_address
        let address = client.get_new_address().await.unwrap();
        let txid = client
            .call::<String>(
                "sendtoaddress",
                &[to_value(address.to_string()).unwrap(), to_value(1).unwrap()],
            )
            .await
            .unwrap()
            .parse::<Txid>()
            .unwrap();

        // get_transaction
        let tx = client.get_transaction(&txid).await.unwrap().tx;
        let got = client.send_raw_transaction(&tx).await.unwrap();
        let expected = txid; // Don't touch this!
        assert_eq!(expected, got);

        // get_raw_transaction_verbosity_zero
        let got = client
            .get_raw_transaction_verbosity_zero(&txid)
            .await
            .unwrap()
            .0
            .compute_txid();
        assert_eq!(expected, got);

        // get_raw_transaction_verbosity_one
        let got = client
            .get_raw_transaction_verbosity_one(&txid)
            .await
            .unwrap()
            .transaction
            .compute_txid();
        assert_eq!(expected, got);

        // get_raw_mempool
        let got = client.get_raw_mempool().await.unwrap();
        let expected = vec![txid];
        assert_eq!(expected, got.0);

        // get_raw_mempool_verbose
        let got = client.get_raw_mempool_verbose().await.unwrap();
        assert_eq!(got.0.len(), 1);
        assert_eq!(got.0.get(&txid).unwrap().height, 101);

        // get_mempool_info
        let got = client.get_mempool_info().await.unwrap();
        assert!(got.loaded.unwrap_or(false));
        assert_eq!(got.size, 1);
        assert_eq!(got.unbroadcast_count, Some(1));

        // estimate_smart_fee
        let got = client.estimate_smart_fee(1).await.unwrap();
        let expected = 1; // 1 sat/vB
        assert_eq!(expected, got);

        // sign_raw_transaction_with_wallet
        let got = client
            .sign_raw_transaction_with_wallet(&tx, None)
            .await
            .unwrap();
        assert!(got.complete);
        assert!(got.errors.is_empty());

        // test_mempool_accept
        let txids = client
            .test_mempool_accept(&tx)
            .await
            .expect("must be able to test mempool accept");
        let got = txids
            .results
            .first()
            .expect("there must be at least one txid");
        assert_eq!(
            got.txid,
            tx.compute_txid(),
            "txids must match in the mempool"
        );

        // send_raw_transaction
        let got = client.send_raw_transaction(&tx).await.unwrap();
        assert!(got.as_byte_array().len() == 32);

        // list_transactions
        let got = client.list_transactions(None).await.unwrap();
        assert_eq!(got.0.len(), 10);

        // list_unspent
        // let's mine one more block
        mine_blocks(&bitcoind, 1, None).unwrap();
        let got = client
            .list_unspent(None, None, None, None, None)
            .await
            .unwrap();
        assert_eq!(got.0.len(), 3);

        // listdescriptors
        let got = client.get_xpriv().await.unwrap().unwrap().network;
        let expected = NetworkKind::Test;
        assert_eq!(expected, got);

        // importdescriptors
        // taken from https://github.com/rust-bitcoin/rust-bitcoin/blob/bb38aeb786f408247d5bbc88b9fa13616c74c009/bitcoin/examples/taproot-psbt.rs#L18C38-L18C149
        let descriptor_string = "tr([e61b318f/20000'/20']tprv8ZgxMBicQKsPd4arFr7sKjSnKFDVMR2JHw9Y8L9nXN4kiok4u28LpHijEudH3mMYoL4pM5UL9Bgdz2M4Cy8EzfErmU9m86ZTw6hCzvFeTg7/101/*)#2plamwqs".to_owned();
        let timestamp = "now".to_owned();
        let list_descriptors = vec![ImportDescriptorInput {
            desc: descriptor_string,
            active: Some(true),
            timestamp,
        }];
        let got = client
            .import_descriptors(list_descriptors, "strata".to_owned())
            .await
            .unwrap()
            .0;
        let expected = vec![ImportDescriptorsResult {
            success: true,
            warnings: Some(vec![
                "Range not given, using default keypool range".to_string()
            ]),
            error: None,
        }];
        assert_eq!(expected, got);

        let psbt_address = client.get_new_address().await.unwrap();
        let psbt_outputs = vec![CreateRawTransactionOutput::AddressAmount {
            address: psbt_address.to_string(),
            amount: 1.0,
        }];

        let funded_psbt = client
            .wallet_create_funded_psbt(&[], &psbt_outputs, None, None, None)
            .await
            .unwrap();
        assert!(!funded_psbt.psbt.inputs.is_empty());
        assert!(funded_psbt.fee.to_sat() > 0);

        let processed_psbt = client
            .wallet_process_psbt(&funded_psbt.psbt.to_string(), None, None, None)
            .await
            .unwrap();
        assert!(!processed_psbt.psbt.inputs.is_empty());
        assert!(processed_psbt.complete);

        let finalized_psbt = client
            .wallet_process_psbt(&funded_psbt.psbt.to_string(), Some(true), None, None)
            .await
            .unwrap();
        assert!(finalized_psbt.complete);
        assert!(finalized_psbt.hex.is_some());
        let signed_tx = finalized_psbt.hex.as_ref().unwrap();
        let signed_txid = signed_tx.compute_txid();
        let got = client
            .test_mempool_accept(signed_tx)
            .await
            .unwrap()
            .results
            .first()
            .unwrap()
            .txid;
        assert_eq!(signed_txid, got);

        let info_address = client.get_new_address().await.unwrap();
        let address_info = client.get_address_info(&info_address).await.unwrap();
        assert_eq!(address_info.address, info_address.as_unchecked().clone());
        assert!(address_info.is_mine);
        assert!(address_info.solvable.unwrap_or(false));

        let unspent_address = client.get_new_address().await.unwrap();
        let unspent_txid = client
            .call::<String>(
                "sendtoaddress",
                &[
                    to_value(unspent_address.to_string()).unwrap(),
                    to_value(1.0).unwrap(),
                ],
            )
            .await
            .unwrap();
        mine_blocks(&bitcoind, 1, None).unwrap();

        let utxos = client
            .list_unspent(Some(1), Some(9_999_999), None, Some(true), None)
            .await
            .unwrap();
        assert!(!utxos.0.is_empty());

        let utxos_filtered = client
            .list_unspent(
                Some(1),
                Some(9_999_999),
                Some(std::slice::from_ref(&unspent_address)),
                Some(true),
                None,
            )
            .await
            .unwrap();
        assert!(!utxos_filtered.0.is_empty());
        let found_utxo = utxos_filtered.0.iter().any(|utxo| {
            utxo.txid.to_string() == unspent_txid
                && utxo.address.clone().assume_checked().to_string() == unspent_address.to_string()
        });
        assert!(found_utxo);

        let query_options = ListUnspentQueryOptions {
            minimum_amount: Some(Amount::from_btc(0.5).unwrap()),
            maximum_amount: Some(Amount::from_btc(2.0).unwrap()),
            maximum_count: Some(10),
        };
        let utxos_with_query = client
            .list_unspent(
                Some(1),
                Some(9_999_999),
                None,
                Some(true),
                Some(query_options),
            )
            .await
            .unwrap();
        assert!(!utxos_with_query.0.is_empty());
        for utxo in &utxos_with_query.0 {
            let amount_btc = utxo.amount.to_btc();
            assert!((0.5..=2.0).contains(&amount_btc));
        }

        let tx = finalized_psbt.hex.unwrap();
        assert!(!tx.input.is_empty());
        assert!(!tx.output.is_empty());
    }

    #[tokio::test()]
    async fn get_tx_out() {
        init_tracing();

        let (bitcoind, client) = get_bitcoind_and_client();

        // network sanity check
        let got = client.network().await.unwrap();
        let expected = Network::Regtest;
        assert_eq!(expected, got);

        let address = bitcoind.client.new_address().unwrap();
        let blocks = mine_blocks(&bitcoind, 101, Some(address)).unwrap();
        let last_block = client.get_block(blocks.first().unwrap()).await.unwrap();
        let coinbase_tx = last_block.coinbase().unwrap();

        // gettxout should work with a non-spent UTXO.
        let got = client
            .get_tx_out(&coinbase_tx.compute_txid(), 0, true)
            .await
            .unwrap();
        assert_eq!(got.tx_out.value, COINBASE_AMOUNT);

        // gettxout should fail with a spent UTXO.
        let new_address = bitcoind.client.new_address().unwrap();
        let send_amount = Amount::from_sat(COINBASE_AMOUNT.to_sat() - 2_000); // 2k sats as fees.
        let _send_tx = bitcoind
            .client
            .send_to_address(&new_address, send_amount)
            .unwrap()
            .txid()
            .unwrap();
        let result = client
            .get_tx_out(&coinbase_tx.compute_txid(), 0, true)
            .await;
        trace!(?result, "gettxout result");
        assert!(result.is_err());
    }

    /// Create two transactions.
    /// 1. Normal one: sends 1 BTC to an address that we control.
    /// 2. CFFP: replaces the first transaction with a different one that we also control.
    ///
    /// This is needed because we must SIGN all these transactions, and we can't sign a transaction
    /// that we don't control.
    #[tokio::test()]
    async fn submit_package() {
        init_tracing();

        let (bitcoind, client) = get_bitcoind_and_client();

        // network sanity check
        let got = client.network().await.unwrap();
        let expected = Network::Regtest;
        assert_eq!(expected, got);

        let blocks = mine_blocks(&bitcoind, 101, None).unwrap();
        let last_block = client.get_block(blocks.first().unwrap()).await.unwrap();
        let coinbase_tx = last_block.coinbase().unwrap();

        let destination = client.get_new_address().await.unwrap();
        let change_address = client.get_new_address().await.unwrap();
        let amount = Amount::from_btc(1.0).unwrap();
        let fees = Amount::from_btc(0.0001).unwrap();
        let change_amount = COINBASE_AMOUNT - amount - fees;
        let amount_minus_fees = Amount::from_sat(amount.to_sat() - 2_000);

        let send_back_address = client.get_new_address().await.unwrap();
        let parent_raw_tx = CreateRawTransactionArguments {
            inputs: vec![CreateRawTransactionInput {
                txid: coinbase_tx.compute_txid().to_string(),
                vout: 0,
            }],
            outputs: vec![
                // Destination
                CreateRawTransactionOutput::AddressAmount {
                    address: destination.to_string(),
                    amount: amount.to_btc(),
                },
                // Change
                CreateRawTransactionOutput::AddressAmount {
                    address: change_address.to_string(),
                    amount: change_amount.to_btc(),
                },
            ],
        };
        let parent = client.create_raw_transaction(parent_raw_tx).await.unwrap();
        let signed_parent = client
            .sign_raw_transaction_with_wallet(&parent, None)
            .await
            .unwrap()
            .tx;

        // sanity check
        let parent_submitted = client.send_raw_transaction(&signed_parent).await.unwrap();

        let child_raw_tx = CreateRawTransactionArguments {
            inputs: vec![CreateRawTransactionInput {
                txid: parent_submitted.to_string(),
                vout: 0,
            }],
            outputs: vec![
                // Send back
                CreateRawTransactionOutput::AddressAmount {
                    address: send_back_address.to_string(),
                    amount: amount_minus_fees.to_btc(),
                },
            ],
        };
        let child = client.create_raw_transaction(child_raw_tx).await.unwrap();
        let signed_child = client
            .sign_raw_transaction_with_wallet(&child, None)
            .await
            .unwrap()
            .tx;

        // Ok now we have a parent and a child transaction.
        let result = client
            .submit_package(&[signed_parent, signed_child])
            .await
            .unwrap();
        assert_eq!(result.tx_results.len(), 2);
        assert_eq!(result.package_msg, "success");
    }

    /// Similar to [`submit_package`], but with where the parent does not pay fees,
    /// and the child has to pay fees.
    ///
    /// This is called 1P1C because it has one parent and one child.
    /// See <https://bitcoinops.org/en/bitcoin-core-28-wallet-integration-guide/>
    /// for more information.
    #[tokio::test]
    async fn submit_package_1p1c() {
        init_tracing();

        let (bitcoind, client) = get_bitcoind_and_client();

        // 1p1c sanity check
        let server_version = bitcoind.client.server_version().unwrap();
        assert!(server_version > 28);

        let destination = client.get_new_address().await.unwrap();

        let blocks = mine_blocks(&bitcoind, 101, None).unwrap();
        let last_block = client.get_block(blocks.first().unwrap()).await.unwrap();
        let coinbase_tx = last_block.coinbase().unwrap();

        let parent_raw_tx = CreateRawTransactionArguments {
            inputs: vec![CreateRawTransactionInput {
                txid: coinbase_tx.compute_txid().to_string(),
                vout: 0,
            }],
            outputs: vec![CreateRawTransactionOutput::AddressAmount {
                address: destination.to_string(),
                amount: COINBASE_AMOUNT.to_btc(),
            }],
        };
        let mut parent = client.create_raw_transaction(parent_raw_tx).await.unwrap();
        parent.version = transaction::Version(3);
        assert_eq!(parent.version, transaction::Version(3));
        trace!(?parent, "parent:");
        let signed_parent = client
            .sign_raw_transaction_with_wallet(&parent, None)
            .await
            .unwrap()
            .tx;
        assert_eq!(signed_parent.version, transaction::Version(3));

        // Assert that the parent tx cannot be broadcasted.
        let parent_broadcasted = client.send_raw_transaction(&signed_parent).await;
        assert!(parent_broadcasted.is_err());

        // 5k sats as fees.
        let amount_minus_fees = Amount::from_sat(COINBASE_AMOUNT.to_sat() - 43_000);
        let child_raw_tx = CreateRawTransactionArguments {
            inputs: vec![CreateRawTransactionInput {
                txid: signed_parent.compute_txid().to_string(),
                vout: 0,
            }],
            outputs: vec![CreateRawTransactionOutput::AddressAmount {
                address: destination.to_string(),
                amount: amount_minus_fees.to_btc(),
            }],
        };
        let mut child = client.create_raw_transaction(child_raw_tx).await.unwrap();
        child.version = transaction::Version(3);
        assert_eq!(child.version, transaction::Version(3));
        trace!(?child, "child:");
        let prev_outputs = vec![PreviousTransactionOutput {
            txid: parent.compute_txid(),
            vout: 0,
            script_pubkey: parent.output[0].script_pubkey.to_hex_string(),
            redeem_script: None,
            witness_script: None,
            amount: Some(COINBASE_AMOUNT.to_btc()),
        }];
        let signed_child = client
            .sign_raw_transaction_with_wallet(&child, Some(prev_outputs))
            .await
            .unwrap()
            .tx;
        assert_eq!(signed_child.version, transaction::Version(3));

        // Assert that the child tx cannot be broadcasted.
        let child_broadcasted = client.send_raw_transaction(&signed_child).await;
        assert!(child_broadcasted.is_err());

        // Let's send as a package 1C1P.
        let result = client
            .submit_package(&[signed_parent, signed_child])
            .await
            .unwrap();
        assert_eq!(result.tx_results.len(), 2);
        assert_eq!(result.package_msg, "success");
    }

    #[tokio::test]
    async fn test_invalid_credentials_return_401_error() {
        init_tracing();

        let (bitcoind, _) = get_bitcoind_and_client();
        let url = bitcoind.rpc_url();

        // Create client with invalid credentials
        let invalid_client = Client::new(
            url,
            "wrong_user".to_string(),
            "wrong_password".to_string(),
            None,
            None,
            None,
        )
        .unwrap();

        // Try to make any RPC call
        let result = invalid_client.get_blockchain_info().await;

        // Verify we get a 401 Status error, not a Parse error
        assert!(result.is_err());
        let error = result.unwrap_err();

        match error {
            ClientError::Status(status_code, message) => {
                assert_eq!(status_code, 401);
                assert!(message.contains("Unauthorized"));
            }
            _ => panic!("Expected Status(401, _) error, but got: {error:?}"),
        }
    }

    #[tokio::test]
    async fn psbt_bump_fee() {
        init_tracing();

        let (bitcoind, client) = get_bitcoind_and_client();

        // Mine blocks to have funds
        mine_blocks(&bitcoind, 101, None).unwrap();

        // Send to the next address
        let destination = client.get_new_address().await.unwrap();
        let amount = Amount::from_btc(0.001).unwrap(); // 0.001 BTC

        // Create transaction with RBF enabled
        let txid = bitcoind
            .client
            .send_to_address_rbf(&destination, amount)
            .unwrap()
            .txid()
            .unwrap();

        // Verify transaction is in mempool (unconfirmed)
        let mempool = client.get_raw_mempool().await.unwrap();
        assert!(
            mempool.0.contains(&txid),
            "Transaction should be in mempool for RBF"
        );

        // Test psbt_bump_fee with default options
        let signed_tx = client
            .psbt_bump_fee(&txid, None)
            .await
            .unwrap()
            .psbt
            .extract_tx()
            .unwrap();
        let signed_txid = signed_tx.compute_txid();
        let got = client
            .test_mempool_accept(&signed_tx)
            .await
            .unwrap()
            .results
            .first()
            .unwrap()
            .txid;
        assert_eq!(
            got, signed_txid,
            "Bumped transaction should be accepted in mempool"
        );

        // Test psbt_bump_fee with custom fee rate
        let options = PsbtBumpFeeOptions {
            fee_rate: Some(FeeRate::from_sat_per_kwu(20)), // 20 sat/vB - higher than default
            ..Default::default()
        };
        trace!(?options, "Calling psbt_bump_fee");
        let signed_tx = client
            .psbt_bump_fee(&txid, Some(options))
            .await
            .unwrap()
            .psbt
            .extract_tx()
            .unwrap();
        let signed_txid = signed_tx.compute_txid();
        let got = client
            .test_mempool_accept(&signed_tx)
            .await
            .unwrap()
            .results
            .first()
            .unwrap()
            .txid;
        assert_eq!(
            got, signed_txid,
            "Bumped transaction should be accepted in mempool"
        );
    }
}
