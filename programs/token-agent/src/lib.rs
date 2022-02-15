use crate::program::TokenAgent;
use std::{ io::Cursor, string::String, result::Result as FnResult };
use arrayref::array_ref;
use num_enum::TryFromPrimitive;
use chrono::{ NaiveDateTime, Datelike };
use anchor_lang::prelude::*;
use anchor_spl::token::{ self, Token, Transfer, Approve };
use anchor_spl::associated_token::{ AssociatedToken };
use solana_program::{ account_info::AccountInfo, clock::Clock };

use net_authority::{ self, cpi::accounts::RecordRevenue, MerchantApproval, ManagerApproval };
use swap_contract::{ cpi::accounts::Swap };

declare_id!("AGNTo5SLwpnyi5Yz9YFP7Qd3jGpW2ZTMbM6xrBWyfBrv");

pub const VERSION_MAJOR: u32 = 1;
pub const VERSION_MINOR: u32 = 0;
pub const VERSION_PATCH: u32 = 0;

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
fn load_struct<T: AccountDeserialize>(acc: &AccountInfo) -> FnResult<T, ProgramError> {
    let mut data: &[u8] = &acc.try_borrow_data()?;
    Ok(T::try_deserialize(&mut data)?)
}

#[inline]
fn store_struct<T: AccountSerialize>(obj: &T, acc: &AccountInfo) -> FnResult<(), ProgramError> {
    let mut data = acc.try_borrow_mut_data()?;
    let disc_bytes = array_ref![data, 0, 8];
    if disc_bytes != &[0; 8] {
        msg!("Account already initialized");
        return Err(ErrorCode::InvalidAccount.into());
    }
    let dst: &mut [u8] = &mut data;
    let mut crs = Cursor::new(dst);
    obj.try_serialize(&mut crs)
}

#[program]
mod token_agent {
    use super::*;

    pub fn store_metadata(ctx: Context<UpdateMetadata>,
        inp_program_name: String,
        inp_developer_name: String,
        inp_developer_url: String,
        inp_source_url: String,
        inp_verify_url: String,
    ) -> ProgramResult {
        let md = &mut ctx.accounts.program_info;
        md.semvar_major = VERSION_MAJOR;
        md.semvar_minor = VERSION_MINOR;
        md.semvar_patch = VERSION_PATCH;
        md.program = ctx.accounts.program.key();
        md.program_name = inp_program_name;
        md.developer_name = inp_developer_name;
        md.developer_url = inp_developer_url;
        md.source_url = inp_source_url;
        md.verify_url = inp_verify_url;
        msg!("Program: {}", ctx.accounts.program.key.to_string());
        msg!("Program Name: {}", md.program_name.as_str());
        msg!("Version: {}.{}.{}", VERSION_MAJOR.to_string(), VERSION_MINOR.to_string(), VERSION_PATCH.to_string());
        msg!("Developer Name: {}", md.developer_name.as_str());
        msg!("Developer URL: {}", md.developer_url.as_str());
        msg!("Source URL: {}", md.source_url.as_str());
        msg!("Verify URL: {}", md.verify_url.as_str());
        Ok(())
    }

