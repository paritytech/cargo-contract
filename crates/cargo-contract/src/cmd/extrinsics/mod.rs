// Copyright 2018-2020 Parity Technologies (UK) Ltd.
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
mod instantiate;
mod remove;
mod upload;

#[cfg(test)]
#[cfg(feature = "integration-tests")]
mod integration_tests;

use anyhow::{
    anyhow,
    Context,
    Ok,
    Result,
};
use colored::Colorize;
use jsonrpsee::{
    core::client::ClientT,
    rpc_params,
    ws_client::WsClientBuilder,
};
use std::{
    io::{
        self,
        Write,
    },
    path::PathBuf,
};

use crate::{
    cmd::{
        Balance,
        Client,
    },
    DEFAULT_KEY_COL_WIDTH,
};

use contract_build::{
    name_value_println,
    CrateMetadata,
    Verbosity,
    VerbosityFlags,
};
use pallet_contracts_primitives::ContractResult;
use scale::{
    Decode,
    Encode,
};
use sp_core::{
    crypto::Pair,
    sr25519,
    Bytes,
};
use sp_weights::Weight;
use subxt::{
    blocks,
    config,
    tx,
    Config,
    OnlineClient,
};

use std::{
    option::Option,
    path::Path,
};

pub use balance::{
    BalanceVariant,
    TokenMetadata,
};
pub use call::CallCommand;
use contract_metadata::ContractMetadata;
pub use contract_transcode::ContractMessageTranscoder;
pub use error::ErrorVariant;
pub use instantiate::InstantiateCommand;
pub use remove::RemoveCommand;
pub use subxt::PolkadotConfig as DefaultConfig;
pub use upload::UploadCommand;

type PairSigner = tx::PairSigner<DefaultConfig, sr25519::Pair>;

/// Arguments required for creating and sending an extrinsic to a substrate node.
#[derive(Clone, Debug, clap::Parser)]
#[clap(group = clap::ArgGroup::new("surigroup").multiple(false))]
#[clap(group = clap::ArgGroup::new("passgroup").multiple(false))]
pub struct ExtrinsicOpts {
    /// Path to a contract build artifact file: a raw `.wasm` file, a `.contract` bundle,
    /// or a `.json` metadata file.
    #[clap(value_parser, conflicts_with = "manifest_path")]
    file: Option<PathBuf>,
    /// Path to the `Cargo.toml` of the contract.
    #[clap(long, value_parser)]
    manifest_path: Option<PathBuf>,
    /// Websockets url of a substrate node.
    #[clap(
        name = "url",
        long,
        value_parser,
        default_value = "ws://localhost:9944"
    )]
    url: url::Url,
    /// Secret key URI for the account interacting with the contract.
    #[clap(group = "surigroup", required = false, name = "suri", long, short('s'), conflicts_with = "suri-path")]
    suri: Option<String>,
    /// Path to a file containing the secret key URI for the account interacting with the contract.
    #[clap(group = "surigroup", required = false, name = "suri-path", long, short('S'), conflicts_with = "suri")]
    suri_path: Option<PathBuf>,
    /// Password for the secret key URI.
    #[clap(group = "passgroup", required = false, name = "password", long, short('p'), conflicts_with = "password-path")]
    password: Option<String>,
    /// Path to a file containing the password for the secret key URI.
    #[clap(group = "passgroup", required = false, name = "password-path", long, short('P'), conflicts_with = "password")]
    password_path: Option<PathBuf>,
    #[clap(flatten)]
    verbosity: VerbosityFlags,
    /// Submit the extrinsic for on-chain execution.
    #[clap(short('x'), long)]
    execute: bool,
    /// The maximum amount of balance that can be charged from the caller to pay for the
    /// storage. consumed.
    #[clap(long)]
    storage_deposit_limit: Option<BalanceVariant>,
    /// Before submitting a transaction, do not dry-run it via RPC first.
    #[clap(long)]
    skip_dry_run: bool,
    /// Before submitting a transaction, do not ask the user for confirmation.
    #[clap(long)]
    skip_confirm: bool,
}

