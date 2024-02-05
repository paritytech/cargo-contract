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
    account_id,
    events::DisplayEvents,
    pallet_contracts_primitives::ContractExecResult,
    state,
    state_call,
    submit_extrinsic,
    BalanceVariant,
    ContractMessageTranscoder,
    ErrorVariant,
    Missing,
    TokenMetadata,
};
use crate::{
    check_env_types,
    extrinsic_calls::Call,
    extrinsic_opts::ExtrinsicOpts,
};

use anyhow::{
    anyhow,
    Result,
};
use ink_env::Environment;
use scale::Encode;
use sp_weights::Weight;
use subxt_signer::sr25519::Keypair;

use core::marker::PhantomData;
use std::str::FromStr;
use subxt::{
    backend::{
        legacy::LegacyRpcMethods,
        rpc::RpcClient,
    },
    ext::{
        scale_decode::IntoVisitor,
        scale_encode::EncodeAsType,
    },
    Config,
    OnlineClient,
};

pub struct CallOpts<C: Config, E: Environment> {
    contract: Option<C::AccountId>,
    message: String,
    args: Vec<String>,
    extrinsic_opts: ExtrinsicOpts<E>,
    gas_limit: Option<u64>,
    proof_size: Option<u64>,
    value: BalanceVariant<E::Balance>,
}

/// A builder for the call command.
pub struct CallCommandBuilder<C: Config, E: Environment, Message, ExtrinsicOptions> {
    opts: CallOpts<C, E>,
    marker: PhantomData<fn() -> (Message, ExtrinsicOptions)>,
}

impl<C: Config, E: Environment> Default
    for CallCommandBuilder<
        C,
        E,
        Missing<state::Message>,
        Missing<state::ExtrinsicOptions>,
    >
where
    E::Balance: FromStr + From<u128>,
{
    fn default() -> Self {
        Self::new()
    }
}

impl<C: Config, E: Environment, T> CallCommandBuilder<C, E, Missing<state::Message>, T>
where
    E::Balance: FromStr + From<u128>,
{
    /// Returns a clean builder for [`CallExec`].
    pub fn new(
    ) -> CallCommandBuilder<C, E, Missing<state::Message>, Missing<state::ExtrinsicOptions>>
    {
        CallCommandBuilder {
            opts: CallOpts {
                contract: None,
                message: String::new(),
                args: Vec::new(),
                extrinsic_opts: ExtrinsicOpts::default(),
                gas_limit: None,
                proof_size: None,
                value: "0".parse().unwrap(),
            },
            marker: PhantomData,
        }
    }

    /// Sets the name of the contract message to call.
    pub fn message<M: Into<String>>(
        self,
        message: M,
    ) -> CallCommandBuilder<C, E, state::Message, T> {
        CallCommandBuilder {
            opts: CallOpts {
                message: message.into(),
                ..self.opts
            },
            marker: PhantomData,
        }
    }
}

impl<C: Config, E: Environment, M>
    CallCommandBuilder<C, E, M, Missing<state::ExtrinsicOptions>>
{
    /// Sets the extrinsic operation.
    pub fn extrinsic_opts(
        self,
        extrinsic_opts: ExtrinsicOpts<E>,
    ) -> CallCommandBuilder<C, E, M, state::ExtrinsicOptions> {
        CallCommandBuilder {
            opts: CallOpts {
                extrinsic_opts,
                ..self.opts
            },
            marker: PhantomData,
        }
    }
}

impl<C: Config, E: Environment, M, X> CallCommandBuilder<C, E, M, X> {
    /// Sets the the address of the the contract to call.
    pub fn contract(self, contract: C::AccountId) -> Self {
        let mut this = self;
        this.opts.contract = Some(contract);
        this
    }

    /// Sets the arguments of the contract message to call.
    pub fn args<T: ToString>(self, args: Vec<T>) -> Self {
        let mut this = self;
        this.opts.args = args.into_iter().map(|arg| arg.to_string()).collect();
        this
    }

    /// Sets the maximum amount of gas to be used for this command.
    pub fn gas_limit(self, gas_limit: Option<u64>) -> Self {
        let mut this = self;
        this.opts.gas_limit = gas_limit;
        this
    }

    /// Sets the maximum proof size for this call.
    pub fn proof_size(self, proof_size: Option<u64>) -> Self {
        let mut this = self;
        this.opts.proof_size = proof_size;
        this
    }

    /// Sets the value to be transferred as part of the call.
    pub fn value(self, value: BalanceVariant<E::Balance>) -> Self {
        let mut this = self;
        this.opts.value = value;
        this
    }
}

