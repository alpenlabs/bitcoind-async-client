use std::collections::BTreeMap;

use bitcoin::{
    address::{self, NetworkUnchecked},
    consensus::{self},
    Address, Amount, FeeRate, Psbt, Transaction, Txid,
};
use serde::{
    de::{self, Visitor},
    Deserialize, Deserializer, Serialize, Serializer,
};

use crate::error::SignRawTransactionWithWalletError;

/// Result of JSON-RPC method `gettxout`.
///
/// > gettxout "txid" n ( include_mempool )
/// >
/// > Returns details about an unspent transaction output.
/// >
/// > Arguments:
/// > 1. txid               (string, required) The transaction id
/// > 2. n                  (numeric, required) vout number
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct GetTxOut {
    /// The hash of the block at the tip of the chain.
    #[serde(rename = "bestblock")]
    pub best_block: String,
    /// The number of confirmations.
    pub confirmations: u32, // TODO: Change this to an i64.
    /// The transaction value in BTC.
    pub value: f64,
    /// The script pubkey.
    #[serde(rename = "scriptPubkey")]
    pub script_pubkey: Option<ScriptPubkey>,
    /// Coinbase or not.
    pub coinbase: bool,
}

/// A script pubkey.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct ScriptPubkey {
    /// Script assembly.
    pub asm: String,
    /// Script hex.
    pub hex: String,
    #[serde(rename = "reqSigs")]
    pub req_sigs: i64,
    /// The type, eg pubkeyhash.
    #[serde(rename = "type")]
    pub type_: String,
    /// Bitcoin address.
    pub address: Option<String>,
}

/// Models the arguments of JSON-RPC method `createrawtransaction`.
///
/// # Note
///
/// Assumes that the transaction is always "replaceable" by default and has a locktime of 0.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct CreateRawTransaction {
    pub inputs: Vec<CreateRawTransactionInput>,
    pub outputs: Vec<CreateRawTransactionOutput>,
}

/// Models the input of JSON-RPC method `createrawtransaction`.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct CreateRawTransactionInput {
    pub txid: String,
    pub vout: u32,
}

/// Models transaction outputs for Bitcoin RPC methods.
///
/// Used by various RPC methods such as `createrawtransaction`, `psbtbumpfee`,
/// and `walletcreatefundedpsbt`. The outputs are specified as key-value pairs,
/// where the keys are addresses and the values are amounts to send.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(untagged)]
pub enum CreateRawTransactionOutput {
    /// A pair of an [`Address`] string and an [`Amount`] in BTC.
    AddressAmount {
        /// An [`Address`] string.
        address: String,
        /// An [`Amount`] in BTC.
        amount: f64,
    },
    /// A payload such as in `OP_RETURN` transactions.
    Data {
        /// The payload.
        data: String,
    },
}

impl Serialize for CreateRawTransactionOutput {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        match self {
            CreateRawTransactionOutput::AddressAmount { address, amount } => {
                let mut map = serde_json::Map::new();
                map.insert(
                    address.clone(),
                    serde_json::Value::Number(serde_json::Number::from_f64(*amount).unwrap()),
                );
                map.serialize(serializer)
            }
            CreateRawTransactionOutput::Data { data } => {
                let mut map = serde_json::Map::new();
                map.insert("data".to_string(), serde_json::Value::String(data.clone()));
                map.serialize(serializer)
            }
        }
    }
}

