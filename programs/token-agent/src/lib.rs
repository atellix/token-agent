//use uuid::Uuid;
use std::{ string::String, mem::size_of, io::Cursor, result::Result as FnResult };
//use byte_slice_cast::*;
use bytemuck::{ Pod, Zeroable };
use num_enum::TryFromPrimitive;
use chrono::{ NaiveDateTime, Datelike };
use anchor_lang::prelude::*;
use anchor_spl::token::{ self, Transfer, TokenAccount, Approve };
use solana_program::{
    program::{ invoke_signed },
    account_info::AccountInfo,
    system_instruction,
    clock::Clock,
};

extern crate slab_alloc;
use slab_alloc::{ SlabPageAlloc, CritMapHeader, LeafNode, AnyNode, CritMap, SlabVec };

const MAX_REBILL_ENTRIES: u32 = 4;

#[repr(u8)]
#[derive(PartialEq, Debug, Eq, Copy, Clone, TryFromPrimitive)]
pub enum AccountDataType { // Specific account data type check to prevent mixing and matching of account parameters
    Undefined,
    Subscription,
    Rebill,
    RebillData, // Specified in RebillDataHeader
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

#[repr(u8)]
#[derive(PartialEq, Debug, Eq, Copy, Clone)]
pub enum SubscriptionPeriod {
    Daily,
    Weekly,
    Monthly,
    Quarterly,
    Yearly,
}

#[repr(u16)]
#[derive(PartialEq, Debug, Eq, Copy, Clone)]
pub enum DT { // Data types
    RebillDataHeader,
    RebillData,
}

#[derive(Copy, Clone)]
#[repr(packed)]
pub struct RebillDataHeader {
    pub data_type: AccountDataType,
    pub subscr_data: Pubkey,            // The subscription data account this rebill data is associated with
    pub manager_key: Pubkey,            // The rebill manager account being assigned
    pub manager_approval: Pubkey,       // The rebill manager approval from the network authority
    pub prev_data: Option<Pubkey>,      // The previous subscription data
    pub next_data: Option<Pubkey>,      // The next subscription data
}
unsafe impl Zeroable for RebillDataHeader {}
unsafe impl Pod for RebillDataHeader {}

impl RebillDataHeader {
    pub fn check_data_type(&self, check: AccountDataType) -> ProgramResult {
        if self.data_type != check {
            msg!("Error: Invalid data type for RebillData");
            return Err(ErrorCode::InvalidDataType.into());
        }
        Ok(())
    }

    pub fn subscr_data(&self) -> Pubkey {
        self.subscr_data
    }

    pub fn manager_key(&self) -> Pubkey {
        self.manager_key
    }

    pub fn manager_approval(&self) -> Pubkey {
        self.manager_approval
    }
}

#[derive(Copy, Clone)]
#[repr(packed)]
pub struct RebillData {
    pub event_uuid: u128,
    pub event_ts: i64,          // The UTC timestamp when the event is being processed
    pub rebill_ts: i64,         // The UTC timestamp that corresponds to the first second of the rebill period
    pub rebill_str: [u8; 32],   // The rebill datestamp string
    pub rebill_strlen: u8,      // The length of the rebill datestamp string
    pub manager_key: Pubkey,    // The public key of the manager doing the rebill
    pub amount: u64,            // The rebill amount in raw tokens (same decimals as the token mint)
}
unsafe impl Zeroable for RebillData {}
unsafe impl Pod for RebillData {}

impl RebillData {
    pub fn event_uuid(&self) -> u128 {
        self.event_uuid
    }

    pub fn event_ts(&self) -> i64 {
        self.event_ts
    }

    pub fn rebill_ts(&self) -> i64 {
        self.rebill_ts
    }

    pub fn rebill_str(&self) -> String {
        let rbslc = &self.rebill_str[0..self.rebill_strlen as usize];
        String::from_utf8(rbslc.to_vec()).expect("Invalid utf8 string")
    }

    pub fn manager_key(&self) -> Pubkey {
        self.manager_key
    }

    pub fn amount(&self) -> u64 {
        self.amount
    }
}

pub fn get_period_string(ts: i64, period: SubscriptionPeriod) -> FnResult<String, ProgramError> {
    let dt = NaiveDateTime::from_timestamp(ts, 0);
    match period {
        SubscriptionPeriod::Daily => Ok(dt.format("%Y%m%d").to_string()),
        SubscriptionPeriod::Weekly => Ok(dt.format("%Yw%U").to_string()),
        SubscriptionPeriod::Monthly => Ok(dt.format("%Y%m").to_string()),
        SubscriptionPeriod::Quarterly => {
            let mut q = dt.date().month().checked_div(3).ok_or(ProgramError::from(ErrorCode::Overflow))?;
            q = q.checked_add(1).ok_or(ProgramError::from(ErrorCode::Overflow))?;
            Ok(format!("{}q{}", dt.format("%Y").to_string(), q.to_string()))
        },
        SubscriptionPeriod::Yearly => Ok(dt.format("%Y").to_string()),
    }
}

#[program]
mod token_agent {
    use super::*;