impl<C: Config, E: Environment>
    CallCommandBuilder<C, E, state::Message, state::ExtrinsicOptions>
where
    C::AccountId: IntoVisitor,
    E::Balance: From<u128>,
{
    /// Preprocesses contract artifacts and options for subsequent contract calls.
    ///
    /// This function prepares the necessary data for making a contract call based on the
    /// provided contract artifacts, message, arguments, and options. It ensures that the
    /// required contract code and message data are available, sets up the client, signer,
    /// and other relevant parameters, preparing for the contract call operation.
    ///
    /// Returns the `CallExec` containing the preprocessed data for the contract call,
    /// or an error in case of failure.
    pub async fn done(self) -> Result<CallExec<C, E>> {
        let artifacts = self.opts.extrinsic_opts.contract_artifacts()?;
        let transcoder = artifacts.contract_transcoder()?;

        let call_data = transcoder.encode(&self.opts.message, &self.opts.args)?;
        tracing::debug!("Message data: {:?}", hex::encode(&call_data));

        let signer = self.opts.extrinsic_opts.signer()?;

        let url = self.opts.extrinsic_opts.url();
        let rpc = RpcClient::from_url(&url).await?;
        let client = OnlineClient::from_rpc_client(rpc.clone()).await?;
        let rpc = LegacyRpcMethods::new(rpc);
        check_env_types(&client, &transcoder, self.opts.extrinsic_opts.verbosity())?;

        let token_metadata = TokenMetadata::query(&rpc).await?;
        let contract = self
            .opts
            .contract
            .ok_or(anyhow!("Contract address not set"))?;

        Ok(CallExec {
            contract,
            message: self.opts.message.clone(),
            args: self.opts.args.clone(),
            opts: self.opts.extrinsic_opts.clone(),
            gas_limit: self.opts.gas_limit,
            proof_size: self.opts.proof_size,
            value: self.opts.value.clone(),
            rpc,
            client,
            transcoder,
            call_data,
            signer,
            token_metadata,
        })
    }
}

pub struct CallExec<C: Config, E: Environment>
where
    E::Balance: From<u128>,
{
    contract: C::AccountId,
    message: String,
    args: Vec<String>,
    opts: ExtrinsicOpts<E>,
    gas_limit: Option<u64>,
    proof_size: Option<u64>,
    value: BalanceVariant<E::Balance>,
    rpc: LegacyRpcMethods<C>,
    client: OnlineClient<C>,
    transcoder: ContractMessageTranscoder,
    call_data: Vec<u8>,
    signer: Keypair,
    token_metadata: TokenMetadata,
}