    pub fn subscribe<'info>(ctx: Context<'_, '_, '_, 'info, CreateSubscr<'info>>,
        inp_link_token: bool,
        inp_initial_amount: u64,
        inp_user_nonce: u8,
        inp_merchant_nonce: u8,
        inp_root_nonce: u8,
        inp_net_nonce: u8,
        inp_subscr_id: u128,
        inp_payment_id: u128,
        inp_period: u8,
        inp_period_budget: u64,
        inp_use_total: bool,
        inp_total_budget: u64,
        inp_next_rebill: i64,
        inp_rebill_max: u32,
        inp_not_valid_before: i64,
        inp_not_valid_after: i64,
        inp_max_delay: i64,
        inp_swap: bool,
        inp_swap_direction: bool,
        inp_swap_root_nonce: u8,
        inp_swap_inb_nonce: u8,
        inp_swap_out_nonce: u8,
        inp_swap_dst_nonce: u8,
    ) -> ProgramResult {
        let clock = Clock::get()?;
        let ts = clock.unix_timestamp;

        // Verify network authority
        let netauth = &ctx.accounts.net_auth.to_account_info().key;
        let acc_mrch_approve = &ctx.accounts.merchant_approval.to_account_info();
        let acc_mgr_approve = &ctx.accounts.manager_approval.to_account_info();
        verify_matching_accounts(netauth, &acc_mrch_approve.owner,
            Some(String::from("Invalid merchant approval owner"))
        )?;
        verify_matching_accounts(netauth, &acc_mgr_approve.owner,
            Some(String::from("Invalid manager approval owner"))
        )?;

        // Verify network authority accounts
        let mut mrch_approval = load_struct::<MerchantApproval>(&ctx.accounts.merchant_approval.to_account_info())?;
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
        let mgr_approval = load_struct::<ManagerApproval>(&ctx.accounts.manager_approval.to_account_info())?;
        if !mgr_approval.active {
            msg!("Inactive manager approval");
            return Err(ErrorCode::NotApproved.into());
        }
        verify_matching_accounts(&mgr_approval.manager_key, &ctx.accounts.manager_key.to_account_info().key,
            Some(String::from("Manager key does not match approval"))
        )?;

        // Verify input
        let period = SubscriptionPeriod::try_from_primitive(inp_period);
        if period.is_err() {
            msg!("Invalid subscription period");
            return Err(ErrorCode::InvalidSubscriptionPeriod.into());
        }
        let mut max_delay: i64 = match period.unwrap() {                // Delay from start of billing cycle to accept rebills
            SubscriptionPeriod::Daily => (60 * 60 * 24 * 90),       // 3 months
            SubscriptionPeriod::Weekly => (60 * 60 * 24 * 90),      // 3 months
            SubscriptionPeriod::Monthly => (60 * 60 * 24 * 365),    // 1 year
            SubscriptionPeriod::Quarterly => (60 * 60 * 24 * 365),  // 1 year
            SubscriptionPeriod::Yearly => (60 * 60 * 24 * 365 * 2), // 2 years
        };
        if inp_max_delay != 0 {
            if inp_max_delay < 43200 { // 12 hours
                msg!("Invalid max_delay below minimum of 12 hours (43200 seconds)");
                return Err(ErrorCode::InvalidTimeframe.into());
            }
            max_delay = inp_max_delay;
        }

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

        let timeframe_start: i64 = ts;
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
            &[&ctx.accounts.user_key.to_account_info().key.to_bytes(), &[inp_user_nonce]], ctx.program_id
        ).map_err(|_| ErrorCode::InvalidNonce)?;
        verify_matching_accounts(&derived_user_key, &ctx.accounts.user_agent.to_account_info().key,
            Some(String::from("Invalid user agent account"))
        )?;

        // Verify merchant's associated token
        let derived_merchant_key = Pubkey::create_program_address(
            &[
                &ctx.accounts.merchant_key.to_account_info().key.to_bytes(),
                &Token::id().to_bytes(),
                &ctx.accounts.token_mint.to_account_info().key.to_bytes(),
                &[inp_merchant_nonce]
            ],
            &AssociatedToken::id()
        ).map_err(|_| ErrorCode::InvalidNonce)?;
        if derived_merchant_key != *ctx.accounts.merchant_token.to_account_info().key {
            msg!("Invalid merchant token account");
            return Err(ErrorCode::InvalidDerivedAccount.into());
        }

        // Setup up token delegate if needed
        if !inp_swap && inp_link_token {
            let cpi_accounts = Approve {
                to: ctx.accounts.token_account.to_account_info(),
                delegate: ctx.accounts.user_agent.to_account_info(),
                authority: ctx.accounts.user_key.to_account_info(),
            };
            let cpi_program = ctx.accounts.token_program.to_account_info();
            let cpi_ctx = CpiContext::new(cpi_program, cpi_accounts);
            token::approve(cpi_ctx, u64::MAX)?;
        }

        // Perform transfer
        let mut swap_account: Pubkey = Pubkey::default();
        let mut net_amount: u64 = inp_initial_amount;
        let mut fee_amount: u64 = 0;
        if inp_initial_amount > 0 {
            // Swap if requested
            if inp_swap {
                let acc_swap_token = ctx.remaining_accounts.get(0).unwrap();        // User Swap Token

                // Setup up token delegate if needed
                if inp_link_token {
                    let cpi_accounts = Approve {
                        to: acc_swap_token.clone(),
                        delegate: ctx.accounts.user_agent.to_account_info(),
                        authority: ctx.accounts.user_key.to_account_info(),
                    };
                    let cpi_program = ctx.accounts.token_program.to_account_info();
                    let cpi_ctx = CpiContext::new(cpi_program, cpi_accounts);
                    token::approve(cpi_ctx, u64::MAX)?;
                }

                // Verify token agent's swap destination associated token
                let derived_swap_key = Pubkey::create_program_address(
                    &[
                        &ctx.accounts.root_key.to_account_info().key.to_bytes(),
                        &Token::id().to_bytes(),
                        &ctx.accounts.token_mint.to_account_info().key.to_bytes(),
                        &[inp_swap_dst_nonce]
                    ],
                    &AssociatedToken::id()
                ).map_err(|_| ErrorCode::InvalidNonce)?;
                if derived_swap_key != *ctx.accounts.token_account.to_account_info().key {
                    msg!("Invalid swap destination token account");
                    return Err(ErrorCode::InvalidDerivedAccount.into());
                }

                //msg!("Atellix: Attempt swap");
                swap_account = acc_swap_token.key();
                let sw_program = ctx.remaining_accounts.get(1).unwrap().clone();
                let sw_accounts = Swap {
                    root_data: ctx.remaining_accounts.get(2).unwrap().clone(),
                    auth_data: ctx.remaining_accounts.get(3).unwrap().clone(),
                    swap_user: ctx.accounts.user_key.to_account_info(),
                    swap_data: ctx.remaining_accounts.get(4).unwrap().clone(),
                    inb_token_src: acc_swap_token.clone(),
                    inb_token_dst: ctx.remaining_accounts.get(5).unwrap().clone(),
                    out_token_src: ctx.remaining_accounts.get(6).unwrap().clone(),
                    out_token_dst: ctx.accounts.token_account.to_account_info(),    // Token agent swap destination
                    fees_token: ctx.remaining_accounts.get(7).unwrap().clone(),
                    token_program: ctx.accounts.token_program.to_account_info(),
                };
                let mut sw_ctx = CpiContext::new(sw_program, sw_accounts);
                if ctx.remaining_accounts.len() > 8 { // Oracle Data Account (if needed)
                    sw_ctx = sw_ctx.with_remaining_accounts(vec![ctx.remaining_accounts.get(8).unwrap().clone()]);
                }
                swap_contract::cpi::swap(sw_ctx, inp_swap_root_nonce, inp_swap_inb_nonce, inp_swap_out_nonce, true, false, inp_swap_direction, inp_initial_amount)?;
            }

            // Transfer tokens
            let user_pda_seeds = &[ctx.accounts.user_key.to_account_info().key.as_ref(), &[inp_user_nonce]];
            let user_pda_signer = &[&user_pda_seeds[..]];
            let root_pda_seeds = &[ctx.program_id.as_ref(), &[inp_root_nonce]];
            let root_pda_signer = &[&root_pda_seeds[..]];
            let mut signer = user_pda_signer;
            let mut token_auth = ctx.accounts.user_agent.to_account_info();
            if inp_swap {
                signer = root_pda_signer;
                token_auth = ctx.accounts.root_key.to_account_info();
            }

            // Calculate fees
            if mrch_approval.fees_bps > 0 {
                let f1: u128 = (net_amount as u128) << 64;
                let f2: u128 = f1.checked_mul(mrch_approval.fees_bps as u128).ok_or(ProgramError::from(ErrorCode::Overflow))?;
                let f3: u128 = f2.checked_div(10000).ok_or(ProgramError::from(ErrorCode::Overflow))?;
                let fees: u64 = (f3 >> 64) as u64;
                if fees > 0 {
                    net_amount = net_amount.checked_sub(fees).ok_or(ProgramError::from(ErrorCode::Overflow))?;
                    fee_amount = fees;
                    let cpi_accounts = Transfer {
                        from: ctx.accounts.token_account.to_account_info(),
                        to: ctx.accounts.fees_account.to_account_info(),
                        authority: token_auth.clone(),
                    };
                    let cpi_program = ctx.accounts.token_program.to_account_info();
                    let cpi_ctx = CpiContext::new_with_signer(cpi_program, cpi_accounts, signer);
                    token::transfer(cpi_ctx, fees)?;
                }
            }

            let cpi_accounts = Transfer {
                from: ctx.accounts.token_account.to_account_info(),
                to: ctx.accounts.merchant_token.to_account_info(),
                authority: token_auth.clone(),
            };
            let cpi_program = ctx.accounts.token_program.to_account_info();
            let cpi_ctx = CpiContext::new_with_signer(cpi_program, cpi_accounts, signer);
            token::transfer(cpi_ctx, net_amount)?;

            // Record merchant revenue
            let na_program = ctx.accounts.net_auth.to_account_info();
            if *na_program.key == net_authority::ID {
                let na_accounts = RecordRevenue {
                    root_data: ctx.accounts.net_root.to_account_info(),
                    auth_data: ctx.accounts.net_rbac.to_account_info(),
                    revenue_admin: ctx.accounts.root_key.to_account_info(),
                    merchant_approval: ctx.accounts.merchant_approval.to_account_info(),
                };
                let na_ctx = CpiContext::new_with_signer(na_program, na_accounts, root_pda_signer);
                //msg!("Atellix: Attempt to record revenue");
                net_authority::cpi::record_revenue(na_ctx, inp_net_nonce, true, net_amount)?;
                mrch_approval = load_struct::<MerchantApproval>(&ctx.accounts.merchant_approval.to_account_info())?;
            }
        }

        // Create subscription data
        let mut subscr = SubscrData::default();
        subscr.user_key = *ctx.accounts.user_key.to_account_info().key;
        subscr.approval_program = *ctx.accounts.net_auth.to_account_info().key;
        subscr.merchant_key = *ctx.accounts.merchant_key.to_account_info().key;
        subscr.merchant_approval = *ctx.accounts.merchant_approval.to_account_info().key;
        subscr.manager_key = *ctx.accounts.manager_key.to_account_info().key;
        subscr.manager_approval = *ctx.accounts.manager_approval.to_account_info().key;
        subscr.token_mint = *ctx.accounts.token_mint.to_account_info().key;
        subscr.token_account = *ctx.accounts.token_account.to_account_info().key;
        subscr.swap_account = swap_account;
        subscr.subscr_id = inp_subscr_id;
        subscr.rebill_max = inp_rebill_max;
        subscr.next_rebill = inp_next_rebill;
        subscr.max_delay = max_delay;
        subscr.not_valid_before = inp_not_valid_before;
        subscr.not_valid_after = inp_not_valid_after;
        subscr.period = inp_period;
        subscr.period_budget = inp_period_budget;
        subscr.use_total = inp_use_total;
        subscr.total_budget = inp_total_budget;
        subscr.swap = inp_swap;
        subscr.swap_direction = inp_swap_direction;
        store_struct::<SubscrData>(&subscr, &ctx.accounts.subscr_data.to_account_info())?;

        msg!("atellix-log");
        emit!(SubscrEvent {
            event_hash: 176440469768111763486207729736362869784, // solana/program/token-agent/subscribe
            slot: clock.slot,
            merchant_tx_id: mrch_approval.tx_count,
            subscr_data: ctx.accounts.subscr_data.key(),
            subscr_id: inp_subscr_id,
            payment_id: inp_payment_id,
            rebill_event: 0,
            total: inp_initial_amount,
            amount: net_amount,
            fees: fee_amount,
            next_rebill: inp_next_rebill,
            swap: inp_swap,
        });

        Ok(())
    }

    pub fn update_subscription<'info>(ctx: Context<'_, '_, '_, 'info, UpdateSubscr<'info>>,
        inp_active: bool,
        inp_link_token: bool,
        inp_amount: u64,
        inp_payment_id: u128,
        inp_user_nonce: u8,
        inp_merchant_nonce: u8,
        inp_root_nonce: u8,
        inp_net_nonce: u8,
        inp_period: u8,
        inp_period_budget: u64,
        inp_use_total: bool,
        inp_total_budget: u64,
        inp_next_rebill: i64,
        inp_rebill_max: u32,
        inp_not_valid_before: i64,
        inp_not_valid_after: i64,
        inp_max_delay: i64,
        inp_swap: bool,
        inp_swap_direction: bool,
        inp_swap_root_nonce: u8,
        inp_swap_inb_nonce: u8,
        inp_swap_out_nonce: u8,
        inp_swap_dst_nonce: u8,
    ) -> ProgramResult {
        let clock = Clock::get()?;
        let ts = clock.unix_timestamp;
        let subscr = &mut ctx.accounts.subscr_data;
        // Verify user key is the same
        verify_matching_accounts(&subscr.user_key, &ctx.accounts.user_key.to_account_info().key,
            Some(String::from("User key does not match"))
        )?;

        // Deactivate if requested by user
        if !inp_active {
            subscr.active = false;
            msg!("atellix-log");
            emit!(SubscrEvent {
                event_hash: 163361025719893016519135760137561968517, // solana/program/token-agent/update_subscription/cancel
                slot: clock.slot,
                merchant_tx_id: 0,
                subscr_data: subscr.key(),
                subscr_id: subscr.subscr_id,
                payment_id: 0,
                rebill_event: 0,
                total: 0,
                amount: 0,
                fees: 0,
                next_rebill: -1,
                swap: subscr.swap,
            });
            return Ok(());
        }

        // Verify network authority is the same
        let netauth = &ctx.accounts.net_auth.to_account_info().key;
        verify_matching_accounts(netauth, &subscr.approval_program,
            Some(String::from("Approval program does not match"))
        )?;

        // Verify user's program derived account
        let derived_user_key = Pubkey::create_program_address(
            &[&ctx.accounts.user_key.to_account_info().key.to_bytes(), &[inp_user_nonce]], ctx.program_id
        ).map_err(|_| ErrorCode::InvalidNonce)?;
        verify_matching_accounts(&derived_user_key, &ctx.accounts.user_agent.to_account_info().key,
            Some(String::from("Invalid user agent account"))
        )?;

        // Verify network authority accounts
        let acc_mrch_approve = &ctx.accounts.merchant_approval.to_account_info();
        let acc_mgr_approve = &ctx.accounts.manager_approval.to_account_info();
        verify_matching_accounts(netauth, &acc_mrch_approve.owner,
            Some(String::from("Invalid merchant approval owner"))
        )?;
        verify_matching_accounts(netauth, &acc_mgr_approve.owner,
            Some(String::from("Invalid manager approval owner"))
        )?;
        let mut mrch_approval = load_struct::<MerchantApproval>(&ctx.accounts.merchant_approval.to_account_info())?;
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
        let mgr_approval = load_struct::<ManagerApproval>(&ctx.accounts.manager_approval.to_account_info())?;
        if !mgr_approval.active {
            msg!("Inactive manager approval");
            return Err(ErrorCode::NotApproved.into());
        }
        verify_matching_accounts(&mgr_approval.manager_key, &ctx.accounts.manager_key.to_account_info().key,
            Some(String::from("Manager key does not match approval"))
        )?;

        // Verify input
        let period = SubscriptionPeriod::try_from_primitive(inp_period);
        if period.is_err() {
            msg!("Invalid subscription period");
            return Err(ErrorCode::InvalidSubscriptionPeriod.into());
        }
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
        if inp_max_delay < 43200 { // 12 hours
            msg!("Invalid max_delay below minimum of 12 hours (43200 seconds)");
            return Err(ErrorCode::InvalidTimeframe.into());
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
        let timeframe_end = timeframe_start.checked_add(inp_max_delay).ok_or(ProgramError::from(ErrorCode::Overflow))?;
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

        // Verify merchant's associated token
        let derived_merchant_key = Pubkey::create_program_address(
            &[
                &ctx.accounts.merchant_key.to_account_info().key.to_bytes(),
                &Token::id().to_bytes(),
                &ctx.accounts.token_mint.to_account_info().key.to_bytes(),
                &[inp_merchant_nonce]
            ],
            &AssociatedToken::id()
        ).map_err(|_| ErrorCode::InvalidNonce)?;
        if derived_merchant_key != *ctx.accounts.merchant_token.to_account_info().key {
            msg!("Invalid merchant token account");
            return Err(ErrorCode::InvalidDerivedAccount.into());
        }

        // Setup up token delegate if needed
        if !inp_swap && inp_link_token {
            let cpi_accounts = Approve {
                to: ctx.accounts.token_account.to_account_info(),
                delegate: ctx.accounts.user_agent.to_account_info(),
                authority: ctx.accounts.user_key.to_account_info(),
            };
            let cpi_program = ctx.accounts.token_program.to_account_info();
            let cpi_ctx = CpiContext::new(cpi_program, cpi_accounts);
            token::approve(cpi_ctx, u64::MAX)?;
        }

        // Link swap token if requested
        let mut swap_account: Pubkey = Pubkey::default();
        if inp_swap {
            let acc_swap_token = ctx.remaining_accounts.get(0).unwrap();        // User Swap Token
            swap_account = acc_swap_token.key();

            // Setup up token delegate if needed
            if inp_link_token {
                let cpi_accounts = Approve {
                    to: acc_swap_token.clone(),
                    delegate: ctx.accounts.user_agent.to_account_info(),
                    authority: ctx.accounts.user_key.to_account_info(),
                };
                let cpi_program = ctx.accounts.token_program.to_account_info();
                let cpi_ctx = CpiContext::new(cpi_program, cpi_accounts);
                token::approve(cpi_ctx, u64::MAX)?;
            }
        }

        let mut net_amount: u64 = inp_amount;
        let mut fee_amount: u64 = 0;
        if inp_amount > 0 {
            // Swap if requested
            if inp_swap {
                //msg!("Atellix: Attempt swap");
                // Verify token agent's swap destination associated token
                let derived_swap_key = Pubkey::create_program_address(
                    &[
                        &ctx.accounts.root_key.to_account_info().key.to_bytes(),
                        &Token::id().to_bytes(),
                        &ctx.accounts.token_mint.to_account_info().key.to_bytes(),
                        &[inp_swap_dst_nonce]
                    ],
                    &AssociatedToken::id()
                ).map_err(|_| ErrorCode::InvalidNonce)?;
                if derived_swap_key != *ctx.accounts.token_account.to_account_info().key {
                    msg!("Invalid swap destination token account");
                    return Err(ErrorCode::InvalidDerivedAccount.into());
                }
                let acc_swap_token = ctx.remaining_accounts.get(0).unwrap();        // User Swap Token
                let sw_program = ctx.remaining_accounts.get(1).unwrap().clone();
                let sw_accounts = Swap {
                    root_data: ctx.remaining_accounts.get(2).unwrap().clone(),
                    auth_data: ctx.remaining_accounts.get(3).unwrap().clone(),
                    swap_user: ctx.accounts.user_key.to_account_info(),
                    swap_data: ctx.remaining_accounts.get(4).unwrap().clone(),
                    inb_token_src: acc_swap_token.clone(),
                    inb_token_dst: ctx.remaining_accounts.get(5).unwrap().clone(),
                    out_token_src: ctx.remaining_accounts.get(6).unwrap().clone(),
                    out_token_dst: ctx.accounts.token_account.to_account_info(),    // Token Agent PDA
                    fees_token: ctx.remaining_accounts.get(7).unwrap().clone(),
                    token_program: ctx.accounts.token_program.to_account_info(),
                };
                let mut sw_ctx = CpiContext::new(sw_program, sw_accounts);
                if ctx.remaining_accounts.len() > 8 { // Oracle Data Account (if needed)
                    sw_ctx = sw_ctx.with_remaining_accounts(vec![ctx.remaining_accounts.get(8).unwrap().clone()]);
                }
                swap_contract::cpi::swap(sw_ctx, inp_swap_root_nonce, inp_swap_inb_nonce, inp_swap_out_nonce, true, false, inp_swap_direction, inp_amount)?;
            }

            let user_pda_seeds = &[subscr.user_key.as_ref(), &[inp_user_nonce]];
            let user_pda_signer = &[&user_pda_seeds[..]];
            let root_pda_seeds = &[ctx.program_id.as_ref(), &[inp_root_nonce]];
            let root_pda_signer = &[&root_pda_seeds[..]];
            let mut signer = user_pda_signer;
            let mut token_auth = ctx.accounts.user_agent.to_account_info();
            if inp_swap {
                signer = root_pda_signer;
                token_auth = ctx.accounts.root_key.to_account_info();
            }

            // Calculate fees
            if mrch_approval.fees_bps > 0 {
                let f1: u128 = (net_amount as u128) << 64;
                let f2: u128 = f1.checked_mul(mrch_approval.fees_bps as u128).ok_or(ProgramError::from(ErrorCode::Overflow))?;
                let f3: u128 = f2.checked_div(10000).ok_or(ProgramError::from(ErrorCode::Overflow))?;
                let fees: u64 = (f3 >> 64) as u64;
                if fees > 0 {
                    net_amount = net_amount.checked_sub(fees).ok_or(ProgramError::from(ErrorCode::Overflow))?;
                    fee_amount = fees;
                    let cpi_accounts = Transfer {
                        from: ctx.accounts.token_account.to_account_info(),
                        to: ctx.accounts.fees_account.to_account_info(),
                        authority: token_auth.clone(),
                    };
                    let cpi_program = ctx.accounts.token_program.to_account_info();
                    let cpi_ctx = CpiContext::new_with_signer(cpi_program, cpi_accounts, signer);
                    token::transfer(cpi_ctx, fees)?;
                }
                //msg!("Starting Amount: {} Ending Amount: {} Fees: {}", inp_amount.to_string(), amount.to_string(), fees.to_string());
            }
            let cpi_accounts = Transfer {
                from: ctx.accounts.token_account.to_account_info(),
                to: ctx.accounts.merchant_token.to_account_info(),
                authority: token_auth.clone(),
            };
            let cpi_program = ctx.accounts.token_program.to_account_info();
            let cpi_ctx = CpiContext::new_with_signer(cpi_program, cpi_accounts, signer);
            token::transfer(cpi_ctx, net_amount)?;

            // Record merchant revenue
            let na_program = ctx.accounts.net_auth.to_account_info();
            if *na_program.key == net_authority::ID {
                let na_accounts = RecordRevenue {
                    root_data: ctx.accounts.net_root.to_account_info(),
                    auth_data: ctx.accounts.net_rbac.to_account_info(),
                    revenue_admin: ctx.accounts.root_key.to_account_info(),
                    merchant_approval: ctx.accounts.merchant_approval.to_account_info(),
                };
                let na_ctx = CpiContext::new_with_signer(na_program, na_accounts, root_pda_signer);
                //msg!("Atellix: Attempt to record revenue");
                net_authority::cpi::record_revenue(na_ctx, inp_net_nonce, true, net_amount)?;
                mrch_approval = load_struct::<MerchantApproval>(&ctx.accounts.merchant_approval.to_account_info())?;
            }
        }

        // Update subscription data
        subscr.active = true;
        subscr.merchant_key = *ctx.accounts.merchant_key.to_account_info().key;
        subscr.merchant_approval = *ctx.accounts.merchant_approval.to_account_info().key;
        subscr.manager_key = *ctx.accounts.manager_key.to_account_info().key;
        subscr.manager_approval = *ctx.accounts.manager_approval.to_account_info().key;
        subscr.token_mint = *ctx.accounts.token_mint.to_account_info().key;
        subscr.token_account = *ctx.accounts.token_account.to_account_info().key;
        subscr.swap_account = swap_account;
        subscr.rebill_max = inp_rebill_max;
        subscr.next_rebill = inp_next_rebill;
        subscr.max_delay = inp_max_delay;
        subscr.not_valid_before = inp_not_valid_before;
        subscr.not_valid_after = inp_not_valid_after;
        subscr.period = inp_period;
        subscr.period_budget = inp_period_budget;
        subscr.use_total = inp_use_total;
        subscr.total_budget = inp_total_budget;
        subscr.swap = inp_swap;
        subscr.swap_direction = inp_swap_direction;

        msg!("atellix-log");
        emit!(SubscrEvent {
            event_hash: 298296161986799263364555576740275705662, // solana/program/token-agent/update_subscription
            slot: clock.slot,
            merchant_tx_id: mrch_approval.tx_count,
            subscr_data: subscr.key(),
            subscr_id: subscr.subscr_id,
            payment_id: inp_payment_id,
            rebill_event: 0,
            total: inp_amount,
            amount: net_amount,
            fees: fee_amount,
            next_rebill: inp_next_rebill,
            swap: inp_swap,
        });

        Ok(())
    }

    pub fn close_subscription(ctx: Context<CloseSubscr>) -> ProgramResult {
        let subscr = &ctx.accounts.subscr_data;
        verify_matching_accounts(&subscr.user_key, ctx.accounts.user_key.to_account_info().key,
            Some(String::from("User key does not match subscription"))
        )?;

        msg!("Closed Subscription: {}", ctx.accounts.subscr_data.to_account_info().key.to_string());
        Ok(())
    }

    pub fn update_manager<'info>(ctx: Context<'_, '_, '_, 'info, UpdateManager<'info>>) -> ProgramResult {
        let subscr = &mut ctx.accounts.subscr_data;
        verify_matching_accounts(&subscr.manager_key, &ctx.accounts.manager_prev.to_account_info().key,
            Some(String::from("Previous manager does not match subscription"))
        )?;
        let mgr_approval = load_struct::<ManagerApproval>(&ctx.accounts.manager_approval.to_account_info())?;
        if !mgr_approval.active {
            msg!("Inactive manager approval");
            return Err(ErrorCode::NotApproved.into());
        }
        verify_matching_accounts(&mgr_approval.manager_key, &ctx.accounts.manager_key.to_account_info().key,
            Some(String::from("Manager key does not match approval"))
        )?;

        subscr.manager_key = *ctx.accounts.manager_key.to_account_info().key;
        subscr.manager_approval = *ctx.accounts.manager_approval.to_account_info().key;
        Ok(())
    }

    pub fn manager_cancel<'info>(ctx: Context<'_, '_, '_, 'info, ManagerCancel<'info>>) -> ProgramResult {
        let clock = Clock::get()?;

        let subscr = &mut ctx.accounts.subscr_data;
        verify_matching_accounts(&subscr.manager_key, &ctx.accounts.manager_key.to_account_info().key,
            Some(String::from("Manager key does not match subscription"))
        )?;
        let mgr_approval = load_struct::<ManagerApproval>(&ctx.accounts.manager_approval.to_account_info())?;
        if !mgr_approval.active {
            msg!("Inactive manager approval");
            return Err(ErrorCode::NotApproved.into());
        }
        verify_matching_accounts(&mgr_approval.manager_key, &ctx.accounts.manager_key.to_account_info().key,
            Some(String::from("Manager key does not match approval"))
        )?;

        subscr.active = false;

        msg!("atellix-log");
        emit!(SubscrEvent {
            event_hash: 14511983483732720963723889670203659368, // solana/program/token-agent/manager_cancel
            slot: clock.slot,
            merchant_tx_id: 0,
            subscr_data: subscr.key(),
            subscr_id: subscr.subscr_id,
            payment_id: 0,
            rebill_event: 0,
            total: 0,
            amount: 0,
            fees: 0,
            next_rebill: -1,
            swap: subscr.swap,
        });

        Ok(())
    }

    pub fn process<'info>(ctx: Context<'_, '_, '_, 'info, ProcessSubscr<'info>>,
        inp_user_nonce: u8,
        inp_merchant_nonce: u8,
        inp_root_nonce: u8,
        inp_net_nonce: u8,
        inp_rebill_ts: i64,
        inp_rebill_str: String,
        inp_next_rebill: i64,
        inp_amount: u64,
        inp_payment_id: u128,
        inp_swap_root_nonce: u8,
        inp_swap_inb_nonce: u8,
        inp_swap_out_nonce: u8,
    ) -> ProgramResult {
        let clock = Clock::get()?;
        let ts = clock.unix_timestamp;

        // Validate accounts
        let subscr = &mut ctx.accounts.subscr_data;
        verify_matching_accounts(&subscr.merchant_key, ctx.accounts.merchant_key.to_account_info().key,
            Some(String::from("Manager key does not match subscription"))
        )?;
        verify_matching_accounts(&subscr.manager_approval, ctx.accounts.manager_approval.to_account_info().key,
            Some(String::from("Manager approval does not match subscription"))
        )?;
        verify_matching_accounts(&subscr.merchant_approval, ctx.accounts.merchant_approval.to_account_info().key,
            Some(String::from("Merchant approval does not match subscription"))
        )?;

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
            &[&subscr.user_key.to_bytes(), &[inp_user_nonce]], ctx.program_id
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
        let derived_merchant_key = Pubkey::create_program_address(
            &[
                &ctx.accounts.merchant_key.to_account_info().key.to_bytes(),
                &Token::id().to_bytes(),
                &ctx.accounts.token_mint.to_account_info().key.to_bytes(),
                &[inp_merchant_nonce]
            ],
            &AssociatedToken::id()
        ).map_err(|_| ErrorCode::InvalidNonce)?;
        verify_matching_accounts(&derived_merchant_key, ctx.accounts.merchant_token.to_account_info().key,
            Some(String::from("Invalid merchant token account"))
        )?;

        // Verify network authority accounts
        let mut mrch_approval = load_struct::<MerchantApproval>(&ctx.accounts.merchant_approval.to_account_info())?;
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
        let mgr_approval = load_struct::<ManagerApproval>(&ctx.accounts.manager_approval.to_account_info())?;
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
        if ts < inp_rebill_ts && false { // TODO: REMOVE THIS BEFORE LAUNCH!!!
            msg!("Attempted rebill before scheduled time");
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

        //msg!("Atellix: Process rebill");

        let mut net_amount: u64 = inp_amount;
        let mut fee_amount: u64 = 0;
        if inp_amount > 0 {
            if inp_amount > subscr.period_budget {
                msg!("Amount exceeds budget");
                return Err(ErrorCode::PeriodBudgetExceeded.into());
            }
            if subscr.use_total {
                if inp_amount > subscr.total_budget {
                    msg!("Amount exceeds total budget");
                    return Err(ErrorCode::TotalBudgetExceeded.into());
                }
                subscr.total_budget = subscr.total_budget.checked_sub(inp_amount).ok_or(ProgramError::from(ErrorCode::Overflow))?;
            }
            // Swap if requested
            let user_pda_seeds = &[subscr.user_key.as_ref(), &[inp_user_nonce]];
            let user_pda_signer = &[&user_pda_seeds[..]];
            if subscr.swap {
                //msg!("Atellix: Attempt swap");
                let acc_swap_token = ctx.remaining_accounts.get(0).unwrap();        // User Swap Token
                verify_matching_accounts(&subscr.swap_account, &acc_swap_token.key(),
                    Some(String::from("Swap token does not match subscription"))
                )?;
                let sw_program = ctx.remaining_accounts.get(1).unwrap().clone();
                let sw_accounts = Swap {
                    root_data: ctx.remaining_accounts.get(2).unwrap().clone(),
                    auth_data: ctx.remaining_accounts.get(3).unwrap().clone(),
                    swap_user: ctx.accounts.user_agent.to_account_info(),          // User Agent (signer)
                    swap_data: ctx.remaining_accounts.get(4).unwrap().clone(),
                    inb_token_src: acc_swap_token.clone(),
                    inb_token_dst: ctx.remaining_accounts.get(5).unwrap().clone(),
                    out_token_src: ctx.remaining_accounts.get(6).unwrap().clone(),
                    out_token_dst: ctx.accounts.token_account.to_account_info(),    // Token Agent PDA
                    fees_token: ctx.remaining_accounts.get(7).unwrap().clone(),
                    token_program: ctx.accounts.token_program.to_account_info(),
                };
                let mut sw_ctx = CpiContext::new_with_signer(sw_program, sw_accounts, user_pda_signer);
                if ctx.remaining_accounts.len() > 8 { // Oracle Data Account (if needed)
                    sw_ctx = sw_ctx.with_remaining_accounts(vec![ctx.remaining_accounts.get(8).unwrap().clone()]);
                }
                swap_contract::cpi::swap(sw_ctx, inp_swap_root_nonce, inp_swap_inb_nonce, inp_swap_out_nonce, true, false, subscr.swap_direction, inp_amount)?;
            }
            let root_pda_seeds = &[ctx.program_id.as_ref(), &[inp_root_nonce]];
            let root_pda_signer = &[&root_pda_seeds[..]];
            let mut signer = user_pda_signer;
            let mut token_auth = ctx.accounts.user_agent.to_account_info();
            if subscr.swap {
                signer = root_pda_signer;
                token_auth = ctx.accounts.root_key.to_account_info();
            }

            // Calculate fees
            if mrch_approval.fees_bps > 0 {
                let f1: u128 = (net_amount as u128) << 64;
                let f2: u128 = f1.checked_mul(mrch_approval.fees_bps as u128).ok_or(ProgramError::from(ErrorCode::Overflow))?;
                let f3: u128 = f2.checked_div(10000).ok_or(ProgramError::from(ErrorCode::Overflow))?;
                let fees: u64 = (f3 >> 64) as u64;
                if fees > 0 {
                    net_amount = net_amount.checked_sub(fees).ok_or(ProgramError::from(ErrorCode::Overflow))?;
                    fee_amount = fees;
                    let cpi_accounts = Transfer {
                        from: ctx.accounts.token_account.to_account_info(),
                        to: ctx.accounts.fees_account.to_account_info(),
                        authority: token_auth.clone(),
                    };
                    let cpi_program = ctx.accounts.token_program.to_account_info();
                    let cpi_ctx = CpiContext::new_with_signer(cpi_program, cpi_accounts, signer);
                    token::transfer(cpi_ctx, fees)?;
                }
                //msg!("Starting Amount: {} Ending Amount: {} Fees: {}", inp_amount.to_string(), amount.to_string(), fees.to_string());
            }
            let cpi_accounts = Transfer {
                from: ctx.accounts.token_account.to_account_info(),
                to: ctx.accounts.merchant_token.to_account_info(),
                authority: token_auth.clone(),
            };
            let cpi_program = ctx.accounts.token_program.to_account_info();
            let cpi_ctx = CpiContext::new_with_signer(cpi_program, cpi_accounts, signer);
            token::transfer(cpi_ctx, net_amount)?;

            // Record merchant revenue
            let na_program = ctx.accounts.net_auth.to_account_info();
            if *na_program.key == net_authority::ID {
                let na_accounts = RecordRevenue {
                    root_data: ctx.accounts.net_root.to_account_info(),
                    auth_data: ctx.accounts.net_rbac.to_account_info(),
                    revenue_admin: ctx.accounts.root_key.to_account_info(),
                    merchant_approval: ctx.accounts.merchant_approval.to_account_info(),
                };
                let na_ctx = CpiContext::new_with_signer(na_program, na_accounts, root_pda_signer);
                //msg!("Atellix: Attempt to record revenue");
                net_authority::cpi::record_revenue(na_ctx, inp_net_nonce, true, net_amount)?;
                mrch_approval = load_struct::<MerchantApproval>(&ctx.accounts.merchant_approval.to_account_info())?
            }
        }

        // Update parameters
        subscr.next_rebill = inp_next_rebill;
        subscr.rebill_events = subscr.rebill_events.checked_add(1).ok_or(ProgramError::from(ErrorCode::Overflow))?;

        msg!("atellix-log");
        emit!(SubscrEvent {
            event_hash: 196800858676461937700417377973077375575, // solana/program/token-agent/process
            slot: clock.slot,
            merchant_tx_id: mrch_approval.tx_count,
            subscr_data: subscr.key(),
            subscr_id: subscr.subscr_id,
            payment_id: inp_payment_id,
            rebill_event: subscr.rebill_events,
            total: inp_amount,
            amount: net_amount,
            fees: fee_amount,
            next_rebill: inp_next_rebill,
            swap: subscr.swap,
        });

        Ok(())
    }

    pub fn merchant_payment<'info>(ctx: Context<'_, '_, '_, 'info, MerchantPayment<'info>>,
        inp_merchant_nonce: u8,
        inp_root_nonce: u8,
        inp_net_nonce: u8,
        inp_payment_id: u128,
        inp_amount: u64,
        inp_swap: bool,
        inp_swap_direction: bool,
        inp_swap_root_nonce: u8,
        inp_swap_inb_nonce: u8,
        inp_swap_out_nonce: u8,
        inp_swap_dst_nonce: u8,
    ) -> ProgramResult {
        let clock = Clock::get()?;

        // Verify merchant's associated token
        let derived_merchant_key = Pubkey::create_program_address(
            &[
                &ctx.accounts.merchant_key.to_account_info().key.to_bytes(),
                &Token::id().to_bytes(),
                &ctx.accounts.token_mint.to_account_info().key.to_bytes(),
                &[inp_merchant_nonce]
            ],
            &AssociatedToken::id()
        ).map_err(|_| ErrorCode::InvalidNonce)?;
        verify_matching_accounts(&derived_merchant_key, ctx.accounts.merchant_token.to_account_info().key,
            Some(String::from("Invalid merchant token account"))
        )?;

        let netauth = &ctx.accounts.net_auth.to_account_info().key;
        let acc_mrch_approve = &ctx.accounts.merchant_approval.to_account_info();
        verify_matching_accounts(netauth, &acc_mrch_approve.owner,
            Some(String::from("Invalid merchant approval owner"))
        )?;

        let mut mrch_approval = load_struct::<MerchantApproval>(&ctx.accounts.merchant_approval.to_account_info())?;
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

        let mut net_amount: u64 = inp_amount;
        let mut fee_amount: u64 = 0;
        if inp_amount > 0 {
            // Swap if requested
            if inp_swap {
                //msg!("Atellix: Attempt swap");
                // Verify token agent's swap destination associated token
                let derived_swap_key = Pubkey::create_program_address(
                    &[
                        &ctx.accounts.root_key.to_account_info().key.to_bytes(),
                        &Token::id().to_bytes(),
                        &ctx.accounts.token_mint.to_account_info().key.to_bytes(),
                        &[inp_swap_dst_nonce]
                    ],
                    &AssociatedToken::id()
                ).map_err(|_| ErrorCode::InvalidNonce)?;
                if derived_swap_key != *ctx.accounts.token_account.to_account_info().key {
                    msg!("Invalid swap destination token account");
                    return Err(ErrorCode::InvalidDerivedAccount.into());
                }
                let acc_swap_token = ctx.remaining_accounts.get(0).unwrap();        // User Swap Token
                let sw_program = ctx.remaining_accounts.get(1).unwrap().clone();
                let sw_accounts = Swap {
                    root_data: ctx.remaining_accounts.get(2).unwrap().clone(),
                    auth_data: ctx.remaining_accounts.get(3).unwrap().clone(),
                    swap_user: ctx.accounts.user_key.to_account_info(),
                    swap_data: ctx.remaining_accounts.get(4).unwrap().clone(),
                    inb_token_src: acc_swap_token.clone(),
                    inb_token_dst: ctx.remaining_accounts.get(5).unwrap().clone(),
                    out_token_src: ctx.remaining_accounts.get(6).unwrap().clone(),
                    out_token_dst: ctx.accounts.token_account.to_account_info(),    // Token Agent PDA
                    fees_token: ctx.remaining_accounts.get(7).unwrap().clone(),
                    token_program: ctx.accounts.token_program.to_account_info(),
                };
                let mut sw_ctx = CpiContext::new(sw_program, sw_accounts);
                if ctx.remaining_accounts.len() > 8 { // Oracle Data Account (if needed)
                    sw_ctx = sw_ctx.with_remaining_accounts(vec![ctx.remaining_accounts.get(8).unwrap().clone()]);
                }
                swap_contract::cpi::swap(sw_ctx, inp_swap_root_nonce, inp_swap_inb_nonce, inp_swap_out_nonce, true, false, inp_swap_direction, inp_amount)?;
            }

            // Transfer tokens
            let root_pda_seeds = &[ctx.program_id.as_ref(), &[inp_root_nonce]];
            let root_pda_signer = &[&root_pda_seeds[..]];
            let mut token_auth = ctx.accounts.user_key.to_account_info();
            if inp_swap {
                token_auth = ctx.accounts.root_key.to_account_info();
            }

            // Calculate fees
            if mrch_approval.fees_bps > 0 {
                let f1: u128 = (net_amount as u128) << 64;
                let f2: u128 = f1.checked_mul(mrch_approval.fees_bps as u128).ok_or(ProgramError::from(ErrorCode::Overflow))?;
                let f3: u128 = f2.checked_div(10000).ok_or(ProgramError::from(ErrorCode::Overflow))?;
                let fees: u64 = (f3 >> 64) as u64;
                if fees > 0 {
                    net_amount = net_amount.checked_sub(fees).ok_or(ProgramError::from(ErrorCode::Overflow))?;
                    fee_amount = fees;
                    let cpi_accounts = Transfer {
                        from: ctx.accounts.token_account.to_account_info(),
                        to: ctx.accounts.fees_account.to_account_info(),
                        authority: token_auth.clone(),
                    };
                    let cpi_program = ctx.accounts.token_program.to_account_info();
                    let cpi_ctx;
                    if inp_swap {
                        cpi_ctx = CpiContext::new_with_signer(cpi_program, cpi_accounts, root_pda_signer);
                    } else {
                        cpi_ctx = CpiContext::new(cpi_program, cpi_accounts);
                    }
                    token::transfer(cpi_ctx, fees)?;
                }
                //msg!("Atellix: Starting Amount: {} Ending Amount: {} Fees: {}", inp_amount.to_string(), amount.to_string(), fees.to_string());
            }
            let cpi_accounts = Transfer {
                from: ctx.accounts.token_account.to_account_info(),
                to: ctx.accounts.merchant_token.to_account_info(),
                authority: token_auth.clone(),
            };
            let cpi_program = ctx.accounts.token_program.to_account_info();
            let cpi_ctx;
            if inp_swap {
                cpi_ctx = CpiContext::new_with_signer(cpi_program, cpi_accounts, root_pda_signer);
            } else {
                cpi_ctx = CpiContext::new(cpi_program, cpi_accounts);
            }
            token::transfer(cpi_ctx, net_amount)?;

            // Record merchant revenue
            let na_program = ctx.accounts.net_auth.to_account_info();
            if *na_program.key == net_authority::ID {
                let na_accounts = RecordRevenue {
                    root_data: ctx.accounts.net_root.to_account_info(),
                    auth_data: ctx.accounts.net_rbac.to_account_info(),
                    revenue_admin: ctx.accounts.root_key.to_account_info(),
                    merchant_approval: ctx.accounts.merchant_approval.to_account_info(),
                };
                let na_ctx = CpiContext::new_with_signer(na_program, na_accounts, root_pda_signer);
                //msg!("Atellix: Attempt to record revenue");
                net_authority::cpi::record_revenue(na_ctx, inp_net_nonce, true, net_amount)?;
                mrch_approval = load_struct::<MerchantApproval>(&ctx.accounts.merchant_approval.to_account_info())?;
            }
        }

        msg!("atellix-log");
        emit!(PaymentEvent {
            event_hash: 43781034894216267743388154650854733336, // solana/program/token-agent/merchant_payment
            slot: clock.slot,
            merchant_tx_id: mrch_approval.tx_count,
            merchant_key: *ctx.accounts.merchant_key.to_account_info().key,
            user_key: *ctx.accounts.user_key.to_account_info().key,
            total: inp_amount,
            amount: net_amount,
            fees: fee_amount,
            payment_id: inp_payment_id,
            swap: inp_swap,
        });

        Ok(())
    }

    pub fn merchant_receive<'info>(ctx: Context<'_, '_, '_, 'info, MerchantReceive<'info>>,
        inp_merchant_nonce: u8,
        inp_root_nonce: u8,
        inp_net_nonce: u8,
        inp_payment_id: u128,
        inp_amount: u64,
        inp_swap: bool,
        inp_swap_direction: bool,
        inp_swap_root_nonce: u8,
        inp_swap_inb_nonce: u8,
        inp_swap_out_nonce: u8,
        inp_swap_dst_nonce: u8,
    ) -> ProgramResult {
        let clock = Clock::get()?;

        // Verify merchant's associated token
        let derived_merchant_key = Pubkey::create_program_address(
            &[
                &ctx.accounts.merchant_key.to_account_info().key.to_bytes(),
                &Token::id().to_bytes(),
                &ctx.accounts.token_mint.to_account_info().key.to_bytes(),
                &[inp_merchant_nonce]
            ],
            &AssociatedToken::id()
        ).map_err(|_| ErrorCode::InvalidNonce)?;
        verify_matching_accounts(&derived_merchant_key, ctx.accounts.merchant_token.to_account_info().key,
            Some(String::from("Invalid merchant token account"))
        )?;

        let netauth = &ctx.accounts.net_auth.to_account_info().key;
        let acc_mrch_approve = &ctx.accounts.merchant_approval.to_account_info();
        verify_matching_accounts(netauth, &acc_mrch_approve.owner,
            Some(String::from("Invalid merchant approval owner"))
        )?;

        let mut mrch_approval = load_struct::<MerchantApproval>(&ctx.accounts.merchant_approval.to_account_info())?;
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

        let mut net_amount: u64 = inp_amount;
        let mut fee_amount: u64 = 0;
        if inp_amount > 0 {
            // Swap if requested
            if inp_swap {
                //msg!("Atellix: Attempt swap");
                // Verify token agent's swap destination associated token
                let derived_swap_key = Pubkey::create_program_address(
                    &[
                        &ctx.accounts.root_key.to_account_info().key.to_bytes(),
                        &Token::id().to_bytes(),
                        &ctx.accounts.token_mint.to_account_info().key.to_bytes(),
                        &[inp_swap_dst_nonce]
                    ],
                    &AssociatedToken::id()
                ).map_err(|_| ErrorCode::InvalidNonce)?;
                if derived_swap_key != *ctx.accounts.token_account.to_account_info().key {
                    msg!("Invalid swap destination token account");
                    return Err(ErrorCode::InvalidDerivedAccount.into());
                }
                let acc_swap_token = ctx.remaining_accounts.get(0).unwrap();        // User Swap Token
                let sw_program = ctx.remaining_accounts.get(1).unwrap().clone();
                let sw_accounts = Swap {
                    root_data: ctx.remaining_accounts.get(2).unwrap().clone(),
                    auth_data: ctx.remaining_accounts.get(3).unwrap().clone(),
                    swap_user: ctx.accounts.merchant_approval.to_account_info(),
                    swap_data: ctx.remaining_accounts.get(4).unwrap().clone(),
                    inb_token_src: acc_swap_token.clone(),
                    inb_token_dst: ctx.remaining_accounts.get(5).unwrap().clone(),
                    out_token_src: ctx.remaining_accounts.get(6).unwrap().clone(),
                    out_token_dst: ctx.accounts.token_account.to_account_info(),    // Token Agent PDA
                    fees_token: ctx.remaining_accounts.get(7).unwrap().clone(),
                    token_program: ctx.accounts.token_program.to_account_info(),
                };
                let mut sw_ctx = CpiContext::new(sw_program, sw_accounts);
                if ctx.remaining_accounts.len() > 8 { // Oracle Data Account (if needed)
                    sw_ctx = sw_ctx.with_remaining_accounts(vec![ctx.remaining_accounts.get(8).unwrap().clone()]);
                }
                swap_contract::cpi::swap(sw_ctx, inp_swap_root_nonce, inp_swap_inb_nonce, inp_swap_out_nonce, false, false, inp_swap_direction, inp_amount)?;
            }

            // Transfer tokens
            let root_pda_seeds = &[ctx.program_id.as_ref(), &[inp_root_nonce]];
            let root_pda_signer = &[&root_pda_seeds[..]];
            let mut token_auth = ctx.accounts.merchant_approval.to_account_info();
            if inp_swap {
                token_auth = ctx.accounts.root_key.to_account_info();
            }

            // Calculate fees
            if mrch_approval.fees_bps > 0 {
                let f1: u128 = (net_amount as u128) << 64;
                let f2: u128 = f1.checked_mul(mrch_approval.fees_bps as u128).ok_or(ProgramError::from(ErrorCode::Overflow))?;
                let f3: u128 = f2.checked_div(10000).ok_or(ProgramError::from(ErrorCode::Overflow))?;
                let fees: u64 = (f3 >> 64) as u64;
                if fees > 0 {
                    net_amount = net_amount.checked_sub(fees).ok_or(ProgramError::from(ErrorCode::Overflow))?;
                    fee_amount = fees;
                    let cpi_accounts = Transfer {
                        from: ctx.accounts.token_account.to_account_info(),
                        to: ctx.accounts.fees_account.to_account_info(),
                        authority: token_auth.clone(),
                    };
                    let cpi_program = ctx.accounts.token_program.to_account_info();
                    let cpi_ctx;
                    if inp_swap {
                        cpi_ctx = CpiContext::new_with_signer(cpi_program, cpi_accounts, root_pda_signer);
                    } else {
                        cpi_ctx = CpiContext::new(cpi_program, cpi_accounts);
                    }
                    token::transfer(cpi_ctx, fees)?;
                }
                //msg!("Atellix: Starting Amount: {} Ending Amount: {} Fees: {}", inp_amount.to_string(), amount.to_string(), fees.to_string());
            }
            let cpi_accounts = Transfer {
                from: ctx.accounts.token_account.to_account_info(),
                to: ctx.accounts.merchant_token.to_account_info(),
                authority: token_auth.clone(),
            };
            let cpi_program = ctx.accounts.token_program.to_account_info();
            let cpi_ctx;
            if inp_swap {
                cpi_ctx = CpiContext::new_with_signer(cpi_program, cpi_accounts, root_pda_signer);
            } else {
                cpi_ctx = CpiContext::new(cpi_program, cpi_accounts);
            }
            token::transfer(cpi_ctx, net_amount)?;

            // Record merchant revenue
            let na_program = ctx.accounts.net_auth.to_account_info();
            if *na_program.key == net_authority::ID {
                let na_accounts = RecordRevenue {
                    root_data: ctx.accounts.net_root.to_account_info(),
                    auth_data: ctx.accounts.net_rbac.to_account_info(),
                    revenue_admin: ctx.accounts.root_key.to_account_info(),
                    merchant_approval: ctx.accounts.merchant_approval.to_account_info(),
                };
                let na_ctx = CpiContext::new_with_signer(na_program, na_accounts, root_pda_signer);
                //msg!("Atellix: Attempt to record revenue");
                net_authority::cpi::record_revenue(na_ctx, inp_net_nonce, true, net_amount)?;
                mrch_approval = load_struct::<MerchantApproval>(&ctx.accounts.merchant_approval.to_account_info())?;
            }
        }

        msg!("atellix-log");
        emit!(PaymentEvent {
            event_hash: 322577841493927779632802603853323858392, // solana/program/token-agent/merchant_receive
            slot: clock.slot,
            merchant_tx_id: mrch_approval.tx_count,
            merchant_key: *ctx.accounts.merchant_key.to_account_info().key,
            user_key: *ctx.accounts.user_key.to_account_info().key,
            total: inp_amount,
            amount: net_amount,
            fees: fee_amount,
            payment_id: inp_payment_id,
            swap: inp_swap,
        });

        Ok(())
    }
