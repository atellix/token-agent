//use uuid::Uuid;
use std::{ string::String, result::Result as FnResult, str::FromStr };
//use bytemuck::{ Pod, Zeroable };
use num_enum::TryFromPrimitive;
use chrono::{ NaiveDateTime, Datelike };
use anchor_lang::prelude::*;
use anchor_spl::token::{ self, Transfer, Approve };
use solana_program::{
    sysvar,
    instruction::{AccountMeta, Instruction},
    program::{ invoke },
    account_info::AccountInfo,
    clock::Clock,
};

#[repr(u8)]
#[derive(PartialEq, Debug, Eq, Copy, Clone, TryFromPrimitive)]
pub enum SubscriptionPeriod {
    Daily,
    Weekly,
    Monthly,
    Quarterly,
    Yearly,
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

    pub fn subscribe(ctx: Context<CreateSubscr>,
        link_token: bool,
        initial_amount: u64,
        initial_tx_uuid: u128,
        inp_user_nonce: u8,
        inp_merchant_nonce: u8,
        inp_subscr_uuid: u128,
        inp_period: u8,
        inp_budget: u64,
        inp_next_rebill: i64,
        inp_pause_enabled: bool,
        inp_rebill_max: u32,
        inp_not_valid_before: i64,
        inp_not_valid_after: i64,
    ) -> ProgramResult {
        let clock = Clock::get()?;
        let ts = clock.unix_timestamp;

        if *ctx.program_id != *ctx.accounts.token_agent.to_account_info().key {
            msg!("Invalid program id");
            return Err(ErrorCode::InvalidProgramId.into());
        }

        // Verify input
        let period = SubscriptionPeriod::try_from_primitive(inp_period);
        if period.is_err() {
            msg!("Invalid subscription period");
            return Err(ErrorCode::InvalidSubscriptionPeriod.into());
        }
        let max_delay: i64 = match period.unwrap() {                // Delay from start of billing cycle to accept rebills
            SubscriptionPeriod::Daily => (60 * 60 * 48),            // 2 days
            SubscriptionPeriod::Weekly => (60 * 60 * 24 * 14),      // 2 weeks
            SubscriptionPeriod::Monthly => (60 * 60 * 24 * 60),     // ~2 months
            SubscriptionPeriod::Quarterly => (60 * 60 * 24 * 180),  // ~2 quarters
            SubscriptionPeriod::Yearly => (60 * 60 * 24 * 365 * 2), // ~2 years
        };
        if inp_not_valid_before < 0 || (inp_not_valid_before > 0 && inp_not_valid_before < ts) {
            msg!("Invalid subscription start");
            return Err(ErrorCode::InvalidTimeframe.into());
        }
        if inp_not_valid_after < 0 || (inp_not_valid_after > 0 && inp_not_valid_after < ts) {
            msg!("Invalid subscription end");
            return Err(ErrorCode::InvalidTimeframe.into());
        }
        if inp_not_valid_after != 0 && inp_not_valid_before != 0 {
            if inp_not_valid_after <= inp_not_valid_before {
                msg!("Invalid timeframe");
                return Err(ErrorCode::InvalidTimeframe.into());
            }
        }
        if inp_next_rebill < 0 {
            msg!("Invalid negative next_rebill");
            return Err(ErrorCode::InvalidTimeframe.into());
        }
        if inp_not_valid_before > 0 && inp_next_rebill < inp_not_valid_before {
            msg!("Next rebill is before start");
            return Err(ErrorCode::InvalidTimeframe.into());
        }
        let mut timeframe_start: i64 = ts;
        if inp_not_valid_before > 0 {
            timeframe_start = inp_not_valid_before;
        }
        let timeframe_end = timeframe_start.checked_add(max_delay).ok_or(ProgramError::from(ErrorCode::Overflow))?;
        if inp_next_rebill < timeframe_start || inp_next_rebill > timeframe_end {
            msg!("Next rebill not within timeframe");
            return Err(ErrorCode::InvalidTimeframe.into());
        }
        let d1 = get_period_string(inp_next_rebill, period.unwrap())?;
        let prev_period = inp_next_rebill.checked_sub(1).ok_or(ProgramError::from(ErrorCode::Overflow))?;
        let d2 = get_period_string(prev_period, period.unwrap())?;
        if d1 == d2 {
            msg!("Next rebill not beginning of period");
            return Err(ErrorCode::InvalidTimeframe.into());
        }

        // Verify user's program derived account
        let derived_user_key = Pubkey::create_program_address(
            &[
                &ctx.accounts.user_key.to_account_info().key.to_bytes(),
                &[inp_user_nonce]
            ],
            ctx.program_id
        ).map_err(|_| ErrorCode::InvalidNonce)?;
        if derived_user_key != *ctx.accounts.user_agent.to_account_info().key {
            msg!("Invalid merchant token account");
            return Err(ErrorCode::InvalidTokenAccount.into());
        }

        // Verify merchant's associated token
        let spl_token: Pubkey = Pubkey::from_str("TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA").unwrap();
        let asc_token: Pubkey = Pubkey::from_str("ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL").unwrap();
        let derived_merchant_key = Pubkey::create_program_address(
            &[
                &ctx.accounts.merchant_key.to_account_info().key.to_bytes(),
                &spl_token.to_bytes(),
                &ctx.accounts.token_mint.to_account_info().key.to_bytes(),
                &[inp_merchant_nonce]
            ],
            &asc_token
        ).map_err(|_| ErrorCode::InvalidNonce)?;
        if derived_merchant_key != *ctx.accounts.merchant_token.to_account_info().key {
            msg!("Invalid merchant token account");
            return Err(ErrorCode::InvalidTokenAccount.into());
        }

        if spl_token != *ctx.accounts.token_program.to_account_info().key {
            msg!("Invalid token program id");
            return Err(ErrorCode::InvalidProgramId.into());
        }

        // Setup up token delegate if needed
        if link_token {
            let cpi_accounts = Approve {
                to: ctx.accounts.token_account.to_account_info(),
                delegate: ctx.accounts.user_agent.to_account_info(),
                authority: ctx.accounts.user_key.to_account_info(),
            };
            let cpi_program = ctx.accounts.token_program.clone();
            let cpi_ctx = CpiContext::new(cpi_program, cpi_accounts);
            token::approve(cpi_ctx, u64::MAX)?;
        }

        // Perform transfer
        if initial_amount > 0 {
            let seeds = &[
                ctx.accounts.user_key.to_account_info().key.as_ref(),
                &[inp_user_nonce],
            ];
            let signer = &[&seeds[..]];
            let cpi_accounts = Transfer {
                from: ctx.accounts.token_account.to_account_info(),
                to: ctx.accounts.merchant_token.to_account_info(),
                authority: ctx.accounts.user_agent.to_account_info(),
            };
            let cpi_program = ctx.accounts.token_program.clone();
            let cpi_ctx = CpiContext::new_with_signer(cpi_program, cpi_accounts, signer);
            token::transfer(cpi_ctx, initial_amount)?;
        }

        // Create subscription data
        let subscr = &mut ctx.accounts.subscr_data;
        // TODO: network authority approvals
        subscr.user_key = *ctx.accounts.user_key.to_account_info().key;
        subscr.user_agent = *ctx.accounts.user_agent.to_account_info().key;
        subscr.merchant_key = *ctx.accounts.merchant_key.to_account_info().key;
        subscr.merchant_token = *ctx.accounts.merchant_token.to_account_info().key;
        subscr.manager_key = *ctx.accounts.manager_key.to_account_info().key;
        subscr.manager_approval = *ctx.accounts.manager_approval.to_account_info().key;
        subscr.token_mint = *ctx.accounts.token_mint.to_account_info().key;
        subscr.token_account = *ctx.accounts.token_account.to_account_info().key;
        subscr.rebill_events = 0;
        subscr.rebill_max = inp_rebill_max;
        subscr.next_rebill = inp_next_rebill;
        subscr.max_delay = max_delay;
        subscr.not_valid_before = inp_not_valid_before;
        subscr.not_valid_after = inp_not_valid_after;
        subscr.subscr_uuid = inp_subscr_uuid;
        subscr.rebill_uuid = 0;
        subscr.period = inp_period;
        subscr.budget = inp_budget;
        subscr.pause_enabled = inp_pause_enabled;
        subscr.paused = false;
        subscr.active = true;

        // TODO: Log event

        Ok(())
    }

