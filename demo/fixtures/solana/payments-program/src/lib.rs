//! A trivial "payments" program for the Aegis demo.
//!
//! Its only job is to exist as a distinct program id that a transaction can invoke, so the
//! demo can show two things:
//!   1. block-by-program  — a rule keyed on this program id refuses the transaction;
//!   2. LUT-delivered sanction — a *sanctioned recipient* is passed via an Address Lookup
//!      Table, so the account key is not in the static part of the message.

use solana_program::{
    account_info::AccountInfo, entrypoint, entrypoint::ProgramResult, msg, pubkey::Pubkey,
};

entrypoint!(process_instruction);

/// Instruction 0: "pay" — logs the recipient. No real value movement (demo only).
pub fn process_instruction(
    _program_id: &Pubkey,
    accounts: &[AccountInfo],
    _data: &[u8],
) -> ProgramResult {
    let recipient = accounts.first().map(|a| a.key.to_string()).unwrap_or_default();
    msg!("payments-program: paying {}", recipient);
    Ok(())
}