/// Result of JSON-RPC method `submitpackage`.
///
/// > submitpackage ["rawtx",...] ( maxfeerate maxburnamount )
/// >
/// > Submit a package of raw transactions (serialized, hex-encoded) to local node.
/// > The package will be validated according to consensus and mempool policy rules. If any
/// > transaction passes, it will be accepted to mempool.
/// > This RPC is experimental and the interface may be unstable. Refer to doc/policy/packages.md
/// > for documentation on package policies.
/// > Warning: successful submission does not mean the transactions will propagate throughout the
/// > network.
/// >
/// > Arguments:
/// > 1. package          (json array, required) An array of raw transactions.
/// > The package must solely consist of a child and its parents. None of the parents may depend on
/// > each other.
/// > The package must be topologically sorted, with the child being the last element in the array.
/// > [
/// > "rawtx",     (string)
/// > ...
/// > ]
#[allow(clippy::doc_lazy_continuation)]
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct SubmitPackage {
    /// The transaction package result message.
    ///
    /// "success" indicates all transactions were accepted into or are already in the mempool.
    pub package_msg: String,
    /// Transaction results keyed by wtxid.
    #[serde(rename = "tx-results")]
    pub tx_results: BTreeMap<String, SubmitPackageTxResult>,
    /// List of txids of replaced transactions.
    #[serde(rename = "replaced-transactions")]
    pub replaced_transactions: Vec<String>,
}

/// Models the per-transaction result included in the JSON-RPC method `submitpackage`.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct SubmitPackageTxResult {
    /// The transaction id.
    pub txid: String,
    /// The wtxid of a different transaction with the same txid but different witness found in the
    /// mempool.
    ///
    /// If set, this means the submitted transaction was ignored.
    #[serde(rename = "other-wtxid")]
    pub other_wtxid: Option<String>,
    /// Sigops-adjusted virtual transaction size.
    pub vsize: i64,
    /// Transaction fees.
    pub fees: Option<SubmitPackageTxResultFees>,
    /// The transaction error string, if it was rejected by the mempool
    pub error: Option<String>,
}

/// Models the fees included in the per-transaction result of the JSON-RPC method `submitpackage`.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct SubmitPackageTxResultFees {
    /// Transaction fee.
    #[serde(rename = "base")]
    pub base_fee: f64,
    /// The effective feerate.
    ///
    /// Will be `None` if the transaction was already in the mempool. For example, the package
    /// feerate and/or feerate with modified fees from the `prioritisetransaction` JSON-RPC method.
    #[serde(rename = "effective-feerate")]
    pub effective_fee_rate: Option<f64>,
    /// If [`Self::effective_fee_rate`] is provided, this holds the wtxid's of the transactions
    /// whose fees and vsizes are included in effective-feerate.
    #[serde(rename = "effective-includes")]
    pub effective_includes: Option<Vec<String>>,
}

/// Result of the JSON-RPC method `getnewaddress`.
///
/// # Note
///
/// This assumes that the UTXOs are present in the underlying Bitcoin
/// client's wallet.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct GetNewAddress(pub String);

impl GetNewAddress {
    /// Converts json straight to a [`Address`].
    pub fn address(self) -> Result<Address<NetworkUnchecked>, address::ParseError> {
        let address = self.0.parse::<Address<_>>()?;
        Ok(address)
    }
}

/// Models the result of JSON-RPC method `testmempoolaccept`.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct TestMempoolAccept {
    /// The transaction id.
    #[serde(deserialize_with = "deserialize_txid")]
    pub txid: Txid,
    /// Rejection reason, if any.
    pub reject_reason: Option<String>,
}

/// Models the result of JSON-RPC method `signrawtransactionwithwallet`.
///
/// # Note
///
/// This assumes that the transactions are present in the underlying Bitcoin
/// client's wallet.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct SignRawTransactionWithWallet {
    /// The Transaction ID.
    pub hex: String,
    /// If the transaction has a complete set of signatures.
    pub complete: bool,
    /// Errors, if any.
    pub errors: Option<Vec<SignRawTransactionWithWalletError>>,
}