/*
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
        let spl_token: Pubkey = Pubkey::from_str(SPL_TOKEN).unwrap();
        let funder_key = ctx.accounts.funder_key.to_account_info();
        let allowance_data = ctx.accounts.allowance_data.to_account_info();
        let sys_program = ctx.accounts.system_program.to_account_info();
        let derived_allowance_key: Pubkey;
        if ctx.remaining_accounts.len() > 0 {
            derived_allowance_key = Pubkey::create_program_address(
                &[
                    &ctx.accounts.user_key.to_account_info().key.to_bytes(),
                    &spl_token.to_bytes(),
                    &ctx.accounts.token_mint.to_account_info().key.to_bytes(),
                    &ctx.accounts.token_account.to_account_info().key.to_bytes(),
                    &ctx.accounts.delegate_key.to_account_info().key.to_bytes(),
                    &ctx.remaining_accounts.get(0).unwrap().key.to_bytes(),
                    &[inp_allowance_nonce]
                ],
                ctx.program_id
            ).map_err(|_| ErrorCode::InvalidNonce)?;
        } else {
            derived_allowance_key = Pubkey::create_program_address(
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
        }
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

        if ctx.remaining_accounts.len() > 0 {
            msg!("Create 1");
            let account_signer_seeds: &[&[_]] = &[
                &ctx.accounts.user_key.to_account_info().key.to_bytes(),
                &spl_token.to_bytes(),
                &ctx.accounts.token_mint.to_account_info().key.to_bytes(),
                &ctx.accounts.token_account.to_account_info().key.to_bytes(),
                &ctx.accounts.delegate_key.to_account_info().key.to_bytes(),
                &ctx.remaining_accounts.get(0).unwrap().key.to_bytes(),
                &[inp_allowance_nonce],
            ];
            msg!("Create Token Allowance");
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
        } else {
            msg!("Create 2");
            let account_signer_seeds: &[&[_]] = &[
                &ctx.accounts.user_key.to_account_info().key.to_bytes(),
                &spl_token.to_bytes(),
                &ctx.accounts.token_mint.to_account_info().key.to_bytes(),
                &ctx.accounts.token_account.to_account_info().key.to_bytes(),
                &ctx.accounts.delegate_key.to_account_info().key.to_bytes(),
                &[inp_allowance_nonce],
            ];
            msg!("Create Token Allowance");
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
        if ctx.remaining_accounts.len() > 0 {
            let pk: Pubkey = *ctx.remaining_accounts.get(0).unwrap().key;
            tka.recipient_key = Some(pk);
        }

        let mut approval_data = &mut allowance_data.try_borrow_mut_data()?;
        let disc_bytes = array_ref![approval_data, 0, 8];
        if disc_bytes != &[0; 8] {
            msg!("Account already initialized");
            return Err(ErrorCode::InvalidAccount.into());
        }
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
        let spl_token: Pubkey = Pubkey::from_str(SPL_TOKEN).unwrap();
        let derived_allowance_key: Pubkey;
        if ctx.remaining_accounts.len() > 0 {
            derived_allowance_key = Pubkey::create_program_address(
                &[
                    &ctx.accounts.user_key.to_account_info().key.to_bytes(),
                    &spl_token.to_bytes(),
                    &ctx.accounts.token_mint.to_account_info().key.to_bytes(),
                    &ctx.accounts.token_account.to_account_info().key.to_bytes(),
                    &ctx.accounts.delegate_key.to_account_info().key.to_bytes(),
                    &ctx.remaining_accounts.get(0).unwrap().key.to_bytes(),
                    &[inp_allowance_nonce]
                ],
                ctx.program_id
            ).map_err(|_| ErrorCode::InvalidNonce)?;
        } else {
            derived_allowance_key = Pubkey::create_program_address(
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
        }
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

        Ok(())
    }

    pub fn delegated_transfer(ctx: Context<DelegatedTransfer>,
        inp_user_nonce: u8,
        inp_allowance_nonce: u8,
        inp_amount: u64,
    ) -> ProgramResult {
        // Verfiy allowance program derived address
        let spl_token: Pubkey = Pubkey::from_str(SPL_TOKEN).unwrap();
        let derived_allowance_key: Pubkey;
        if ctx.accounts.allowance_data.recipient_key.is_some() {
            derived_allowance_key = Pubkey::create_program_address(
                &[
                    &ctx.accounts.user_key.to_account_info().key.to_bytes(),
                    &spl_token.to_bytes(),
                    &ctx.accounts.token_mint.to_account_info().key.to_bytes(),
                    &ctx.accounts.user_token.to_account_info().key.to_bytes(),
                    &ctx.accounts.delegate_key.to_account_info().key.to_bytes(),
                    &ctx.accounts.token_recipient.to_account_info().key.to_bytes(),
                    &[inp_allowance_nonce]
                ],
                ctx.program_id
            ).map_err(|_| ErrorCode::InvalidNonce)?;
        } else {
            derived_allowance_key = Pubkey::create_program_address(
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
        }
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

        let ald = &mut ctx.accounts.allowance_data;

        // Verify accounts match
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
        // Check recipient_key option
        if ald.recipient_key.is_some() {
            if ald.recipient_key.unwrap() != *ctx.accounts.token_recipient.to_account_info().key {
                msg!("Invalid recipient account");
                return Err(ErrorCode::InvalidAccount.into());
            }
        }

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
            //msg!("Transfer amount: {}", inp_amount.to_string());
            //msg!("Begin: {}", ald.amount.to_string());
            let diff = ald.amount.checked_sub(inp_amount);
            if diff.is_some() {
                // Perform transfer
                ald.amount = diff.unwrap();
                //msg!("End: {}", ald.amount.to_string());
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
    } */
}

