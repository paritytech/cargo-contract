# contract-build

A crate for building [`ink!`](https://github.com/paritytech/ink) smart contracts. Used by
[`cargo-contract`](https://github.com/paritytech/cargo-contract).

## Usage

```rust
use contract_build::{
    ManifestPath,
    Verbosity,
    BuildArtifacts,
    BuildMode,
    Features,
    Network,
    OptimizationPasses,
    OutputType,
    UnstableFlags,
    Target,
    ImageVariant,
};

let manifest_path = ManifestPath::new("my-contract/Cargo.toml").unwrap();

let args = contract_build::ExecuteArgs {
    manifest_path,
    verbosity: Verbosity::Default,
    build_mode: BuildMode::Release,
    features: Features::default(),
    network: Network::Online,
    build_artifact: BuildArtifacts::All,
    unstable_flags: UnstableFlags::default(),
    optimization_passes: Some(OptimizationPasses::default()),
    keep_debug_symbols: false,
    extra_lints: false,
    output_type: OutputType::Json,
    skip_wasm_validation: false,
    target: Target::Wasm,
    max_memory_pages: 16,
    image: ImageVariant::Default,
};

contract_build::execute(args);
```