/// Models the optional previous transaction outputs argument for the method
/// `signrawtransactionwithwallet`.
///
/// These are the outputs that this transaction depends on but may not yet be in the block chain.
/// Widely used for One Parent One Child (1P1C) Relay in Bitcoin >28.0.
///
/// > transaction outputs
/// > [
/// > {                            (json object)
/// > "txid": "hex",             (string, required) The transaction id
/// > "vout": n,                 (numeric, required) The output number
/// > "scriptPubKey": "hex",     (string, required) The output script
/// > "redeemScript": "hex",     (string, optional) (required for P2SH) redeem script
/// > "witnessScript": "hex",    (string, optional) (required for P2WSH or P2SH-P2WSH) witness
/// > script
/// > "amount": amount,          (numeric or string, optional) (required for Segwit inputs) the
/// > amount spent
/// > },
/// > ...
/// > ]
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct PreviousTransactionOutput {
    /// The transaction id.
    #[serde(deserialize_with = "deserialize_txid")]
    pub txid: Txid,
    /// The output number.
    pub vout: u32,
    /// The output script.
    #[serde(rename = "scriptPubKey")]
    pub script_pubkey: String,
    /// The redeem script.
    #[serde(rename = "redeemScript")]
    pub redeem_script: Option<String>,
    /// The witness script.
    #[serde(rename = "witnessScript")]
    pub witness_script: Option<String>,
    /// The amount spent.
    pub amount: Option<f64>,
}

/// Models the result of the JSON-RPC method `listdescriptors`.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct ListDescriptors {
    /// The descriptors
    pub descriptors: Vec<ListDescriptor>,
}

/// Models the Descriptor in the result of the JSON-RPC method `listdescriptors`.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct ListDescriptor {
    /// The descriptor.
    pub desc: String,
}

/// Models the result of the JSON-RPC method `importdescriptors`.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct ImportDescriptors {
    /// The descriptors
    pub descriptors: Vec<ListDescriptor>,
}

/// Models the Descriptor in the result of the JSON-RPC method `importdescriptors`.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct ImportDescriptor {
    /// The descriptor.
    pub desc: String,
    /// Set this descriptor to be the active descriptor
    /// for the corresponding output type/externality.
    pub active: Option<bool>,
    /// Time from which to start rescanning the blockchain for this descriptor,
    /// in UNIX epoch time. Can also be a string "now"
    pub timestamp: String,
}
/// Models the Descriptor in the result of the JSON-RPC method `importdescriptors`.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct ImportDescriptorResult {
    /// Result.
    pub success: bool,
}

/// Models the `createwallet` JSON-RPC method.
///
/// # Note
///
/// This can also be used for the `loadwallet` JSON-RPC method.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct CreateWallet {
    /// Wallet name
    pub wallet_name: String,
    /// Load on startup
    pub load_on_startup: Option<bool>,
}

/// Deserializes the amount in BTC into proper [`Amount`]s.
fn deserialize_bitcoin<'d, D>(deserializer: D) -> Result<Amount, D::Error>
where
    D: Deserializer<'d>,
{
    struct SatVisitor;

    impl Visitor<'_> for SatVisitor {
        type Value = Amount;

        fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
            write!(formatter, "a float representation of btc values expected")
        }

        fn visit_f64<E>(self, v: f64) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            let amount = Amount::from_btc(v).expect("Amount deserialization failed");
            Ok(amount)
        }
    }
    deserializer.deserialize_any(SatVisitor)
}

/// Serializes the optional [`Amount`] into BTC.
fn serialize_option_bitcoin<S>(amount: &Option<Amount>, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    match amount {
        Some(amt) => serializer.serialize_some(&amt.to_btc()),
        None => serializer.serialize_none(),
    }
}

/// Deserializes the fee rate from sat/vB into proper [`FeeRate`].
///
/// Note: Bitcoin Core 0.21+ uses sat/vB for fee rates for most RPC methods/results.
fn deserialize_feerate<'d, D>(deserializer: D) -> Result<FeeRate, D::Error>
where
    D: Deserializer<'d>,
{
    struct FeeRateVisitor;

    impl Visitor<'_> for FeeRateVisitor {
        type Value = FeeRate;

        fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
            write!(
                formatter,
                "a numeric representation of fee rate in sat/vB expected"
            )
        }

        fn visit_f64<E>(self, v: f64) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            // The value is already in sat/vB (Bitcoin Core 0.21+)
            let sat_per_vb = v.round() as u64;
            let fee_rate = FeeRate::from_sat_per_vb(sat_per_vb)
                .ok_or_else(|| de::Error::custom("Invalid fee rate"))?;
            Ok(fee_rate)
        }
    }
    deserializer.deserialize_any(FeeRateVisitor)
}

