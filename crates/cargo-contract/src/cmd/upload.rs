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

use crate::{
    call_with_config,
    ErrorVariant,
};
use std::{
    fmt::{
        Debug,
        Display,
    },
    str::FromStr,
};

use super::{
    config::SignerConfig,
    display_dry_run_result_warning,
    parse_balance,
    prompt_confirm_unverifiable_upload,
    CLIExtrinsicOpts,
};
use anyhow::Result;
use contract_build::name_value_println;
use contract_extrinsics::{
    Chain,
    DisplayEvents,
    ExtrinsicOptsBuilder,
    TokenMetadata,
    UploadCommandBuilder,
    UploadExec,
};
use ink_env::Environment;
use serde::Serialize;
use subxt::{
    config::ExtrinsicParams,
    ext::{
        scale_decode::IntoVisitor,
        scale_encode::EncodeAsType,
    },
    Config,
};
use url::Url;

#[derive(Debug, clap::Args)]
#[clap(name = "upload", about = "Upload a contract's code")]
pub struct UploadCommand {
    #[clap(flatten)]
    extrinsic_cli_opts: CLIExtrinsicOpts,
    /// Export the call output in JSON format.
    #[clap(long, conflicts_with = "verbose")]
    output_json: bool,
    /// The chain config to be used as part of the call.
    #[clap(name = "config", long, default_value = "Polkadot")]
    config: String,
}

impl UploadCommand {
    /// Returns whether to export the call output in JSON format.
    pub fn output_json(&self) -> bool {
        self.output_json
    }

    pub async fn handle(&self) -> Result<(), ErrorVariant> {
        call_with_config!(self, run, self.config.as_str())
    }

    async fn run<C: Config + Environment + SignerConfig<C>>(
        &self,
    ) -> Result<(), ErrorVariant>
    where
        <C as Config>::AccountId: IntoVisitor + FromStr + EncodeAsType,
        <<C as Config>::AccountId as FromStr>::Err: Display,
        C::Balance:
            Into<u128> + From<u128> + Display + Default + FromStr + Serialize + Debug,
        <C::ExtrinsicParams as ExtrinsicParams<C>>::OtherParams: Default,
        <C as Config>::Hash: IntoVisitor + EncodeAsType + From<[u8; 32]>,
    {
        let signer = C::Signer::from_str(&self.extrinsic_cli_opts.suri)
            .map_err(|_| anyhow::anyhow!("Failed to parse suri option"))?;
        let token_metadata = if let Some(chain) = &self.extrinsic_cli_opts.chain {
            TokenMetadata::query::<C>(&Url::parse(chain.end_point()).unwrap()).await?
        } else {
            TokenMetadata::query::<C>(&self.extrinsic_cli_opts.url).await?
        };
        let storage_deposit_limit = self
            .extrinsic_cli_opts
            .storage_deposit_limit
            .clone()
            .map(|b| parse_balance(&b, &token_metadata))
            .transpose()
            .map_err(|e| {
                anyhow::anyhow!("Failed to parse storage_deposit_limit option: {}", e)
            })?;
        let extrinsic_opts = ExtrinsicOptsBuilder::new(signer)
            .file(self.extrinsic_cli_opts.file.clone())
            .manifest_path(self.extrinsic_cli_opts.manifest_path.clone())
            .url(self.extrinsic_cli_opts.url.clone())
            .chain(self.extrinsic_cli_opts.chain.clone())
            .storage_deposit_limit(storage_deposit_limit)
            .done();

        let upload_exec: UploadExec<C, C, _> =
            UploadCommandBuilder::new(extrinsic_opts).done().await?;
        let code_hash = upload_exec.code().code_hash();
        let metadata = upload_exec.client().metadata();

        if !self.extrinsic_cli_opts.execute {
            match upload_exec.upload_code_rpc().await? {
                Ok(result) => {
                    let upload_result = UploadDryRunResult {
                        result: String::from("Success!"),
                        code_hash: format!("{:?}", result.code_hash),
                        deposit: result.deposit,
                    };
                    if self.output_json() {
                        println!("{}", upload_result.to_json()?);
                    } else {
                        upload_result.print();
                        display_dry_run_result_warning("upload");
                    }
                }
                Err(err) => {
                    let err = ErrorVariant::from_dispatch_error(&err, &metadata)?;
                    if self.output_json() {
                        return Err(err)
                    } else {
                        name_value_println!("Result", err);
                    }
                }
            }
        } else {
            if let Chain::Production(name) = upload_exec.opts().chain_and_endpoint().0 {
                if !upload_exec.opts().is_verifiable()? {
                    prompt_confirm_unverifiable_upload(&name)?
                }
            }
            let upload_result = upload_exec.upload_code().await?;
            let display_events = DisplayEvents::from_events::<C, C>(
                &upload_result.events,
                None,
                &metadata,
            )?;
            let output_events = if self.output_json() {
                display_events.to_json()?
            } else {
                display_events.display_events::<C>(
                    self.extrinsic_cli_opts.verbosity()?,
                    &token_metadata,
                )?
            };
            if let Some(code_stored) = upload_result.code_stored {
                let code_hash: <C as Config>::Hash = code_stored.code_hash;
                if self.output_json() {
                    // Create a JSON object with the events and the code hash.
                    let json_object = serde_json::json!({
                        "events": serde_json::from_str::<serde_json::Value>(&output_events)?,
                        "code_hash": code_hash,
                    });
                    println!("{}", serde_json::to_string_pretty(&json_object)?);
                } else {
                    println!("{}", output_events);
                    name_value_println!("Code hash", format!("{:?}", code_hash));
                }
            } else {
                let code_hash = hex::encode(code_hash);
                return Err(anyhow::anyhow!(
                    "This contract has already been uploaded with code hash: 0x{code_hash}"
                )
                .into())
            }
        }
        Ok(())
    }
}

#[derive(serde::Serialize)]
pub struct UploadDryRunResult<Balance> {
    pub result: String,
    pub code_hash: String,
    pub deposit: Balance,
}

impl<Balance> UploadDryRunResult<Balance>
where
    Balance: Debug + Serialize,
{
    pub fn to_json(&self) -> Result<String> {
        Ok(serde_json::to_string_pretty(self)?)
    }

    pub fn print(&self) {
        name_value_println!("Result", self.result);
        name_value_println!("Code hash", format!("{:?}", self.code_hash));
        name_value_println!("Deposit", format!("{:?}", self.deposit));
    }
}