impl ExtrinsicOpts {
    /// Load contract artifacts.
    pub fn contract_artifacts(&self) -> Result<ContractArtifacts> {
        ContractArtifacts::from_manifest_or_file(
            self.manifest_path.as_ref(),
            self.file.as_ref(),
        )
    }

    /// Load suri and password data.
    pub fn suri_data(&self) -> Result<SuriData> {
        SuriData::from_suri_and_password_files(
            self.suri_path.as_ref(),
            self.password_path.as_ref(),
        )
    }

    /// Returns the signer for contract extrinsics.
    pub fn signer(&self) -> Result<sr25519::Pair> {
        match &self.suri_data() {
            // TODO - replace with `Ok` from `anyhow` throughout if possible
            // instead of using `std::result::Result`
            std::result::Result::Ok(d) => {
                println!("d is {:#?}", d);
                // // get suri_path
                // let sp = match &d.suri_path {
                //     // TODO - figure out how to avoid using `.clone`
                //     Some(sp) => sp.as_path().display().to_string(),
                //     None => anyhow::bail!("suri path not provided"),
                // };
                // println!("sp is {:#?}", sp);

                // // get password_path
                // let pp = match &d.password_path {
                //     // TODO - figure out how to avoid using `.clone`
                //     Some(pp) => Some(pp.as_path().display().to_string()),
                //     None => anyhow::bail!("password path not provided"),
                // };
                // println!("pp is {:#?}", pp);

                // get suri
                let suri = match &d.suri {
                    // remove newline characters
                    Some(s) => s.trim().to_string(),
                    None => anyhow::bail!("suri not provided"),
                };
                println!("suri is {:#?}", suri);

                // get password
                let password = match &d.password {
                    // remove newline characters
                    Some(p) => Some(p.trim().to_string()),
                    None => anyhow::bail!("password not provided"),
                };
                println!("password is {:#?}", password);
                return Pair::from_string(&suri, password.as_ref().map(String::as_ref))
                    .map_err(|_| anyhow::anyhow!("Secret string error"))
            },
            std::result::Result::Err(_e) => anyhow::bail!("suri data not provided {}", _e),
        };
    }

    /// Returns the verbosity
    pub fn verbosity(&self) -> Result<Verbosity> {
        TryFrom::try_from(&self.verbosity)
    }

    /// Convert URL to String without omitting the default port
    pub fn url_to_string(&self) -> String {
        let mut res = self.url.to_string();
        match (self.url.port(), self.url.port_or_known_default()) {
            (None, Some(port)) => {
                res.insert_str(res.len() - 1, &format!(":{port}"));
                res
            }
            _ => res,
        }
    }

    /// Get the storage deposit limit converted to compact for passing to extrinsics.
    pub fn storage_deposit_limit(
        &self,
        token_metadata: &TokenMetadata,
    ) -> Result<Option<scale::Compact<Balance>>> {
        Ok(self
            .storage_deposit_limit
            .as_ref()
            .map(|bv| bv.denominate_balance(token_metadata))
            .transpose()?
            .map(Into::into))
    }
}

/// The Suri of a contract.
#[derive(Debug)]
pub struct Suri(Option<String>);

impl Suri {
    /// The suri of the contract
    pub fn suri(&self) -> Result<&String> {
        match &self.0 {
            Some(s) => return Ok(s),
            None => anyhow::bail!("suri not available"),
        }
    }
}

/// The Password of a contract.
#[derive(Debug)]
pub struct Password(Option<String>);

impl Password {
    /// The password of the contract
    pub fn password(&self) -> Result<&String> {
        match &self.0 {
            Some(p) => return Ok(p),
            None => anyhow::bail!("password not available"),
        }
    }
}

/// Suri and password data for use with extrinsic commands.
#[derive(Debug)]
pub struct SuriData {
    /// The expected path of the file containing the suri data path.
    suri_path: Option<PathBuf>,
    /// The expected path of the file containing the password data path.
    password_path: Option<PathBuf>,
    /// The suri if data file exists.
    suri: Option<String>,
    /// The password if data file exists.
    password: Option<String>,
}

