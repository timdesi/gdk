use crate::be::{BEOutPoint, BEScript, BESigHashType, BETransaction, BETransactionEntry, BETxid};
use crate::descriptor::parse_single_sig_descriptor;
use crate::slip132::{decode_from_slip132_string, extract_bip32_account};
use crate::util::{is_confidential_txoutsecrets, now, weight_to_vsize};
use crate::NetworkId;
use crate::NetworkParameters;
use bitcoin::Network;
use elements::confidential;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::error::Error;
use crate::scripts::ScriptType;
use crate::wally::MasterBlindingKey;
use bitcoin::blockdata::transaction::EcdsaSighashType as BitcoinSigHashType;
use bitcoin::hashes::hex::ToHex;
use bitcoin::hashes::{sha256, Hash};
use bitcoin::util::bip32::{
    ChildNumber, DerivationPath, ExtendedPrivKey, ExtendedPubKey, Fingerprint,
};
use std::convert::TryFrom;
use std::fmt;
use std::fmt::Display;

#[derive(Debug, Deserialize)]
pub struct InitParam {
    pub log_level: String,

    #[serde(rename = "registrydir")]
    pub registry_dir: String,
}

pub type Balances = HashMap<String, i64>;

// =========== v exchange rate stuff v ===========

// TODO use these types from bitcoin-exchange-rates lib once it's in there

#[derive(Debug, Clone, PartialEq)]
pub struct ExchangeRate {
    pub currency: String,
    pub rate: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ExchangeRateError {
    pub message: String,
    pub error: ExchangeRateErrorType,
}

impl Display for ExchangeRateError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        fmt::Debug::fmt(self, f)
    }
}

impl std::error::Error for ExchangeRateError {}

impl Display for ExchangeRateErrorType {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        fmt::Debug::fmt(self, f)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum ExchangeRateOk {
    NoBackends, // this probably should't be a hard error,
    // so we label it an Ok result
    RateOk(ExchangeRate),
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub enum ExchangeRateErrorType {
    FetchError,
    ParseError,
}

pub type ExchangeRateRes = Result<ExchangeRateOk, ExchangeRateError>;

impl ExchangeRateOk {
    pub fn ok(currency: String, rate: f64) -> ExchangeRateOk {
        ExchangeRateOk::RateOk(ExchangeRate {
            currency,
            rate,
        })
    }

