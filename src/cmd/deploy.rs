// Copyright 2018-2019 Parity Technologies (UK) Ltd.
// This file is part of ink!.
//
// ink! is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// ink! is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.
//
// You should have received a copy of the GNU General Public License
// along with ink!.  If not, see <http://www.gnu.org/licenses/>.

use std::{fs, io::Read, path::PathBuf};

use anyhow::{Context, Result};
use futures::future::Future;
use substrate_primitives::{crypto::Pair, sr25519, H256};
use subxt::{contracts, system::System, DefaultNodeRuntime};

use crate::cmd::build::{self, CrateMetadata};

/// Load the wasm blob from the specified path.
///
/// Defaults to the target contract wasm in the current project, inferred via the crate metadata.
fn load_contract_code(path: Option<&PathBuf>) -> Result<Vec<u8>> {
    let default_wasm_path = build::collect_crate_metadata(path).map(CrateMetadata::dest_wasm)?;
    let contract_wasm_path = path.unwrap_or(&default_wasm_path);

    let mut data = Vec::new();
    let mut file = fs::File::open(&contract_wasm_path)
        .context(format!("Failed to open {}", contract_wasm_path.display()))?;
    file.read_to_end(&mut data)?;

    Ok(data)
}

/// Attempt to extract the code hash from the extrinsic result.
///
/// Returns an Error if the `Contracts::CodeStored` is not found or cannot be decoded.
fn extract_code_hash<T: System>(extrinsic_result: subxt::ExtrinsicSuccess<T>) -> Result<H256> {
    match extrinsic_result.find_event::<H256>("Contracts", "CodeStored") {
        Some(Ok(hash)) => Ok(hash),
        Some(Err(err)) => Err(anyhow::anyhow!("Failed to decode code hash: {}", err)),
        None => Err(anyhow::anyhow!(
            "Failed to find Contracts::CodeStored Event"
        )),
    }
}

/// Put contract code to a smart contract enabled substrate chain.
/// Returns the code hash of the deployed contract if successful.
///
/// Optionally supply the contract wasm path, defaults to destination contract file inferred from
/// Cargo.toml of the current contract project.
///
/// Creates an extrinsic with the `Contracts::put_code` Call, submits via RPC, then waits for
/// the `ContractsEvent::CodeStored` event.
pub(crate) fn execute_deploy(
    url: url::Url,
    suri: &str,
    password: Option<&str>,
    gas: u64,
    contract_wasm_path: Option<&PathBuf>,
) -> Result<String> {
    let signer = match sr25519::Pair::from_string(suri, password) {
        Ok(signer) => signer,
        Err(_) => anyhow::bail!("Secret string error"),
    };

    let code = load_contract_code(contract_wasm_path)?;

    let fut = subxt::ClientBuilder::<DefaultNodeRuntime>::new()
        .set_url(url)
        .build()
        .and_then(|cli| cli.xt(signer, None))
        .and_then(move |xt| xt.submit_and_watch(contracts::put_code(gas, code)));

    let mut rt = tokio::runtime::Runtime::new()?;
    if let Ok(extrinsic_success) = rt.block_on(fut) {
        log::debug!("Deploy success: {:?}", extrinsic_success);

        let code_hash = extract_code_hash(extrinsic_success)?;
        Ok(format!("Code hash: {:?}", code_hash))
    } else {
        Err(anyhow::anyhow!("Deploy error"))
    }
}

#[cfg(test)]
mod tests {
    use std::{fs, io::Write, path};

    use assert_matches::assert_matches;

    #[test]
    #[ignore] // depends on a local substrate node running
    fn deploy_contract() {
        const CONTRACT: &str = r#"
(module
    (func (export "call"))
    (func (export "deploy"))
)
"#;
        let wasm = wabt::wat2wasm(CONTRACT).expect("invalid wabt");

        let out_dir = path::Path::new(env!("OUT_DIR"));

        let target_dir = path::Path::new("./target");
        let _ = fs::create_dir(target_dir);

        let wasm_path = out_dir.join("flipper-pruned.wasm");
        let mut file = fs::File::create(&wasm_path).unwrap();
        let _ = file.write_all(&wasm);

        let url = url::Url::parse("ws://localhost:9944").unwrap();
        let result = super::execute_deploy(url, "//Alice", None, 500_000, Some(&wasm_path));

        assert_matches!(result, Ok(_));
    }
}