    pub fn create_subscription(ctx: Context<CreateSubscr>,
//      link_token: bool,
//      initial_payment: bool,
//      initial_amount: u64,
//      initial_uuid: u64,
        inp_subscr_uuid: u128,
        inp_period: u8,
        inp_budget: u64,
        inp_pause_enabled: bool,
        inp_rebill_max: u32,
        inp_max_delay: u64,
        inp_not_valid_before: u64,
        inp_not_valid_after: u64,
    ) -> ProgramResult {
        // Setup up token delegate if needed

        // Create subscription data
        let subscr = &mut ctx.accounts.subscr_data;
        // TODO: network authority approvals
        subscr.user_key = *ctx.accounts.user_key.to_account_info().key;
        subscr.merchant_key = *ctx.accounts.merchant_key.to_account_info().key;
        subscr.token_mint = *ctx.accounts.token_mint.to_account_info().key;
        subscr.token_account = *ctx.accounts.token_account.to_account_info().key;
        subscr.rebill_data = *ctx.accounts.rebill_data.to_account_info().key;
        subscr.rebill_max = inp_rebill_max;
        subscr.max_delay = inp_max_delay;
        subscr.not_valid_before = inp_not_valid_before;
        subscr.not_valid_after = inp_not_valid_after;
        subscr.subscr_uuid = inp_subscr_uuid;
        subscr.period = inp_period;
        subscr.budget = inp_budget;
        subscr.pause_enabled = inp_pause_enabled;

        // Create rebill data
        let rbdata: &mut[u8] = &mut ctx.accounts.rebill_data.try_borrow_mut_data()?;
        let pt = SlabPageAlloc::new(rbdata);
        pt.setup_page_table();
        pt.allocate::<SlabVec, RebillDataHeader>(DT::RebillDataHeader as u16, 1).expect("Failed to allocate");
        pt.allocate::<CritMap, RebillData>(DT::RebillData as u16, MAX_REBILL_ENTRIES as usize).expect("Failed to allocate");
        let mut rebill_header_vec = SlabVec::new();
        let rebill_header = RebillDataHeader {
            data_type: AccountDataType::RebillData,
            subscr_data: *ctx.accounts.subscr_data.to_account_info().key,
            manager_key: *ctx.accounts.manager_key.to_account_info().key,
            manager_approval: *ctx.accounts.manager_approval.to_account_info().key,
            next_data: None,
            prev_data: None,
        };
        *pt.index_mut::<RebillDataHeader>(DT::RebillDataHeader as u16, rebill_header_vec.next_index() as usize) = rebill_header;
        *pt.header_mut::<SlabVec>(DT::RebillDataHeader as u16) = rebill_header_vec;

        // TODO: Log event

        Ok(())
    }

/*    pub fn update_subscription() -> ProgramResult {
        Ok(())
    }

    pub fn update_subscription_manager() -> ProgramResult {
        Ok(())
    } */

    pub fn process_subscription(ctx: Context<ProcessSubscr>,
        inp_event_uuid: u128,
        inp_rebill_ts: i64,
        inp_rebill_str: String,
        inp_amount: u64,
    ) -> ProgramResult {
        let clock = Clock::get()?;
        let ts = clock.unix_timestamp;
        msg!("Clock Timestamp: {}", ts.to_string());

        let d1 = get_period_string(ts, SubscriptionPeriod::Daily)?;
        msg!("Daily: {}", d1.to_string());
        let d2 = get_period_string(ts, SubscriptionPeriod::Weekly)?;
        msg!("Weekly: {}", d2.to_string());
        let d3 = get_period_string(ts, SubscriptionPeriod::Monthly)?;
        msg!("Monthly: {}", d3.to_string());
        let d4 = get_period_string(ts, SubscriptionPeriod::Quarterly)?;
        msg!("Quarterly: {}", d4.to_string());
        let d5 = get_period_string(ts, SubscriptionPeriod::Yearly)?;
        msg!("Yearly: {}", d5.to_string());

        /* let dt = NaiveDateTime::from_timestamp(ts, 0);
        let q = (dt.date().month() / 3).checked_add(1);
        if q == None {
            msg!("Overflow");
            return Err(ErrorCode::Overflow.into());
        }
        msg!("Day: {}", dt.format("%Y%m%d").to_string());
        msg!("Week: {}", dt.format("%Yw%U").to_string());
        msg!("Month: {}", dt.format("%Y%m").to_string());
        msg!("Quarter: {}q{}", dt.format("%Y").to_string(), q.unwrap().to_string());
        msg!("Year: {}", dt.format("%Y").to_string()); */
        Ok(())
    }

/*    pub fn approve_allowance() -> ProgramResult {
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
    pub merchant_approval: AccountInfo<'info>,
    pub manager_key: AccountInfo<'info>,
    //pub merchant_approval: ProgramAccount<'info, MerchantApproval>,
    pub manager_approval: AccountInfo<'info>,
    //pub manager_approval: ProgramAccount<'info, ManagerApproval>,
    //pub abort_authority: ProgramAccount<'info, MerchantApproval>,
    #[account(signer)]
    pub user_key: AccountInfo<'info>,
    pub token_mint: AccountInfo<'info>,
    pub token_account: AccountInfo<'info>,
    #[account(init)]
    pub rebill_data: AccountInfo<'info>,
}

#[derive(Accounts)]
pub struct ProcessSubscr<'info> {
    #[account(mut)]
    pub subscr_data: ProgramAccount<'info, SubscrData>,
    #[account(mut)]
    pub rebill_data: AccountInfo<'info>,
    #[account(signer)]
    pub manager_key: AccountInfo<'info>,
    pub manager_approval: AccountInfo<'info>,
}

#[account]
pub struct SubscrData {
    pub data_type: u8,                  // AccountDataType to prevent mixing and matching of data
    //pub approval_program: Pubkey,       // The address of the network authority program that signs approvals
    pub merchant_key: Pubkey,           // The merchant account that receives subscription payments
    pub merchant_approval: Pubkey,      // The merchant approval record from the network authority
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
    pub max_delay: u64,                 // The number of seconds after the start of the rebill period the manager can be delayed in attempting to rebill
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
    #[msg("Invalid data type")]
    InvalidDataType,
    #[msg("Overflow")]
    Overflow,
}
