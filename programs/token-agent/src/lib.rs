//use uuid::Uuid;
use std::{ io::Cursor, string::String, result::Result as FnResult, str::FromStr };
//use bytemuck::{ Pod, Zeroable };
use num_enum::TryFromPrimitive;
use chrono::{ NaiveDateTime, Datelike };
use anchor_lang::prelude::*;
use anchor_spl::token::{ self, Transfer, Approve };
use solana_program::{
    sysvar, system_instruction,
    instruction::{ AccountMeta, Instruction },
    program::{ invoke, invoke_signed },
    account_info::AccountInfo,
    clock::Clock,
};

use net_authority::{ cpi::accounts::RecordRevenue, MerchantApproval, ManagerApproval };

declare_id!("yPiRxxJKpHoZhoDZZtSVbGBJMXT8e9FyG5cCmWxzgY7");

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

fn verify_matching_accounts(left: &Pubkey, right: &Pubkey, error_msg: Option<String>) -> ProgramResult {
    if *left != *right {
        if error_msg.is_some() {
            msg!(error_msg.unwrap().as_str());
            msg!("Expected: {}", left.to_string());
            msg!("Received: {}", right.to_string());
        }
        return Err(ErrorCode::InvalidAccount.into());
    }
    Ok(())
}

#[inline]
fn store_struct<T: AccountSerialize>(obj: &T, acc: &AccountInfo) -> FnResult<(), ProgramError> {
    let mut data = acc.try_borrow_mut_data()?;
    let dst: &mut [u8] = &mut data;
    let mut crs = Cursor::new(dst);
    obj.try_serialize(&mut crs)
}

#[program]
mod token_agent {
    use super::*;

