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

use anyhow::Result;
use scale::{Encode, Decode, Input};
use ink_metadata::{InkProject, MessageParamSpec, Selector, MessageSpec};
use scale_info::{
	form::{CompactForm, Form},
	RegistryReadOnly, Type, TypeDef, TypeDefComposite, TypeDefPrimitive,
};
use std::str::FromStr;

pub struct Codec {
	metadata: InkProject,
}

impl Codec {
	pub fn new(metadata: InkProject) -> Self {
		Self { metadata }
	}

	fn registry(&self) -> &RegistryReadOnly {
		self.metadata.registry()
	}

	pub fn encode_constructor<I, S>(&self, name: &str, args: I) -> Result<Vec<u8>>
		where
			I: IntoIterator<Item = S>,
			S: AsRef<str>,
	{
		let constructors = self
			.metadata
			.spec()
			.constructors()
			.iter()
			.map(|m| m.name.clone())
			.collect::<Vec<_>>();

		let constructor_spec = self
			.metadata
			.spec()
			.constructors()
			.iter()
			.find(|msg| msg.name.contains(&name.to_string()))
			.ok_or(anyhow::anyhow!(
                "A contract call named '{}' was not found. Expected one of {:?}",
                name,
                constructors
            ))?;

		self.encode(&constructor_spec.selector, &constructor_spec.args, args)
	}

	pub fn encode_message<I, S>(&self, name: &str, args: I) -> Result<Vec<u8>>
		where
			I: IntoIterator<Item = S>,
			S: AsRef<str>,
	{
		let msg_spec = self.find_message_spec(name)?;
		self.encode(&msg_spec.selector(), &msg_spec.args(), args)
	}

	fn find_message_spec(&self, name: &str) -> Result<&MessageSpec<CompactForm>> {
		let calls = self
			.metadata
			.spec()
			.messages()
			.iter()
			.map(|m| m.name().clone())
			.collect::<Vec<_>>();

		self
			.metadata
			.spec()
			.messages()
			.iter()
			.find(|msg| msg.name().contains(&name.to_string()))
			.ok_or(anyhow::anyhow!(
                "A contract call named '{}' was not found. Expected one of {:?}",
                name,
                calls
            ))
	}

	fn encode<I, S>(
		&self,
		spec_selector: &Selector,
		spec_args: &[MessageParamSpec<CompactForm>],
		args: I,
	) -> Result<Vec<u8>>
		where
			I: IntoIterator<Item = S>,
			S: AsRef<str>,
	{
		let mut args = spec_args
			.iter()
			.zip(args)
			.map(|(spec, arg)| {
				let ty = resolve_type(self.registry(), spec.ty().ty())?;
				ty.type_def()
					.encode_arg(&self.registry(), arg.as_ref())
			})
			.collect::<Result<Vec<_>>>()?
			.concat();
		let mut encoded = spec_selector.to_bytes().to_vec();
		encoded.append(&mut args);
		Ok(encoded)
	}

	pub fn decode_events<I>(&self, data: &mut I) -> Result<DecodedEvent>
		where
			I: Input,
	{
		let variant_index = data.read_byte()?;
		let event_spec = self
			.metadata
			.spec()
			.events()
			.get(variant_index as usize)
			.ok_or(anyhow::anyhow!(
                "Event variant {} not found in contract metadata",
                variant_index
            ))?;
		let mut args = Vec::new();
		for arg in event_spec.args() {
			args.push(DecodedEventArg {
				name: arg.name().to_string(),
				value: "TODO".to_string(), // todo: resolve and decode type
			})
		}

		Ok(DecodedEvent {
			name: event_spec.name().to_string(),
			args,
		})
	}

	pub fn decode_return(&self, name: &str, data: Vec<u8>) -> Result<String> {
		let msg_spec = self.find_message_spec(name)?;
		if let Some(return_ty) = msg_spec.return_type().opt_type() {
			let ty = resolve_type(&self.registry(), return_ty.ty())?;
			ty.type_def().decode_to_string(self.registry(), &mut &data[..])
		} else {
			Ok(String::new())
		}
	}
}

fn resolve_type(
	registry: &RegistryReadOnly,
	symbol: &<CompactForm as Form>::Type,
) -> Result<Type<CompactForm>> {
	let ty = registry.resolve(symbol.id()).ok_or(anyhow::anyhow!(
        "Failed to resolve type with id '{}'",
        symbol.id()
    ))?;
	Ok(ty.clone())
}

pub trait EncodeContractArg {
	// todo: rename
	fn encode_arg(&self, registry: &RegistryReadOnly, arg: &str) -> Result<Vec<u8>>;
}

pub trait DecodeToString {
	fn decode_to_string(&self, registry: &RegistryReadOnly, input: &mut &[u8]) -> Result<String>;
}

impl EncodeContractArg for TypeDef<CompactForm> {
	fn encode_arg(&self, registry: &RegistryReadOnly, arg: &str) -> Result<Vec<u8>> {
		match self {
			TypeDef::Array(array) => {
				let ty = resolve_type(registry, array.type_param())?;
				match ty.type_def() {
					TypeDef::Primitive(TypeDefPrimitive::U8) => Ok(hex::decode(arg)?),
					_ => Err(anyhow::anyhow!("Only byte (u8) arrays supported")),
				}
			}
			TypeDef::Primitive(primitive) => primitive.encode_arg(registry, arg),
			TypeDef::Composite(composite) => composite.encode_arg(registry, arg),
			_ => unimplemented!(),
		}
	}
}