#[derive(Accounts)]
pub struct UpdateMetadata<'info> {
    #[account(constraint = program.programdata_address() == Some(program_data.key()))]
    pub program: Program<'info, TokenAgent>,
    #[account(constraint = program_data.upgrade_authority_address == Some(program_admin.key()))]
    pub program_data: Account<'info, ProgramData>,
    #[account(mut)]
    pub program_admin: Signer<'info>,
    #[account(init_if_needed, seeds = [program_id.as_ref(), b"metadata"], bump, payer = program_admin, space = 584)]
    pub program_info: Account<'info, ProgramMetadata>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
#[instruction(inp_link_token: bool, inp_initial_amount: u64, inp_user_nonce: u8, inp_merchant_nonce: u8, inp_root_nonce: u8)]
pub struct CreateSubscr<'info> {
    #[account(mut)]
    pub subscr_data: UncheckedAccount<'info>,
    pub net_auth: UncheckedAccount<'info>,
    pub net_rbac: UncheckedAccount<'info>,
    pub net_root: UncheckedAccount<'info>,
    #[account(seeds = [program_id.as_ref()], bump = inp_root_nonce)]
    pub root_key: UncheckedAccount<'info>,
    pub merchant_key: UncheckedAccount<'info>,
    #[account(mut)]
    pub merchant_approval: UncheckedAccount<'info>,
    #[account(mut)]
    pub merchant_token: UncheckedAccount<'info>,
    pub manager_key: UncheckedAccount<'info>,
    pub manager_approval: UncheckedAccount<'info>,
    #[account(mut)]
    pub user_key: Signer<'info>,
    pub user_agent: UncheckedAccount<'info>,
    #[account(address = token::ID)]
    pub token_program: UncheckedAccount<'info>,
    pub token_mint: UncheckedAccount<'info>,
    #[account(mut)]
    pub token_account: UncheckedAccount<'info>,
    #[account(mut)]
    pub fees_account: UncheckedAccount<'info>,
}

