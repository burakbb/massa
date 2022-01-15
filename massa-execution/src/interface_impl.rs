/// Implementation of the interface used in the execution external library
///
use std::str::FromStr;

use crate::types::ExecutionContext;
use anyhow::{bail, Result};
use assembly_simulator::{Bytecode, Interface, InterfaceClone};
use massa_models::{
    output_event::{EventExecutionContext, SCOutputEvent},
    Address, Amount,
};
use std::sync::{Arc, Mutex};
use tracing::debug;

macro_rules! context_guard {
    ($self:ident) => {
        $self
            .context
            .lock()
            .expect("Failed to acquire lock on context.")
    };
}

#[derive(Clone)]
pub(crate) struct InterfaceImpl {
    context: Arc<Mutex<ExecutionContext>>,
}

impl InterfaceImpl {
    pub fn new(context: Arc<Mutex<ExecutionContext>>) -> InterfaceImpl {
        InterfaceImpl { context }
    }
}

impl InterfaceClone for InterfaceImpl {
    fn clone_box(&self) -> Box<dyn Interface> {
        Box::new(self.clone())
    }
}

impl Interface for InterfaceImpl {
    fn print(&self, message: &str) -> Result<()> {
        debug!("SC print: {}", message);
        Ok(())
    }

    fn get_module(&self, address: &String) -> Result<Vec<u8>> {
        let address = Address::from_str(address)?;
        let bytecode = { context_guard!(self).ledger_step.get_module(&address) };
        match bytecode {
            Some(bytecode) => {
                context_guard!(self).call_stack.push_back(address);
                Ok(bytecode)
            }
            _ => bail!("Error bytecode not found"),
        }
    }

    /// Returns zero as a default if address not found.
    fn get_balance(&self) -> Result<u64> {
        let context = context_guard!(self);
        let address = match context.call_stack.back() {
            Some(addr) => addr,
            _ => bail!("Failed to read call stack current address"),
        };
        Ok(context.ledger_step.get_balance(address).to_raw())
    }

    /// Returns zero as a default if address not found.
    fn get_balance_for(&self, address: &String) -> Result<u64> {
        let address = Address::from_str(address)?;
        Ok(context_guard!(self)
            .ledger_step
            .get_balance(&address)
            .to_raw())
    }

    fn exit_success(&self) -> Result<()> {
        match context_guard!(self).call_stack.pop_back() {
            Some(_) => Ok(()),
            _ => bail!("Call stack Out of bound"),
        }
    }

    /// Requires a new address that contains the sent bytecode.
    ///
    /// Generate a new address with a concatenation of the block_id hash, the
    /// operation index in the block and the index of address owned in context.
    ///
    /// Insert in the ledger the given bytecode in the generated address
    fn create_module(&self, module: &Bytecode) -> Result<assembly_simulator::Address> {
        let mut context = context_guard!(self);
        let (slot, created_addr_index) = (context.slot, context.created_addr_index);
        let mut data: Vec<u8> = slot.to_bytes_key().to_vec();
        data.append(&mut created_addr_index.to_be_bytes().to_vec());
        if context.read_only {
            data.push(0u8);
        } else {
            data.push(1u8);
        }
        let address = Address(massa_hash::hash::Hash::compute_from(&data));
        let res = address.to_bs58_check();
        context
            .ledger_step
            .set_module(address, Some(module.clone()));
        context.owned_addresses.insert(address);
        context.created_addr_index += 1;
        Ok(res)
    }

    /// Requires the data at the address
    fn get_data_for(&self, address: &assembly_simulator::Address, key: &str) -> Result<Bytecode> {
        let addr = &Address::from_bs58_check(address)?;
        // @damip is it ok to get a hash like that?
        let key = massa_hash::hash::Hash::from_bs58_check(key)?;
        let context = context_guard!(self);
        match context.ledger_step.get_data_entry(addr, &key) {
            Some(bytecode) => Ok(bytecode),
            _ => bail!("Data entry not found"),
        }
    }

    /// Requires to replace the data in the current address
    ///
    /// Note:
    /// The execution lib will allways use the current context address for the update
    fn set_data_for(
        &self,
        address: &assembly_simulator::Address,
        key: &str,
        value: &Bytecode,
    ) -> Result<()> {
        let addr = Address::from_str(address)?;
        let key = massa_hash::hash::Hash::from_bs58_check(key)?;
        let mut context = context_guard!(self);
        let is_curr = match context.call_stack.back() {
            Some(curr_address) => addr == *curr_address,
            _ => false,
        };
        if context.owned_addresses.contains(&addr) || is_curr {
            context.ledger_step.set_data_entry(addr, key, value.clone());
            Ok(())
        } else {
            bail!("You don't have the write access to this entry")
        }
    }

