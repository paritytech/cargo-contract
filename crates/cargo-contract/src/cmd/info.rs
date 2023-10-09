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

use super::{
    basic_display_format_extended_contract_info,
    display_all_contracts,
    DefaultConfig,
};
use anyhow::{
    anyhow,
    Result,
};
use contract_analyze::determine_language;
use contract_extrinsics::{
    fetch_all_contracts,
    fetch_contract_info,
    fetch_wasm_code,
    url_to_string,
    Balance,
    CodeHash,
    ContractInfo,
    ErrorVariant,
};
use std::{
    fmt::Debug,
    io::Write,
};
use subxt::{
    backend::{
        legacy::LegacyRpcMethods,
        rpc::RpcClient,
    },
    Config,
    OnlineClient,
};

#[derive(Debug, clap::Args)]
#[clap(name = "info", about = "Get infos from a contract")]
pub struct InfoCommand {
    /// The address of the contract to display info of.
    #[clap(
        name = "contract",
        long,
        env = "CONTRACT",
        required_unless_present = "all"
    )]
    contract: Option<<DefaultConfig as Config>::AccountId>,
    /// Websockets url of a substrate node.
    #[clap(
        name = "url",
        long,
        value_parser,
        default_value = "ws://localhost:9944"
    )]
    url: url::Url,
    /// Export the instantiate output in JSON format.
    #[clap(name = "output-json", long)]
    output_json: bool,
    /// Display the contract's Wasm bytecode.
    #[clap(name = "binary", long, conflicts_with = "all")]
    binary: bool,
    /// Display all contracts addresses
    #[clap(name = "all", long)]
    all: bool,
}

impl InfoCommand {
    pub async fn run(&self) -> Result<(), ErrorVariant> {
        let rpc_cli = RpcClient::from_url(url_to_string(&self.url)).await?;
        let client =
            OnlineClient::<DefaultConfig>::from_rpc_client(rpc_cli.clone()).await?;
        let rpc = LegacyRpcMethods::<DefaultConfig>::new(rpc_cli.clone());

        // All flag applied
        if self.all {
            let contracts = fetch_all_contracts(&client, &rpc).await?;

            if self.output_json {
                let contracts_json = serde_json::json!({
                    "contracts": contracts
                });
                println!("{}", serde_json::to_string_pretty(&contracts_json)?);
            } else {
                display_all_contracts(&contracts)
            }
            Ok(())
        } else {
            // Contract arg shall be always present in this case, it is enforced by
            // clap configuration
            let contract = self
                .contract
                .as_ref()
                .expect("Contract argument was not provided");

            let info_to_json = fetch_contract_info(contract, &rpc, &client)
                .await?
                .ok_or(anyhow!(
                    "No contract information was found for account id {}",
                    contract
                ))?;

            let wasm_code = fetch_wasm_code(&client, &rpc, info_to_json.code_hash())
                .await?
                .ok_or(anyhow!(
                    "Contract wasm code was not found for account id {}",
                    contract
                ))?;
            // Binary flag applied
            if self.binary {
                if self.output_json {
                    let wasm = serde_json::json!({
                        "wasm": format!("0x{}", hex::encode(wasm_code))
                    });
                    println!("{}", serde_json::to_string_pretty(&wasm)?);
                } else {
                    std::io::stdout()
                        .write_all(&wasm_code)
                        .expect("Writing to stdout failed")
                }
            } else if self.output_json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&ExtendedContractInfo::new(
                        info_to_json,
                        &wasm_code
                    ))?
                )
            } else {
                basic_display_format_extended_contract_info(&ExtendedContractInfo::new(
                    info_to_json,
                    &wasm_code,
                ))
            }
            Ok(())
        }
    }
}

#[derive(serde::Serialize)]
pub struct ExtendedContractInfo {
    pub trie_id: String,
    pub code_hash: CodeHash,
    pub storage_items: u32,
    pub storage_item_deposit: Balance,
    pub source_language: String,
}

impl ExtendedContractInfo {
    pub fn new(contract_info: ContractInfo, code: &[u8]) -> Self {
        let language = match determine_language(code).ok() {
            Some(lang) => lang.to_string(),
            None => "Unknown".to_string(),
        };
        ExtendedContractInfo {
            trie_id: contract_info.trie_id().to_string(),
            code_hash: *contract_info.code_hash(),
            storage_items: contract_info.storage_items(),
            storage_item_deposit: contract_info.storage_item_deposit(),
            source_language: language,
        }
    }
}