    pub fn fund_token(ctx: Context<FundToken>, inp_nonce: u8) -> ProgramResult {
        // Accounts
        let av = ctx.remaining_accounts;
        let funding_account = av.get(0).unwrap();
        let token_mint = av.get(1).unwrap();
        let token_owner = av.get(2).unwrap();
        let token_account = av.get(3).unwrap();
        let token_program = av.get(4).unwrap();
        let system_program = av.get(5).unwrap();
        let system_rent = av.get(6).unwrap();

        // Verify merchant associated token
        let spl_token: Pubkey = Pubkey::from_str("TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA").unwrap();
        let asc_token: Pubkey = Pubkey::from_str("ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL").unwrap();
        let derived_key = Pubkey::create_program_address(
            &[
                &token_owner.key.to_bytes(),
                &spl_token.to_bytes(),
                &token_mint.key.to_bytes(),
                &[inp_nonce]
            ],
            &asc_token
        ).map_err(|_| ErrorCode::InvalidNonce)?;
        if derived_key != *token_account.key {
            msg!("Invalid token account");
            return Err(ErrorCode::InvalidTokenAccount.into());
        }

        if spl_token != *token_program.key {
            msg!("Invalid token program id");
            return Err(ErrorCode::InvalidProgramId.into());
        }

        if asc_token != *ctx.accounts.asc_token_account.to_account_info().key {
            msg!("Invalid associated token program id");
            return Err(ErrorCode::InvalidProgramId.into());
        }

        // Fund associated token account
        let instr = Instruction {
            program_id: asc_token,
            accounts: vec![
                AccountMeta::new(*funding_account.key, true),
                AccountMeta::new(*token_account.key, false),
                AccountMeta::new_readonly(*token_owner.key, false),
                AccountMeta::new_readonly(*token_mint.key, false),
                AccountMeta::new_readonly(solana_program::system_program::id(), false),
                AccountMeta::new_readonly(spl_token, false),
                AccountMeta::new_readonly(sysvar::rent::id(), false),
            ],
            data: vec![],
        };
        invoke(
            &instr,
            &[
                funding_account.clone(),
                token_account.clone(),
                token_owner.clone(),
                token_mint.clone(),
                system_program.clone(),
                token_program.clone(),
                system_rent.clone(),
            ]
        );
        Ok(())
    }

/*    pub fn pause() -> ProgramResult {
        Ok(())
    }

    pub fn unpause() -> ProgramResult {
        Ok(())
    }

    pub fn update_manager() -> ProgramResult {
        Ok(())
    } */