/// Deserializes the transaction id string into proper [`Txid`]s.
fn deserialize_txid<'d, D>(deserializer: D) -> Result<Txid, D::Error>
where
    D: Deserializer<'d>,
{
    struct TxidVisitor;

    impl Visitor<'_> for TxidVisitor {
        type Value = Txid;

        fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
            write!(formatter, "a transaction id string expected")
        }

        fn visit_str<E>(self, v: &str) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            let txid = v.parse::<Txid>().expect("invalid txid");

            Ok(txid)
        }
    }
    deserializer.deserialize_any(TxidVisitor)
}

/// Deserializes a base64-encoded PSBT string into proper [`Psbt`]s.
///
/// # Note
///
/// Expects a valid base64-encoded PSBT as defined in BIP 174. The PSBT
/// string must contain valid transaction data and metadata for successful parsing.
fn deserialize_psbt<'d, D>(deserializer: D) -> Result<Psbt, D::Error>
where
    D: Deserializer<'d>,
{
    struct PsbtVisitor;

    impl Visitor<'_> for PsbtVisitor {
        type Value = Psbt;

        fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
            write!(formatter, "a base64-encoded PSBT string expected")
        }

        fn visit_str<E>(self, v: &str) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            v.parse::<Psbt>()
                .map_err(|e| E::custom(format!("failed to deserialize PSBT: {e}")))
        }
    }
    deserializer.deserialize_any(PsbtVisitor)
}

/// Deserializes an optional base64-encoded PSBT string into `Option<Psbt>`.
///
/// # Note
///
/// When the JSON field is `null` or missing, returns `None`. When present,
/// deserializes the base64 PSBT string using the same validation as [`deserialize_psbt`].
fn deserialize_option_psbt<'d, D>(deserializer: D) -> Result<Option<Psbt>, D::Error>
where
    D: Deserializer<'d>,
{
    let opt: Option<String> = Option::deserialize(deserializer)?;
    match opt {
        Some(s) => s
            .parse::<Psbt>()
            .map(Some)
            .map_err(|e| de::Error::custom(format!("failed to deserialize PSBT: {e}"))),
        None => Ok(None),
    }
}

fn deserialize_option_tx<'d, D>(deserializer: D) -> Result<Option<Transaction>, D::Error>
where
    D: Deserializer<'d>,
{
    let opt: Option<String> = Option::deserialize(deserializer)?;
    match opt {
        Some(s) => consensus::encode::deserialize_hex::<Transaction>(&s)
            .map(Some)
            .map_err(|e| de::Error::custom(format!("failed to deserialize transaction hex: {e}"))),
        None => Ok(None),
    }
}

/// Deserializes the address string into proper [`Address`]s.
///
/// # Note
///
/// The user is responsible for ensuring that the address is valid,
/// since this functions returns an [`Address<NetworkUnchecked>`].
fn deserialize_address<'d, D>(deserializer: D) -> Result<Address<NetworkUnchecked>, D::Error>
where
    D: Deserializer<'d>,
{
    struct AddressVisitor;
    impl Visitor<'_> for AddressVisitor {
        type Value = Address<NetworkUnchecked>;

        fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
            write!(formatter, "a Bitcoin address string expected")
        }

        fn visit_str<E>(self, v: &str) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            v.parse::<Address<_>>()
                .map_err(|e| E::custom(format!("failed to deserialize address: {e}")))
        }
    }
    deserializer.deserialize_any(AddressVisitor)
}

