// Copyright (c) 2022 MASSA LABS <info@massa.net>
#![feature(int_roundings)]
#![warn(unused_crate_dependencies)]

pub mod error;
mod export_pos;
mod settings;

use massa_models::{
    rolls::{RollUpdate, RollUpdates},
    Address, Operation, OperationType,
};

mod proof_of_stake;
pub use proof_of_stake::*;

use error::ProofOfStakeError;
pub use export_pos::ExportProofOfStake;
pub use settings::ProofOfStakeConfig;

mod thread_cycle_state;
pub use thread_cycle_state::ThreadCycleState;

/// Roll specific method on operation
pub trait OperationRollInterface {
    /// get roll related modifications
    fn get_roll_updates(&self) -> Result<RollUpdates, ProofOfStakeError>;
}

impl OperationRollInterface for Operation {
    fn get_roll_updates(&self) -> Result<RollUpdates, ProofOfStakeError> {
        let mut res = RollUpdates::default();
        match self.op {
            OperationType::Transaction { .. } => {}
            OperationType::RollBuy { roll_count } => {
                res.apply(
                    &Address::from_public_key(&self.sender_public_key),
                    &RollUpdate {
                        roll_purchases: roll_count,
                        roll_sales: 0,
                    },
                )?;
            }
            OperationType::RollSell { roll_count } => {
                res.apply(
                    &Address::from_public_key(&self.sender_public_key),
                    &RollUpdate {
                        roll_purchases: 0,
                        roll_sales: roll_count,
                    },
                )?;
            }
            OperationType::ExecuteSC { .. } => {}
        }
        Ok(res)
    }
}