    pub fn process(ctx: Context<ProcessSubscr>,
        inp_rebill_uuid: u128,
        inp_rebill_ts: i64,
        inp_rebill_str: String,
        inp_next_rebill: i64,
        inp_amount: u64,
    ) -> ProgramResult {
        let clock = Clock::get()?;
        let ts = clock.unix_timestamp;
        msg!("Clock Timestamp: {}", ts.to_string());

        // Validate accounts
        // TODO: Ensure account owner programs
        let subscr = &mut ctx.accounts.subscr_data;
        if subscr.manager_key != *ctx.accounts.manager_key.to_account_info().key {
            msg!("Invalid account: manager_key does not match subscription");
            return Err(ErrorCode::InvalidAccount.into());
        }
        if subscr.manager_approval != *ctx.accounts.manager_approval.to_account_info().key {
            msg!("Invalid account: manager_approval does not match subscription");
            return Err(ErrorCode::InvalidAccount.into());
        }
        // TODO: Ensure manager_approval matches manager
        if !subscr.active {
            msg!("Inactive subscription");
            return Err(ErrorCode::InactiveSubscription.into());
        }
        if subscr.rebill_max > 0 && subscr.rebill_max >= subscr.rebill_events {
            msg!("Maximum rebills reached");
            return Err(ErrorCode::MaxRebills.into());
        }

        // Validate timeframe
        let period = SubscriptionPeriod::try_from_primitive(subscr.period);
        if period.is_err() {
            msg!("Invalid subscription period");
            return Err(ErrorCode::InvalidSubscriptionPeriod.into());
        }
        if subscr.not_valid_before > 0 && ts < subscr.not_valid_before {
            msg!("Subscription not valid yet");
            return Err(ErrorCode::NotValidYet.into());
        }
        if subscr.not_valid_after > 0 && ts > subscr.not_valid_after {
            msg!("Subscription expired");
            return Err(ErrorCode::SubscriptionExpired.into());
        }
        if inp_rebill_ts < 0 {
            msg!("Invalid negative rebill timestamp");
            return Err(ErrorCode::InvalidTimeframe.into());
        }
        if ts < inp_rebill_ts && false { // <=== TESTING ONLY !!! REMOVE BEFORE LAUNCH !!!{
            msg!("Invalid rebill timestamp after current time");
            return Err(ErrorCode::InvalidTimeframe.into());
        }
        if subscr.next_rebill != inp_rebill_ts {
            msg!("Rebill timestamp does not match subscription");
            return Err(ErrorCode::InvalidTimeframe.into());
        }
        let timeframe_end = inp_rebill_ts.checked_add(subscr.max_delay).ok_or(ProgramError::from(ErrorCode::Overflow))?;
        if ts > timeframe_end {
            msg!("Rebill expired");
            return Err(ErrorCode::RebillExpired.into());
        }
        let d1 = get_period_string(inp_rebill_ts, period.unwrap())?;
        if inp_rebill_str != d1 {   
            msg!("Invalid rebill period string");
            return Err(ErrorCode::InvalidTimeframe.into());
        }
        let d2 = get_period_string(inp_next_rebill, period.unwrap())?;
        let prev_period = inp_next_rebill.checked_sub(1).ok_or(ProgramError::from(ErrorCode::Overflow))?;
        let d3 = get_period_string(prev_period, period.unwrap())?;
        if d2 == d3 {
            msg!("Next rebill not beginning of period");
            return Err(ErrorCode::InvalidTimeframe.into());
        }
        if d1 != d3 {
            msg!("Next rebill out of sequence");
            return Err(ErrorCode::InvalidTimeframe.into());
        }

        msg!("Rebill ready!");

        // Update parameters
        subscr.next_rebill = inp_next_rebill;
        subscr.rebill_uuid = inp_rebill_uuid;
        subscr.rebill_events = subscr.rebill_events.checked_add(1).ok_or(ProgramError::from(ErrorCode::Overflow))?;
        Ok(())
    }