impl SuriData {
    /// Given an suri path and an associated password path,
    /// load the metadata where possible.
    pub fn from_suri_and_password_files(
        suri_path: Option<&PathBuf>,
        password_path: Option<&PathBuf>,
    ) -> Result<SuriData> {
        let mut suri = "".to_string();
        let mut password = "".to_string();

        match suri_path {
            Some(sp) => {
                match sp.extension().and_then(|ext| ext.to_str()) {
                    Some("txt") => {
                        tracing::debug!("Loading suri path from `{}`", sp.display());
                        let file_name = sp.file_stem()
                            .context("suri file has unreadable name")?
                            .to_str()
                            .context("Error parsing filename string")?;
                        // TODO - consider storing `Vec<u8>` in Suri struct instead of `String`
                        let _s: String = match String::from_utf8(std::fs::read(sp)?) {
                            std::result::Result::Ok(s) => s,
                            std::result::Result::Err(_e) => {
                                anyhow::bail!(
                                    "unable to convert Vec<u8> to String for suri_path {}", _e
                                )
                            }
                        };
                        let s = Suri(Some(_s));
                        let dir = sp.parent().map_or_else(PathBuf::new, PathBuf::from);
                        let metadata_path = dir.join(format!("{file_name}.txt"));
                        if metadata_path.exists() {
                            suri = match s.suri() {
                                std::result::Result::Ok(s) => s.to_string(),
                                std::result::Result::Err(_e) => {
                                    anyhow::bail!("unable to get value for Suri {}", _e)
                                }
                            };
                        } else {
                            println!("suri file does not exist");
                        }
                        suri = match s.suri() {
                            std::result::Result::Ok(s) => s.to_string(),
                            std::result::Result::Err(_e) => {
                                anyhow::bail!("unable to get value for Suri {}", _e)
                            }
                        };
                    }
                    Some(ext) => anyhow::bail!(
                        "Invalid extension {ext}, expected `.txt`"
                    ),
                    None => {
                        anyhow::bail!(
                            "suri path has no extension, expected `.txt`"
                        )
                    }
                };
            },
            None => {
                anyhow::bail!(
                    "suri path has no extension, expected `.txt`"
                )
            }
        }

        match password_path {
            Some(pp) => {
                match pp.extension().and_then(|ext| ext.to_str()) {
                    Some("txt") => {
                        tracing::debug!("Loading password path from `{}`", pp.display());
                        let file_name = pp.file_stem()
                            .context("password file has unreadable name")?
                            .to_str()
                            .context("Error parsing filename string")?;
                        let _p: String = match String::from_utf8(std::fs::read(pp)?) {
                            std::result::Result::Ok(p) => p,
                            std::result::Result::Err(_e) => {
                                anyhow::bail!(
                                    "unable to convert Vec<u8> to String for password_path {}", _e
                                )
                            }
                        };
                        let p = Password(Some(_p));

                        let dir = pp.parent().map_or_else(PathBuf::new, PathBuf::from);
                        let metadata_path = dir.join(format!("{file_name}.txt"));
                        if metadata_path.exists() {
                            password = match p.password() {
                                std::result::Result::Ok(p) => p.to_string(),
                                std::result::Result::Err(_e) => {
                                    anyhow::bail!("unable to get value for Password {}", _e)
                                }
                            };
                        } else {
                            println!("password file does not exist");
                        }
                        password = match p.password() {
                            std::result::Result::Ok(p) => p.to_string(),
                            std::result::Result::Err(_e) => {
                                anyhow::bail!("unable to get value for Password {}", _e)
                            }
                        };
                    }
                    Some(ext) => anyhow::bail!(
                        "Invalid extension {ext}, expected `.txt`"
                    ),
                    None => {
                        anyhow::bail!(
                            "password path has no extension, expected `.txt`"
                        )
                    }
                };
            },
            None => {
                anyhow::bail!(
                    "password path has no extension, expected `.txt`"
                )
            }
        }

        Ok(Self {
            // TODO - figure out how to avoid using `cloned()`
            suri_path: suri_path.cloned(),
            password_path: password_path.cloned(),
            suri: Some(suri),
            password: Some(password),
        })
    }
}

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