#[derive(Accounts)]
pub struct UpdateSubscr<'info> {
    #[account(mut)]
    pub subscr_data: Account<'info, SubscrData>,
    pub net_auth: UncheckedAccount<'info>,
    pub net_rbac: UncheckedAccount<'info>,
    pub net_root: UncheckedAccount<'info>,
    pub root_key: UncheckedAccount<'info>,
    pub merchant_key: UncheckedAccount<'info>,
    #[account(mut)]
    pub merchant_approval: UncheckedAccount<'info>,
    #[account(mut)]
    pub merchant_token: UncheckedAccount<'info>,
    pub manager_key: UncheckedAccount<'info>,
    pub manager_approval: UncheckedAccount<'info>,
    pub user_key: Signer<'info>,
    pub user_agent: UncheckedAccount<'info>,
    #[account(address = token::ID)]
    pub token_program: UncheckedAccount<'info>,
    pub token_mint: UncheckedAccount<'info>,
    #[account(mut)]
    pub token_account: UncheckedAccount<'info>,
    #[account(mut)]
    pub fees_account: UncheckedAccount<'info>,
}

#[derive(Accounts)]
pub struct CloseSubscr<'info> {
    #[account(mut, close = fee_recipient)]
    pub subscr_data: Account<'info, SubscrData>,
    pub user_key: Signer<'info>,
    #[account(mut)]
    pub fee_recipient: Signer<'info>,
}