impl<C: Config, E: Environment> CallExec<C, E>
where
    C::Signature: From<subxt_signer::sr25519::Signature>,
    <C::ExtrinsicParams as subxt::config::ExtrinsicParams<C>>::OtherParams: Default,
    C::Address: From<subxt_signer::sr25519::PublicKey>,
    C::AccountId: From<subxt_signer::sr25519::PublicKey> + EncodeAsType + IntoVisitor,
    E::Balance: From<u128>,
{
    /// Simulates a contract call without modifying the blockchain.
    ///
    /// This function performs a dry run simulation of a contract call, capturing
    /// essential information such as the contract address, gas consumption, and
    /// storage deposit. The simulation is executed without actually executing the
    /// call on the blockchain.
    ///
    /// Returns the dry run simulation result of type [`ContractExecResult`], which
    /// includes information about the simulated call, or an error in case of failure.
    pub async fn call_dry_run(&self) -> Result<ContractExecResult<E::Balance, ()>> {
        let storage_deposit_limit = self
            .opts
            .storage_deposit_limit_balance(&self.token_metadata)?;
        let call_request = CallRequest {
            origin: account_id::<C>(&self.signer),
            dest: self.contract.clone(),
            value: self.value.denominate_balance(&self.token_metadata)?,
            gas_limit: None,
            storage_deposit_limit,
            input_data: self.call_data.clone(),
        };
        state_call(&self.rpc, "ContractsApi_call", call_request).await
    }

    /// Calls a contract on the blockchain with a specified gas limit.
    ///
    /// This function facilitates the process of invoking a contract, specifying the gas
    /// limit for the operation. It interacts with the blockchain's runtime API to
    /// execute the contract call and provides the resulting events from the call.
    ///
    /// Returns the events generated from the contract call, or an error in case of
    /// failure.
    pub async fn call(&self, gas_limit: Option<Weight>) -> Result<DisplayEvents, ErrorVariant> {
        if !self
            .transcoder()
            .metadata()
            .spec()
            .messages()
            .iter()
            .find(|msg| msg.label() == &self.message)
            .expect("message exist after calling CallExec::done()")
            .mutates()
        {
            let inner = anyhow!(
                "Tried to execute a call on the immutable contract message '{}'. Please do a dry-run instead.",
                &self.message
            );
            return Err(inner.into())
        }

        // use user specified values where provided, otherwise estimate
        let gas_limit = match gas_limit {
            Some(gas_limit) => gas_limit,
            None => self.estimate_gas().await?,
        };
        tracing::debug!("calling contract {:?}", self.contract);
        let storage_deposit_limit = self
            .opts
            .storage_deposit_limit_balance(&self.token_metadata)?;

        let call = Call::new(
            self.contract.clone().into(),
            self.value.denominate_balance(&self.token_metadata)?,
            gas_limit,
            storage_deposit_limit,
            self.call_data.clone(),
        )
        .build();

        let result =
            submit_extrinsic(&self.client, &self.rpc, &call, &self.signer).await?;

        let display_events = DisplayEvents::from_events::<C, E>(
            &result,
            Some(&self.transcoder),
            &self.client.metadata(),
        )?;

        Ok(display_events)
    }

    /// Estimates the gas required for a contract call without modifying the blockchain.
    ///
    /// This function provides a gas estimation for contract calls, considering the
    /// user-specified values or using estimates based on a dry run. The estimated gas
    /// weight is returned, or an error is reported if the estimation fails.
    ///
    /// Returns the estimated gas weight of type [`Weight`] for contract calls, or an
    /// error.
    pub async fn estimate_gas(&self) -> Result<Weight> {
        match (self.gas_limit, self.proof_size) {
            (Some(ref_time), Some(proof_size)) => {
                Ok(Weight::from_parts(ref_time, proof_size))
            }
            _ => {
                let call_result = self.call_dry_run().await?;
                match call_result.result {
                    Ok(_) => {
                        // use user specified values where provided, otherwise use the
                        // estimates
                        let ref_time = self
                            .gas_limit
                            .unwrap_or_else(|| call_result.gas_required.ref_time());
                        let proof_size = self
                            .proof_size
                            .unwrap_or_else(|| call_result.gas_required.proof_size());
                        Ok(Weight::from_parts(ref_time, proof_size))
                    }
                    Err(ref err) => {
                        let object = ErrorVariant::from_dispatch_error(
                            err,
                            &self.client.metadata(),
                        )?;
                        Err(anyhow!("Pre-submission dry-run failed. Error: {}", object))
                    }
                }
            }
        }
    }

    /// Returns the address of the the contract to call.
    pub fn contract(&self) -> &C::AccountId {
        &self.contract
    }

    /// Returns the name of the contract message to call.
    pub fn message(&self) -> &str {
        &self.message
    }

    /// Returns the arguments of the contract message to call.
    pub fn args(&self) -> &Vec<String> {
        &self.args
    }

    /// Returns the extrinsic options.
    pub fn opts(&self) -> &ExtrinsicOpts<E> {
        &self.opts
    }

    /// Returns the maximum amount of gas to be used for this command.
    pub fn gas_limit(&self) -> Option<u64> {
        self.gas_limit
    }

    /// Returns the maximum proof size for this call.
    pub fn proof_size(&self) -> Option<u64> {
        self.proof_size
    }

    /// Returns the value to be transferred as part of the call.
    pub fn value(&self) -> &BalanceVariant<E::Balance> {
        &self.value
    }

    /// Returns the client.
    pub fn client(&self) -> &OnlineClient<C> {
        &self.client
    }

    /// Returns the contract message transcoder.
    pub fn transcoder(&self) -> &ContractMessageTranscoder {
        &self.transcoder
    }

    /// Returns the call data.
    pub fn call_data(&self) -> &Vec<u8> {
        &self.call_data
    }

    /// Returns the signer.
    pub fn signer(&self) -> &Keypair {
        &self.signer
    }

    /// Returns the token metadata.
    pub fn token_metadata(&self) -> &TokenMetadata {
        &self.token_metadata
    }
}

/// A struct that encodes RPC parameters required for a call to a smart contract.
///
/// Copied from `pallet-contracts-rpc-runtime-api`.
#[derive(Encode)]
struct CallRequest<AccountId, Balance> {
    origin: AccountId,
    dest: AccountId,
    value: Balance,
    gas_limit: Option<Weight>,
    storage_deposit_limit: Option<Balance>,
    input_data: Vec<u8>,
}