/// Create a new [`PairSigner`] from the given [`sr25519::Pair`].
pub fn pair_signer(pair: sr25519::Pair) -> PairSigner {
    PairSigner::new(pair)
}

const STORAGE_DEPOSIT_KEY: &str = "Storage Deposit";
pub const MAX_KEY_COL_WIDTH: usize = STORAGE_DEPOSIT_KEY.len() + 1;

/// Print to stdout the fields of the result of a `instantiate` or `call` dry-run via RPC.
pub fn display_contract_exec_result<R, const WIDTH: usize>(
    result: &ContractResult<R, Balance>,
) -> Result<()> {
    let mut debug_message_lines = std::str::from_utf8(&result.debug_message)
        .context("Error decoding UTF8 debug message bytes")?
        .lines();
    name_value_println!("Gas Consumed", format!("{:?}", result.gas_consumed), WIDTH);
    name_value_println!("Gas Required", format!("{:?}", result.gas_required), WIDTH);
    name_value_println!(
        STORAGE_DEPOSIT_KEY,
        format!("{:?}", result.storage_deposit),
        WIDTH
    );

    // print debug messages aligned, only first line has key
    if let Some(debug_message) = debug_message_lines.next() {
        name_value_println!("Debug Message", format!("{debug_message}"), WIDTH);
    }

    for debug_message in debug_message_lines {
        name_value_println!("", format!("{debug_message}"), WIDTH);
    }
    Ok(())
}

pub fn display_contract_exec_result_debug<R, const WIDTH: usize>(
    result: &ContractResult<R, Balance>,
) -> Result<()> {
    let mut debug_message_lines = std::str::from_utf8(&result.debug_message)
        .context("Error decoding UTF8 debug message bytes")?
        .lines();
    if let Some(debug_message) = debug_message_lines.next() {
        name_value_println!("Debug Message", format!("{debug_message}"), WIDTH);
    }

    for debug_message in debug_message_lines {
        name_value_println!("", format!("{debug_message}"), WIDTH);
    }
    Ok(())
}

pub fn display_dry_run_result_warning(command: &str) {
    println!("Your {} call {} been executed.", command, "has not".bold());
    println!(
            "To submit the transaction and execute the call on chain, add {} flag to the command.",
            "-x/--execute".bold()
        );
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
    <T::ExtrinsicParams as config::ExtrinsicParams<T::Index, T::Hash>>::OtherParams:
        Default,
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

/// Prompt the user to confirm transaction submission.
fn prompt_confirm_tx<F: FnOnce()>(show_details: F) -> Result<()> {
    println!(
        "{} (skip with --skip-confirm)",
        "Confirm transaction details:".bright_white().bold()
    );
    show_details();
    print!(
        "{} ({}/n): ",
        "Submit?".bright_white().bold(),
        "Y".bright_white().bold()
    );

    let mut buf = String::new();
    io::stdout().flush()?;
    io::stdin().read_line(&mut buf)?;
    match buf.trim().to_lowercase().as_str() {
        // default is 'y'
        "y" | "" => Ok(()),
        "n" => Err(anyhow!("Transaction not submitted")),
        c => Err(anyhow!("Expected either 'y' or 'n', got '{}'", c)),
    }
}

fn print_dry_running_status(msg: &str) {
    println!(
        "{:>width$} {} (skip with --skip-dry-run)",
        "Dry-running".green().bold(),
        msg.bright_white().bold(),
        width = DEFAULT_KEY_COL_WIDTH
    );
}

fn print_gas_required_success(gas: Weight) {
    println!(
        "{:>width$} Gas required estimated at {}",
        "Success!".green().bold(),
        gas.to_string().bright_white(),
        width = DEFAULT_KEY_COL_WIDTH
    );
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