impl EncodeContractArg for TypeDefPrimitive {
	fn encode_arg(&self, _: &RegistryReadOnly, arg: &str) -> Result<Vec<u8>> {
		match self {
			TypeDefPrimitive::Bool => Ok(bool::encode(&bool::from_str(arg)?)),
			TypeDefPrimitive::Char => unimplemented!("scale codec not implemented for char"),
			TypeDefPrimitive::Str => Ok(str::encode(arg)),
			TypeDefPrimitive::U8 => Ok(u8::encode(&u8::from_str(arg)?)),
			TypeDefPrimitive::U16 => Ok(u16::encode(&u16::from_str(arg)?)),
			TypeDefPrimitive::U32 => Ok(u32::encode(&u32::from_str(arg)?)),
			TypeDefPrimitive::U64 => Ok(u64::encode(&u64::from_str(arg)?)),
			TypeDefPrimitive::U128 => Ok(u128::encode(&u128::from_str(arg)?)),
			TypeDefPrimitive::I8 => Ok(i8::encode(&i8::from_str(arg)?)),
			TypeDefPrimitive::I16 => Ok(i16::encode(&i16::from_str(arg)?)),
			TypeDefPrimitive::I32 => Ok(i32::encode(&i32::from_str(arg)?)),
			TypeDefPrimitive::I64 => Ok(i64::encode(&i64::from_str(arg)?)),
			TypeDefPrimitive::I128 => Ok(i128::encode(&i128::from_str(arg)?)),
		}
	}
}

impl EncodeContractArg for TypeDefComposite<CompactForm> {
	fn encode_arg(&self, registry: &RegistryReadOnly, arg: &str) -> Result<Vec<u8>> {
		if self.fields().len() != 1 {
			panic!("Only single field structs currently supported")
		}
		let field = self.fields().iter().next().unwrap();
		if field.name().is_none() {
			let ty = resolve_type(registry, field.ty())?;
			ty.type_def().encode_arg(registry, arg)
		} else {
			panic!("Only tuple structs currently supported")
		}
	}
}

impl DecodeToString for TypeDef<CompactForm> {
	fn decode_to_string(&self, registry: &RegistryReadOnly, input: &mut &[u8]) -> Result<String> {
		match self {
			TypeDef::Primitive(primitive) => primitive.decode_to_string(registry, input),
			def => unimplemented!("{:?}", def),
		}
	}
}

impl DecodeToString for TypeDefPrimitive {
	fn decode_to_string(&self, _: &RegistryReadOnly, input: &mut &[u8]) -> Result<String>
	{
		match self {
			TypeDefPrimitive::Bool => Ok(bool::decode(&mut &input[..])?.to_string()),
			prim => unimplemented!("{:?}", prim),
			// TypeDefPrimitive::Char => unimplemented!("scale codec not implemented for char"),
			// TypeDefPrimitive::Str => Ok(str::encode(arg)),
			// TypeDefPrimitive::U8 => Ok(u8::encode(&u8::from_str(arg)?)),
			// TypeDefPrimitive::U16 => Ok(u16::encode(&u16::from_str(arg)?)),
			// TypeDefPrimitive::U32 => Ok(u32::encode(&u32::from_str(arg)?)),
			// TypeDefPrimitive::U64 => Ok(u64::encode(&u64::from_str(arg)?)),
			// TypeDefPrimitive::U128 => Ok(u128::encode(&u128::from_str(arg)?)),
			// TypeDefPrimitive::I8 => Ok(i8::encode(&i8::from_str(arg)?)),
			// TypeDefPrimitive::I16 => Ok(i16::encode(&i16::from_str(arg)?)),
			// TypeDefPrimitive::I32 => Ok(i32::encode(&i32::from_str(arg)?)),
			// TypeDefPrimitive::I64 => Ok(i64::encode(&i64::from_str(arg)?)),
			// TypeDefPrimitive::I128 => Ok(i128::encode(&i128::from_str(arg)?)),
		}
	}
}

#[derive(Debug)]
pub struct DecodedEvent {
	name: String,
	args: Vec<DecodedEventArg>,
}

#[derive(Debug)]
pub struct DecodedEventArg {
	name: String,
	value: String,
}

//
// fn decode_event(registry: &RegistryReadOnly, input: &[u8]) -> Result<DecodedEvent> {
// 	match self {
// 		TypeDef::Array(array) => {
// 			match resolve_type(registry, array.type_param.id)? {
// 				Type { type_def: TypeDef::Primitive(TypeDefPrimitive::U8), .. } => {
// 					let len = <Compact<u32>>::decode(data)?;
// 					let mut bytes = Vec::new();
// 					for _ in 0..len.0 {
// 						bytes.push(u8::decode(data)?)
// 					}
// 				},
// 				_ => Err(anyhow::anyhow!("Only byte (u8) arrays supported")),
// 			}
// 		},
// 		TypeDef::Primitive(primitive) => primitive.encode_arg(registry, arg),
// 		TypeDef::Composite(composite) => composite.encode_arg(registry, arg),
// 		_ => unimplemented!(),
// 	}
// }
