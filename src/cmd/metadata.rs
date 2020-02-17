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

use crate::{tmp_manifest::TmpManifest, util};
use anyhow::Result;
use std::path::PathBuf;

/// Executes build of the smart-contract which produces a wasm binary that is ready for deploying.
///
/// It does so by invoking build by cargo and then post processing the final binary.
pub(crate) fn execute_generate_metadata(dir: Option<&PathBuf>) -> Result<String> {
    util::assert_channel()?;
    println!("  Generating metadata");

    let cargo_metadata = crate::util::get_cargo_metadata(dir)?;
    // todo: use tmp manifest in invocation
    let _tmp_manifest = TmpManifest::from_working_dir(dir)?
        .with_added_crate_type("rlib")?
        .write()?;

    util::invoke_cargo(
        "run",
        &[
            "--package",
            "abi-gen",
            "--release",
            // "--no-default-features", // Breaks builds for MacOS (linker errors), we should investigate this issue asap!
        ],
        dir,
    )?;

    let mut out_path = cargo_metadata.target_directory;
    out_path.push("metadata.json");

    Ok(format!(
        "Your metadata file is ready.\nYou can find it here:\n{}",
        out_path.display()
    ))
}

#[cfg(feature = "test-ci-only")]
#[cfg(test)]
mod tests {
    use crate::{
        cmd::{execute_generate_metadata, execute_new},
        util::tests::with_tmp_dir,
    };

    #[test]
    fn generate_metadata() {
        with_tmp_dir(|path| {
            execute_new("new_project", Some(path)).expect("new project creation failed");
            let working_dir = path.join("new_project");
            execute_generate_metadata(Some(&working_dir)).expect("generate metadata failed");

            let mut abi_file = working_dir;
            abi_file.push("target");
            abi_file.push("metadata.json");
            assert!(abi_file.exists())
        });
    }
}