    pub fn subscribe(ctx: Context<CreateSubscr>,
        link_token: bool,
        initial_amount: u64,
        _initial_tx_uuid: u128, // TODO: THIS!
        inp_user_nonce: u8,
        inp_merchant_nonce: u8,
        inp_root_nonce: u8,
        inp_net_nonce: u8,
        inp_subscr_uuid: u128,
        inp_period: u8,
        inp_budget: u64,
        inp_next_rebill: i64,
        inp_pause_enabled: bool,
        inp_rebill_max: u32,
        inp_not_valid_before: i64,
        inp_not_valid_after: i64,
        inp_swap: bool,
    ) -> ProgramResult {
        let clock = Clock::get()?;
        let ts = clock.unix_timestamp;

        // Verify network authority accounts
        let netauth = &ctx.accounts.net_auth.to_account_info().key;
        let acc_mrch_approve = &ctx.accounts.merchant_approval.to_account_info();
        let acc_mgr_approve = &ctx.accounts.manager_approval.to_account_info();
        verify_matching_accounts(netauth, &acc_mrch_approve.owner,
            Some(String::from("Invalid merchant approval owner"))
        )?;
        verify_matching_accounts(netauth, &acc_mgr_approve.owner,
            Some(String::from("Invalid manager approval owner"))
        )?;
        let mrch_approval = &ctx.accounts.merchant_approval;
        if !mrch_approval.active {
            msg!("Inactive merchant approval");
            return Err(ErrorCode::NotApproved.into());
        }
        verify_matching_accounts(&mrch_approval.merchant_key, &ctx.accounts.merchant_key.to_account_info().key,
            Some(String::from("Merchant key does not match approval"))
        )?;
        verify_matching_accounts(&mrch_approval.token_mint, &ctx.accounts.token_mint.to_account_info().key,
            Some(String::from("Token mint does not match approval"))
        )?;
        verify_matching_accounts(&mrch_approval.fees_account, &ctx.accounts.fees_account.to_account_info().key,
            Some(String::from("Fees account does not match approval"))
        )?;
        let mgr_approval = &ctx.accounts.manager_approval;
        if !mgr_approval.active {
            msg!("Inactive manager approval");
            return Err(ErrorCode::NotApproved.into());
        }
        verify_matching_accounts(&mgr_approval.manager_key, &ctx.accounts.manager_key.to_account_info().key,
            Some(String::from("Manager key does not match approval"))
        )?;

        // Verify root key
        let acc_root_expected = Pubkey::create_program_address(&[ctx.program_id.as_ref(), &[inp_root_nonce]], ctx.program_id)
            .map_err(|_| ErrorCode::InvalidDerivedAccount)?;
        verify_matching_accounts(ctx.accounts.root_key.to_account_info().key, &acc_root_expected, Some(String::from("Invalid root key")))?;

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
            msg!("Invalid user agent account");
            return Err(ErrorCode::InvalidDerivedAccount.into());
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
            return Err(ErrorCode::InvalidDerivedAccount.into());
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
            // Swap if requested

            // Transfer tokens
            let seeds = &[
                ctx.accounts.user_key.to_account_info().key.as_ref(),
                &[inp_user_nonce],
            ];
            let signer = &[&seeds[..]];

            // Calculate fees
            let mut amount: u64 = initial_amount;
            if mrch_approval.fees_bps > 0 {
                let f1: u128 = (amount as u128) << 64;
                let f2: u128 = f1.checked_mul(mrch_approval.fees_bps as u128).ok_or(ProgramError::from(ErrorCode::Overflow))?;
                let f3: u128 = f2.checked_div(10000).ok_or(ProgramError::from(ErrorCode::Overflow))?;
                let fees: u64 = (f3 >> 64) as u64;
                if fees > 0 {
                    amount = amount.checked_sub(fees).ok_or(ProgramError::from(ErrorCode::Overflow))?;
                    let cpi_accounts = Transfer {
                        from: ctx.accounts.token_account.to_account_info(),
                        to: ctx.accounts.fees_account.to_account_info(),
                        authority: ctx.accounts.user_agent.to_account_info(),
                    };
                    let cpi_program = ctx.accounts.token_program.clone();
                    let cpi_ctx = CpiContext::new_with_signer(cpi_program, cpi_accounts, signer);
                    token::transfer(cpi_ctx, fees)?;
                }
            }
            let cpi_accounts = Transfer {
                from: ctx.accounts.token_account.to_account_info(),
                to: ctx.accounts.merchant_token.to_account_info(),
                authority: ctx.accounts.user_agent.to_account_info(),
            };
            let cpi_program = ctx.accounts.token_program.clone();
            let cpi_ctx = CpiContext::new_with_signer(cpi_program, cpi_accounts, signer);
            token::transfer(cpi_ctx, amount)?;

            // Record merchant revenue
            let seeds = &[ctx.program_id.as_ref(), &[inp_root_nonce]];
            let signer = &[&seeds[..]];
            let na_accounts = RecordRevenue {
                root_data: ctx.accounts.net_root.clone(),
                auth_data: ctx.accounts.net_rbac.clone(),
                revenue_admin: ctx.accounts.root_key.clone(),
                merchant_approval: ctx.accounts.merchant_approval.to_account_info(),
            };
            let na_program = ctx.accounts.net_auth.clone();
            let na_ctx = CpiContext::new_with_signer(na_program, na_accounts, signer);
            msg!("Atellix: Attempt to record revenue");
            net_authority::cpi::record_revenue(na_ctx, inp_net_nonce, true, amount)?;
        }

        // Create subscription data
        let mut subscr = SubscrData::default();
        subscr.user_key = *ctx.accounts.user_key.to_account_info().key;
        subscr.user_agent = *ctx.accounts.user_agent.to_account_info().key;
        subscr.approval_program = *ctx.accounts.net_auth.to_account_info().key;
        subscr.merchant_key = *ctx.accounts.merchant_key.to_account_info().key;
        subscr.merchant_token = *ctx.accounts.merchant_token.to_account_info().key;
        subscr.merchant_approval = *ctx.accounts.merchant_approval.to_account_info().key;
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
        store_struct::<SubscrData>(&subscr, &ctx.accounts.subscr_data.to_account_info())?;

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
        let sys_program = av.get(5).unwrap();
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
            return Err(ErrorCode::InvalidDerivedAccount.into());
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
                sys_program.clone(),
                token_program.clone(),
                system_rent.clone(),
            ]
        )?;
        Ok(())
    }

