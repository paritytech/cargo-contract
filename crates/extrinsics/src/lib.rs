// Copyright 2018-2023 Parity Technologies (UK) Ltd.
// This file is part of cargo-contract.
//
// cargo-contract is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// cargo-contract is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.
//
// You should have received a copy of the GNU General Public License
// along with cargo-contract.  If not, see <http://www.gnu.org/licenses/>.

mod balance;
mod call;
mod error;
mod events;
mod extrinsic_opts;
mod instantiate;
mod remove;
mod runtime_api;
mod upload;

#[cfg(test)]
#[cfg(feature = "integration-tests")]
mod integration_tests;

use subxt::utils::AccountId32;

use anyhow::{
    anyhow,
    Context,
    Ok,
    Result,
};
use jsonrpsee::{
    core::client::ClientT,
    rpc_params,
    ws_client::WsClientBuilder,
};
use std::path::PathBuf;

use crate::runtime_api::api;
use contract_build::{
    CrateMetadata,
    DEFAULT_KEY_COL_WIDTH,
};
use scale::{
    Decode,
    Encode,
};
use sp_core::{
    hashing,
    Bytes,
};
use subxt::{
    blocks,
    config,
    tx,
    Config,
    OnlineClient,
};
use subxt_signer::sr25519::Keypair;

use std::{
    option::Option,
    path::Path,
};

pub use balance::{
    BalanceVariant,
    TokenMetadata,
};
pub use call::{
    CallCommandBuilder,
    CallExec,
    CallRequest,
};
use contract_metadata::ContractMetadata;
pub use contract_transcode::ContractMessageTranscoder;
pub use error::{
    ErrorVariant,
    GenericError,
};
pub use events::DisplayEvents;
pub use extrinsic_opts::{
    state,
    ExtrinsicOptsBuilder,
    Missing,
};
pub use instantiate::{
    Code,
    InstantiateArgs,
    InstantiateCommandBuilder,
    InstantiateDryRunResult,
    InstantiateExec,
    InstantiateExecResult,
};
pub use remove::{
    RemoveCommandBuilder,
    RemoveExec,
};

pub use subxt::PolkadotConfig as DefaultConfig;
pub use upload::{
    CodeUploadRequest,
    UploadCommandBuilder,
    UploadExec,
    UploadResult,
};

pub type Client = OnlineClient<DefaultConfig>;
pub type Balance = u128;
pub type CodeHash = <DefaultConfig as Config>::Hash;

/// Contract artifacts for use with extrinsic commands.
#[derive(Debug)]
pub struct ContractArtifacts {
    /// The original artifact path
    artifacts_path: PathBuf,
    /// The expected path of the file containing the contract metadata.
    metadata_path: PathBuf,
    /// The deserialized contract metadata if the expected metadata file exists.
    metadata: Option<ContractMetadata>,
    /// The Wasm code of the contract if available.
    pub code: Option<WasmCode>,
}

impl ContractArtifacts {
    /// Load contract artifacts.
    pub fn from_manifest_or_file(
        manifest_path: Option<&PathBuf>,
        file: Option<&PathBuf>,
    ) -> Result<ContractArtifacts> {
        let artifact_path = match (manifest_path, file) {
            (manifest_path, None) => {
                let crate_metadata = CrateMetadata::from_manifest_path(
                    manifest_path,
                    contract_build::Target::Wasm,
                )?;

                if crate_metadata.contract_bundle_path().exists() {
                    crate_metadata.contract_bundle_path()
                } else if crate_metadata.metadata_path().exists() {
                    crate_metadata.metadata_path()
                } else {
                    anyhow::bail!(
                        "Failed to find any contract artifacts in target directory. \n\
                        Run `cargo contract build --release` to generate the artifacts."
                    )
                }
            }
            (None, Some(artifact_file)) => artifact_file.clone(),
            (Some(_), Some(_)) => {
                anyhow::bail!("conflicting options: --manifest-path and --file")
            }
        };
        Self::from_artifact_path(artifact_path.as_path())
    }
    /// Given a contract artifact path, load the contract code and metadata where
    /// possible.
    fn from_artifact_path(path: &Path) -> Result<Self> {
        tracing::debug!("Loading contracts artifacts from `{}`", path.display());
        let (metadata_path, metadata, code) =
            match path.extension().and_then(|ext| ext.to_str()) {
                Some("contract") | Some("json") => {
                    let metadata = ContractMetadata::load(path)?;
                    let code = metadata.clone().source.wasm.map(|wasm| WasmCode(wasm.0));
                    (PathBuf::from(path), Some(metadata), code)
                }
                Some("wasm") => {
                    let file_name = path.file_stem()
                        .context("WASM bundle file has unreadable name")?
                        .to_str()
                        .context("Error parsing filename string")?;
                    let code = Some(WasmCode(std::fs::read(path)?));
                    let dir = path.parent().map_or_else(PathBuf::new, PathBuf::from);
                    let metadata_path = dir.join(format!("{file_name}.json"));
                    if !metadata_path.exists() {
                        (metadata_path, None, code)
                    } else {
                        let metadata = ContractMetadata::load(&metadata_path)?;
                        (metadata_path, Some(metadata), code)
                    }
                }
                Some(ext) => anyhow::bail!(
                    "Invalid artifact extension {ext}, expected `.contract`, `.json` or `.wasm`"
                ),
                None => {
                    anyhow::bail!(
                        "Artifact path has no extension, expected `.contract`, `.json`, or `.wasm`"
                    )
                }
            };
        Ok(Self {
            artifacts_path: path.into(),
            metadata_path,
            metadata,
            code,
        })
    }