#[derive(Accounts)]
pub struct UpdateManager<'info> {
    #[account(mut)]
    pub subscr_data: Account<'info, SubscrData>,
    pub manager_prev: Signer<'info>,
    pub manager_key: UncheckedAccount<'info>,
    pub manager_approval: UncheckedAccount<'info>,
}

#[derive(Accounts)]
pub struct ManagerCancel<'info> {
    #[account(mut)]
    pub subscr_data: Account<'info, SubscrData>,
    pub manager_key: Signer<'info>,
    pub manager_approval: UncheckedAccount<'info>,
}

#[derive(Accounts)]
pub struct ProcessSubscr<'info> {
    #[account(mut)]
    pub subscr_data: Account<'info, SubscrData>,
    pub net_auth: UncheckedAccount<'info>,
    pub net_rbac: UncheckedAccount<'info>,
    pub net_root: UncheckedAccount<'info>,
    pub root_key: UncheckedAccount<'info>,
    pub merchant_key: UncheckedAccount<'info>,
    #[account(mut)]
    pub merchant_approval: UncheckedAccount<'info>,
    #[account(mut)]
    pub merchant_token: UncheckedAccount<'info>,
    pub manager_key: Signer<'info>,
    pub manager_approval: UncheckedAccount<'info>,
    pub user_agent: UncheckedAccount<'info>,
    #[account(address = token::ID)]
    pub token_program: UncheckedAccount<'info>,
    pub token_mint: UncheckedAccount<'info>,
    #[account(mut)]
    pub token_account: UncheckedAccount<'info>,
    #[account(mut)]
    pub fees_account: UncheckedAccount<'info>,
}