/*    pub fn update_manager() -> ProgramResult {
        Ok(())
    } */

    pub fn process(ctx: Context<ProcessSubscr>,
        inp_user_nonce: u8,
        inp_merchant_nonce: u8,
        inp_root_nonce: u8,
        inp_net_nonce: u8,
        inp_rebill_uuid: u128,
        inp_rebill_ts: i64,
        inp_rebill_str: String,
        inp_next_rebill: i64,
        inp_amount: u64,
    ) -> ProgramResult {
        let clock = Clock::get()?;
        let ts = clock.unix_timestamp;

        // Validate accounts
        let subscr = &mut ctx.accounts.subscr_data;
        if subscr.manager_key != *ctx.accounts.manager_key.to_account_info().key {
            msg!("Invalid account: manager_key does not match subscription");
            return Err(ErrorCode::InvalidAccount.into());
        }
        if subscr.manager_approval != *ctx.accounts.manager_approval.to_account_info().key {
            msg!("Invalid account: manager_approval does not match subscription");
            return Err(ErrorCode::InvalidAccount.into());
        }

        if !subscr.active {
            msg!("Inactive subscription");
            return Err(ErrorCode::InactiveSubscription.into());
        }
        if subscr.rebill_max > 0 && subscr.rebill_max >= subscr.rebill_events {
            msg!("Maximum rebills reached");
            return Err(ErrorCode::MaxRebills.into());
        }

        // Verify user's program derived account
        let derived_user_key = Pubkey::create_program_address(
            &[
                &subscr.user_key.to_bytes(),
                &[inp_user_nonce]
            ],
            ctx.program_id
        ).map_err(|_| ErrorCode::InvalidNonce)?;
        verify_matching_accounts(&derived_user_key, ctx.accounts.user_agent.to_account_info().key,
            Some(String::from("Invalid user agent account"))
        )?;

        // Verfiy token account and mint
        verify_matching_accounts(&subscr.token_account, &ctx.accounts.token_account.to_account_info().key,
            Some(String::from("Token account does not match subscription"))
        )?;
        verify_matching_accounts(&subscr.token_mint, &ctx.accounts.token_mint.to_account_info().key,
            Some(String::from("Token mint does not match subscription"))
        )?;

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
        verify_matching_accounts(&derived_merchant_key, ctx.accounts.merchant_token.to_account_info().key,
            Some(String::from("Invalid merchant token account"))
        )?;

        // Verify network authority accounts
        let acc_mrch_approve = &ctx.accounts.merchant_approval.to_account_info();
        let acc_mgr_approve = &ctx.accounts.manager_approval.to_account_info();
        verify_matching_accounts(&subscr.approval_program, &acc_mrch_approve.owner,
            Some(String::from("Invalid merchant approval owner"))
        )?;
        verify_matching_accounts(&subscr.approval_program, &acc_mgr_approve.owner,
            Some(String::from("Invalid manager approval owner"))
        )?;
        let mrch_approval = &ctx.accounts.merchant_approval;
        if !mrch_approval.active {
            msg!("Inactive merchant approval");
            return Err(ErrorCode::NotApproved.into());
        }
        verify_matching_accounts(&mrch_approval.merchant_key, &ctx.accounts.merchant_key.to_account_info().key,
            Some(String::from("Merchant key does not match approval"))
        )?;
        verify_matching_accounts(&mrch_approval.token_mint, &ctx.accounts.token_mint.to_account_info().key,
            Some(String::from("Token mint does not match approval"))
        )?;
        verify_matching_accounts(&mrch_approval.fees_account, &ctx.accounts.fees_account.to_account_info().key,
            Some(String::from("Fees account does not match approval"))
        )?;
        let mgr_approval = &ctx.accounts.manager_approval;
        if !mgr_approval.active {
            msg!("Inactive manager approval");
            return Err(ErrorCode::NotApproved.into());
        }
        verify_matching_accounts(&mgr_approval.manager_key, &ctx.accounts.manager_key.to_account_info().key,
            Some(String::from("Manager key does not match approval"))
        )?;

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
            return Err(ErrorCode::Expired.into());
        }
        if inp_rebill_ts < 0 {
            msg!("Invalid negative rebill timestamp");
            return Err(ErrorCode::InvalidTimeframe.into());
        }
        if ts < inp_rebill_ts && false { // <=== TESTING ONLY !!! REMOVE BEFORE LAUNCH !!!
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
            return Err(ErrorCode::Expired.into());
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
        if inp_amount > subscr.budget {
            msg!("Amount exceeds budget");
            return Err(ErrorCode::BudgetExceeded.into());
        }

        msg!("Process rebill");

        if inp_amount > 0 {
            let seeds = &[
                subscr.user_key.as_ref(),
                &[inp_user_nonce],
            ];
            let signer = &[&seeds[..]];

            // Calculate fees
            let mut amount: u64 = inp_amount;
            if mrch_approval.fees_bps > 0 {
                let f1: u128 = (amount as u128) << 64;
                let f2: u128 = f1.checked_mul(mrch_approval.fees_bps as u128).ok_or(ProgramError::from(ErrorCode::Overflow))?;
                let f3: u128 = f2.checked_div(10000).ok_or(ProgramError::from(ErrorCode::Overflow))?;
                let fees: u64 = (f3 >> 64) as u64;
                if fees > 0 {
                    amount = amount.checked_sub(fees).ok_or(ProgramError::from(ErrorCode::Overflow))?;
                    let cpi_accounts = Transfer {
                        from: ctx.accounts.token_account.to_account_info(),
                        to: ctx.accounts.fees_account.to_account_info(),
                        authority: ctx.accounts.user_agent.to_account_info(),
                    };
                    let cpi_program = ctx.accounts.token_program.clone();
                    let cpi_ctx = CpiContext::new_with_signer(cpi_program, cpi_accounts, signer);
                    token::transfer(cpi_ctx, fees)?;
                }
                msg!("Starting Amount: {} Ending Amount: {} Fees: {}", inp_amount.to_string(), amount.to_string(), fees.to_string());
            }
            let cpi_accounts = Transfer {
                from: ctx.accounts.token_account.to_account_info(),
                to: ctx.accounts.merchant_token.to_account_info(),
                authority: ctx.accounts.user_agent.to_account_info(),
            };
            let cpi_program = ctx.accounts.token_program.clone();
            let cpi_ctx = CpiContext::new_with_signer(cpi_program, cpi_accounts, signer);
            token::transfer(cpi_ctx, amount)?;

            // Record merchant revenue
            let seeds = &[ctx.program_id.as_ref(), &[inp_root_nonce]];
            let signer = &[&seeds[..]];
            let na_accounts = RecordRevenue {
                root_data: ctx.accounts.net_root.clone(),
                auth_data: ctx.accounts.net_rbac.clone(),
                revenue_admin: ctx.accounts.root_key.clone(),
                merchant_approval: ctx.accounts.merchant_approval.to_account_info(),
            };
            let na_program = ctx.accounts.net_auth.clone();
            let na_ctx = CpiContext::new_with_signer(na_program, na_accounts, signer);
            msg!("Atellix: Attempt to record revenue");
            net_authority::cpi::record_revenue(na_ctx, inp_net_nonce, true, amount)?;
        }

        // Update parameters
        subscr.next_rebill = inp_next_rebill;
        subscr.rebill_uuid = inp_rebill_uuid;
        subscr.rebill_events = subscr.rebill_events.checked_add(1).ok_or(ProgramError::from(ErrorCode::Overflow))?;
        Ok(())
    }

    pub fn create_allowance(ctx: Context<CreateAllowance>,
        link_token: bool,
        inp_user_nonce: u8,
        inp_allowance_nonce: u8,
        inp_data_size: u64,
        inp_data_rent: u64,
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

        // Verfiy allowance program derived address
        let spl_token: Pubkey = Pubkey::from_str("TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA").unwrap();
        let funder_key = ctx.remaining_accounts.get(0).unwrap();
        let allowance_data = ctx.remaining_accounts.get(1).unwrap();
        let sys_program = ctx.remaining_accounts.get(2).unwrap();
        let derived_allowance_key = Pubkey::create_program_address(
            &[
                &ctx.accounts.user_key.to_account_info().key.to_bytes(),
                &spl_token.to_bytes(),
                &ctx.accounts.token_mint.to_account_info().key.to_bytes(),
                &ctx.accounts.token_account.to_account_info().key.to_bytes(),
                &ctx.accounts.delegate_key.to_account_info().key.to_bytes(),
                &[inp_allowance_nonce]
            ],
            ctx.program_id
        ).map_err(|_| ErrorCode::InvalidNonce)?;
        if derived_allowance_key != *allowance_data.key {
            msg!("Invalid allowance account");
            return Err(ErrorCode::InvalidDerivedAccount.into());
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
            msg!("Invalid allowance account");
            return Err(ErrorCode::InvalidDerivedAccount.into());
        }

        let account_signer_seeds: &[&[_]] = &[
            &ctx.accounts.user_key.to_account_info().key.to_bytes(),
            &spl_token.to_bytes(),
            &ctx.accounts.token_mint.to_account_info().key.to_bytes(),
            &ctx.accounts.token_account.to_account_info().key.to_bytes(),
            &ctx.accounts.delegate_key.to_account_info().key.to_bytes(),
            &[inp_allowance_nonce],
        ];
        msg!("Create allowance account");
        invoke_signed(
            &system_instruction::create_account(
                funder_key.key,
                allowance_data.key,
                inp_data_rent,
                inp_data_size,
                ctx.program_id
            ),
            &[
                funder_key.clone(),
                allowance_data.clone(),
                sys_program.clone(),
            ],
            &[account_signer_seeds],
        )?;

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

        let mut tka = TokenAllowance {
            user_key: *ctx.accounts.user_key.to_account_info().key,
            user_agent: *ctx.accounts.user_agent.to_account_info().key,
            delegate_key: *ctx.accounts.delegate_key.to_account_info().key,
            token_mint: *ctx.accounts.token_mint.to_account_info().key,
            token_account: *ctx.accounts.token_account.to_account_info().key,
            recipient_key: None,
            not_valid_before: inp_not_valid_before,
            not_valid_after: inp_not_valid_after,
            amount: inp_amount,
        };
        if ctx.remaining_accounts.len() > 3 {
            let pk: Pubkey = *ctx.remaining_accounts.get(3).unwrap().key;
            tka.recipient_key = Some(pk);
        }

        let mut approval_data = &mut allowance_data.try_borrow_mut_data()?;
        let approval_dst: &mut [u8] = &mut approval_data;
        let mut approval_crs = std::io::Cursor::new(approval_dst);
        tka.try_serialize(&mut approval_crs)?;

        Ok(())
    }

    pub fn update_allowance(ctx: Context<UpdateAllowance>,
        link_token: bool,
        inp_user_nonce: u8,
        inp_allowance_nonce: u8,
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

        // Verfiy allowance program derived address
        let spl_token: Pubkey = Pubkey::from_str("TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA").unwrap();
        let derived_allowance_key = Pubkey::create_program_address(
            &[
                &ctx.accounts.user_key.to_account_info().key.to_bytes(),
                &spl_token.to_bytes(),
                &ctx.accounts.token_mint.to_account_info().key.to_bytes(),
                &ctx.accounts.token_account.to_account_info().key.to_bytes(),
                &ctx.accounts.delegate_key.to_account_info().key.to_bytes(),
                &[inp_allowance_nonce]
            ],
            ctx.program_id
        ).map_err(|_| ErrorCode::InvalidNonce)?;
        if derived_allowance_key != *ctx.accounts.allowance_data.to_account_info().key {
            msg!("Invalid allowance account");
            return Err(ErrorCode::InvalidDerivedAccount.into());
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
            msg!("Invalid allowance account");
            return Err(ErrorCode::InvalidDerivedAccount.into());
        }

        // Verify accounts match
        let ald = &mut ctx.accounts.allowance_data;
        if ald.user_key != *ctx.accounts.user_key.to_account_info().key {
            msg!("Invalid user account");
            return Err(ErrorCode::InvalidAccount.into());
        }
        if ald.token_account != *ctx.accounts.token_account.to_account_info().key {
            msg!("Invalid token account");
            return Err(ErrorCode::InvalidAccount.into());
        }
        if ald.token_mint != *ctx.accounts.token_mint.to_account_info().key {
            msg!("Invalid token mint account");
            return Err(ErrorCode::InvalidAccount.into());
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
        tka.not_valid_before = inp_not_valid_before;
        tka.not_valid_after = inp_not_valid_after;
        tka.amount = inp_amount;
        if ctx.remaining_accounts.len() > 0 {
            let pk: Pubkey = *ctx.remaining_accounts.get(0).unwrap().key;
            tka.recipient_key = Some(pk);
        } else {
            tka.recipient_key = None;
        }

        Ok(())
    }

    pub fn delegated_transfer(ctx: Context<DelegatedTransfer>,
        inp_user_nonce: u8,
        inp_allowance_nonce: u8,
        inp_amount: u64,
    ) -> ProgramResult {
        // Verfiy allowance program derived address
        let spl_token: Pubkey = Pubkey::from_str("TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA").unwrap();
        let derived_allowance_key = Pubkey::create_program_address(
            &[
                &ctx.accounts.user_key.to_account_info().key.to_bytes(),
                &spl_token.to_bytes(),
                &ctx.accounts.token_mint.to_account_info().key.to_bytes(),
                &ctx.accounts.user_token.to_account_info().key.to_bytes(),
                &ctx.accounts.delegate_key.to_account_info().key.to_bytes(),
                &[inp_allowance_nonce]
            ],
            ctx.program_id
        ).map_err(|_| ErrorCode::InvalidNonce)?;
        if derived_allowance_key != *ctx.accounts.allowance_data.to_account_info().key {
            msg!("Invalid allowance account");
            return Err(ErrorCode::InvalidDerivedAccount.into());
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
            msg!("Invalid allowance account");
            return Err(ErrorCode::InvalidDerivedAccount.into());
        }

        // Verify accounts match
        let ald = &mut ctx.accounts.allowance_data;
        if ald.user_key != *ctx.accounts.user_key.to_account_info().key {
            msg!("Invalid user account");
            return Err(ErrorCode::InvalidAccount.into());
        }
        if ald.user_agent != *ctx.accounts.user_agent.to_account_info().key {
            msg!("Invalid user agent account");
            return Err(ErrorCode::InvalidAccount.into());
        }
        if ald.delegate_key != *ctx.accounts.delegate_key.to_account_info().key {
            let right: Pubkey = *ctx.accounts.delegate_key.to_account_info().key;
            msg!("Invalid delegate account");
            msg!("Expected: {}", ald.delegate_key.to_string());
            msg!("Received: {}", right.to_string());
            return Err(ErrorCode::InvalidAccount.into());
        }
        if ald.token_mint != *ctx.accounts.token_mint.to_account_info().key {
            msg!("Invalid token mint account");
            return Err(ErrorCode::InvalidAccount.into());
        }
        if ald.token_account != *ctx.accounts.user_token.to_account_info().key {
            msg!("Invalid user token account");
            return Err(ErrorCode::InvalidAccount.into());
        }
        // TODO: check recipient_key option

        // Validate timeframe
        if ald.not_valid_before > 0 || ald.not_valid_after > 0 {
            let clock = Clock::get()?;
            let ts = clock.unix_timestamp;
            if ald.not_valid_before > 0 && ts < ald.not_valid_before {
                msg!("Allowance not valid yet");
                return Err(ErrorCode::NotValidYet.into());
            }
            if ald.not_valid_after > 0 && ts > ald.not_valid_after {
                msg!("Allowance expired");
                return Err(ErrorCode::Expired.into());
            }
        }

        if inp_amount > 0 {
            msg!("Transfer amount: {}", inp_amount.to_string());
            msg!("Begin: {}", ald.amount.to_string());
            let diff = ald.amount.checked_sub(inp_amount);
            if diff.is_some() {
                // Perform transfer
                ald.amount = diff.unwrap();
                msg!("End: {}", ald.amount.to_string());
                let seeds = &[
                    ctx.accounts.user_key.to_account_info().key.as_ref(),
                    &[inp_user_nonce],
                ];
                let signer = &[&seeds[..]];
                let cpi_accounts = Transfer {
                    from: ctx.accounts.user_token.to_account_info(),
                    to: ctx.accounts.token_recipient.to_account_info(),
                    authority: ctx.accounts.user_agent.to_account_info(),
                };
                let cpi_program = ctx.accounts.token_program.clone();
                let cpi_ctx = CpiContext::new_with_signer(cpi_program, cpi_accounts, signer);
                token::transfer(cpi_ctx, inp_amount)?;
            } else {
                msg!("Amount exceeds allowance");
                return Err(ErrorCode::AllowanceExceeded.into());
            }
        }
        Ok(())
    }
}