    /// Get the path of the artifact file used to load the artifacts.
    pub fn artifact_path(&self) -> &Path {
        self.artifacts_path.as_path()
    }

    /// Get contract metadata, if available.
    ///
    /// ## Errors
    /// - No contract metadata could be found.
    /// - Invalid contract metadata.
    pub fn metadata(&self) -> Result<ContractMetadata> {
        self.metadata.clone().ok_or_else(|| {
            anyhow!(
                "No contract metadata found. Expected file {}",
                self.metadata_path.as_path().display()
            )
        })
    }

    /// Get the code hash from the contract metadata.
    pub fn code_hash(&self) -> Result<[u8; 32]> {
        let metadata = self.metadata()?;
        Ok(metadata.source.hash.0)
    }

    /// Construct a [`ContractMessageTranscoder`] from contract metadata.
    pub fn contract_transcoder(&self) -> Result<ContractMessageTranscoder> {
        let metadata = self.metadata()?;
        ContractMessageTranscoder::try_from(metadata)
            .context("Failed to deserialize ink project metadata from contract metadata")
    }
}

/// The Wasm code of a contract.
#[derive(Debug)]
pub struct WasmCode(Vec<u8>);

impl WasmCode {
    /// The hash of the contract code: uniquely identifies the contract code on-chain.
    pub fn code_hash(&self) -> [u8; 32] {
        contract_build::code_hash(&self.0)
    }
}

/// Get the account id from the Keypair
pub fn account_id(keypair: &Keypair) -> AccountId32 {
    subxt::tx::Signer::<DefaultConfig>::account_id(keypair)
}

/// Wait for the transaction to be included successfully into a block.
///
/// # Errors
///
/// If a runtime Module error occurs, this will only display the pallet and error indices.
/// Dynamic lookups of the actual error will be available once the following issue is
/// resolved: <https://github.com/paritytech/subxt/issues/443>.
///
/// # Finality
///
/// Currently this will report success once the transaction is included in a block. In the
/// future there could be a flag to wait for finality before reporting success.
async fn submit_extrinsic<T, Call, Signer>(
    client: &OnlineClient<T>,
    call: &Call,
    signer: &Signer,
) -> core::result::Result<blocks::ExtrinsicEvents<T>, subxt::Error>
where
    T: Config,
    Call: tx::TxPayload,
    Signer: tx::Signer<T>,
    <T::ExtrinsicParams as config::ExtrinsicParams<T::Hash>>::OtherParams: Default,
{
    client
        .tx()
        .sign_and_submit_then_watch_default(call, signer)
        .await?
        .wait_for_in_block()
        .await?
        .wait_for_success()
        .await
}

async fn state_call<A: Encode, R: Decode>(url: &str, func: &str, args: A) -> Result<R> {
    let cli = WsClientBuilder::default().build(&url).await?;
    let params = rpc_params![func, Bytes(args.encode())];
    let bytes: Bytes = cli.request("state_call", params).await?;
    Ok(R::decode(&mut bytes.as_ref())?)
}

/// Parse a hex encoded 32 byte hash. Returns error if not exactly 32 bytes.
pub fn parse_code_hash(input: &str) -> Result<<DefaultConfig as Config>::Hash> {
    let bytes = contract_build::util::decode_hex(input)?;
    if bytes.len() != 32 {
        anyhow::bail!("Code hash should be 32 bytes in length")
    }
    let mut arr = [0u8; 32];
    arr.copy_from_slice(&bytes);
    Ok(arr.into())
}