/// Signature hash types for Bitcoin transactions.
///
/// These types specify which parts of a transaction are included in the signature
/// hash calculation when signing transaction inputs. Used with wallet signing
/// operations like `wallet_process_psbt`.
///
/// # Note
///
/// These correspond to the SIGHASH flags defined in Bitcoin's script system
/// and BIP 143 (witness transaction digest).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum SighashType {
    /// Use the default signature hash type (equivalent to SIGHASH_ALL).
    Default,

    /// Sign all inputs and all outputs of the transaction.
    ///
    /// This is the most common and secure signature type, ensuring the entire
    /// transaction structure cannot be modified after signing.
    All,

    /// Sign all inputs but no outputs.
    ///
    /// Allows outputs to be modified after signing, useful for donation scenarios
    /// where the exact destination amounts can be adjusted.
    None,

    /// Sign all inputs and the output with the same index as this input.
    ///
    /// Used in scenarios where multiple parties contribute inputs and want to
    /// ensure their corresponding output is protected.
    Single,

    /// Combination of SIGHASH_ALL with ANYONECANPAY flag.
    ///
    /// Signs all outputs but only this specific input, allowing other inputs
    /// to be added or removed. Useful for crowdfunding transactions.
    #[serde(rename = "ALL|ANYONECANPAY")]
    AllPlusAnyoneCanPay,

    /// Combination of SIGHASH_NONE with ANYONECANPAY flag.
    ///
    /// Signs only this specific input with no outputs committed, providing
    /// maximum flexibility for transaction modification.
    #[serde(rename = "NONE|ANYONECANPAY")]
    NonePlusAnyoneCanPay,

    /// Combination of SIGHASH_SINGLE with ANYONECANPAY flag.
    ///
    /// Signs only this input and its corresponding output, allowing other
    /// inputs and outputs to be modified independently.
    #[serde(rename = "SINGLE|ANYONECANPAY")]
    SinglePlusAnyoneCanPay,
}

/// Options for creating a funded PSBT with wallet inputs.
///
/// Used with `wallet_create_funded_psbt` to control funding behavior,
/// fee estimation, and transaction policies when the wallet automatically
/// selects inputs to fund the specified outputs.
///
/// # Note
///
/// All fields are optional and will use Bitcoin Core defaults if not specified.
/// Fee rate takes precedence over confirmation target if both are provided.
#[derive(Clone, Debug, PartialEq, Serialize, Default)]
pub struct WalletCreateFundedPsbtOptions {
    /// Fee rate in sat/vB (satoshis per virtual byte) for the transaction.
    ///
    /// If specified, this overrides the `conf_target` parameter for fee estimation.
    /// Must be a positive value representing the desired fee density.
    #[serde(default, rename = "fee_rate", skip_serializing_if = "Option::is_none")]
    pub fee_rate: Option<f64>,

    /// Whether to lock the selected UTXOs to prevent them from being spent by other transactions.
    ///
    /// When `true`, the wallet will temporarily lock the selected unspent outputs
    /// until the transaction is broadcast or manually unlocked. Default is `false`.
    #[serde(
        default,
        rename = "lockUnspents",
        skip_serializing_if = "Option::is_none"
    )]
    pub lock_unspents: Option<bool>,

    /// Target number of confirmations for automatic fee estimation.
    ///
    /// Represents the desired number of blocks within which the transaction should
    /// be confirmed. Higher values result in lower fees but longer confirmation times.
    /// Ignored if `fee_rate` is specified.
    #[serde(
        default,
        rename = "conf_target",
        skip_serializing_if = "Option::is_none"
    )]
    pub conf_target: Option<u16>,

    /// Whether the transaction should be BIP-125 opt-in Replace-By-Fee (RBF) enabled.
    ///
    /// When `true`, allows the transaction to be replaced with a higher-fee version
    /// before confirmation. Useful for fee bumping if the initial fee proves insufficient.
    #[serde(
        default,
        rename = "replaceable",
        skip_serializing_if = "Option::is_none"
    )]
    pub replaceable: Option<bool>,
}

