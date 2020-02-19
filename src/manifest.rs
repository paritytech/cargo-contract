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

use anyhow::{Context, Result};
use cargo_metadata::{
    Metadata as CargoMetadata,
    PackageId,
};
use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
};
use toml::value;

const MANIFEST_FILE: &str = "Cargo.toml";

/// Create an amended copy of `Cargo.toml`.
///
/// Relative paths are rewritten to absolute paths.
pub struct Manifest {
    path: PathBuf,
    toml: value::Table,
}

impl Manifest {
    /// Create new CargoToml for the given manifest path.
    ///
    /// The path *must* be to a `Cargo.toml`.
    pub fn new(path: &PathBuf) -> Result<Manifest> {
        if let Some(file_name) = path.file_name() {
            if file_name != MANIFEST_FILE {
                anyhow::bail!("Manifest file must be a Cargo.toml")
            }
        }

        let toml = fs::read_to_string(&path).context("Loading Cargo.toml")?;
        let toml: value::Table = toml::from_str(&toml)?;

        Ok(Manifest { path: path.clone(), toml })
    }

    /// Create a new Manifest from the given directory path.
    ///
    /// Passing `None` will assume the current directory so just `Cargo.toml`
    pub fn from_dir<P: AsRef<Path>>(path: Option<P>) -> Result<Manifest> {
        if let Some(path) = path {
            let path = path.as_ref();
            if !path.is_dir() {
                anyhow::bail!("{} is not a directory", path.display())
            }
            Self::new(&path.join(MANIFEST_FILE))
        } else {
            Self::new(&PathBuf::from(MANIFEST_FILE))
        }
    }

    /// Get mutable reference to `[lib] crate-types = []` section
    fn get_crate_types_mut(&mut self) -> Result<&mut value::Array> {
        let lib = self.toml
            .get_mut("lib")
            .ok_or(anyhow::anyhow!("lib section not found"))?;
        let crate_types = lib
            .get_mut("crate-type")
            .ok_or(anyhow::anyhow!("crate-type section not found"))?;

        crate_types
            .as_array_mut()
            .ok_or(anyhow::anyhow!("crate-types should be an Array"))
    }

    /// Add a value to the `[lib] crate-types = []` section.
    ///
    /// If the value already exists, does nothing.
    pub fn with_added_crate_type(&mut self, crate_type: &str) -> Result<&mut Self> {
        let crate_types = self.get_crate_types_mut()?;
        if !crate_type_exists(crate_type, crate_types) {
            crate_types.push(crate_type.into());
        }
        Ok(self)
    }

    /// Remove a value from the `[lib] crate-types = []` section
    ///
    /// If the value does not exist, does nothing.
    pub fn with_removed_crate_type(&mut self, crate_type: &str) -> Result<&mut Self> {
        let crate_types = self.get_crate_types_mut()?;
        if crate_type_exists(crate_type, crate_types) {
            crate_types.retain(|v| v.as_str().map_or(true, |s| s != crate_type));
        }
        Ok(self)
    }