#[derive(Accounts)]
pub struct MerchantPayment<'info> {
    pub net_auth: UncheckedAccount<'info>,
    pub net_rbac: UncheckedAccount<'info>,
    pub net_root: UncheckedAccount<'info>,
    pub root_key: UncheckedAccount<'info>,
    pub merchant_key: UncheckedAccount<'info>,
    #[account(mut)]
    pub merchant_approval: UncheckedAccount<'info>,
    #[account(mut)]
    pub merchant_token: UncheckedAccount<'info>,
    pub user_key: Signer<'info>,
    #[account(address = token::ID)]
    pub token_program: UncheckedAccount<'info>,
    pub token_mint: UncheckedAccount<'info>,
    #[account(mut)]
    pub token_account: UncheckedAccount<'info>,
    #[account(mut)]
    pub fees_account: UncheckedAccount<'info>,
}

#[derive(Accounts)]
pub struct MerchantReceive<'info> {
    pub net_auth: UncheckedAccount<'info>,
    pub net_rbac: UncheckedAccount<'info>,
    pub net_root: UncheckedAccount<'info>,
    pub root_key: UncheckedAccount<'info>,
    pub merchant_key: UncheckedAccount<'info>,
    #[account(mut)]
    pub merchant_approval: Signer<'info>,
    #[account(mut)]
    pub merchant_token: UncheckedAccount<'info>,
    pub user_key: UncheckedAccount<'info>,
    #[account(address = token::ID)]
    pub token_program: UncheckedAccount<'info>,
    pub token_mint: UncheckedAccount<'info>,
    #[account(mut)]
    pub token_account: UncheckedAccount<'info>,
    #[account(mut)]
    pub fees_account: UncheckedAccount<'info>,
}
/*
#[derive(Accounts)]
pub struct CreateAllowance<'info> {
    #[account(mut)]
    pub allowance_data: AccountInfo<'info>,
    #[account(signer)]
    pub user_key: AccountInfo<'info>,
    pub user_agent: AccountInfo<'info>,
    pub delegate_key: AccountInfo<'info>,
    pub token_mint: AccountInfo<'info>,
    #[account(mut)]
    pub token_account: AccountInfo<'info>,
    #[account(address = token::ID)]
    pub token_program: AccountInfo<'info>,
    #[account(address = system_program::ID)]
    pub system_program: AccountInfo<'info>,
    #[account(signer)]
    pub funder_key: AccountInfo<'info>,
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
    #[account(address = token::ID)]
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
    #[account(address = token::ID)]
    pub token_program: AccountInfo<'info>,
} */