/// Result of the `walletcreatefundedpsbt` RPC method.
///
/// Contains a funded PSBT created by the wallet with automatically selected inputs
/// to cover the specified outputs, along with fee information and change output details.
///
/// # Note
///
/// The PSBT returned is not signed and requires further processing with
/// `wallet_process_psbt` or `finalize_psbt` before broadcasting.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct WalletCreateFundedPsbt {
    /// The funded PSBT with inputs selected by the wallet.
    ///
    /// Contains the unsigned transaction structure with all necessary
    /// input and output information for subsequent signing operations.
    #[serde(deserialize_with = "deserialize_psbt")]
    pub psbt: Psbt,

    /// The fee amount in BTC paid by this transaction.
    ///
    /// Represents the total fee calculated based on the selected inputs,
    /// outputs, and the specified fee rate or confirmation target.
    #[serde(deserialize_with = "deserialize_bitcoin")]
    pub fee: Amount,

    /// The position of the change output in the transaction outputs array.
    ///
    /// If no change output was created (exact amount match), this will be -1.
    /// Otherwise, indicates the zero-based index of the change output.
    #[serde(rename = "changepos")]
    pub change_pos: i32,
}

/// Result of the `walletprocesspsbt` and `finalizepsbt` RPC methods.
///
/// Contains the processed PSBT state, completion status, and optionally the
/// extracted final transaction. This struct handles the Bitcoin Core's PSBT
/// workflow where PSBTs can be incrementally signed and eventually finalized.
///
/// # Note
///
/// The `psbt` field contains the updated PSBT after processing, while `hex`
/// contains the final transaction only when `complete` is `true` and extraction
/// is requested. Both fields may be `None` depending on the operation context.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct WalletProcessPsbtResult {
    /// The processed Partially Signed Bitcoin Transaction.
    ///
    /// Contains the PSBT after wallet processing with any signatures or input data
    /// that could be added. Will be `None` if the transaction was fully extracted
    /// and the PSBT is no longer needed.
    #[serde(deserialize_with = "deserialize_option_psbt")]
    pub psbt: Option<Psbt>,

    /// Whether the transaction is complete and ready for broadcast.
    ///
    /// `true` indicates all required signatures have been collected and the
    /// transaction can be finalized. `false` means more signatures are needed
    /// before the transaction can be broadcast to the network.
    pub complete: bool,

    /// The final transaction ready for broadcast (when complete).
    ///
    /// Contains the fully signed and finalized transaction when `complete` is `true`
    /// and extraction was requested. Will be `None` for incomplete transactions or
    /// when extraction is not performed.
    #[serde(
        deserialize_with = "deserialize_option_tx",
        skip_serializing_if = "Option::is_none",
        default
    )]
    pub hex: Option<Transaction>,
}

/// Result of the `getaddressinfo` RPC method.
///
/// Provides detailed information about a Bitcoin address, including ownership
/// status, watching capabilities, and spending permissions within the wallet.
///
/// # Note
///
/// Optional fields may be `None` if the wallet doesn't have specific information
/// about the address or if the address is not related to the wallet.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct GetAddressInfo {
    /// The Bitcoin address that was queried.
    ///
    /// Returns the same address that was provided as input to `getaddressinfo`,
    /// validated and parsed into the proper Address type.
    #[serde(deserialize_with = "deserialize_address")]
    pub address: Address<NetworkUnchecked>,

    /// Whether the address belongs to the wallet (can receive payments to it).
    ///
    /// `true` if the wallet owns the private key or can generate signatures for this address.
    /// `false` if the address is not owned by the wallet. `None` if ownership status is unknown.
    #[serde(rename = "ismine")]
    pub is_mine: Option<bool>,

    /// Whether the address is watch-only (monitored but not spendable).
    ///
    /// `true` if the wallet watches this address for incoming transactions but cannot
    /// spend from it (no private key). `false` if the address is fully controlled.
    /// `None` if watch status is not applicable.
    #[serde(rename = "iswatchonly")]
    pub is_watchonly: Option<bool>,

    /// Whether the wallet knows how to spend coins sent to this address.
    ///
    /// `true` if the wallet has enough information (private keys, scripts) to create
    /// valid spending transactions from this address. `false` if the address cannot
    /// be spent by this wallet. `None` if spendability cannot be determined.
    pub solvable: Option<bool>,
}

