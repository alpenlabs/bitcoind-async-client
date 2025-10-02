use bitcoin::{Amount, FeeRate, Txid};
use serde::{
    de::{self, Visitor},
    Deserialize, Deserializer, Serialize, Serializer,
};

/// Models the arguments of JSON-RPC method `createrawtransaction`.
///
/// # Note
///
/// Assumes that the transaction is always "replaceable" by default and has a locktime of 0.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct CreateRawTransactionArguments {
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
    /// A pair of an [`bitcoin::Address`] string and an [`Amount`] in BTC.
    AddressAmount {
        /// An [`bitcoin::Address`] string.
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

/// Models the Descriptor in the result of the JSON-RPC method `importdescriptors`.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct ImportDescriptorInput {
    /// The descriptor.
    pub desc: String,
    /// Set this descriptor to be the active descriptor
    /// for the corresponding output type/externality.
    pub active: Option<bool>,
    /// Time from which to start rescanning the blockchain for this descriptor,
    /// in UNIX epoch time. Can also be a string "now"
    pub timestamp: String,
}

/// Models the `createwallet` JSON-RPC method.
///
/// # Note
///
/// This can also be used for the `loadwallet` JSON-RPC method.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct CreateWalletArguments {
    /// Wallet name
    pub name: String,
    /// Load on startup
    pub load_on_startup: Option<bool>,
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

/// Signature hash types for Bitcoin transactions.
///
/// These types specify which parts of a transaction are included in the signature
/// hash calculation when signing transaction inputs. Used with wallet signing
/// operations like `walletprocesspsbt`.
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