#[account]
pub struct SubscrData {
    pub user_key: Pubkey,               // The user that owns this subscription
    pub approval_program: Pubkey,       // The address of the network authority program that signs approvals
    pub merchant_key: Pubkey,           // The merchant account that receives subscription payments
    pub merchant_approval: Pubkey,      // The merchant approval record from the network authority
    pub manager_key: Pubkey,            // The rebill manager account being assigned
    pub manager_approval: Pubkey,       // The rebill manager approval from the network authority
    pub token_mint: Pubkey,             // The token mint to pay for the subscription
    pub token_account: Pubkey,          // The token account to pay for the subscription
    pub swap_account: Pubkey,           // The token account to swap from if using a different mint for payments
    // Subscription details below
    pub subscr_id: u128,                // External subscription UUID
    pub rebill_events: u32,             // Count of rebill events
    pub rebill_max: u32,                // Maximum number of times to rebill (0 = unlimited)
    pub next_rebill: i64,               // The start of the next rebilling period (actual rebilling may happen later)
    pub not_valid_before: i64,          // UTC timestamp before which no subscription processing can occur
    pub not_valid_after: i64,           // UTC timestamp after which no subscription processing can occur
    pub max_delay: i64,                 // The number of seconds after the start of the rebill period the manager can be delayed in attempting to rebill
    pub period: u8,                     // Subscription rebill period
    pub period_budget: u64,             // Per-rebill budget (maximum amount, not necessarily the amount that will be billed which could be less)
    pub use_total: bool,                // Enable a total budget for the entire subscription (for manager initiated payments, user initiated payments do not count towards this limit)
    pub total_budget: u64,              // Total budget for the entire subscription
    pub active: bool,                   // Subscription is active
    pub swap: bool,                     // Swap tokens before payment
    pub swap_direction: bool,           // Swap direction
}

impl Default for SubscrData {
    fn default() -> Self {
        Self {
            user_key: Pubkey::default(),
            approval_program: Pubkey::default(),
            merchant_key: Pubkey::default(),
            merchant_approval: Pubkey::default(),
            manager_key: Pubkey::default(),
            manager_approval: Pubkey::default(),
            token_mint: Pubkey::default(),
            token_account: Pubkey::default(),
            swap_account: Pubkey::default(),
            subscr_id: 0,
            rebill_events: 0,
            rebill_max: 0,
            next_rebill: 0,
            not_valid_before: 0,
            not_valid_after: 0,
            max_delay: 0,
            period: 0,
            period_budget: 0,
            use_total: false,
            total_budget: 0,
            active: true,
            swap: false,
            swap_direction: true,
        }
    }
}

#[account]
pub struct TokenAllowance {
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

#[event]
pub struct SubscrEvent {
    pub event_hash: u128,
    pub slot: u64,
    pub merchant_tx_id: u64,
    pub subscr_data: Pubkey,
    pub subscr_id: u128,
    pub payment_id: u128,
    pub rebill_event: u32,
    pub total: u64,
    pub amount: u64,
    pub fees: u64,
    pub next_rebill: i64,
    pub swap: bool,
}

#[event]
pub struct PaymentEvent {
    pub event_hash: u128,
    pub slot: u64,
    pub merchant_tx_id: u64,
    pub merchant_key: Pubkey,
    pub user_key: Pubkey,
    pub payment_id: u128,
    pub total: u64,
    pub amount: u64,
    pub fees: u64,
    pub swap: bool,
}

#[account]
pub struct ProgramMetadata {
    pub semvar_major: u32,
    pub semvar_minor: u32,
    pub semvar_patch: u32,
    pub program: Pubkey,
    pub program_name: String,   // Max len 64
    pub developer_name: String, // Max len 64
    pub developer_url: String,  // Max len 128
    pub source_url: String,     // Max len 128
    pub verify_url: String,     // Max len 128
}
// 8 + (4 * 3) + (4 * 5) + (64 * 2) + (128 * 3) + 32
// Data length (with discrim): 584 bytes

#[error]
pub enum ErrorCode {
    #[msg("Inactive subscription")]
    InactiveSubscription,
    #[msg("Invalid program id")]
    InvalidProgramId,
    #[msg("Invalid subscription period")]
    InvalidSubscriptionPeriod,
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
    #[msg("Total budget exceeded")]
    TotalBudgetExceeded,
    #[msg("Period budget exceeded")]
    PeriodBudgetExceeded,
    #[msg("Allowance exceeded")]
    AllowanceExceeded,
    #[msg("Access denied")]
    AccessDenied,
    #[msg("Subscription not valid yet")]
    NotValidYet,
    #[msg("Expired")]
    Expired,
    #[msg("Maximum rebills reached")]
    MaxRebills,
    #[msg("Overflow")]
    Overflow,
}