    /// Replace relative paths with absolute paths with the working directory.
    ///
    /// Enables the use of a temporary amended copy of the manifest.
    ///
    /// # Rewrites
    ///
    /// - `[lib]/path`
    /// - `[dependencies]`
    fn rewrite_relative_paths(&mut self) -> Result<&mut Self> {
        let abs_path = self.path.canonicalize()?;
        let abs_dir = abs_path.parent()
            .expect("The manifest path is a file path so has a parent; qed");

        // Rewrite `[lib] path =` value to an absolute path.
        // Defaults to src/lib.rs if not specified
        let lib = self.toml
            .get_mut("lib")
            .ok_or(anyhow::anyhow!("lib section not found"))?;

        let to_absolute = |value_id: String, existing_path: &mut value::Value| -> Result<()> {
            let path_str = existing_path.as_str()
                .ok_or(anyhow::anyhow!("{} should be a string", value_id))?;
            let path = PathBuf::from(path_str);
            if path.is_relative() {
                let lib_abs = abs_dir.join(path);
                log::debug!("Rewriting {} to '{}'", value_id, lib_abs.display());
                *existing_path = value::Value::String(lib_abs.to_string_lossy().into())
            }
            Ok(())
        };

        match lib.get_mut("path") {
            Some(existing_path) => to_absolute("lib/path".into(), existing_path)?,
            None => {
                let lib_table = lib
                    .as_table_mut()
                    .ok_or(anyhow::anyhow!("lib section should be a table"))?;
                let inferred_lib_path = abs_dir.join("src").join("lib.rs");
                if !inferred_lib_path.exists() {
                    anyhow::bail!(
                        "No `[lib] path =` specified, and the default `src/lib.rs` was not found"
                    )
                }
                let path = inferred_lib_path.to_string_lossy();
                log::debug!("Adding inferred path '{}'", path);
                lib_table.insert("path".into(), value::Value::String(path.into()));
            }
        }

        // Rewrite any dependency relative paths
        if let Some(dependencies) = self.toml.get_mut("dependencies") {
            let table = dependencies.as_table_mut()
                .ok_or(anyhow::anyhow!("dependencies should be a table"))?;
            for (name, value) in table {
                if let Some(dependency) = value.as_table_mut() {
                    if let Some(dep_path) = dependency.get_mut("path") {
                        to_absolute(format!("dependency {}", name), dep_path)?;
                    }
                }
            }
        }

        Ok(self)
    }

    /// Writes the amended manifest to the given directory.
    pub fn write<P: AsRef<Path>>(&self, path: P) -> Result<PathBuf> {
        let dir = path.as_ref();
        if !dir.is_dir() {
            anyhow::bail!("{} should be a directory", dir.display())
        }
        let manifest_path = dir.join(MANIFEST_FILE);

        let updated_toml = toml::to_string(&self.toml)?;
        fs::write(&manifest_path, updated_toml)
            .context(format!("Writing updated Cargo.toml to {}", manifest_path.display()))?;
        Ok(manifest_path)
    }

    /// Create the amended manifest in a temporary directory, executing the supplied function
    /// before the temporary file is cleaned up.
    pub fn using_temp<F>(&mut self, f: F) -> Result<()>
    where
        F: FnOnce(&Path) -> Result<()>,
    {
        self.rewrite_relative_paths()?;

        let tmp_dir = tempfile::Builder::new().prefix(".cargo-contract_").tempdir()?;
        let path = self.write(&tmp_dir)?;
        log::debug!("Using temp manifest '{}'", path.display());

        f(path.as_path())
    }
}

fn crate_type_exists(crate_type: &str, crate_types: &value::Array) -> bool {
    crate_types
        .iter()
        .any(|v| v.as_str().map_or(false, |s| s == crate_type))
}

struct Workspace {
    root: Manifest,
    members: HashMap<PathBuf, Manifest>,
}

impl Workspace {
    pub fn new(metadata: CargoMetadata) -> Result<Self> {
        let member_manifest = |package_id: &PackageId| -> Result<(PathBuf, Manifest)> {
            // package id e.g. "abi-gen 0.1.0 (path+file:///home/code/test/.ink/abi_gen)"
            let open_bracket = package_id.repr.find('(')
                .ok_or(anyhow::anyhow!("Package id should contain opening bracket for url"))?;
            let close_bracket = package_id.repr.find(')')
                .ok_or(anyhow::anyhow!("Package id should contain closing bracket for url"))?;
            let raw_url = &package_id.repr[open_bracket..close_bracket];
            let url = url::Url::parse(raw_url)?;
            if url.scheme() != "path+file" {
                anyhow::bail!("Workspace member package should be a file path")
            }
            let path = PathBuf::from(url.path());
            let manifest = Manifest::from_dir(Some(&path))?;
            Ok((path, manifest))
        };

        let root = Manifest::from_dir(Some(metadata.workspace_root))?;
        let members = metadata.workspace_members
            .iter()
            .map(member_manifest)
            .collect::<Result<HashMap<_,_>>>()?;

        Ok(Workspace { root, members })
    }

//    pub fn cop
}
