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

use crate::{
    crate_metadata::CrateMetadata,
    util,
    workspace::{ManifestPath, Workspace},
    UnstableFlags, Verbosity,
};
use anyhow::Result;
use blake2::digest::{Update as _, VariableOutput as _};
use contract_metadata::{
    Compiler, Contract, ContractMetadata, Language, Source, SourceCompiler, SourceLanguage,
    SourceWasm, User,
};
use semver::Version;
use std::{fs, path::PathBuf};
use url::Url;

const METADATA_FILE: &str = "metadata.json";

/// Executes the metadata generation process
struct GenerateMetadataCommand {
    crate_metadata: CrateMetadata,
    verbosity: Option<Verbosity>,
    include_wasm: bool,
    unstable_options: UnstableFlags,
}

impl GenerateMetadataCommand {
    pub fn exec(&self) -> Result<PathBuf> {
        util::assert_channel()?;
        println!("  Generating metadata");

        let cargo_meta = &self.crate_metadata.cargo_meta;
        let out_path = cargo_meta.target_directory.join(METADATA_FILE);
        let target_dir = cargo_meta.target_directory.clone();

        // build the extended contract project metadata
        let (source_meta, contract_meta, user_meta) = self.extended_metadata()?;

        let generate_metadata = |manifest_path: &ManifestPath| -> Result<()> {
            let target_dir_arg = format!("--target-dir={}", target_dir.to_string_lossy());
            let stdout = util::invoke_cargo(
                "run",
                &[
                    "--package",
                    "metadata-gen",
                    &manifest_path.cargo_arg(),
                    &target_dir_arg,
                    "--release",
                ],
                self.crate_metadata.manifest_path.directory(),
                self.verbosity,
            )?;

            let ink_meta: serde_json::Map<String, serde_json::Value> =
                serde_json::from_slice(&stdout)?;
            let metadata = ContractMetadata::new(source_meta, contract_meta, user_meta, ink_meta);
            let contents = serde_json::to_string_pretty(&metadata)?;
            fs::write(&out_path, contents)?;
            Ok(())
        };

        if self.unstable_options.original_manifest {
            generate_metadata(&self.crate_metadata.manifest_path)?;
        } else {
            Workspace::new(&cargo_meta, &self.crate_metadata.root_package.id)?
                .with_root_package_manifest(|manifest| {
                    manifest
                        .with_added_crate_type("rlib")?
                        .with_profile_release_lto(false)?;
                    Ok(())
                })?
                .with_metadata_gen_package()?
                .using_temp(generate_metadata)?;
        }

        Ok(out_path)
    }

    /// Generate the extended contract project metadata
    fn extended_metadata(&self) -> Result<(Source, Contract, Option<User>)> {
        let contract_package = &self.crate_metadata.root_package;
        let ink_version = &self.crate_metadata.ink_version;
        let rust_version = Version::parse(&rustc_version::version()?.to_string())?;
        let contract_name = contract_package.name.clone();
        let contract_version = Version::parse(&contract_package.version.to_string())?;
        let contract_authors = contract_package.authors.clone();
        // optional
        let description = contract_package.description.clone();
        let documentation = self.crate_metadata.documentation.clone();
        let repository = contract_package
            .repository
            .as_ref()
            .map(|repo| Url::parse(&repo))
            .transpose()?;
        let homepage = self.crate_metadata.homepage.clone();
        let license = contract_package.license.clone();
        let hash = self.wasm_hash()?;

        let source = {
            let lang = SourceLanguage::new(Language::Ink, ink_version.clone());
            let compiler = SourceCompiler::new(Compiler::RustC, rust_version);
            let maybe_wasm = match self.include_wasm {
                true => {
                    let wasm = fs::read(&self.crate_metadata.dest_wasm)?;
                    // The Wasm which we read must have the same hash as `source.hash`
                    debug_assert_eq!(blake2_hash(wasm.clone().as_slice()), hash);
                    Some(SourceWasm::new(wasm))
                }
                false => None,
            };
            Source::new(maybe_wasm, hash, lang, compiler)
        };

        // Required contract fields
        let mut builder = Contract::builder();
        builder
            .name(contract_name)
            .version(contract_version)
            .authors(contract_authors);

        if let Some(description) = description {
            builder.description(description);
        }

        if let Some(documentation) = documentation {
            builder.documentation(documentation);
        }

        if let Some(repository) = repository {
            builder.repository(repository);
        }

        if let Some(homepage) = homepage {
            builder.homepage(homepage);
        }

        if let Some(license) = license {
            builder.license(license);
        }

        let contract = builder
            .build()
            .map_err(|err| anyhow::anyhow!("Invalid contract metadata builder state: {}", err))?;

        // user defined metadata
        let user = self.crate_metadata.user.clone().map(User::new);

        Ok((source, contract, user))
    }