#[derive(Accounts)]
pub struct CreateSubscr<'info> {
    #[account(mut)]
    pub subscr_data: AccountInfo<'info>,
    pub net_auth: AccountInfo<'info>,
    pub net_rbac: AccountInfo<'info>,
    pub net_root: AccountInfo<'info>,
    pub root_key: AccountInfo<'info>,
    pub merchant_key: AccountInfo<'info>,
    #[account(mut)]
    pub merchant_approval: Account<'info, MerchantApproval>,
    #[account(mut)]
    pub merchant_token: AccountInfo<'info>,
    pub manager_key: AccountInfo<'info>,
    pub manager_approval: Account<'info, ManagerApproval>,
    #[account(signer)]
    pub user_key: AccountInfo<'info>,
    pub user_agent: AccountInfo<'info>,
    pub token_program: AccountInfo<'info>,
    pub token_mint: AccountInfo<'info>,
    #[account(mut)]
    pub token_account: AccountInfo<'info>,
    #[account(mut)]
    pub fees_account: AccountInfo<'info>,
}

#[derive(Accounts)]
pub struct ProcessSubscr<'info> {
    #[account(mut)]
    pub subscr_data: ProgramAccount<'info, SubscrData>,
    pub net_auth: AccountInfo<'info>,
    pub net_rbac: AccountInfo<'info>,
    pub net_root: AccountInfo<'info>,
    pub root_key: AccountInfo<'info>,
    pub merchant_key: AccountInfo<'info>,
    #[account(mut)]
    pub merchant_approval: Account<'info, MerchantApproval>,
    #[account(mut)]
    pub merchant_token: AccountInfo<'info>,
    #[account(signer)]
    pub manager_key: AccountInfo<'info>,
    pub manager_approval: Account<'info, ManagerApproval>,
    pub user_agent: AccountInfo<'info>,
    pub token_program: AccountInfo<'info>,
    pub token_mint: AccountInfo<'info>,
    #[account(mut)]
    pub token_account: AccountInfo<'info>,
    #[account(mut)]
    pub fees_account: AccountInfo<'info>,
}