    pub fn no_backends() -> ExchangeRateOk {
        ExchangeRateOk::NoBackends
    }
}

// =========== ^ exchange rate stuff ^ ===========

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct AddressAmount {
    pub address: String, // could be bitcoin or elements
    pub satoshi: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub asset_id: Option<String>,
}

impl AddressAmount {
    pub fn asset_id(&self) -> Option<elements::issuance::AssetId> {
        self.asset_id.as_ref().and_then(|a| a.parse().ok())
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct LoginData {
    pub wallet_hash_id: String,
    pub xpub_hash_id: String,
}

#[derive(Serialize, Deserialize, Debug, PartialEq, Eq, Hash, Clone)]
#[serde(rename_all = "snake_case")]
pub enum UtxoStrategy {
    /// Add utxos until the addressees amounts and fees are covered
    Default,

    /// Uses all and only the utxos specified by the caller
    Manual,
}

impl Default for UtxoStrategy {
    fn default() -> Self {
        UtxoStrategy::Default
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct CreateTransaction {
    #[serde(default)]
    pub addressees: Vec<AddressAmount>,
    pub fee_rate: Option<u64>, // in satoshi/kbyte
    pub subaccount: u32,
    #[serde(default)]
    pub send_all: bool,
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub previous_transaction: Option<TxListItem>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub memo: Option<String>,
    #[serde(default)]
    pub utxos: CreateTxUtxos,
    /// Minimum number of confirmations for coin selection
    #[serde(default)]
    pub num_confs: u32,
    #[serde(default)]
    pub confidential_utxos_only: bool,
    #[serde(default)]
    pub utxo_strategy: UtxoStrategy,
}

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct CreatePset {
    pub subaccount: u32,
    pub send_asset: String,
    pub send_amount: u64,
    pub recv_asset: String,
    pub recv_amount: u64,
}

#[derive(Serialize, Deserialize)]
pub struct SwapInput {
    pub txid: String,
    pub vout: u32,
    pub asset: String,
    pub asset_bf: String,
    pub value: u64,
    pub value_bf: String,
}

#[derive(Serialize, Deserialize)]
pub struct PsetMeta {
    pub inputs: Vec<SwapInput>,
    pub recv_addr: String,
    pub change_addr: String,
}

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct SignPset {
    pub subaccount: u32,
    pub send_asset: String,
    pub send_amount: u64,
    pub recv_asset: String,
    pub recv_amount: u64,
    pub pset: String,
}

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct SignedPsetMeta {
    pub pset: String,
}

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct GetTransactionsOpt {
    pub first: usize,
    pub count: usize,
    pub subaccount: u32,
    pub num_confs: Option<u32>,
}

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct GetBalanceOpt {
    pub subaccount: u32,
    pub num_confs: u32,
    #[serde(rename = "confidential")]
    pub confidential_utxos_only: Option<bool>,
}

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct GetUnspentOpt {
    pub subaccount: u32,
    pub num_confs: Option<u32>,
    #[serde(rename = "confidential")]
    pub confidential_utxos_only: Option<bool>,
    pub all_coins: Option<bool>, // unused
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct LoadStoreOpt {
    pub master_xpub: ExtendedPubKey,
    pub master_xpub_fingerprint: Option<Fingerprint>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct GetMasterBlindingKeyResult {
    /// Master blinding key, when encoded in json is an hex of 128 chars
    ///
    /// If the master blinding key is missing from the store,
    /// it is None and the caller should set it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub master_blinding_key: Option<MasterBlindingKey>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct SetMasterBlindingKeyOpt {
    /// Master blinding key, when encoded in json is an hex of 128 chars
    pub master_blinding_key: MasterBlindingKey,
}

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct GetAddressOpt {
    pub subaccount: u32,
    pub address_type: Option<String>, // unused
    pub is_internal: Option<bool>,    // true = get an internal change address
}

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct CreateAccountOpt {
    pub subaccount: u32,
    pub name: String,
    // The account xpub if passed by the caller
    pub xpub: Option<ExtendedPubKey>,
    #[serde(default)]
    pub discovered: bool,
    #[serde(default)]
    pub is_already_created: bool,
    #[serde(skip_deserializing, skip_serializing)]
    pub allow_gaps: bool,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct DiscoverAccountOpt {
    #[serde(rename = "type")]
    pub script_type: ScriptType,
    pub xpub: ExtendedPubKey,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct GetAccountPathOpt {
    pub subaccount: u32,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct GetAccountPathResult {
    pub path: Vec<ChildNumber>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct GetNextAccountOpt {
    #[serde(rename = "type")]
    pub script_type: ScriptType,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct GetAvailableCurrenciesParams {
    /// The url to use to fetch the available currency pairs.
    #[serde(rename = "currency_url")]
    pub url: String,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct RenameAccountOpt {
    pub subaccount: u32,
    pub new_name: String,
}

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct SPVCommonParams {
    /// In which network we are verifying the transaction
    pub network: crate::network::NetworkParameters,

    /// Maximum timeout for network calls,
    /// the final timeout in seconds is roughly equivalent to 2 + `timeout` * 2
    ///
    /// Cannot be specified if `network.proxy` is non empty.
    pub timeout: Option<u8>,

    /// If callers are not handling a cache of the already verified tx, they can set this params to
    /// to enable the cache in the callee side.
    /// Encryption is needed to encrypt the cache content to avoid leaking the txids of the transactions
    pub encryption_key: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct SPVVerifyTxParams {
    #[serde(flatten)]
    pub params: SPVCommonParams,

    /// The `txid` of the transaction to verify
    pub txid: String,

    /// The `height` of the block containing the transaction to be verified
    pub height: u32,
}

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct SPVDownloadHeadersParams {
    #[serde(flatten)]
    pub params: SPVCommonParams,

    /// Number of headers to download at every attempt, it defaults to 2016, useful to set lower
    /// for testing
    pub headers_to_download: Option<usize>,
}

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct SPVDownloadHeadersResult {
    /// Current height tip of the headers downloaded
    pub height: u32,

    /// A reorg happened, any proof with height higher than this struct height must be considered
    /// invalid
    pub reorg: bool,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "snake_case")]
pub enum SPVVerifyTxResult {
    Unconfirmed,
    InProgress,
    Verified,
    NotVerified,
    NotLongest,
    Disabled,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct TransactionMeta {
    #[serde(flatten)]
    pub create_transaction: Option<CreateTransaction>,
    #[serde(rename = "transaction")]
    pub hex: String,
    #[serde(rename = "txhash")]
    pub txid: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub height: Option<u32>,
    pub timestamp: u64, // in microseconds, for confirmed tx is block time for unconfirmed is when created or when list_tx happens
    pub error: String,
    pub addressees_read_only: bool,
    pub is_sweep: bool,
    pub satoshi: Balances,
    pub fee: u64,
    pub network: Option<Network>,
    #[serde(rename = "type")]
    pub type_: String,
    pub changes_used: Option<u32>,
    pub rbf_optin: bool,
    pub spv_verified: SPVVerifyTxResult,
    #[serde(rename = "transaction_weight")]
    pub weight: usize,
    #[serde(rename = "transaction_vsize")]
    pub vsize: usize,
    #[serde(skip)]
    pub size: usize,
    // The utxos used in the transaction
    #[serde(default)]
    pub used_utxos: Vec<UnspentOutput>,
    #[serde(rename = "transaction_version")]
    pub version: u32,
    #[serde(rename = "transaction_locktime")]
    pub lock_time: u32,
    pub transaction_outputs: Vec<TransactionOutput>,
}

impl From<BETransaction> for TransactionMeta {
    fn from(transaction: BETransaction) -> Self {
        let txid = transaction.txid().to_string();
        let hex = transaction.serialize().to_hex();
        let timestamp = now();
        let rbf_optin = transaction.rbf_optin();
        let weight = transaction.get_weight();

        TransactionMeta {
            create_transaction: None,
            height: None,
            timestamp,
            txid,
            hex,
            error: "".to_string(),
            addressees_read_only: false,
            is_sweep: false,
            satoshi: HashMap::new(),
            fee: 0,
            network: None,
            type_: "unknown".to_string(),
            changes_used: None,
            spv_verified: SPVVerifyTxResult::InProgress,
            rbf_optin,
            weight,
            vsize: weight_to_vsize(weight),
            size: transaction.get_size(),
            used_utxos: vec![],
            version: transaction.version(),
            lock_time: transaction.lock_time(),
            transaction_outputs: vec![],
        }
    }
}
impl From<BETransactionEntry> for TransactionMeta {
    fn from(txe: BETransactionEntry) -> Self {
        let mut txm: TransactionMeta = txe.tx.into();
        // Overwrite with correct (v)size and weight
        // (i.e. the ones before stripping the witness)
        txm.weight = txe.weight;
        txm.vsize = weight_to_vsize(txe.weight);
        txm.size = txe.size;
        txm
    }
}
impl TransactionMeta {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        transaction: impl Into<TransactionMeta>,
        height: Option<u32>,
        timestamp: Option<u64>,
        satoshi: Balances,
        fee: u64,
        network: Network,
        type_: String,
        create_transaction: CreateTransaction,
        spv_verified: SPVVerifyTxResult,
    ) -> Self {
        let mut wgtx: TransactionMeta = transaction.into();
        let timestamp = timestamp.unwrap_or_else(now);

        wgtx.create_transaction = Some(create_transaction);
        wgtx.height = height;
        wgtx.timestamp = timestamp;
        wgtx.satoshi = satoshi;
        wgtx.network = Some(network);
        wgtx.fee = fee;
        wgtx.type_ = type_;
        wgtx.spv_verified = spv_verified;
        wgtx
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransactionOutput {
    pub address: String, // Only used by Trezor
    pub address_type: String,

    /// True if the corresponding scriptpubkey belongs to the account (not the wallet)
    pub is_relevant: bool,

    pub subaccount: u32,
    pub is_internal: bool,
    pub is_change: bool, // Same as is_internal
    pub pointer: u32,    // child_number in bip32 terminology
    pub user_path: Vec<ChildNumber>,

    pub pt_idx: u32, // vout
    #[serde(rename = "script")]
    pub script_pubkey: String,
    pub satoshi: u64,
}

/// Input and output element for get_transactions
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetTxInOut {
    /// The address of the input or output.
    ///
    /// For Liquid is blinded for relevant elements, and unblinded for not relevant elements.
    ///
    /// For not relevant Liquid inputs it might be empty,
    /// because we don't need to fecth previous transaction for the fee computation.
    pub address: String,

    /// The address type of the element.
    ///
    /// Empty if the element is not relevant.
    // TODO: use an enum and sort out AddressType/ScriptType
    pub address_type: String,

    /// Whether the elements is an input or an output.
    pub is_output: bool,

    /// Whether the corresponding scriptpubkey belongs to the account (not the wallet).
    pub is_relevant: bool,

    /// Whether the element is spent.
    ///
    /// For outputs the computation is expensive and might require additional network calls,
    /// thus for now it is always false.
    pub is_spent: bool,

    /// The subaccount the element belongs to.
    ///
    /// 0 if not relevant.
    pub subaccount: u32,

    /// Whether the element belongs to the internal chain.
    ///
    /// False if not relevant.
    pub is_internal: bool,

    /// Child number in bip32 terminology.
    ///
    /// 0 if not relevant.
    pub pointer: u32,

    /// If output the vout, if input the vin.
    pub pt_idx: u32,

    /// The amount associated to the element.
    ///
    /// For liquid is 0 if the amount cannot be unblinded.
    pub satoshi: u64,

    /// Multisig field, always 0.
    pub script_type: u32,

    /// Multisig field, always 0.
    pub subtype: u32,

    // Liquid fields
    /// The asset id.
    ///
    /// None if not liquid or not unblindable.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub asset_id: Option<String>,

    /// The asset blinder (aka asset blinding factor or abf).
    ///
    /// None if not liquid or not unblindable.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "assetblinder")]
    pub asset_blinder: Option<String>,

    /// The amount blinder (aka value blinding factor or vbf).
    ///
    /// None if not liquid or not unblindable.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "amountblinder")]
    pub amount_blinder: Option<String>,

    /// Whether the amount and asset are blinded or not.
    ///
    /// None for not relevant elements.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_blinded: Option<bool>,

    /// Whether the address is a confidential address or not.
    ///
    /// None for not relevant elements.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_confidential: Option<bool>,

    /// Blinding public key.
    ///
    /// None for not relevant elements.
    /// Note that it might not be the key that was actually used by the sender.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub blinding_key: Option<String>,

    /// Unblinded address.
    ///
    /// None for not relevant elements.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unconfidential_address: Option<String>,

    /// Scriptpukey.
    ///
    /// None for not relevant Liquid inputs (for which the address is the empty string).
    #[serde(rename = "script")]
    pub script_pubkey: String,
}

/// Transaction type
///
/// Note that the follwing types might be inaccurate for complex
/// transactions such as coinjoins or involving multiple (sub)accounts.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TransactionType {
    Unknown,
    Incoming,
    Outgoing,
    Redeposit,
    #[serde(rename = "not unblindable")]
    NotUnblindable,
    Mixed,
}

impl Default for TransactionType {
    fn default() -> Self {
        Self::Unknown
    }
}

impl TransactionType {
    pub fn user_signed(&self) -> bool {
        match self {
            TransactionType::Outgoing | TransactionType::Redeposit | TransactionType::Mixed => true,
            _ => false,
        }
    }
}

// TODO remove TxListItem, make TransactionMeta compatible and automatically serialized
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TxListItem {
    pub block_height: u32,
    pub created_at_ts: u64, // in microseconds
    #[serde(rename = "type")]
    pub type_: TransactionType,
    pub memo: String,
    pub txhash: String,
    // Disable to be able detect swap transactions
    //#[serde(serialize_with = "serialize_tx_balances")]
    pub satoshi: Balances,
    pub rbf_optin: bool,
    pub can_cpfp: bool,
    pub can_rbf: bool,
    pub spv_verified: String,
    pub fee: u64,
    pub fee_rate: u64,
    pub inputs: Vec<GetTxInOut>,
    pub outputs: Vec<GetTxInOut>,
    #[serde(skip)]
    pub transaction_size: usize,
    pub transaction_vsize: usize,
    pub transaction_weight: usize,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct AccountInfo {
    #[serde(rename = "pointer")]
    pub account_num: u32,
    #[serde(rename = "type")]
    pub script_type: ScriptType,
    #[serde(flatten)]
    pub settings: AccountSettings,
    pub required_ca: u32,     // unused, always 0
    pub receiving_id: String, // unused, always ""
    pub bip44_discovered: bool,
    pub user_path: Vec<ChildNumber>,
    pub core_descriptors: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub slip132_extended_pubkey: Option<String>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct AccountInfoPruned {
    #[serde(rename = "pointer")]
    pub account_num: u32,
    #[serde(rename = "type")]
    pub script_type: ScriptType,
    #[serde(flatten)]
    pub settings: AccountSettings,
    pub required_ca: u32,     // unused, always 0
    pub receiving_id: String, // unused, always ""
    pub bip44_discovered: bool,
}

impl From<AccountInfo> for AccountInfoPruned {
    fn from(info: AccountInfo) -> Self {
        Self {
            account_num: info.account_num,
            script_type: info.script_type,
            settings: info.settings.clone(),
            required_ca: info.required_ca,
            receiving_id: info.receiving_id.clone(),
            bip44_discovered: info.bip44_discovered,
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Credentials {
    pub mnemonic: String,
    #[serde(default)]
    pub bip39_passphrase: String,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "snake_case")]
pub enum WatchOnlyCredentials {
    Slip132ExtendedPubkeys(Vec<String>),
    CoreDescriptors(Vec<String>),
}

/// An intermediate struct to hold account data
#[derive(Debug, Clone)]
pub struct AccountData {
    pub account_num: u32,
    pub xpub: ExtendedPubKey,
    pub master_xpub_fingerprint: Option<Fingerprint>,
}

fn from_slip132_extended_pubkey(s: &str, expected_is_mainnet: bool) -> Result<AccountData, Error> {
    let (is_mainnet, script_type, xpub) = decode_from_slip132_string(s)?;
    if is_mainnet != expected_is_mainnet {
        return Err(Error::MismatchingNetwork);
    }

    let bip32_account = extract_bip32_account(&xpub)?;
    let account_num = bip32_account * 16 + script_type.num();

    Ok(AccountData {
        account_num,
        xpub,
        master_xpub_fingerprint: None,
    })
}

fn from_descriptor(s: &str, expected_is_mainnet: bool) -> Result<AccountData, Error> {
    let coin_type = if expected_is_mainnet {
        0
    } else {
        1
    };
    let (script_type, xpub, bip32_account, master_xpub_fingerprint) =
        parse_single_sig_descriptor(s, coin_type)?;
    let is_mainnet = match xpub.network {
        Network::Bitcoin => true,
        _ => false,
    };
    if is_mainnet != expected_is_mainnet {
        return Err(Error::MismatchingNetwork);
    }

    let account_num = bip32_account * 16 + script_type.num();

    Ok(AccountData {
        account_num,
        xpub,
        master_xpub_fingerprint: Some(master_xpub_fingerprint),
    })
}

impl WatchOnlyCredentials {
    // Derive an extended public key to use for the store
    pub fn store_master_xpub(
        &self,
        net_params: &NetworkParameters,
    ) -> Result<ExtendedPubKey, Error> {
        let network = if net_params.mainnet {
            Network::Bitcoin
        } else {
            Network::Testnet
        };
        let b = serde_json::to_vec(self).unwrap();
        let seed = sha256::Hash::hash(&b);
        let xprv = ExtendedPrivKey::new_master(network, &seed)?;
        let xpub = ExtendedPubKey::from_priv(&crate::EC, &xprv);
        Ok(xpub)
    }

    pub fn accounts(self, is_mainnet: bool) -> Result<(Vec<AccountData>, Fingerprint), Error> {
        let r: Result<Vec<_>, _> = match self {
            WatchOnlyCredentials::Slip132ExtendedPubkeys(keys) => {
                keys.iter().map(|k| from_slip132_extended_pubkey(&k, is_mainnet)).collect()
            }
            WatchOnlyCredentials::CoreDescriptors(descriptors) => {
                descriptors.iter().map(|d| from_descriptor(&d, is_mainnet)).collect()
            }
        };
        // Handle duplicates
        let mut m = HashMap::<u32, ExtendedPubKey>::new();
        let mut master_xpub_fingerprint = None;
        for a in r? {
            if let Some(old) = m.insert(a.account_num, a.xpub.clone()) {
                if old != a.xpub {
                    return Err(Error::MismatchingXpub);
                }
            };
            // Check all master_xpub fingerprints are equal
            match master_xpub_fingerprint {
                None => {
                    master_xpub_fingerprint = a.master_xpub_fingerprint;
                }
                Some(f) => {
                    if Some(f) != a.master_xpub_fingerprint {
                        return Err(Error::MismatchingDescriptor);
                    }
                }
            }
        }
        let v = m
            .iter()
            .map(|(k, v)| AccountData {
                account_num: *k,
                xpub: *v,
                master_xpub_fingerprint: master_xpub_fingerprint.clone(),
            })
            .collect();
        let master_xpub_fingerprint = master_xpub_fingerprint.unwrap_or_default();
        Ok((v, master_xpub_fingerprint))
    }
}

#[derive(Serialize, Deserialize, Debug)]
pub struct AddressPointer {
    pub subaccount: u32,
    pub address_type: String,
    pub address: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "blinding_script")]
    pub script_pubkey: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub blinding_key: Option<String>,
    pub pointer: u32, // child_number in bip32 terminology
    pub user_path: Vec<ChildNumber>,
    pub is_internal: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_confidential: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unconfidential_address: Option<String>,
}

// This one is simple enough to derive a serializer
#[derive(Serialize, Debug, Clone, Deserialize)]
pub struct FeeEstimate(pub u64);
pub struct TxsResult(pub Vec<TxListItem>);

/// Change to the model of Settings and Pricing structs could break old versions.
/// You can't remove fields, change fields type and if you add a new field, it must be Option<T>
#[derive(Serialize, Deserialize, Debug, PartialEq, Clone)]
pub struct Settings {
    pub unit: String,
    pub required_num_blocks: u32,
    pub altimeout: u32,
    pub pricing: Pricing,
    pub sound: bool,
}

impl Settings {
    pub fn update(&mut self, json: &serde_json::Value) {
        if let Some(unit) = json.get("unit").and_then(|v| v.as_str()) {
            self.unit = unit.to_string();
        }
        if let Some(required_num_blocks) = json.get("required_num_blocks").and_then(|v| v.as_u64())
        {
            self.required_num_blocks = required_num_blocks as u32;
        }
        if let Some(altimeout) = json.get("altimeout").and_then(|v| v.as_u64()) {
            self.altimeout = altimeout as u32;
        }
        if let Some(pricing) =
            json.get("pricing").and_then(|v| serde_json::from_value(v.clone()).ok())
        {
            self.pricing = pricing;
        }
        if let Some(sound) = json.get("sound").and_then(|v| v.as_bool()) {
            self.sound = sound;
        }
    }
}

#[derive(Default, Serialize, Deserialize, Clone, Debug)]
pub struct AccountSettings {
    pub name: String,
    pub hidden: bool,
}

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct UpdateAccountOpt {
    pub subaccount: u32,
    pub name: Option<String>,
    pub hidden: Option<bool>,
}

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct SetAccountHiddenOpt {
    pub subaccount: u32,
    pub hidden: bool,
}

/// see comment for struct Settings
#[derive(Serialize, Deserialize, Debug, PartialEq, Clone)]
pub struct Pricing {
    currency: String,
    exchange: String,
}

impl Default for Settings {
    fn default() -> Self {
        let pricing = Pricing {
            currency: "USD".to_string(),
            exchange: "BITFINEX".to_string(),
        };
        Settings {
            unit: "BTC".to_string(),
            required_num_blocks: 12,
            altimeout: 5,
            pricing,
            sound: true,
        }
    }
}

impl SPVVerifyTxResult {
    pub fn as_i32(&self) -> i32 {
        match self {
            SPVVerifyTxResult::InProgress => 0,
            SPVVerifyTxResult::Verified => 1,
            SPVVerifyTxResult::NotVerified => 2,
            SPVVerifyTxResult::Disabled => 3,
            SPVVerifyTxResult::NotLongest => 4,
            SPVVerifyTxResult::Unconfirmed => 5,
        }
    }
}

impl Display for SPVVerifyTxResult {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SPVVerifyTxResult::InProgress => write!(f, "in_progress"),
            SPVVerifyTxResult::Verified => write!(f, "verified"),
            SPVVerifyTxResult::NotVerified => write!(f, "not_verified"),
            SPVVerifyTxResult::Disabled => write!(f, "disabled"),
            SPVVerifyTxResult::NotLongest => write!(f, "not_longest"),
            SPVVerifyTxResult::Unconfirmed => write!(f, "unconfirmed"),
        }
    }
}

// In create_transaction, the caller passes the utxos in the same format as they are returned by
// get_unspent_output, but we only care about the outpoint, since we can obtain the remaining data
// from the db.
// CreateTxUtxo and CreateTxUtxos allows us to accept the serialized GetUnspentOutputs, ignoring
// the fields we are not interested in.
#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CreateTxUtxo {
    #[serde(rename = "txhash")]
    pub txid: String,
    #[serde(rename = "pt_idx")]
    pub vout: u32,
}

pub type CreateTxUtxos = HashMap<String, Vec<CreateTxUtxo>>;

impl CreateTxUtxo {
    pub fn outpoint(&self, id: NetworkId) -> Result<BEOutPoint, Error> {
        let betxid = BETxid::from_hex(&self.txid, id)?;
        Ok(BEOutPoint::new(betxid, self.vout))
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Txo {
    pub outpoint: BEOutPoint,

    pub height: Option<u32>,

    pub public_key: bitcoin::PublicKey,
    pub script_pubkey: BEScript,
    pub script_code: BEScript,

    pub subaccount: u32,
    pub script_type: ScriptType,

    /// The full path from the master key
    pub user_path: Vec<ChildNumber>,

    pub satoshi: u64,

    pub sequence: Option<u32>,

    /// The Liquid commitment preimages (asset, satoshi and blinders)
    pub txoutsecrets: Option<elements::TxOutSecrets>,
    /// The Liquid commitments
    pub txoutcommitments: Option<(confidential::Asset, confidential::Value, confidential::Nonce)>,
}

impl Txo {
    pub fn confidential(&self) -> Option<bool> {
        self.txoutsecrets.as_ref().map(is_confidential_txoutsecrets)
    }

    pub fn is_confidential(&self) -> bool {
        self.confidential().unwrap_or(false)
    }

    pub fn asset_id(&self) -> Option<elements::issuance::AssetId> {
        self.txoutsecrets.as_ref().map(|s| s.asset.clone())
    }
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GetUnspentOutputs(pub HashMap<String, Vec<UnspentOutput>>);

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UnspentOutput {
    pub address_type: String,
    pub block_height: u32,
    pub pointer: u32,
    pub pt_idx: u32,
    pub satoshi: u64,
    pub subaccount: u32,
    pub txhash: String,
    /// `true` iff belongs to internal chain, i.e. is change
    pub is_internal: bool,
    pub user_path: Vec<ChildNumber>,
    #[serde(skip)]
    pub scriptpubkey: BEScript,
    /// This can be Some only when this describes an input
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sequence: Option<u32>,
    /// This can be Some only when this describes an input
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "user_sighash")]
    pub sighash: Option<u32>,
    #[serde(rename = "prevout_script")]
    pub script_code: String,
    pub public_key: String,
    #[serde(default)]
    #[serde(skip_serializing)]
    pub skip_signing: bool,

    // liquid fields
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_blinded: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_confidential: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub asset_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "assetblinder")]
    pub asset_blinder: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "amountblinder")]
    pub amount_blinder: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "asset_tag")]
    pub asset_commitment: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "commitment")]
    pub value_commitment: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub nonce_commitment: Option<String>,
}

impl UnspentOutput {
    pub fn sighash(&self) -> Result<BESigHashType, Error> {
        let is_elements = self.asset_id.is_some();
        let sighash = self.sighash.unwrap_or(BitcoinSigHashType::All as u32);
        BESigHashType::from_u32(sighash, is_elements)
    }
}

impl TryFrom<Txo> for UnspentOutput {
    type Error = Error;

    fn try_from(txo: Txo) -> Result<Self, Error> {
        let (is_internal, pointer) = parse_path(&txo.user_path.clone().into())?;
        let asset_id = txo.txoutsecrets.as_ref().map(|s| s.asset.to_hex());
        let is_blinded = txo.confidential();
        let is_confidential = txo.txoutsecrets.as_ref().map(|_| false);
        let asset_blinder = txo.txoutsecrets.as_ref().map(|s| s.asset_bf.to_hex());
        let amount_blinder = txo.txoutsecrets.as_ref().map(|s| s.value_bf.to_hex());
        let (asset_commitment, value_commitment, nonce_commitment) = match &txo.txoutcommitments {
            None => (None, None, None),
            Some((a, v, n)) => (
                Some(elements::encode::serialize_hex(a)),
                Some(elements::encode::serialize_hex(v)),
                Some(elements::encode::serialize_hex(n)),
            ),
        };
        Ok(Self {
            txhash: txo.outpoint.txid().to_hex(),
            pt_idx: txo.outpoint.vout(),
            block_height: txo.height.unwrap_or(0),
            public_key: txo.public_key.to_string(),
            scriptpubkey: txo.script_pubkey,
            script_code: txo.script_code.to_hex(),
            subaccount: txo.subaccount,
            address_type: txo.script_type.to_string(),
            user_path: txo.user_path,
            is_internal,
            pointer,
            satoshi: txo.satoshi,
            sequence: txo.sequence,
            sighash: None,
            skip_signing: false,
            is_blinded,
            is_confidential,
            asset_id,
            asset_blinder,
            amount_blinder,
            asset_commitment,
            value_commitment,
            nonce_commitment,
        })
    }
}

/// Partially parse the derivation path and return (is_internal, address_pointer)
pub fn parse_path(path: &DerivationPath) -> Result<(bool, u32), Error> {
    let address_pointer;
    let is_internal;
    let mut iter = path.into_iter().rev();
    if let Some(&ChildNumber::Normal {
        index,
    }) = iter.next()
    {
        // last
        address_pointer = index;
    } else {
        return Err(Error::Generic("Unexpected derivation path".into()));
    };
    if let Some(&ChildNumber::Normal {
        index,
    }) = iter.next()
    {
        // second-to-last
        is_internal = index == 1;
    } else {
        return Err(Error::Generic("Unexpected derivation path".into()));
    };
    Ok((is_internal, address_pointer))
}

// Output of get_transaction_details
#[derive(Serialize, Debug, Clone)]
pub struct TransactionDetails {
    pub transaction: String,
    pub txhash: String,
    pub transaction_locktime: u32,
    pub transaction_version: u32,
    #[serde(skip)]
    pub transaction_size: usize,
    pub transaction_vsize: usize,
    pub transaction_weight: usize,
}

// Output of get_scriptpubkey_data
#[derive(Serialize, Debug, Clone)]
pub struct ScriptPubKeyData {
    pub subaccount: u32,
    pub branch: u32, // Always 1
    pub pointer: u32,
    pub subtype: u32, // Always 0
    pub is_internal: bool,
}

impl From<&BETransactionEntry> for TransactionDetails {
    fn from(tx_entry: &BETransactionEntry) -> Self {
        Self {
            transaction: tx_entry.tx.serialize().to_hex(),
            txhash: tx_entry.tx.txid().to_string(),
            transaction_locktime: tx_entry.tx.lock_time(),
            transaction_version: tx_entry.tx.version(),
            transaction_size: tx_entry.size,
            transaction_vsize: weight_to_vsize(tx_entry.weight),
            transaction_weight: tx_entry.weight,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct GetPreviousAddressesOpt {
    /// The subaccount to get the addresses for.
    pub subaccount: u32,

    /// The last pointer returned by a previous call.
    ///
    /// Use None to get the newest generated addresses.
    pub last_pointer: Option<u32>,

    /// Whether to get the addresses belonging to the internal chain or the external one.
    #[serde(default)]
    pub is_internal: bool,

    /// The number of addresses to return at most.
    ///
    /// This is needed for pagination.
    pub count: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct PreviousAddress {
    /// The address.
    ///
    /// For Liquid is blinded.
    pub address: String,
    pub address_type: String,
    pub subaccount: u32,
    pub is_internal: bool,

    /// The last child number in bip32 terminology.
    pub pointer: u32,

    #[serde(rename = "script")]
    pub script_pubkey: String,

    /// The full path from the master key
    pub user_path: Vec<ChildNumber>,

    /// The number of transactions where either an input or an output has a script pubkey matching
    /// this address.
    pub tx_count: u32,

    // Liquid fields, None if Bitcoin
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_confidential: Option<bool>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub unconfidential_address: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub blinding_script: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub blinding_key: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct PreviousAddresses {
    /// The last pointer returned by this call.
    ///
    /// None if all addresses have been fetched.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_pointer: Option<u32>,

    /// The previous addresses
    pub list: Vec<PreviousAddress>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AddressDataRequest {
    pub address: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AddressDataResult {
    pub user_path: Vec<ChildNumber>,
}

#[cfg(test)]
mod test {
    use crate::model::{parse_path, CreateTxUtxos, GetUnspentOutputs};
    use bitcoin::util::bip32::DerivationPath;

    #[test]
    fn test_path() {
        let path_external: DerivationPath = "m/44'/1'/0'/0/0".parse().unwrap();
        let path_internal: DerivationPath = "m/44'/1'/0'/1/0".parse().unwrap();
        assert_eq!(parse_path(&path_external).unwrap(), (false, 0u32));
        assert_eq!(parse_path(&path_internal).unwrap(), (true, 0u32));
    }

    #[test]
    fn test_unspent() {
        let json_str = r#"{"btc": [{"address_type": "p2wsh", "block_height": 1806588, "pointer": 3509, "pt_idx": 1, "satoshi": 3650144, "subaccount": 0, "txhash": "08711d45d4867d7834b133a425da065b252eb6a9b206d57e2bbb226a344c5d13", "is_internal": false, "is_blinded": false, "user_path": [2147483692, 2147483649, 2147483648, 0, 1], "prevout_script": "51", "public_key": "020202020202020202020202020202020202020202020202020202020202020202", "asset_id": ""}, {"address_type": "p2wsh", "block_height": 1835681, "pointer": 3510, "pt_idx": 0, "satoshi": 5589415, "subaccount": 0, "txhash": "fbd00e5b9e8152c04214c72c791a78a65fdbab68b5c6164ff0d8b22a006c5221", "is_internal": false, "is_blinded": false, "user_path": [2147483692, 2147483649, 2147483648, 0, 2], "prevout_script": "51", "public_key": "020202020202020202020202020202020202020202020202020202020202020202", "asset_id": ""}, {"address_type": "p2wsh", "block_height": 1835821, "pointer": 3511, "pt_idx": 0, "satoshi": 568158, "subaccount": 0, "txhash": "e5b358fb8366960130b97794062718d7f4fbe721bf274f47493a19326099b811", "is_internal": false, "is_blinded": false, "user_path": [2147483692, 2147483649, 2147483648, 0, 3], "prevout_script": "51", "public_key": "020202020202020202020202020202020202020202020202020202020202020202", "asset_id": ""}]}"#;
        let _json: GetUnspentOutputs = serde_json::from_str(json_str).unwrap();
        let _json: CreateTxUtxos = serde_json::from_str(json_str).unwrap();
    }
}