    /// Compile the contract and then hash the resulting wasm
    fn wasm_hash(&self) -> Result<[u8; 32]> {
        super::build::execute_with_metadata(
            &self.crate_metadata,
            self.verbosity,
            self.unstable_options.clone(),
        )?;

        let wasm = fs::read(&self.crate_metadata.dest_wasm)?;
        Ok(blake2_hash(wasm.as_slice()))
    }
}

/// Returns the blake2 hash of the submitted slice.
fn blake2_hash(code: &[u8]) -> [u8; 32] {
    let mut output = [0u8; 32];
    let mut blake2 = blake2::VarBlake2b::new_keyed(&[], 32);
    blake2.update(code);
    blake2.finalize_variable(|result| output.copy_from_slice(result));
    output
}

/// Generates a file with metadata describing the ABI of the smart-contract.
///
/// It does so by generating and invoking a temporary workspace member.
pub(crate) fn execute(
    manifest_path: ManifestPath,
    verbosity: Option<Verbosity>,
    include_wasm: bool,
    unstable_options: UnstableFlags,
) -> Result<PathBuf> {
    let crate_metadata = CrateMetadata::collect(&manifest_path)?;
    GenerateMetadataCommand {
        crate_metadata,
        verbosity,
        include_wasm,
        unstable_options,
    }
    .exec()
}

#[cfg(feature = "test-ci-only")]
#[cfg(test)]
mod tests {
    use crate::{
        cmd, crate_metadata::CrateMetadata, util::tests::with_tmp_dir, ManifestPath, UnstableFlags,
    };
    use blake2::digest::{Update as _, VariableOutput as _};
    use contract_metadata::*;
    use serde_json::{Map, Value};
    use std::{fmt::Write, fs};
    use toml::value;

    struct TestContractManifest {
        toml: value::Table,
        manifest_path: ManifestPath,
    }

    impl TestContractManifest {
        fn new(manifest_path: ManifestPath) -> anyhow::Result<Self> {
            Ok(Self {
                toml: toml::from_slice(&fs::read(&manifest_path)?)?,
                manifest_path,
            })
        }

        fn package_mut(&mut self) -> anyhow::Result<&mut value::Table> {
            self.toml
                .get_mut("package")
                .ok_or(anyhow::anyhow!("package section not found"))?
                .as_table_mut()
                .ok_or(anyhow::anyhow!("package section should be a table"))
        }

        /// Add a key/value to the `[package.metadata.contract.user]` section
        fn add_user_metadata_value(
            &mut self,
            key: &'static str,
            value: value::Value,
        ) -> anyhow::Result<()> {
            self.package_mut()?
                .entry("metadata")
                .or_insert(value::Value::Table(Default::default()))
                .as_table_mut()
                .ok_or(anyhow::anyhow!("metadata section should be a table"))?
                .entry("contract")
                .or_insert(value::Value::Table(Default::default()))
                .as_table_mut()
                .ok_or(anyhow::anyhow!(
                    "metadata.contract section should be a table"
                ))?
                .entry("user")
                .or_insert(value::Value::Table(Default::default()))
                .as_table_mut()
                .ok_or(anyhow::anyhow!(
                    "metadata.contract.user section should be a table"
                ))?
                .insert(key.into(), value);
            Ok(())
        }

        fn add_package_value(
            &mut self,
            key: &'static str,
            value: value::Value,
        ) -> anyhow::Result<()> {
            self.package_mut()?.insert(key.into(), value);
            Ok(())
        }

        fn write(&self) -> anyhow::Result<()> {
            let toml = toml::to_string(&self.toml)?;
            fs::write(&self.manifest_path, toml).map_err(Into::into)
        }
    }