#[derive(Accounts)]
pub struct FundToken<'info> {
    pub asc_token_account: AccountInfo<'info>,
}

#[derive(Accounts)]
pub struct CreateAllowance<'info> {
    #[account(signer)]
    pub user_key: AccountInfo<'info>,
    pub user_agent: AccountInfo<'info>,
    pub delegate_key: AccountInfo<'info>,
    pub token_mint: AccountInfo<'info>,
    #[account(mut)]
    pub token_account: AccountInfo<'info>,
    pub token_program: AccountInfo<'info>,
}

#[derive(Accounts)]
pub struct UpdateAllowance<'info> {
    #[account(mut)]
    pub allowance_data: ProgramAccount<'info, TokenAllowance>,
    #[account(signer)]
    pub user_key: AccountInfo<'info>,
    pub user_agent: AccountInfo<'info>,
    pub delegate_key: AccountInfo<'info>,
    #[account(mut)]
    pub token_account: AccountInfo<'info>,
    pub token_mint: AccountInfo<'info>,
    pub token_program: AccountInfo<'info>,
}

#[derive(Accounts)]
pub struct DelegatedTransfer<'info> {
    #[account(mut)]
    pub allowance_data: ProgramAccount<'info, TokenAllowance>,
    #[account(signer)]
    pub delegate_key: AccountInfo<'info>,
    pub user_key: AccountInfo<'info>,
    pub user_agent: AccountInfo<'info>,
    #[account(mut)]
    pub user_token: AccountInfo<'info>,
    pub token_mint: AccountInfo<'info>,
    #[account(mut)]
    pub token_recipient: AccountInfo<'info>,
    pub token_program: AccountInfo<'info>,
}