/// Query options for filtering unspent transaction outputs.
///
/// Used with `list_unspent` to apply additional filtering criteria
/// beyond confirmation counts and addresses, allowing precise UTXO selection
/// based on amount ranges and result limits.
///
/// # Note
///
/// All fields are optional and can be combined. UTXOs must satisfy all
/// specified criteria to be included in the results.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ListUnspentQueryOptions {
    /// Minimum amount that UTXOs must have to be included.
    ///
    /// Only unspent outputs with a value greater than or equal to this amount
    /// will be returned. Useful for filtering out dust or very small UTXOs.
    #[serde(serialize_with = "serialize_option_bitcoin")]
    pub minimum_amount: Option<Amount>,

    /// Maximum amount that UTXOs can have to be included.
    ///
    /// Only unspent outputs with a value less than or equal to this amount
    /// will be returned. Useful for finding smaller UTXOs or avoiding large ones.
    #[serde(serialize_with = "serialize_option_bitcoin")]
    pub maximum_amount: Option<Amount>,

    /// Maximum number of UTXOs to return in the result set.
    ///
    /// Limits the total number of unspent outputs returned, regardless of how many
    /// match the other criteria. Useful for pagination or limiting response size.
    pub maximum_count: Option<u32>,
}

/// Options for psbtbumpfee RPC method.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct PsbtBumpFeeOptions {
    /// Confirmation target in blocks.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub conf_target: Option<u16>,

    /// Fee rate in sat/vB.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fee_rate: Option<FeeRate>,

    /// Whether the new transaction should be BIP-125 replaceable.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub replaceable: Option<bool>,

    /// Fee estimate mode ("unset", "economical", "conservative").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub estimate_mode: Option<String>,

    /// New transaction outputs to replace the existing ones.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub outputs: Option<Vec<CreateRawTransactionOutput>>,

    /// Index of the change output to recycle from the original transaction.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub original_change_index: Option<u32>,
}