    fn get_data(&self, key: &str) -> Result<Bytecode> {
        let context = context_guard!(self);
        let addr = match context.call_stack.back() {
            Some(addr) => addr,
            _ => bail!("Failed to read call stack current address"),
        };
        let key = massa_hash::hash::Hash::from_bs58_check(key)?;
        match context.ledger_step.get_data_entry(addr, &key) {
            Some(bytecode) => Ok(bytecode),
            _ => bail!("Data entry not found"),
        }
    }

    fn set_data(&self, key: &str, value: &Bytecode) -> Result<()> {
        let mut context = context_guard!(self);
        let addr = match context.call_stack.back() {
            Some(addr) => *addr,
            _ => bail!("Failed to read call stack current address"),
        };
        let key = massa_hash::hash::Hash::from_bs58_check(key)?;
        context.ledger_step.set_data_entry(addr, key, value.clone());
        Ok(())
    }

    /// Transfer coins from the current address to a target address
    /// to_address: target address
    /// raw_amount: amount to transfer (in raw u64)
    fn transfer_coins(&self, to_address: &String, raw_amount: u64) -> Result<()> {
        let to_address = Address::from_str(to_address)?;
        let mut context = context_guard!(self);
        let from_address = match context.call_stack.back() {
            Some(addr) => *addr,
            _ => bail!("Failed to read call stack current address"),
        };
        let amount = Amount::from_raw(raw_amount);
        // debit
        context
            .ledger_step
            .set_balance_delta(from_address, amount, false)?;
        // credit
        if let Err(err) = context
            .ledger_step
            .set_balance_delta(to_address, amount, true)
        {
            // cancel debit
            context
                .ledger_step
                .set_balance_delta(from_address, amount, true)
                .expect("credit failed after same-amount debit succeeded");
            bail!("Error crediting destination balance: {}", err);
        }
        Ok(())
    }

    /// Transfer coins from the current address to a target address
    /// from_address: source address
    /// to_address: target address
    /// raw_amount: amount to transfer (in raw u64)
    fn transfer_coins_for(
        &self,
        from_address: &String,
        to_address: &String,
        raw_amount: u64,
    ) -> Result<()> {
        let from_address = Address::from_str(from_address)?;
        let to_address = Address::from_str(to_address)?;
        let mut context = context_guard!(self);
        let is_curr = match context.call_stack.back() {
            Some(curr_address) => from_address == *curr_address,
            _ => false,
        };
        if !context.owned_addresses.contains(&from_address) && !is_curr {
            bail!("You don't have the spending access to this entry")
        }
        let amount = Amount::from_raw(raw_amount);
        // debit
        context
            .ledger_step
            .set_balance_delta(from_address, amount, false)?;
        // credit
        if let Err(err) = context
            .ledger_step
            .set_balance_delta(to_address, amount, true)
        {
            // cancel debit
            context
                .ledger_step
                .set_balance_delta(from_address, amount, true)
                .expect("credit failed after same-amount debit succeeded");
            bail!("Error crediting destination balance: {}", err);
        }
        Ok(())
    }

    /// Return the list of owned adresses of a given SC user
    fn get_owned_addresses(&self) -> Result<Vec<assembly_simulator::Address>> {
        Ok(context_guard!(self)
            .owned_addresses
            .iter()
            .map(|addr| addr.to_bs58_check())
            .collect())
    }

    fn get_call_stack(&self) -> Result<Vec<assembly_simulator::Address>> {
        Ok(context_guard!(self)
            .call_stack
            .iter()
            .map(|addr| addr.to_bs58_check())
            .collect())
    }

    fn generate_event(&self, data: String) -> Result<()> {
        let context = context_guard!(self);
        let slot = context.slot;
        let block = context.opt_block_id;
        let call_stack = context.call_stack.clone();
        let context = EventExecutionContext {
            slot,
            block,
            call_stack,
        };
        let event = SCOutputEvent { context, data };
        debug!("SC event: {:?}", event);
        // TODO store the event somewhere
        Ok(())
    }
}