#[account]
pub struct SubscrData {
    pub user_key: Pubkey,               // The user that owns this subscription
    pub user_agent: Pubkey,             // The program derived address for token delegation
    pub approval_program: Pubkey,       // The address of the network authority program that signs approvals
    pub merchant_key: Pubkey,           // The merchant account that receives subscription payments
    pub merchant_approval: Pubkey,      // The merchant approval record from the network authority
    pub merchant_token: Pubkey,         // The merchant associated token account to receive payments for this token
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

impl Default for SubscrData {
    fn default() -> Self {
        Self {
            user_key: Pubkey::default(),
            user_agent: Pubkey::default(),
            approval_program: Pubkey::default(),
            merchant_key: Pubkey::default(),
            merchant_approval: Pubkey::default(),
            merchant_token: Pubkey::default(),
            manager_key: Pubkey::default(),
            manager_approval: Pubkey::default(),
            token_mint: Pubkey::default(),
            token_account: Pubkey::default(),
            rebill_data: Pubkey::default(),
            rebill_events: 0,
            rebill_max: 0,
            next_rebill: 0,
            not_valid_before: 0,
            not_valid_after: 0,
            max_delay: 0,
            subscr_uuid: 0,
            rebill_uuid: 0,
            period: 0,
            budget: 0,
            pause_enabled: false,
            paused: false,
            active: false,
        }
    }
}

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
    #[msg("Invalid subscription period")]
    InactiveSubscription,
    #[msg("Invalid program id")]
    InvalidProgramId,
    #[msg("Invalid subscription period")]
    InvalidSubscriptionPeriod,
    #[msg("Invalid max delay")]
    InvalidMaxDelay,
    #[msg("Invalid derived account")]
    InvalidDerivedAccount,
    #[msg("Invalid timeframe")]
    InvalidTimeframe,
    #[msg("Invalid data type")]
    InvalidDataType,
    #[msg("Invalid account")]
    InvalidAccount,
    #[msg("Invalid nonce")]
    InvalidNonce,
    #[msg("Not approved")]
    NotApproved,
    #[msg("Budget exceeded")]
    BudgetExceeded,
    #[msg("Allowance exceeded")]
    AllowanceExceeded,
    #[msg("Subscription not valid yet")]
    NotValidYet,
    #[msg("Expired")]
    Expired,
    #[msg("Maximum rebills reached")]
    DuplicateRebill,
    #[msg("Duplicate rebill")]
    MaxRebills,
    #[msg("Overflow")]
    Overflow,
}