/// Result of the psbtbumpfee RPC method.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct PsbtBumpFee {
    /// The base64-encoded unsigned PSBT of the new transaction.
    #[serde(deserialize_with = "deserialize_psbt")]
    pub psbt: Psbt,

    /// The fee of the replaced transaction.
    #[serde(deserialize_with = "deserialize_feerate")]
    pub origfee: FeeRate,

    /// The fee of the new transaction.
    #[serde(deserialize_with = "deserialize_feerate")]
    pub fee: FeeRate,

    /// Errors encountered during processing (if any).
    pub errors: Option<Vec<String>>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json;

    // Taken from https://docs.rs/bitcoin/0.32.6/src/bitcoin/psbt/mod.rs.html#1515-1520
    // BIP 174 test vector with inputs and outputs (more realistic than empty transaction)
    const TEST_PSBT: &str = "cHNidP8BAHUCAAAAASaBcTce3/KF6Tet7qSze3gADAVmy7OtZGQXE8pCFxv2AAAAAAD+////AtPf9QUAAAAAGXapFNDFmQPFusKGh2DpD9UhpGZap2UgiKwA4fUFAAAAABepFDVF5uM7gyxHBQ8k0+65PJwDlIvHh7MuEwAAAQD9pQEBAAAAAAECiaPHHqtNIOA3G7ukzGmPopXJRjr6Ljl/hTPMti+VZ+UBAAAAFxYAFL4Y0VKpsBIDna89p95PUzSe7LmF/////4b4qkOnHf8USIk6UwpyN+9rRgi7st0tAXHmOuxqSJC0AQAAABcWABT+Pp7xp0XpdNkCxDVZQ6vLNL1TU/////8CAMLrCwAAAAAZdqkUhc/xCX/Z4Ai7NK9wnGIZeziXikiIrHL++E4sAAAAF6kUM5cluiHv1irHU6m80GfWx6ajnQWHAkcwRAIgJxK+IuAnDzlPVoMR3HyppolwuAJf3TskAinwf4pfOiQCIAGLONfc0xTnNMkna9b7QPZzMlvEuqFEyADS8vAtsnZcASED0uFWdJQbrUqZY3LLh+GFbTZSYG2YVi/jnF6efkE/IQUCSDBFAiEA0SuFLYXc2WHS9fSrZgZU327tzHlMDDPOXMMJ/7X85Y0CIGczio4OFyXBl/saiK9Z9R5E5CVbIBZ8hoQDHAXR8lkqASECI7cr7vCWXRC+B3jv7NYfysb3mk6haTkzgHNEZPhPKrMAAAAAAAAA";

    // Valid Bitcoin transaction hex (Genesis block coinbase transaction)
    const TEST_TX_HEX: &str = "01000000010000000000000000000000000000000000000000000000000000000000000000ffffffff4d04ffff001d0104455468652054696d65732030332f4a616e2f32303039204368616e63656c6c6f72206f6e206272696e6b206f66207365636f6e64206261696c6f757420666f722062616e6b73ffffffff0100f2052a01000000434104678afdb0fe5548271967f1a67130b7105cd6a828e03909a67962e0ea1f61deb649f6bc3f4cef38c4f35504e51ec112de5c384df7ba0b8d578a4c702b6bf11d5fac00000000";

    #[test]
    fn test_wallet_process_psbt_result() {
        let valid_psbt = TEST_PSBT;

        // Test complete with hex
        let test_tx_hex = TEST_TX_HEX;
        let json1 = format!(r#"{{"psbt":"{valid_psbt}","complete":true,"hex":"{test_tx_hex}"}}"#);
        let result1: WalletProcessPsbtResult = serde_json::from_str(&json1).unwrap();
        assert!(result1.psbt.is_some());
        assert!(result1.complete);
        assert!(result1.hex.is_some());
        let tx = result1.hex.unwrap();
        assert!(!tx.input.is_empty());
        assert!(!tx.output.is_empty());

        // Test incomplete without hex
        let json2 = format!(r#"{{"psbt":"{valid_psbt}","complete":false}}"#);
        let result2: WalletProcessPsbtResult = serde_json::from_str(&json2).unwrap();
        assert!(result2.psbt.is_some());
        assert!(!result2.complete);
    }

    #[test]
    fn test_sighashtype_serialize() {
        let sighash = SighashType::All;
        let serialized = serde_json::to_string(&sighash).unwrap();
        assert_eq!(serialized, "\"ALL\"");

        let sighash2 = SighashType::AllPlusAnyoneCanPay;
        let serialized2 = serde_json::to_string(&sighash2).unwrap();
        assert_eq!(serialized2, "\"ALL|ANYONECANPAY\"");
    }

    #[test]
    fn test_list_unspent_query_options_camelcase() {
        let options = ListUnspentQueryOptions {
            minimum_amount: Some(Amount::from_btc(0.5).unwrap()),
            maximum_amount: Some(Amount::from_btc(2.0).unwrap()),
            maximum_count: Some(10),
        };
        let serialized = serde_json::to_string(&options).unwrap();

        assert!(serialized.contains("\"minimumAmount\":0.5"));
        assert!(serialized.contains("\"maximumAmount\":2.0"));
        assert!(serialized.contains("\"maximumCount\":10"));
    }

    #[test]
    fn test_psbt_parsing() {
        // Test valid PSBT parsing
        let valid_psbt = TEST_PSBT;
        let json1 = format!(r#"{{"psbt":"{valid_psbt}","fee":0.001,"changepos":-1}}"#);
        let result1: WalletCreateFundedPsbt = serde_json::from_str(&json1).unwrap();
        assert!(!result1.psbt.inputs.is_empty()); // BIP 174 test vector has inputs

        // Test invalid PSBT parsing fails
        let invalid_psbt = "invalid_base64";
        let json2 = format!(r#"{{"psbt":"{invalid_psbt}","fee":0.001,"changepos":-1}}"#);
        let result2 = serde_json::from_str::<WalletCreateFundedPsbt>(&json2);
        assert!(result2.is_err());
    }
}