    pub fn set_allowance(ctx: Context<SetAllowance>,
        link_token: bool,
        inp_user_nonce: u8,
        inp_amount: u64,
        inp_not_valid_before: i64,
        inp_not_valid_after: i64,
    ) -> ProgramResult {
        let clock = Clock::get()?;
        let ts = clock.unix_timestamp;

        // Validate input
        if inp_not_valid_before < 0 || (inp_not_valid_before > 0 && inp_not_valid_before < ts) {
            msg!("Invalid allowance start");
            return Err(ErrorCode::InvalidTimeframe.into());
        }
        if inp_not_valid_after < 0 || (inp_not_valid_after > 0 && inp_not_valid_after < ts) {
            msg!("Invalid allowance end");
            return Err(ErrorCode::InvalidTimeframe.into());
        }
        if inp_not_valid_after != 0 && inp_not_valid_before != 0 {
            if inp_not_valid_after <= inp_not_valid_before {
                msg!("Invalid timeframe");
                return Err(ErrorCode::InvalidTimeframe.into());
            }
        }

        // Verify user's program derived account
        let derived_user_key = Pubkey::create_program_address(
            &[
                &ctx.accounts.user_key.to_account_info().key.to_bytes(),
                &[inp_user_nonce]
            ],
            ctx.program_id
        ).map_err(|_| ErrorCode::InvalidNonce)?;
        if derived_user_key != *ctx.accounts.user_agent.to_account_info().key {
            msg!("Invalid merchant token account");
            return Err(ErrorCode::InvalidTokenAccount.into());
        }

        // Setup up token delegate if needed
        if link_token {
            let cpi_accounts = Approve {
                to: ctx.accounts.token_account.to_account_info(),
                delegate: ctx.accounts.user_agent.to_account_info(),
                authority: ctx.accounts.user_key.to_account_info(),
            };
            let cpi_program = ctx.accounts.token_program.clone();
            let cpi_ctx = CpiContext::new(cpi_program, cpi_accounts);
            token::approve(cpi_ctx, u64::MAX)?;
        }

        let tka = &mut ctx.accounts.allowance_data;
        tka.user_key = *ctx.accounts.user_key.to_account_info().key;
        tka.user_agent = *ctx.accounts.user_agent.to_account_info().key;
        tka.delegate_key = *ctx.accounts.delegate_key.to_account_info().key;
        tka.token_mint = *ctx.accounts.token_mint.to_account_info().key;
        tka.token_account = *ctx.accounts.token_account.to_account_info().key;
        tka.not_valid_before = inp_not_valid_before;
        tka.not_valid_after = inp_not_valid_after;
        tka.amount = inp_amount;
        if ctx.remaining_accounts.len() > 0 {
            let pk: Pubkey = ctx.remaining_accounts.get(0).unwrap().key;
            tka.recipient_key = Some(pk);
        } else {
            tka.recipient_key = None;
        }

        Ok(())
    }

