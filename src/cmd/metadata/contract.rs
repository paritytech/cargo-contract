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

use core::fmt::{Display, Formatter, Result as DisplayResult, Write};
use semver::Version;
use serde::{Serialize, Serializer};
use serde_json::{Map, Value};
use url::Url;

const METADATA_VERSION: &str = "0.1.0";

/// An entire ink! project for metadata file generation purposes.
#[derive(Debug, Serialize)]
pub struct ContractMetadata {
    metadata_version: semver::Version,
    source: Source,
    contract: Contract,
    #[serde(skip_serializing_if = "Option::is_none")]
    user: Option<User>,
    /// Raw JSON of the metadata generated by the ink! contract itself
    #[serde(flatten)]
    ink: Map<String, Value>,
}

impl ContractMetadata {
    /// Construct a new ContractMetadata
    pub fn new(
        source: Source,
        contract: Contract,
        user: Option<User>,
        ink: Map<String, Value>,
    ) -> Self {
        let metadata_version = semver::Version::parse(METADATA_VERSION)
            .expect("METADATA_VERSION is a valid semver string");

        ContractMetadata {
            metadata_version,
            source,
            contract,
            user,
            ink,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct Source {
    #[serde(serialize_with = "serialize_as_byte_str")]
    hash: [u8; 32],
    language: SourceLanguage,
    compiler: SourceCompiler,
}

impl Source {
    /// Constructs a new InkProjectSource.
    pub fn new(hash: [u8; 32], language: SourceLanguage, compiler: SourceCompiler) -> Self {
        Source {
            hash,
            language,
            compiler,
        }
    }
}

/// The language and version in which a smart contract is written.
#[derive(Debug)]
pub struct SourceLanguage {
    language: Language,
    version: Version,
}

impl SourceLanguage {
    /// Constructs a new SourceLanguage.
    pub fn new(language: Language, version: Version) -> Self {
        SourceLanguage { language, version }
    }
}

impl Serialize for SourceLanguage {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

impl Display for SourceLanguage {
    fn fmt(&self, f: &mut Formatter<'_>) -> DisplayResult {
        write!(f, "{} {}", self.language, self.version)
    }
}

/// The language in which the smart contract is written.
#[derive(Debug)]
pub enum Language {
    Ink,
    Solidity,
    AssemblyScript,
}

impl Display for Language {
    fn fmt(&self, f: &mut Formatter<'_>) -> DisplayResult {
        match self {
            Self::Ink => write!(f, "ink!"),
            Self::Solidity => write!(f, "Solidity"),
            Self::AssemblyScript => write!(f, "AssemblyScript"),
        }
    }
}

/// A compiler used to compile a smart contract.
#[derive(Debug)]
pub struct SourceCompiler {
    compiler: Compiler,
    version: Version,
}

impl Display for SourceCompiler {
    fn fmt(&self, f: &mut Formatter<'_>) -> DisplayResult {
        write!(f, "{} {}", self.compiler, self.version)
    }
}

impl Serialize for SourceCompiler {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

impl SourceCompiler {
    pub fn new(compiler: Compiler, version: Version) -> Self {
        SourceCompiler { compiler, version }
    }
}

/// Compilers used to compile a smart contract.
#[derive(Debug, Serialize)]
pub enum Compiler {
    RustC,
    Solang,
}

impl Display for Compiler {
    fn fmt(&self, f: &mut Formatter<'_>) -> DisplayResult {
        match self {
            Self::RustC => write!(f, "rustc"),
            Self::Solang => write!(f, "solang"),
        }
    }
}

/// Metadata about a smart contract.
#[derive(Debug, Serialize)]
pub struct Contract {
    name: String,
    version: Version,
    authors: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    documentation: Option<Url>,
    #[serde(skip_serializing_if = "Option::is_none")]
    repository: Option<Url>,
    #[serde(skip_serializing_if = "Option::is_none")]
    homepage: Option<Url>,
    #[serde(skip_serializing_if = "Option::is_none")]
    license: Option<String>,
}

impl Contract {
    /// Constructs a new Contract.
    pub fn new(
        name: String,
        version: Version,
        authors: Vec<String>,
        description: Option<String>,
        documentation: Option<Url>,
        repository: Option<Url>,
        homepage: Option<Url>,
        license: Option<String>,
    ) -> Self {
        Contract {
            name,
            version,
            authors,
            description,
            documentation,
            repository,
            homepage,
            license,
        }
    }
}

/// Additional user defined metadata, can be any valid json.
#[derive(Debug, Serialize)]
pub struct User {
    #[serde(flatten)]
    json: Map<String, Value>,
}

impl User {
    /// Constructs a new InkProjectUser
    pub fn new(json: Map<String, Value>) -> Self {
        User { json }
    }
}

/// Serializes the given bytes as byte string.
fn serialize_as_byte_str<S>(bytes: &[u8], serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    if bytes.is_empty() {
        // Return empty string without prepended `0x`.
        return serializer.serialize_str("");
    }
    let mut hex = String::with_capacity(bytes.len() * 2 + 2);
    write!(hex, "0x").expect("failed writing to string");
    for byte in bytes {
        write!(hex, "{:02x}", byte).expect("failed writing to string");
    }
    serializer.serialize_str(&hex)
}