/// Fetch the contract info from the storage using the provided client.
pub async fn fetch_contract_info(
    contract: &AccountId32,
    client: &Client,
) -> Result<Option<ContractInfo>> {
    let info_contract_call = api::storage().contracts().contract_info_of(contract);

    let contract_info_of = client
        .storage()
        .at_latest()
        .await?
        .fetch(&info_contract_call)
        .await?;

    match contract_info_of {
        Some(info_result) => {
            let convert_trie_id = hex::encode(info_result.trie_id.0);
            Ok(Some(ContractInfo {
                trie_id: convert_trie_id,
                code_hash: info_result.code_hash,
                storage_items: info_result.storage_items,
                storage_item_deposit: info_result.storage_item_deposit,
            }))
        }
        None => Ok(None),
    }
}

#[derive(serde::Serialize)]
pub struct ContractInfo {
    trie_id: String,
    code_hash: CodeHash,
    storage_items: u32,
    storage_item_deposit: Balance,
}

impl ContractInfo {
    /// Convert and return contract info in JSON format.
    pub fn to_json(&self) -> Result<String> {
        Ok(serde_json::to_string_pretty(self)?)
    }

    /// Return the trie_id of the contract.
    pub fn trie_id(&self) -> &str {
        &self.trie_id
    }

    /// Return the code_hash of the contract.
    pub fn code_hash(&self) -> &CodeHash {
        &self.code_hash
    }

    /// Return the number of storage items of the contract.
    pub fn storage_items(&self) -> u32 {
        self.storage_items
    }

    /// Return the storage item deposit of the contract.
    pub fn storage_item_deposit(&self) -> Balance {
        self.storage_item_deposit
    }
}

/// Fetch the contract wasm code from the storage using the provided client and code hash.
pub async fn fetch_wasm_code(hash: CodeHash, client: &Client) -> Result<Option<Vec<u8>>> {
    let pristine_code_address = api::storage().contracts().pristine_code(hash);

    let pristine_bytes = client
        .storage()
        .at_latest()
        .await?
        .fetch(&pristine_code_address)
        .await?
        .map(|v| v.0);

    Ok(pristine_bytes)
}

/// Fetch all contracts addresses from the storage using the provided client and count of
/// requested elements starting from an optional address
pub async fn fetch_all_contracts(
    client: &Client,
    count: u32,
    from: Option<&AccountId32>,
) -> Result<Vec<AccountId32>> {
    // Pallet name Twox128 hash
    let pallet_hash = hashing::twox_128(b"Contracts");

    // Map name Twox128 hash
    let map_hash = hashing::twox_128(b"ContractInfoOf");
    let key = [pallet_hash, map_hash].concat();

    let start_key = from
        .map(|e| [key.clone(), hashing::twox_64(&e.0).to_vec(), e.0.to_vec()].concat());

    let keys = client
        .rpc()
        .storage_keys_paged(key.as_slice(), count, start_key.as_deref(), None)
        .await?;

    // StorageKey is a concatention of Twox128("Contracts"), Twox128("ContractInfoOf") and
    // Twox64Concat(AccountId)
    let contracts = keys
        .into_iter()
        .map(|e| {
            let mut account =
                e.0.get(16 + 16 + 8..)
                    .ok_or(anyhow!("Slice out of range"))?;
            AccountId32::decode(&mut account)
                .map_err(|err| anyhow!("Derialization error: {}", err))
        })
        .collect::<Result<_, _>>()?;

    Ok(contracts)
}

/// Copy of `pallet_contracts_primitives::StorageDeposit` which implements `Serialize`,
/// required for json output.
#[derive(Eq, PartialEq, Ord, PartialOrd, Clone, serde::Serialize)]
pub enum StorageDeposit {
    /// The transaction reduced storage consumption.
    ///
    /// This means that the specified amount of balance was transferred from the involved
    /// contracts to the call origin.
    Refund(Balance),
    /// The transaction increased overall storage usage.
    ///
    /// This means that the specified amount of balance was transferred from the call
    /// origin to the contracts involved.
    Charge(Balance),
}

impl From<&pallet_contracts_primitives::StorageDeposit<Balance>> for StorageDeposit {
    fn from(deposit: &pallet_contracts_primitives::StorageDeposit<Balance>) -> Self {
        match deposit {
            pallet_contracts_primitives::StorageDeposit::Refund(balance) => {
                Self::Refund(*balance)
            }
            pallet_contracts_primitives::StorageDeposit::Charge(balance) => {
                Self::Charge(*balance)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_code_hash_works() {
        // with 0x prefix
        assert!(parse_code_hash(
            "0xd43593c715fdd31c61141abd04a99fd6822c8558854ccde39a5684e7a56da27d"
        )
        .is_ok());
        // without 0x prefix
        assert!(parse_code_hash(
            "d43593c715fdd31c61141abd04a99fd6822c8558854ccde39a5684e7a56da27d"
        )
        .is_ok())
    }
}