    #[test]
    fn generate_metadata() {
        env_logger::try_init().ok();
        with_tmp_dir(|path| {
            cmd::new::execute("new_project", Some(path)).expect("new project creation failed");
            let working_dir = path.join("new_project");
            let manifest_path = ManifestPath::new(working_dir.join("Cargo.toml"))?;

            // add optional metadata fields
            let mut test_manifest = TestContractManifest::new(manifest_path)?;
            test_manifest.add_package_value("description", "contract description".into())?;
            test_manifest.add_package_value("documentation", "http://documentation.com".into())?;
            test_manifest.add_package_value("repository", "http://repository.com".into())?;
            test_manifest.add_package_value("homepage", "http://homepage.com".into())?;
            test_manifest.add_package_value("license", "Apache-2.0".into())?;
            test_manifest
                .add_user_metadata_value("some-user-provided-field", "and-its-value".into())?;
            test_manifest.add_user_metadata_value(
                "more-user-provided-fields",
                vec!["and", "their", "values"].into(),
            )?;
            test_manifest.write()?;

            let crate_metadata = CrateMetadata::collect(&test_manifest.manifest_path)?;
            let metadata_file = cmd::metadata::execute(
                test_manifest.manifest_path,
                None,
                true,
                UnstableFlags::default(),
            )?;
            let metadata_json: Map<String, Value> =
                serde_json::from_slice(&fs::read(&metadata_file)?)?;

            assert!(
                metadata_file.exists(),
                format!("Missing metadata file '{}'", metadata_file.display())
            );

            let source = metadata_json.get("source").expect("source not found");
            let hash = source.get("hash").expect("source.hash not found");
            let language = source.get("language").expect("source.language not found");
            let compiler = source.get("compiler").expect("source.compiler not found");
            let wasm = source.get("wasm").expect("source.wasm not found");

            let contract = metadata_json.get("contract").expect("contract not found");
            let name = contract.get("name").expect("contract.name not found");
            let version = contract.get("version").expect("contract.version not found");
            let authors = contract
                .get("authors")
                .expect("contract.authors not found")
                .as_array()
                .expect("contract.authors is an array")
                .iter()
                .map(|author| author.as_str().expect("author is a string"))
                .collect::<Vec<_>>();
            let description = contract
                .get("description")
                .expect("contract.description not found");
            let documentation = contract
                .get("documentation")
                .expect("contract.documentation not found");
            let repository = contract
                .get("repository")
                .expect("contract.repository not found");
            let homepage = contract
                .get("homepage")
                .expect("contract.homepage not found");
            let license = contract.get("license").expect("contract.license not found");

            let user = metadata_json.get("user").expect("user section not found");

            // calculate wasm hash
            let fs_wasm = fs::read(&crate_metadata.dest_wasm)?;
            let expected_hash = blake2_hash(&fs_wasm[..]);
            let expected_wasm = build_byte_str(&fs_wasm);

            let expected_language =
                SourceLanguage::new(Language::Ink, crate_metadata.ink_version).to_string();
            let expected_rustc_version =
                semver::Version::parse(&rustc_version::version()?.to_string())?;
            let expected_compiler =
                SourceCompiler::new(Compiler::RustC, expected_rustc_version).to_string();
            let mut expected_user_metadata = serde_json::Map::new();
            expected_user_metadata
                .insert("some-user-provided-field".into(), "and-its-value".into());
            expected_user_metadata.insert(
                "more-user-provided-fields".into(),
                serde_json::Value::Array(
                    vec!["and".into(), "their".into(), "values".into()].into(),
                ),
            );

            assert_eq!(expected_hash, hash.as_str().unwrap());
            assert_eq!(expected_wasm, wasm.as_str().unwrap());
            assert_eq!(expected_language, language.as_str().unwrap());
            assert_eq!(expected_compiler, compiler.as_str().unwrap());
            assert_eq!(crate_metadata.package_name, name.as_str().unwrap());
            assert_eq!(
                crate_metadata.root_package.version.to_string(),
                version.as_str().unwrap()
            );
            assert_eq!(crate_metadata.root_package.authors, authors);
            assert_eq!("contract description", description.as_str().unwrap());
            assert_eq!("http://documentation.com/", documentation.as_str().unwrap());
            assert_eq!("http://repository.com/", repository.as_str().unwrap());
            assert_eq!("http://homepage.com/", homepage.as_str().unwrap());
            assert_eq!("Apache-2.0", license.as_str().unwrap());
            assert_eq!(&expected_user_metadata, user.as_object().unwrap());

            Ok(())
        })
    }

    fn build_byte_str(bytes: &[u8]) -> String {
        let mut str = String::new();
        write!(str, "0x").expect("failed writing to string");
        for byte in bytes {
            write!(str, "{:02x}", byte).expect("failed writing to string");
        }
        str
    }
}
