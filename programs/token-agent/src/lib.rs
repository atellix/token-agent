//use uuid::Uuid;
//use std::{ mem::size_of, io::Cursor };
//use bytemuck::{ Pod, Zeroable };
//use byte_slice_cast::*;
use num_enum::TryFromPrimitive;
use anchor_lang::prelude::*;
use anchor_spl::token::{ self, Transfer, TokenAccount, Approve };
use solana_program::{
    program::{ invoke_signed },
    account_info::AccountInfo,
    system_instruction,
};

//extern crate slab_alloc;
//use slab_alloc::{ SlabPageAlloc, CritMapHeader, LeafNode, AnyNode, CritMap, SlabVec };

#[repr(u8)]
#[derive(PartialEq, Debug, Eq, Copy, Clone)]
pub enum AccountDataType { // Specific account data type check to prevent mixing and matching of account parameters
    Undefined,
    Subscription,
    Rebill,
    RebillManager,
    TokenAllowance,
}

#[repr(u32)]
#[derive(PartialEq, Debug, Eq, Copy, Clone)]
pub enum Status { // Status types
    Unallocated,
    Active,
    Deleted,
}

#[program]
mod token_agent {
    use super::*;

    pub fn create_subscription(ctx: Context<CreateSubscr>,
//        bool link_token
    ) -> ProgramResult {
        Ok(())
    }

/*    pub fn update_subscription() -> ProgramResult {
        Ok(())
    }

    pub fn update_subscription_manager() -> ProgramResult {
        Ok(())
    }

    pub fn process_subscription() -> ProgramResult {
        Ok(())
    }

    pub fn approve_allowance() -> ProgramResult {
        Ok(())
    }

    pub fn revoke_allowance() -> ProgramResult {
        Ok(())
    }

    pub fn delegated_transfer() -> ProgramResult {
        Ok(())
    } */
}

#[derive(Accounts)]
pub struct CreateSubscr<'info> {
    //pub approval_program: AccountInfo<'info>,
    #[account(init)]
    pub subscr_data: ProgramAccount<'info, SubscrData>,
    pub merchant_key: AccountInfo<'info>,
    //pub merchant_approval: ProgramAccount<'info, MerchantApproval>,
    //pub abort_authority: ProgramAccount<'info, MerchantApproval>,
    #[account(signer)]
    pub user_key: AccountInfo<'info>,
    pub token_mint: AccountInfo<'info>,
    pub token_account: AccountInfo<'info>,
    #[account(init)]
    pub rebill_data: AccountInfo<'info>,
}

#[repr(u8)]
#[derive(PartialEq, Debug, Eq, Copy, Clone)]
pub enum SubscriptionPeriod {
    Daily,
    Weekly,
    Monthly,
    Quarterly,
    Yearly,
}

#[account]
pub struct SubscrData {
    pub data_type: u8,                  // AccountDataType to prevent mixing and matching of data
    //pub approval_program: Pubkey,       // The address of the network authority program that signs approvals
    pub merchant_key: Pubkey,           // The merchant account that receives subscription payments
    //pub merchant_approval: Pubkey,      // The merchant approval record from the network authority
    //pub abort_authority: Pubkey,        // The abort authority from the network authority to abort in case of hacks
    pub user_key: Pubkey,               // The user that owns this subscription
    pub token_mint: Pubkey,             // The token mint to pay for the subscription
    pub token_account: Pubkey,          // The token account to pay for the subscription
    pub rebill_data: Pubkey,            // The rebill data account to track subscription rebills and prevent duplicates
    // Subscription details below
    pub rebill_events: u32,             // Count of rebill events
    pub rebill_max: u32,                // Maximum number of times to rebill (0 = unlimited)
    pub not_valid_before: u64,          // UTC timestamp before which no subscription processing can occur
    pub not_valid_after: u64,           // UTC timestamp after which no subscription processing can occur
    pub subscr_uuid: u128,              // Subscription UUID
    pub period: u8,                     // Subscription rebill period
    pub budget: u64,                    // Subscription budget (maximum amount, not necessarily the amount that will be billed which could be less)
    pub pause_enabled: bool,            // Subscription able to be paused
    pub paused: bool,                   // Subscription is paused
}

// TODO: Merchant approval
// TODO: Rebill approval
// TODO: Abort authority

#[account]
pub struct AssignRebillManager {
    pub data_type: u8,                  // AccountDataType to prevent mixing and matching of data
    pub subscr_data: Pubkey,            // The subscription data for this assignment
    pub rebill_approval: Pubkey,        // The rebill manager approval from the network authority
    pub manager_key: Pubkey,            // The rebill manager account being assigned
}

#[account]
pub struct TokenAllowance {
    pub data_type: u8,                  // AccountDataType to prevent mixing and matching of data
    //pub abort_authority: Pubkey,        // The abort authority from the network authority to abort in case of hacks
    pub user_key: Pubkey,               // The user that owns the tokens
    pub delegate_key: Pubkey,           // The delegate granted an allowance of tokens to transfer
    pub recipient_key: Option<Pubkey>,  // Optional recipient key to limit where tokens can be transferred to
    pub token_mint: Pubkey,             // The token mint for the allowance
    pub token_account: Pubkey,          // The token account for the allowance
    pub not_valid_before: u64,          // UTC timestamp before which no subscription processing can occur
    pub not_valid_after: u64,           // UTC timestamp after which no subscription processing can occur
    pub amount: u64,                    // The amount of tokens for the allowance (same decimals as underlying token)
}

#[error]
pub enum ErrorCode {
    #[msg("Access denied")]
    AccessDenied,
}