    pub fn delegated_transfer() -> ProgramResult {
        Ok(())
    }
}

#[derive(Accounts)]
pub struct CreateSubscr<'info> {
    //pub approval_program: AccountInfo<'info>,
    #[account(init)]
    pub subscr_data: ProgramAccount<'info, SubscrData>,
    pub merchant_key: AccountInfo<'info>,
    pub merchant_approval: AccountInfo<'info>,
    #[account(mut)]
    pub merchant_token: AccountInfo<'info>,
    pub manager_key: AccountInfo<'info>,
    //pub merchant_approval: ProgramAccount<'info, MerchantApproval>,
    pub manager_approval: AccountInfo<'info>,
    //pub manager_approval: ProgramAccount<'info, ManagerApproval>,
    //pub abort_authority: ProgramAccount<'info, MerchantApproval>,
    #[account(signer)]
    pub user_key: AccountInfo<'info>,
    pub user_agent: AccountInfo<'info>,
    pub token_program: AccountInfo<'info>,
    pub token_mint: AccountInfo<'info>,
    #[account(mut)]
    pub token_account: AccountInfo<'info>,
    pub token_agent: AccountInfo<'info>,
}

#[derive(Accounts)]
pub struct ProcessSubscr<'info> {
    #[account(mut)]
    pub subscr_data: ProgramAccount<'info, SubscrData>,
    #[account(signer)]
    pub manager_key: AccountInfo<'info>,
    pub manager_approval: AccountInfo<'info>,
    //pub token_agent: AccountInfo<'info>,
}

#[derive(Accounts)]
pub struct FundToken<'info> {
    pub asc_token_account: AccountInfo<'info>,
}

#[derive(Accounts)]
pub struct SetAllowance<'info> {
    #[account(init)]
    pub allowance_data: ProgramAccount<'info, TokenAllowance>,
    #[account(signer)]
    pub user_key: AccountInfo<'info>,
    pub user_agent: AccountInfo<'info>,
    pub delegate_key: AccountInfo<'info>,
    pub token_mint: AccountInfo<'info>,
    pub token_account: AccountInfo<'info>,
}

#[account]
pub struct SubscrData {
    pub user_key: Pubkey,               // The user that owns this subscription
    pub user_agent: Pubkey,             // The program derived address for token delegation
    //pub approval_program: Pubkey,       // The address of the network authority program that signs approvals
    pub merchant_key: Pubkey,           // The merchant account that receives subscription payments
    pub merchant_approval: Pubkey,      // The merchant approval record from the network authority
    pub merchant_token: Pubkey,         // The merchant associated token account to receive payments for this token
    //pub abort_authority: Pubkey,        // The abort authority from the network authority to abort in case of hacks
    pub manager_key: Pubkey,            // The rebill manager account being assigned
    pub manager_approval: Pubkey,       // The rebill manager approval from the network authority
    pub token_mint: Pubkey,             // The token mint to pay for the subscription
    pub token_account: Pubkey,          // The token account to pay for the subscription
    pub rebill_data: Pubkey,            // The rebill data account to track subscription rebills and prevent duplicates
    // Subscription details below
    pub rebill_events: u32,             // Count of rebill events
    pub rebill_max: u32,                // Maximum number of times to rebill (0 = unlimited)
    pub next_rebill: i64,               // The start of the next rebilling period (actual rebilling may happen later)
    pub not_valid_before: i64,          // UTC timestamp before which no subscription processing can occur
    pub not_valid_after: i64,           // UTC timestamp after which no subscription processing can occur
    pub max_delay: i64,                 // The number of seconds after the start of the rebill period the manager can be delayed in attempting to rebill
    pub subscr_uuid: u128,              // Subscription UUID
    pub rebill_uuid: u128,              // Last Rebill UUID
    pub period: u8,                     // Subscription rebill period
    pub budget: u64,                    // Subscription budget (maximum amount, not necessarily the amount that will be billed which could be less)
    pub pause_enabled: bool,            // Subscription able to be paused
    pub paused: bool,                   // Subscription is paused
    pub active: bool,                   // Subscription is active
}

// TODO: Merchant approval
// TODO: Rebill approval
// TODO: Abort authority

#[account]
pub struct TokenAllowance {
    //pub abort_authority: Pubkey,        // The abort authority from the network authority to abort in case of hacks
    pub user_key: Pubkey,               // The user that owns the tokens
    pub user_agent: Pubkey,             // The program derived address for delegation of the SPL token
    pub delegate_key: Pubkey,           // The delegate granted an allowance of tokens to transfer
    pub recipient_key: Option<Pubkey>,  // Optional recipient key to limit where tokens can be transferred to
    pub token_mint: Pubkey,             // The token mint for the allowance
    pub token_account: Pubkey,          // The token account for the allowance
    pub not_valid_before: i64,          // UTC timestamp before which no subscription processing can occur
    pub not_valid_after: i64,           // UTC timestamp after which no subscription processing can occur
    pub amount: u64,                    // The amount of tokens for the allowance (same decimals as underlying token)
}

#[error]
pub enum ErrorCode {
    #[msg("Access denied")]
    AccessDenied,
    #[msg("Invalid subscription period")]
    InactiveSubscription,
    #[msg("Invalid program id")]
    InvalidProgramId,
    #[msg("Invalid subscription period")]
    InvalidSubscriptionPeriod,
    #[msg("Invalid max delay")]
    InvalidMaxDelay,
    #[msg("Invalid token account")]
    InvalidTokenAccount,
    #[msg("Invalid timeframe")]
    InvalidTimeframe,
    #[msg("Invalid data type")]
    InvalidDataType,
    #[msg("Invalid account")]
    InvalidAccount,
    #[msg("Invalid nonce")]
    InvalidNonce,
    #[msg("Subscription not valid yet")]
    NotValidYet,
    #[msg("Subscription expired")]
    SubscriptionExpired,
    #[msg("Rebill expired")]
    RebillExpired,
    #[msg("Maximum rebills reached")]
    DuplicateRebill,
    #[msg("Duplicate rebill")]
    MaxRebills,
    #[msg("Overflow")]
    Overflow,
}
