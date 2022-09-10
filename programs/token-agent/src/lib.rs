use crate::program::TokenAgent;
use std::{ io::Cursor, string::String, result::Result as FnResult };
use arrayref::array_ref;
use num_enum::TryFromPrimitive;
use chrono::{ NaiveDateTime, Datelike };
use anchor_lang::prelude::*;
use anchor_spl::token::{ self, Token, TokenAccount, Transfer };
use anchor_spl::associated_token::{ AssociatedToken };
use solana_program::{ system_program, account_info::AccountInfo, clock::Clock };

use net_authority::{ self, cpi::accounts::RecordTransaction, MerchantApproval, ManagerApproval };
use swap_contract::{ cpi::accounts::Swap };
use token_delegate::{ self, cpi::accounts::{ DelegateApprove, DelegateTransfer } };

declare_id!("AGNTcdPiqzTvTczVNihCFQAoaT6Q6xqrtRMWkExyHCdm");

pub const VERSION_MAJOR: u32 = 1;
pub const VERSION_MINOR: u32 = 0;
pub const VERSION_PATCH: u32 = 2;

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
            let mut q = dt.date().month().checked_div(3).ok_or(error!(ErrorCode::Overflow))?;
            q = q.checked_add(1).ok_or(error!(ErrorCode::Overflow))?;
            Ok(format!("{}q{}", dt.format("%Y").to_string(), q.to_string()))
        },
        SubscriptionPeriod::Yearly => Ok(dt.format("%Y").to_string()),
    }
}

#[repr(u8)]
#[derive(PartialEq, Debug, Eq, Copy, Clone, TryFromPrimitive)]
pub enum SwapMode {
    AtxSwapContractV1,
}

fn verify_matching_accounts(left: &Pubkey, right: &Pubkey, error_msg: Option<String>) -> anchor_lang::Result<()> {
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

fn verify_manager_approval(netauth: &Pubkey, manager_approval: &AccountInfo) -> anchor_lang::Result<()> {
    verify_matching_accounts(netauth, &manager_approval.owner,
        Some(String::from("Invalid manager approval owner"))
    )?;
    let mgr_approval = load_struct::<ManagerApproval>(manager_approval)?;
    if !mgr_approval.active {
        msg!("Inactive manager approval");
        return Err(ErrorCode::NotApproved.into());
    }
    Ok(())
}

fn get_manager_key(manager_approval: &AccountInfo) -> anchor_lang::Result<Pubkey> {
    let mgr_approval = load_struct::<ManagerApproval>(manager_approval)?;
    Ok(mgr_approval.manager_key)
}

fn verify_merchant_approval(
    dest_nonce: u8,
    netauth: &Pubkey,
    merchant_approval: &AccountInfo,
    merchant_token: &AccountInfo,
    fees_account: &AccountInfo,
) -> anchor_lang::Result<u32> {
    verify_matching_accounts(netauth, &merchant_approval.owner,
        Some(String::from("Invalid merchant approval owner"))
    )?;
    let mrch_approval = load_struct::<MerchantApproval>(merchant_approval)?;
    if !mrch_approval.active {
        msg!("Inactive merchant approval");
        return Err(ErrorCode::NotApproved.into());
    }
    verify_matching_accounts(&mrch_approval.fees_account, fees_account.key,
        Some(String::from("Fees account does not match approval"))
    )?;
    // Verify merchant's associated token
    let derived_merchant_key = Pubkey::create_program_address(
        &[
            &mrch_approval.dest_account.to_bytes(),
            &Token::id().to_bytes(),
            &mrch_approval.token_mint.to_bytes(),
            &[dest_nonce]
        ],
        &AssociatedToken::id()
    ).map_err(|_| ErrorCode::InvalidNonce)?;
    if derived_merchant_key != *merchant_token.key {
        msg!("Invalid merchant token account");
        return Err(ErrorCode::InvalidDerivedAccount.into());
    }
    Ok(mrch_approval.fees_bps)
}

fn get_token_mint(merchant_approval: &AccountInfo) -> anchor_lang::Result<Pubkey> {
    let mrch_approval = load_struct::<MerchantApproval>(merchant_approval)?;
    Ok(mrch_approval.token_mint)
}

fn get_merchant_key(merchant_approval: &AccountInfo) -> anchor_lang::Result<Pubkey> {
    let mrch_approval = load_struct::<MerchantApproval>(merchant_approval)?;
    Ok(mrch_approval.merchant_key)
}

fn get_tx_count(merchant_approval: &AccountInfo) -> anchor_lang::Result<u64> {
    let mrch_approval = load_struct::<MerchantApproval>(merchant_approval)?;
    Ok(mrch_approval.tx_count)
}

fn calculate_fees(net_amount: u64, fees_bps: u32) -> anchor_lang::Result<u64> {
    let f1: u128 = (net_amount as u128) << 64;
    let f2: u128 = f1.checked_mul(fees_bps as u128).ok_or(error!(ErrorCode::Overflow))?;
    let f3: u128 = f2.checked_div(10000).ok_or(error!(ErrorCode::Overflow))?;
    let fees: u64 = (f3 >> 64) as u64;
    Ok(fees)
}

#[inline]
fn load_struct<T: AccountDeserialize>(acc: &AccountInfo) -> FnResult<T, ProgramError> {
    let mut data: &[u8] = &acc.try_borrow_data()?;
    Ok(T::try_deserialize(&mut data)?)
}

#[inline]
fn store_struct<T: AccountSerialize>(obj: &T, acc: &AccountInfo) -> FnResult<(), Error> {
    let mut data = acc.try_borrow_mut_data()?;
    let disc_bytes = array_ref![data, 0, 8];
    if disc_bytes != &[0; 8] {
        msg!("Account already initialized");
        return Err(error!(ErrorCode::InvalidAccount));
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
    ) -> anchor_lang::Result<()> {
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
        inp_dest_nonce: u8,
        inp_root_nonce: u8,
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
        inp_swap_mode: u8,
        inp_swap_data_nonce: u8,
        inp_swap_inb_nonce: u8,
        inp_swap_out_nonce: u8,
        inp_swap_dst_nonce: u8,
    ) -> anchor_lang::Result<()> {
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
        /*verify_matching_accounts(&mrch_approval.token_mint, &ctx.accounts.token_account.mint,
            Some(String::from("Token mint does not match approval"))
        )?;*/
        verify_matching_accounts(&mrch_approval.fees_account, &ctx.accounts.fees_account.to_account_info().key,
            Some(String::from("Fees account does not match approval"))
        )?;
        let mgr_approval = load_struct::<ManagerApproval>(&ctx.accounts.manager_approval.to_account_info())?;
        if !mgr_approval.active {
            msg!("Inactive manager approval");
            return Err(ErrorCode::NotApproved.into());
        }

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

        let timeframe_end = timeframe_start.checked_add(max_delay).ok_or(error!(ErrorCode::Overflow))?;
        if inp_next_rebill < timeframe_start || inp_next_rebill > timeframe_end {
            msg!("Next rebill not within timeframe");
            return Err(ErrorCode::InvalidTimeframe.into());
        }
        let d1 = get_period_string(inp_next_rebill, period.unwrap())?;
        let prev_period = inp_next_rebill.checked_sub(1).ok_or(error!(ErrorCode::Overflow))?;
        let d2 = get_period_string(prev_period, period.unwrap())?;
        if d1 == d2 {
            msg!("Next rebill not beginning of period");
            return Err(ErrorCode::InvalidTimeframe.into());
        }

        // Verify merchant's associated token
        let derived_merchant_key = Pubkey::create_program_address(
            &[
                &mrch_approval.dest_account.to_bytes(),
                &Token::id().to_bytes(),
                &mrch_approval.token_mint.to_bytes(),
                &[inp_dest_nonce]
            ],
            &AssociatedToken::id()
        ).map_err(|_| ErrorCode::InvalidNonce)?;
        if derived_merchant_key != *ctx.accounts.merchant_token.to_account_info().key {
            msg!("Invalid merchant token account");
            return Err(ErrorCode::InvalidDerivedAccount.into());
        }

        // Setup up token delegate if needed
        if !inp_swap && inp_link_token {
            let cpi_accounts = DelegateApprove {
                allowance: ctx.accounts.allowance.to_account_info(),
                allowance_payer: ctx.accounts.user_key.to_account_info(),
                owner: ctx.accounts.user_key.to_account_info(),
                delegate: ctx.accounts.root_key.to_account_info(),
                delegate_root: ctx.accounts.delegate_root.to_account_info(),
                token_account: ctx.accounts.token_account.to_account_info(),
                token_program: ctx.accounts.token_program.to_account_info(),
                system_program: ctx.accounts.system_program.to_account_info(),
            };
            let cpi_program = ctx.accounts.delegate_program.to_account_info();
            let cpi_ctx = CpiContext::new(cpi_program, cpi_accounts);
            token_delegate::cpi::delegate_approve(cpi_ctx, true, u64::MAX, u64::MAX)?;
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
                    let cpi_accounts = DelegateApprove {
                        allowance: ctx.accounts.allowance.to_account_info(),
                        allowance_payer: ctx.accounts.user_key.to_account_info(),
                        owner: ctx.accounts.user_key.to_account_info(),
                        delegate: ctx.accounts.root_key.to_account_info(),
                        delegate_root: ctx.accounts.delegate_root.to_account_info(),
                        token_account: acc_swap_token.clone(),
                        token_program: ctx.accounts.token_program.to_account_info(),
                        system_program: ctx.accounts.system_program.to_account_info(),
                    };
                    let cpi_program = ctx.accounts.delegate_program.to_account_info();
                    let cpi_ctx = CpiContext::new(cpi_program, cpi_accounts);
                    token_delegate::cpi::delegate_approve(cpi_ctx, true, u64::MAX, u64::MAX)?;
                }

                // Verify token agent's swap destination associated token
                let derived_swap_key = Pubkey::create_program_address(
                    &[
                        &ctx.accounts.root_key.to_account_info().key.to_bytes(),
                        &Token::id().to_bytes(),
                        &mrch_approval.token_mint.to_bytes(),
                        &[inp_swap_dst_nonce]
                    ],
                    &AssociatedToken::id()
                ).map_err(|_| ErrorCode::InvalidNonce)?;
                if derived_swap_key != *ctx.accounts.token_account.to_account_info().key {
                    msg!("Invalid swap destination token account");
                    return Err(ErrorCode::InvalidDerivedAccount.into());
                }

                let swap_mode = SwapMode::try_from_primitive(inp_swap_mode);
                if swap_mode.is_err() {
                    msg!("Invalid swap mode");
                    return Err(ErrorCode::InvalidSwapMode.into());
                }
                if swap_mode.unwrap() == SwapMode::AtxSwapContractV1 {
                    //msg!("Atellix: Attempt swap");
                    swap_account = acc_swap_token.key();
                    let sw_program = ctx.remaining_accounts.get(1).unwrap().clone();
                    let sw_accounts = Swap {
                        swap_user: ctx.accounts.user_key.to_account_info(),
                        swap_data: ctx.remaining_accounts.get(2).unwrap().clone(),
                        inb_token_src: acc_swap_token.clone(),
                        inb_token_dst: ctx.remaining_accounts.get(3).unwrap().clone(),
                        out_token_src: ctx.remaining_accounts.get(4).unwrap().clone(),
                        out_token_dst: ctx.accounts.token_account.to_account_info(),    // Token agent swap destination
                        fees_token: ctx.remaining_accounts.get(5).unwrap().clone(),
                        token_program: ctx.accounts.token_program.to_account_info(),
                    };
                    let mut sw_ctx = CpiContext::new(sw_program, sw_accounts);
                    if ctx.remaining_accounts.len() > 6 { // Oracle Data Account (if needed)
                        sw_ctx = sw_ctx.with_remaining_accounts(vec![ctx.remaining_accounts.get(6).unwrap().clone()]);
                    }
                    swap_contract::cpi::swap(sw_ctx, inp_swap_data_nonce, inp_swap_inb_nonce, inp_swap_out_nonce, 0, inp_swap_direction, false, true, inp_initial_amount)?;
                }
            }

            let root_pda_seeds = &[ctx.program_id.as_ref(), &[inp_root_nonce]];
            let root_pda_signer = &[&root_pda_seeds[..]];

            // Calculate fees
            if mrch_approval.fees_bps > 0 {
                let f1: u128 = (net_amount as u128) << 64;
                let f2: u128 = f1.checked_mul(mrch_approval.fees_bps as u128).ok_or(error!(ErrorCode::Overflow))?;
                let f3: u128 = f2.checked_div(10000).ok_or(error!(ErrorCode::Overflow))?;
                let fees: u64 = (f3 >> 64) as u64;
                if fees > 0 {
                    net_amount = net_amount.checked_sub(fees).ok_or(error!(ErrorCode::Overflow))?;
                    fee_amount = fees;
                    let cpi_accounts = Transfer {
                        from: ctx.accounts.token_account.to_account_info(),
                        to: ctx.accounts.fees_account.to_account_info(),
                        authority: if inp_swap { ctx.accounts.root_key.to_account_info() } else { ctx.accounts.user_key.to_account_info() },
                    };
                    let cpi_program = ctx.accounts.token_program.to_account_info();
                    let cpi_ctx = if inp_swap {
                        CpiContext::new_with_signer(cpi_program, cpi_accounts, root_pda_signer)
                    } else {
                        CpiContext::new(cpi_program, cpi_accounts)
                    };
                    token::transfer(cpi_ctx, fees)?;
                }
            }

            let cpi_accounts = Transfer {
                from: ctx.accounts.token_account.to_account_info(),
                to: ctx.accounts.merchant_token.to_account_info(),
                authority: if inp_swap { ctx.accounts.root_key.to_account_info() } else { ctx.accounts.user_key.to_account_info() },
            };
            let cpi_program = ctx.accounts.token_program.to_account_info();
            let cpi_ctx = if inp_swap {
                CpiContext::new_with_signer(cpi_program, cpi_accounts, root_pda_signer)
            } else {
                CpiContext::new(cpi_program, cpi_accounts)
            };
            token::transfer(cpi_ctx, net_amount)?;

            // Record merchant revenue
            let na_program = ctx.accounts.net_auth.to_account_info();
            if *na_program.key == net_authority::ID {
                let na_accounts = RecordTransaction {
                    tx_admin: ctx.accounts.root_key.to_account_info(),
                    merchant_approval: ctx.accounts.merchant_approval.to_account_info(),
                };
                let na_ctx = CpiContext::new_with_signer(na_program, na_accounts, root_pda_signer);
                //msg!("Atellix: Attempt to record transaction");
                net_authority::cpi::record_tx(na_ctx)?;
                mrch_approval = load_struct::<MerchantApproval>(&ctx.accounts.merchant_approval.to_account_info())?;
            }
        }

        // Create subscription data
        let mut subscr = SubscrData::default();
        subscr.user_key = *ctx.accounts.user_key.to_account_info().key;
        subscr.approval_program = *ctx.accounts.net_auth.to_account_info().key;
        subscr.merchant_key = mrch_approval.merchant_key;
        subscr.merchant_approval = *ctx.accounts.merchant_approval.to_account_info().key;
        subscr.manager_key = mgr_approval.manager_key;
        subscr.manager_approval = *ctx.accounts.manager_approval.to_account_info().key;
        subscr.token_mint = mrch_approval.token_mint;
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
        subscr.swap_mode = inp_swap_mode;
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
        inp_dest_nonce: u8,
        inp_root_nonce: u8,
        inp_active: bool,
        inp_link_token: bool,
        inp_amount: u64,
        inp_payment_id: u128,
        inp_next_rebill: i64,
        //inp_rebill_max: u32,
        inp_period: u8,
        inp_period_budget: u64,
        //inp_use_total: bool,
        //inp_total_budget: u64,
        inp_max_delay: i64,
        inp_not_valid_before: i64,
        inp_not_valid_after: i64,
        inp_swap: bool,
        inp_swap_direction: bool,
        inp_swap_mode: u8,
        inp_swap_data_nonce: u8,
        inp_swap_inb_nonce: u8,
        inp_swap_out_nonce: u8,
        inp_swap_dst_nonce: u8,
    ) -> anchor_lang::Result<()> {

        /*msg!("inp_merchant_nonce: {}", inp_merchant_nonce.to_string());
        msg!("inp_root_nonce: {}", inp_root_nonce.to_string());
        msg!("inp_active: {}", inp_active.to_string());
        msg!("inp_link_token: {}", inp_link_token.to_string());
        msg!("inp_payment_id: {}", inp_payment_id.to_string());
        msg!("inp_amount: {}", inp_amount.to_string());
        msg!("inp_period: {}", inp_period.to_string());
        msg!("inp_period_budget: {}", inp_period_budget.to_string());
        msg!("inp_next_rebill: {}", inp_next_rebill.to_string());
        msg!("inp_max_delay: {}", inp_max_delay.to_string());
        msg!("inp_not_valid_before: {}", inp_not_valid_before.to_string());
        msg!("inp_not_valid_after: {}", inp_not_valid_after.to_string());
        msg!("inp_swap: {}", inp_swap.to_string());
        msg!("inp_swap_direction: {}", inp_swap_direction.to_string());
        msg!("inp_swap_mode: {}", inp_swap_mode.to_string());
        msg!("inp_swap_data_nonce: {}", inp_swap_data_nonce.to_string());
        msg!("inp_swap_inb_nonce: {}", inp_swap_inb_nonce.to_string());
        msg!("inp_swap_out_nonce: {}", inp_swap_out_nonce.to_string());
        msg!("inp_swap_dst_nonce: {}", inp_swap_dst_nonce.to_string());*/

        let clock = Clock::get()?;
        let ts = clock.unix_timestamp;
        let mut subscr = load_struct::<SubscrData>(&ctx.accounts.subscr_data.to_account_info())?;
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
                subscr_data: *ctx.accounts.subscr_data.to_account_info().key,
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
        verify_matching_accounts(&ctx.accounts.net_auth.to_account_info().key, &subscr.approval_program,
            Some(String::from("Approval program does not match"))
        )?;

        // Verify network authority accounts
        let fees_bps: u32 = verify_merchant_approval(
            inp_dest_nonce,
            &ctx.accounts.net_auth.to_account_info().key,
            &ctx.accounts.merchant_approval.to_account_info(),
            &ctx.accounts.merchant_token.to_account_info(),
            &ctx.accounts.fees_account.to_account_info(),
        )?;
        verify_manager_approval(&ctx.accounts.net_auth.to_account_info().key, &ctx.accounts.manager_approval.to_account_info())?;

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
            msg!("Invalid max_delay: {} below minimum of 12 hours (43200 seconds)", inp_max_delay.to_string());
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
        let timeframe_end = timeframe_start.checked_add(inp_max_delay).ok_or(error!(ErrorCode::Overflow))?;
        if inp_next_rebill < timeframe_start || inp_next_rebill > timeframe_end {
            msg!("Next rebill not within timeframe");
            return Err(ErrorCode::InvalidTimeframe.into());
        }
        let d1 = get_period_string(inp_next_rebill, period.unwrap())?;
        let prev_period = inp_next_rebill.checked_sub(1).ok_or(error!(ErrorCode::Overflow))?;
        let d2 = get_period_string(prev_period, period.unwrap())?;
        if d1 == d2 {
            msg!("Next rebill not beginning of period");
            return Err(ErrorCode::InvalidTimeframe.into());
        }

       // Setup up token delegate if needed
        if !inp_swap && inp_link_token {
            let cpi_accounts = DelegateApprove {
                allowance: ctx.accounts.allowance.to_account_info(),
                allowance_payer: ctx.accounts.user_key.to_account_info(),
                owner: ctx.accounts.user_key.to_account_info(),
                delegate: ctx.accounts.root_key.to_account_info(),
                delegate_root: ctx.accounts.delegate_root.to_account_info(),
                token_account: ctx.accounts.token_account.to_account_info(),
                token_program: ctx.accounts.token_program.to_account_info(),
                system_program: ctx.accounts.system_program.to_account_info(),
            };
            let cpi_program = ctx.accounts.delegate_program.to_account_info();
            let cpi_ctx = CpiContext::new(cpi_program, cpi_accounts);
            token_delegate::cpi::delegate_approve(cpi_ctx, true, u64::MAX, u64::MAX)?;
        }

        // Link swap token if requested
        if inp_swap {
            let acc_swap_token = ctx.remaining_accounts.get(0).unwrap();        // User Swap Token

            // Setup up token delegate if needed
            if inp_link_token {
                let cpi_accounts = DelegateApprove {
                    allowance: ctx.accounts.allowance.to_account_info(),
                    allowance_payer: ctx.accounts.user_key.to_account_info(),
                    owner: ctx.accounts.user_key.to_account_info(),
                    delegate: ctx.accounts.root_key.to_account_info(),
                    delegate_root: ctx.accounts.delegate_root.to_account_info(),
                    token_account: acc_swap_token.clone(),
                    token_program: ctx.accounts.token_program.to_account_info(),
                    system_program: ctx.accounts.system_program.to_account_info(),
                };
                let cpi_program = ctx.accounts.delegate_program.to_account_info();
                let cpi_ctx = CpiContext::new(cpi_program, cpi_accounts);
                token_delegate::cpi::delegate_approve(cpi_ctx, true, u64::MAX, u64::MAX)?;
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
                        &get_token_mint(&ctx.accounts.merchant_approval.to_account_info())?.to_bytes(),
                        &[inp_swap_dst_nonce]
                    ],
                    &AssociatedToken::id()
                ).map_err(|_| ErrorCode::InvalidNonce)?;
                if derived_swap_key != *ctx.accounts.token_account.to_account_info().key {
                    msg!("Invalid swap destination token account");
                    return Err(ErrorCode::InvalidDerivedAccount.into());
                }
                let swap_mode = SwapMode::try_from_primitive(inp_swap_mode);
                if swap_mode.is_err() {
                    msg!("Invalid swap mode: {}", inp_swap_mode.to_string());
                    return Err(ErrorCode::InvalidSwapMode.into());
                }
                if swap_mode.unwrap() == SwapMode::AtxSwapContractV1 {
                    let acc_swap_token = ctx.remaining_accounts.get(0).unwrap();        // User Swap Token
                    let sw_program = ctx.remaining_accounts.get(1).unwrap().clone();
                    let sw_accounts = Swap {
                        swap_user: ctx.accounts.user_key.to_account_info(),
                        swap_data: ctx.remaining_accounts.get(2).unwrap().clone(),
                        inb_token_src: acc_swap_token.clone(),
                        inb_token_dst: ctx.remaining_accounts.get(3).unwrap().clone(),
                        out_token_src: ctx.remaining_accounts.get(4).unwrap().clone(),
                        out_token_dst: ctx.accounts.token_account.to_account_info(),    // Token Agent PDA
                        fees_token: ctx.remaining_accounts.get(5).unwrap().clone(),
                        token_program: ctx.accounts.token_program.to_account_info(),
                    };
                    let mut sw_ctx = CpiContext::new(sw_program, sw_accounts);
                    if ctx.remaining_accounts.len() > 6 { // Oracle Data Account (if needed)
                        sw_ctx = sw_ctx.with_remaining_accounts(vec![ctx.remaining_accounts.get(6).unwrap().clone()]);
                    }
                    swap_contract::cpi::swap(sw_ctx, inp_swap_data_nonce, inp_swap_inb_nonce, inp_swap_out_nonce, 0, inp_swap_direction, false, true, inp_amount)?;
                }
            }

            let root_pda_seeds = &[ctx.program_id.as_ref(), &[inp_root_nonce]];
            let root_pda_signer = &[&root_pda_seeds[..]];

            // Calculate fees
            if fees_bps > 0 {
                let fees: u64 = calculate_fees(net_amount, fees_bps)?;
                if fees > 0 {
                    net_amount = net_amount.checked_sub(fees).ok_or(error!(ErrorCode::Overflow))?;
                    fee_amount = fees;
                    let cpi_accounts = Transfer {
                        from: ctx.accounts.token_account.to_account_info(),
                        to: ctx.accounts.fees_account.to_account_info(),
                        authority: if inp_swap { ctx.accounts.root_key.to_account_info() } else { ctx.accounts.user_key.to_account_info() },
                    };
                    let cpi_program = ctx.accounts.token_program.to_account_info();
                    let cpi_ctx = if inp_swap {
                        CpiContext::new_with_signer(cpi_program, cpi_accounts, root_pda_signer)
                    } else {
                        CpiContext::new(cpi_program, cpi_accounts)
                    };
                    token::transfer(cpi_ctx, fees)?;
                }
                //msg!("Starting Amount: {} Ending Amount: {} Fees: {}", inp_amount.to_string(), amount.to_string(), fees.to_string());
            }
            let cpi_accounts = Transfer {
                from: ctx.accounts.token_account.to_account_info(),
                to: ctx.accounts.merchant_token.to_account_info(),
                authority: if inp_swap { ctx.accounts.root_key.to_account_info() } else { ctx.accounts.user_key.to_account_info() },
            };
            let cpi_program = ctx.accounts.token_program.to_account_info();
            let cpi_ctx = if inp_swap {
                CpiContext::new_with_signer(cpi_program, cpi_accounts, root_pda_signer)
            } else {
                CpiContext::new(cpi_program, cpi_accounts)
            };
            token::transfer(cpi_ctx, net_amount)?;

            // Record merchant revenue
            let na_program = ctx.accounts.net_auth.to_account_info();
            if *na_program.key == net_authority::ID {
                let na_accounts = RecordTransaction {
                    tx_admin: ctx.accounts.root_key.to_account_info(),
                    merchant_approval: ctx.accounts.merchant_approval.to_account_info(),
                };
                let na_ctx = CpiContext::new_with_signer(na_program, na_accounts, root_pda_signer);
                //msg!("Atellix: Attempt to record transaction");
                net_authority::cpi::record_tx(na_ctx)?;
            }
        }

        // Update subscription data
        subscr.active = true;
        subscr.merchant_key = get_merchant_key(&ctx.accounts.merchant_approval.to_account_info())?;
        subscr.merchant_approval = *ctx.accounts.merchant_approval.to_account_info().key;
        subscr.manager_key = get_manager_key(&ctx.accounts.manager_approval.to_account_info())?;
        subscr.manager_approval = *ctx.accounts.manager_approval.to_account_info().key;
        subscr.token_mint = get_token_mint(&ctx.accounts.merchant_approval.to_account_info())?;
        subscr.token_account = *ctx.accounts.token_account.to_account_info().key;
        subscr.swap_account = if inp_swap { *ctx.remaining_accounts.get(0).unwrap().key } else { Pubkey::default() };
        //subscr.rebill_max = inp_rebill_max;
        subscr.next_rebill = inp_next_rebill;
        subscr.max_delay = inp_max_delay;
        subscr.not_valid_before = inp_not_valid_before;
        subscr.not_valid_after = inp_not_valid_after;
        subscr.period = inp_period;
        subscr.period_budget = inp_period_budget;
        //subscr.use_total = inp_use_total;
        //subscr.total_budget = inp_total_budget;
        subscr.swap = inp_swap;
        subscr.swap_direction = inp_swap_direction;
        subscr.swap_mode = inp_swap_mode;
        store_struct(&subscr, &ctx.accounts.subscr_data.to_account_info())?;

        msg!("atellix-log");
        emit!(SubscrEvent {
            event_hash: 298296161986799263364555576740275705662, // solana/program/token-agent/update_subscription
            slot: clock.slot,
            merchant_tx_id: get_tx_count(&ctx.accounts.merchant_approval.to_account_info())?,
            subscr_data: *ctx.accounts.subscr_data.to_account_info().key,
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

    pub fn close_subscription(ctx: Context<CloseSubscr>) -> anchor_lang::Result<()> {
        let subscr = &ctx.accounts.subscr_data;
        verify_matching_accounts(&subscr.user_key, ctx.accounts.user_key.to_account_info().key,
            Some(String::from("User key does not match subscription"))
        )?;

        msg!("Closed Subscription: {}", ctx.accounts.subscr_data.to_account_info().key.to_string());
        Ok(())
    }

    pub fn update_manager<'info>(ctx: Context<'_, '_, '_, 'info, UpdateManager<'info>>) -> anchor_lang::Result<()> {
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

    pub fn manager_cancel<'info>(ctx: Context<'_, '_, '_, 'info, ManagerCancel<'info>>) -> anchor_lang::Result<()> {
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
        inp_merchant_nonce: u8,
        inp_root_nonce: u8,
        inp_rebill_ts: i64,
        inp_rebill_str: String,
        inp_next_rebill: i64,
        inp_amount: u64,
        inp_payment_id: u128,
        inp_swap_data_nonce: u8,
        inp_swap_inb_nonce: u8,
        inp_swap_out_nonce: u8,
        inp_swap_estimate: u64,
    ) -> anchor_lang::Result<()> {
        let clock = Clock::get()?;
        let ts = clock.unix_timestamp;

        // Validate accounts
        let mut subscr = load_struct::<SubscrData>(&ctx.accounts.subscr_data.to_account_info())?;
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

        // Verfiy token account and mint
        verify_matching_accounts(&subscr.token_account, &ctx.accounts.token_account.to_account_info().key,
            Some(String::from("Token account does not match subscription"))
        )?;
        verify_matching_accounts(&subscr.token_mint, &ctx.accounts.token_account.mint,
            Some(String::from("Token mint does not match subscription"))
        )?;

        // Verify merchant's associated token
        let mut mrch_approval = load_struct::<MerchantApproval>(&ctx.accounts.merchant_approval.to_account_info())?;
        let derived_merchant_key = Pubkey::create_program_address(
            &[
                &mrch_approval.merchant_key.to_bytes(),
                &Token::id().to_bytes(),
                &ctx.accounts.token_account.mint.to_bytes(),
                &[inp_merchant_nonce]
            ],
            &AssociatedToken::id()
        ).map_err(|_| ErrorCode::InvalidNonce)?;
        verify_matching_accounts(&derived_merchant_key, ctx.accounts.merchant_token.to_account_info().key,
            Some(String::from("Invalid merchant token account"))
        )?;

        // Verify network authority accounts
        if !mrch_approval.active {
            msg!("Inactive merchant approval");
            return Err(ErrorCode::NotApproved.into());
        }
        verify_matching_accounts(&subscr.merchant_key, &mrch_approval.merchant_key,
            Some(String::from("Merchant key does not match subscription"))
        )?;
        verify_matching_accounts(&mrch_approval.token_mint, &ctx.accounts.token_account.mint,
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
        let timeframe_end = inp_rebill_ts.checked_add(subscr.max_delay).ok_or(error!(ErrorCode::Overflow))?;
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
        let prev_period = inp_next_rebill.checked_sub(1).ok_or(error!(ErrorCode::Overflow))?;
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
                subscr.total_budget = subscr.total_budget.checked_sub(inp_amount).ok_or(error!(ErrorCode::Overflow))?;
            }
            // Swap if requested
            let root_pda_seeds = &[ctx.program_id.as_ref(), &[inp_root_nonce]];
            let root_pda_signer = &[&root_pda_seeds[..]];
            if subscr.swap {
                //msg!("Atellix: Attempt swap");
                let acc_swap_token = ctx.remaining_accounts.get(0).unwrap();        // User Swap Token
                verify_matching_accounts(&subscr.swap_account, &acc_swap_token.key(),
                    Some(String::from("Swap token does not match subscription"))
                )?;
                let token_user_amount: u64 = load_struct::<TokenAccount>(acc_swap_token).unwrap().amount;
                let token_swap_amount: u64 = load_struct::<TokenAccount>(ctx.remaining_accounts.get(3).unwrap()).unwrap().amount;
                let token_transfer;
                if inp_swap_estimate == 0 || token_user_amount < inp_swap_estimate {
                    // swap estimate >= user tokens, transfer all
                    token_transfer = token_user_amount;
                } else {
                    // swap estimate < user tokens, use estimate
                    token_transfer = inp_swap_estimate;
                }
                // Delegated transfer all tokens to token-agent owned swap input account then transfer remaining back below
                let cpi_accounts = DelegateTransfer {
                    allowance: ctx.accounts.allowance.to_account_info(),
                    delegate: ctx.accounts.root_key.to_account_info(),
                    delegate_root: ctx.accounts.delegate_root.to_account_info(),
                    from: acc_swap_token.clone(),
                    to: ctx.remaining_accounts.get(3).unwrap().clone(),
                    token_program: ctx.accounts.token_program.to_account_info(),
                };
                let cpi_program = ctx.accounts.delegate_program.to_account_info();
                let cpi_ctx = CpiContext::new_with_signer(cpi_program, cpi_accounts, root_pda_signer);
                token_delegate::cpi::delegate_transfer(cpi_ctx, token_transfer)?; // TODO: use swap estimate if less

                let swap_mode = SwapMode::try_from_primitive(subscr.swap_mode);
                if swap_mode.is_err() {
                    msg!("Invalid swap mode");
                    return Err(ErrorCode::InvalidSwapMode.into());
                }
                if swap_mode.unwrap() == SwapMode::AtxSwapContractV1 {
                    // Perform swap
                    let sw_program = ctx.remaining_accounts.get(1).unwrap().clone();
                    let sw_accounts = Swap {
                        swap_user: ctx.accounts.root_key.to_account_info(),             // Root key (signer)
                        swap_data: ctx.remaining_accounts.get(2).unwrap().clone(),
                        inb_token_src: ctx.remaining_accounts.get(3).unwrap().clone(),  // Token Agent PDA (input)
                        inb_token_dst: ctx.remaining_accounts.get(4).unwrap().clone(),
                        out_token_src: ctx.remaining_accounts.get(5).unwrap().clone(),
                        out_token_dst: ctx.accounts.token_account.to_account_info(),    // Token Agent PDA (output)
                        fees_token: ctx.remaining_accounts.get(6).unwrap().clone(),
                        token_program: ctx.accounts.token_program.to_account_info(),
                    };
                    let mut sw_ctx = CpiContext::new_with_signer(sw_program, sw_accounts, root_pda_signer);
                    if ctx.remaining_accounts.len() > 7 { // Oracle Data Account (if needed)
                        sw_ctx = sw_ctx.with_remaining_accounts(vec![ctx.remaining_accounts.get(7).unwrap().clone()]);
                    }
                    swap_contract::cpi::swap(sw_ctx, inp_swap_data_nonce, inp_swap_inb_nonce, inp_swap_out_nonce, 0, subscr.swap_direction, false, true, inp_amount)?;
                }

                // Transfer remaining tokens back ;)
                let token_post_amount: u64 = load_struct::<TokenAccount>(ctx.remaining_accounts.get(3).unwrap()).unwrap().amount;
                let token_return: u64 = token_post_amount.checked_sub(token_swap_amount).ok_or(error!(ErrorCode::Overflow))?;
                let cpi_accounts = Transfer {
                    from: ctx.remaining_accounts.get(3).unwrap().clone(),
                    to: acc_swap_token.clone(),
                    authority: ctx.accounts.root_key.to_account_info(),
                };
                let cpi_program = ctx.accounts.token_program.to_account_info();
                let cpi_ctx = CpiContext::new_with_signer(cpi_program, cpi_accounts, root_pda_signer);
                token::transfer(cpi_ctx, token_return)?;
            }

            // Calculate fees
            if mrch_approval.fees_bps > 0 {
                let f1: u128 = (net_amount as u128) << 64;
                let f2: u128 = f1.checked_mul(mrch_approval.fees_bps as u128).ok_or(error!(ErrorCode::Overflow))?;
                let f3: u128 = f2.checked_div(10000).ok_or(error!(ErrorCode::Overflow))?;
                let fees: u64 = (f3 >> 64) as u64;
                if fees > 0 {
                    net_amount = net_amount.checked_sub(fees).ok_or(error!(ErrorCode::Overflow))?;
                    fee_amount = fees;
                    if subscr.swap {
                        let cpi_accounts = Transfer {
                            from: ctx.accounts.token_account.to_account_info(),
                            to: ctx.accounts.fees_account.to_account_info(),
                            authority: ctx.accounts.root_key.to_account_info(),
                        };
                        let cpi_program = ctx.accounts.token_program.to_account_info();
                        let cpi_ctx = CpiContext::new_with_signer(cpi_program, cpi_accounts, root_pda_signer);
                        token::transfer(cpi_ctx, fees)?;
                    } else {
                        let cpi_accounts = DelegateTransfer {
                            allowance: ctx.accounts.allowance.to_account_info(),
                            delegate: ctx.accounts.root_key.to_account_info(),
                            delegate_root: ctx.accounts.delegate_root.to_account_info(),
                            from: ctx.accounts.token_account.to_account_info(),
                            to: ctx.accounts.fees_account.to_account_info(),
                            token_program: ctx.accounts.token_program.to_account_info(),
                        };
                        let cpi_program = ctx.accounts.delegate_program.to_account_info();
                        let cpi_ctx = CpiContext::new_with_signer(cpi_program, cpi_accounts, root_pda_signer);
                        token_delegate::cpi::delegate_transfer(cpi_ctx, fees)?;
                    }
                }
                //msg!("Starting Amount: {} Ending Amount: {} Fees: {}", inp_amount.to_string(), amount.to_string(), fees.to_string());
            }
            if subscr.swap {
                let cpi_accounts = Transfer {
                    from: ctx.accounts.token_account.to_account_info(),
                    to: ctx.accounts.merchant_token.to_account_info(),
                    authority: ctx.accounts.root_key.to_account_info(),
                };
                let cpi_program = ctx.accounts.token_program.to_account_info();
                let cpi_ctx = CpiContext::new_with_signer(cpi_program, cpi_accounts, root_pda_signer);
                token::transfer(cpi_ctx, net_amount)?;
            } else {
                let cpi_accounts = DelegateTransfer {
                    allowance: ctx.accounts.allowance.to_account_info(),
                    delegate: ctx.accounts.root_key.to_account_info(),
                    delegate_root: ctx.accounts.delegate_root.to_account_info(),
                    from: ctx.accounts.token_account.to_account_info(),
                    to: ctx.accounts.fees_account.to_account_info(),
                    token_program: ctx.accounts.token_program.to_account_info(),
                };
                let cpi_program = ctx.accounts.delegate_program.to_account_info();
                let cpi_ctx = CpiContext::new_with_signer(cpi_program, cpi_accounts, root_pda_signer);
                token_delegate::cpi::delegate_transfer(cpi_ctx, net_amount)?;
            }

            // Record merchant revenue
            let na_program = ctx.accounts.net_auth.to_account_info();
            if *na_program.key == net_authority::ID {
                let na_accounts = RecordTransaction {
                    tx_admin: ctx.accounts.root_key.to_account_info(),
                    merchant_approval: ctx.accounts.merchant_approval.to_account_info(),
                };
                let na_ctx = CpiContext::new_with_signer(na_program, na_accounts, root_pda_signer);
                //msg!("Atellix: Attempt to record transaction");
                net_authority::cpi::record_tx(na_ctx)?;
                mrch_approval = load_struct::<MerchantApproval>(&ctx.accounts.merchant_approval.to_account_info())?
            }
        }

        // Update parameters
        subscr.next_rebill = inp_next_rebill;
        subscr.rebill_events = subscr.rebill_events.checked_add(1).ok_or(error!(ErrorCode::Overflow))?;
        store_struct(&subscr, &ctx.accounts.subscr_data.to_account_info())?;

        msg!("atellix-log");
        emit!(SubscrEvent {
            event_hash: 196800858676461937700417377973077375575, // solana/program/token-agent/process
            slot: clock.slot,
            merchant_tx_id: mrch_approval.tx_count,
            subscr_data: *ctx.accounts.subscr_data.to_account_info().key,
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
        inp_dest_nonce: u8,
        inp_root_nonce: u8,
        inp_payment_id: u128,
        inp_amount: u64,
        inp_swap: bool,
        inp_swap_direction: bool,
        inp_swap_mode: u8,
        inp_swap_data_nonce: u8,
        inp_swap_inb_nonce: u8,
        inp_swap_out_nonce: u8,
        inp_swap_dst_nonce: u8,
    ) -> anchor_lang::Result<()> {
        let clock = Clock::get()?;

        // Verify merchant's associated token for the destination account
        let mut mrch_approval = load_struct::<MerchantApproval>(&ctx.accounts.merchant_approval.to_account_info())?;
        let derived_merchant_key = Pubkey::create_program_address(
            &[
                &mrch_approval.dest_account.to_bytes(),
                &Token::id().to_bytes(),
                &ctx.accounts.token_account.mint.to_bytes(),
                &[inp_dest_nonce]
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

        if !mrch_approval.active {
            msg!("Inactive merchant approval");
            return Err(ErrorCode::NotApproved.into());
        }
        verify_matching_accounts(&mrch_approval.token_mint, &ctx.accounts.token_account.mint,
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
                        &ctx.accounts.token_account.mint.to_bytes(),
                        &[inp_swap_dst_nonce]
                    ],
                    &AssociatedToken::id()
                ).map_err(|_| ErrorCode::InvalidNonce)?;
                if derived_swap_key != *ctx.accounts.token_account.to_account_info().key {
                    msg!("Invalid swap destination token account");
                    return Err(ErrorCode::InvalidDerivedAccount.into());
                }
                let swap_mode = SwapMode::try_from_primitive(inp_swap_mode);
                if swap_mode.is_err() {
                    msg!("Invalid swap mode");
                    return Err(ErrorCode::InvalidSwapMode.into());
                }
                if swap_mode.unwrap() == SwapMode::AtxSwapContractV1 {
                    let acc_swap_token = ctx.remaining_accounts.get(0).unwrap();        // User Swap Token
                    let sw_program = ctx.remaining_accounts.get(1).unwrap().clone();
                    let sw_accounts = Swap {
                        swap_user: ctx.accounts.user_key.to_account_info(),
                        swap_data: ctx.remaining_accounts.get(2).unwrap().clone(),
                        inb_token_src: acc_swap_token.clone(),
                        inb_token_dst: ctx.remaining_accounts.get(3).unwrap().clone(),
                        out_token_src: ctx.remaining_accounts.get(4).unwrap().clone(),
                        out_token_dst: ctx.accounts.token_account.to_account_info(),    // Token Agent PDA
                        fees_token: ctx.remaining_accounts.get(5).unwrap().clone(),
                        token_program: ctx.accounts.token_program.to_account_info(),
                    };
                    let mut sw_ctx = CpiContext::new(sw_program, sw_accounts);
                    if ctx.remaining_accounts.len() > 6 { // Oracle Data Account (if needed)
                        sw_ctx = sw_ctx.with_remaining_accounts(vec![ctx.remaining_accounts.get(6).unwrap().clone()]);
                    }
                    swap_contract::cpi::swap(sw_ctx, inp_swap_data_nonce, inp_swap_inb_nonce, inp_swap_out_nonce, 0, inp_swap_direction, false, true, inp_amount)?;
                }
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
                let f2: u128 = f1.checked_mul(mrch_approval.fees_bps as u128).ok_or(error!(ErrorCode::Overflow))?;
                let f3: u128 = f2.checked_div(10000).ok_or(error!(ErrorCode::Overflow))?;
                let fees: u64 = (f3 >> 64) as u64;
                if fees > 0 {
                    net_amount = net_amount.checked_sub(fees).ok_or(error!(ErrorCode::Overflow))?;
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

            // Record merchant transaction
            let na_program = ctx.accounts.net_auth.to_account_info();
            if *na_program.key == net_authority::ID {
                let na_accounts = RecordTransaction {
                    tx_admin: ctx.accounts.root_key.to_account_info(),
                    merchant_approval: ctx.accounts.merchant_approval.to_account_info(),
                };
                let na_ctx = CpiContext::new_with_signer(na_program, na_accounts, root_pda_signer);
                //msg!("Atellix: Attempt to record transaction");
                net_authority::cpi::record_tx(na_ctx)?;
                mrch_approval = load_struct::<MerchantApproval>(&ctx.accounts.merchant_approval.to_account_info())?;
            }
        }

        msg!("atellix-log");
        emit!(PaymentEvent {
            event_hash: 43781034894216267743388154650854733336, // solana/program/token-agent/merchant_payment
            slot: clock.slot,
            merchant_tx_id: mrch_approval.tx_count,
            merchant_key: mrch_approval.merchant_key,
            merchant_token: *ctx.accounts.merchant_token.to_account_info().key,
            dest_account: mrch_approval.dest_account,
            user_key: *ctx.accounts.user_key.to_account_info().key,
            total: inp_amount,
            amount: net_amount,
            fees: fee_amount,
            payment_id: inp_payment_id,
            swap: inp_swap,
        });

        Ok(())
    }

    /*pub fn merchant_receive<'info>(ctx: Context<'_, '_, '_, 'info, MerchantReceive<'info>>,
        inp_merchant_nonce: u8,
        inp_root_nonce: u8,
        inp_payment_id: u128,
        inp_amount: u64,
        inp_swap: bool,
        inp_swap_direction: bool,
        inp_swap_mode: u8,
        inp_swap_data_nonce: u8,
        inp_swap_inb_nonce: u8,
        inp_swap_out_nonce: u8,
        inp_swap_dst_nonce: u8,
    ) -> anchor_lang::Result<()> {
        let clock = Clock::get()?;

        // Verify merchant's associated token
        let mut mrch_approval = load_struct::<MerchantApproval>(&ctx.accounts.merchant_approval.to_account_info())?;
        let derived_merchant_key = Pubkey::create_program_address(
            &[
                &mrch_approval.merchant_key.to_bytes(),
                &Token::id().to_bytes(),
                &ctx.accounts.token_account.mint.to_bytes(),
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

        if !mrch_approval.active {
            msg!("Inactive merchant approval");
            return Err(ErrorCode::NotApproved.into());
        }
        verify_matching_accounts(&mrch_approval.token_mint, &ctx.accounts.token_account.mint,
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
                        &ctx.accounts.token_account.mint.to_bytes(),
                        &[inp_swap_dst_nonce]
                    ],
                    &AssociatedToken::id()
                ).map_err(|_| ErrorCode::InvalidNonce)?;
                if derived_swap_key != *ctx.accounts.token_account.to_account_info().key {
                    msg!("Invalid swap destination token account");
                    return Err(ErrorCode::InvalidDerivedAccount.into());
                }
                let swap_mode = SwapMode::try_from_primitive(inp_swap_mode);
                if swap_mode.is_err() {
                    msg!("Invalid swap mode");
                    return Err(ErrorCode::InvalidSwapMode.into());
                }
                if swap_mode.unwrap() == SwapMode::AtxSwapContractV1 {
                    let acc_swap_token = ctx.remaining_accounts.get(0).unwrap();        // User Swap Token
                    let sw_program = ctx.remaining_accounts.get(1).unwrap().clone();
                    let sw_accounts = Swap {
                        swap_user: ctx.accounts.merchant_approval.to_account_info(),
                        swap_data: ctx.remaining_accounts.get(2).unwrap().clone(),
                        inb_token_src: acc_swap_token.clone(),
                        inb_token_dst: ctx.remaining_accounts.get(3).unwrap().clone(),
                        out_token_src: ctx.remaining_accounts.get(4).unwrap().clone(),
                        out_token_dst: ctx.accounts.token_account.to_account_info(),    // Token Agent PDA
                        fees_token: ctx.remaining_accounts.get(5).unwrap().clone(),
                        token_program: ctx.accounts.token_program.to_account_info(),
                    };
                    let mut sw_ctx = CpiContext::new(sw_program, sw_accounts);
                    if ctx.remaining_accounts.len() > 6 { // Oracle Data Account (if needed)
                        sw_ctx = sw_ctx.with_remaining_accounts(vec![ctx.remaining_accounts.get(6).unwrap().clone()]);
                    }
                    swap_contract::cpi::swap(sw_ctx, inp_swap_data_nonce, inp_swap_inb_nonce, inp_swap_out_nonce, 0, inp_swap_direction, false, false, inp_amount)?;
                }
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
                let f2: u128 = f1.checked_mul(mrch_approval.fees_bps as u128).ok_or(error!(ErrorCode::Overflow))?;
                let f3: u128 = f2.checked_div(10000).ok_or(error!(ErrorCode::Overflow))?;
                let fees: u64 = (f3 >> 64) as u64;
                if fees > 0 {
                    net_amount = net_amount.checked_sub(fees).ok_or(error!(ErrorCode::Overflow))?;
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

            // Record merchant transaction
            let na_program = ctx.accounts.net_auth.to_account_info();
            if *na_program.key == net_authority::ID {
                let na_accounts = RecordTransaction {
                    tx_admin: ctx.accounts.root_key.to_account_info(),
                    merchant_approval: ctx.accounts.merchant_approval.to_account_info(),
                };
                let na_ctx = CpiContext::new_with_signer(na_program, na_accounts, root_pda_signer);
                //msg!("Atellix: Attempt to record transaction");
                net_authority::cpi::record_tx(na_ctx)?;
                mrch_approval = load_struct::<MerchantApproval>(&ctx.accounts.merchant_approval.to_account_info())?;
            }
        }

        msg!("atellix-log");
        emit!(PaymentEvent {
            event_hash: 322577841493927779632802603853323858392, // solana/program/token-agent/merchant_receive
            slot: clock.slot,
            merchant_tx_id: mrch_approval.tx_count,
            merchant_key: mrch_approval.merchant_key,
            user_key: *ctx.accounts.user_key.to_account_info().key,
            total: inp_amount,
            amount: net_amount,
            fees: fee_amount,
            payment_id: inp_payment_id,
            swap: inp_swap,
        });

        Ok(())
    }*/
}

#[derive(Accounts)]
pub struct UpdateMetadata<'info> {
    #[account(constraint = program.programdata_address().unwrap() == Some(program_data.key()))]
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
#[instruction(inp_link_token: bool, inp_initial_amount: u64, inp_dest_nonce: u8, inp_root_nonce: u8)]
pub struct CreateSubscr<'info> {
    #[account(mut)]
    pub subscr_data: UncheckedAccount<'info>,
    pub net_auth: UncheckedAccount<'info>,
    #[account(seeds = [program_id.as_ref()], bump = inp_root_nonce)]
    pub root_key: UncheckedAccount<'info>,
    #[account(mut)]
    pub merchant_approval: UncheckedAccount<'info>,
    #[account(mut)]
    pub merchant_token: UncheckedAccount<'info>,
    pub manager_approval: UncheckedAccount<'info>,
    #[account(mut)]
    pub user_key: Signer<'info>,
    #[account(address = token::ID)]
    pub token_program: UncheckedAccount<'info>,
    #[account(mut)]
    pub token_account: UncheckedAccount<'info>,
    #[account(mut)]
    pub fees_account: UncheckedAccount<'info>,
    #[account(address = token_delegate::ID)]
    pub delegate_program: UncheckedAccount<'info>,
    pub delegate_root: UncheckedAccount<'info>,
    #[account(mut)]
    pub allowance: UncheckedAccount<'info>,
    #[account(address = system_program::ID)]
    pub system_program: UncheckedAccount<'info>,
}

#[derive(Accounts)]
#[instruction(inp_dest_nonce: u8, inp_root_nonce: u8)]
pub struct UpdateSubscr<'info> {
    #[account(mut)]
    pub subscr_data: UncheckedAccount<'info>,
    pub net_auth: UncheckedAccount<'info>,
    #[account(seeds = [program_id.as_ref()], bump = inp_root_nonce)]
    pub root_key: UncheckedAccount<'info>,
    #[account(mut)]
    pub merchant_approval: UncheckedAccount<'info>,
    #[account(mut)]
    pub merchant_token: UncheckedAccount<'info>,
    pub manager_approval: UncheckedAccount<'info>,
    pub user_key: Signer<'info>,
    #[account(address = token::ID)]
    pub token_program: UncheckedAccount<'info>,
    #[account(mut)]
    pub token_account: UncheckedAccount<'info>,
    #[account(mut)]
    pub fees_account: UncheckedAccount<'info>,
    #[account(address = token_delegate::ID)]
    pub delegate_program: UncheckedAccount<'info>,
    pub delegate_root: UncheckedAccount<'info>,
    #[account(mut)]
    pub allowance: UncheckedAccount<'info>,
    #[account(address = system_program::ID)]
    pub system_program: UncheckedAccount<'info>,
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
#[instruction(inp_merchant_nonce: u8, inp_root_nonce: u8)]
pub struct ProcessSubscr<'info> {
    #[account(mut)]
    pub subscr_data: UncheckedAccount<'info>,
    pub net_auth: UncheckedAccount<'info>,
    #[account(seeds = [program_id.as_ref()], bump = inp_root_nonce)]
    pub root_key: UncheckedAccount<'info>,
    #[account(mut)]
    pub merchant_approval: UncheckedAccount<'info>,
    #[account(mut)]
    pub merchant_token: UncheckedAccount<'info>,
    pub manager_key: Signer<'info>,
    pub manager_approval: UncheckedAccount<'info>,
    #[account(address = token::ID)]
    pub token_program: UncheckedAccount<'info>,
    #[account(mut)]
    pub token_account: Account<'info, TokenAccount>,
    #[account(mut)]
    pub fees_account: UncheckedAccount<'info>,
    #[account(address = token_delegate::ID)]
    pub delegate_program: UncheckedAccount<'info>,
    pub delegate_root: UncheckedAccount<'info>,
    #[account(mut)]
    pub allowance: UncheckedAccount<'info>,
}

#[derive(Accounts)]
#[instruction(inp_merchant_nonce: u8, inp_root_nonce: u8)]
pub struct MerchantPayment<'info> {
    pub net_auth: UncheckedAccount<'info>,
    #[account(seeds = [program_id.as_ref()], bump = inp_root_nonce)]
    pub root_key: UncheckedAccount<'info>,
    #[account(mut)]
    pub merchant_approval: UncheckedAccount<'info>,
    #[account(mut)]
    pub merchant_token: UncheckedAccount<'info>,
    pub user_key: Signer<'info>,
    #[account(address = token::ID)]
    pub token_program: UncheckedAccount<'info>,
    #[account(mut)]
    pub token_account: Account<'info, TokenAccount>,
    #[account(mut)]
    pub fees_account: UncheckedAccount<'info>,
}

/*#[derive(Accounts)]
#[instruction(inp_merchant_nonce: u8, inp_root_nonce: u8)]
pub struct MerchantReceive<'info> {
    pub net_auth: UncheckedAccount<'info>,
    #[account(seeds = [program_id.as_ref()], bump = inp_root_nonce)]
    pub root_key: UncheckedAccount<'info>,
    #[account(mut)]
    pub merchant_approval: Signer<'info>,
    #[account(mut)]
    pub merchant_token: UncheckedAccount<'info>,
    pub user_key: UncheckedAccount<'info>,
    #[account(address = token::ID)]
    pub token_program: UncheckedAccount<'info>,
    #[account(mut)]
    pub token_account: Account<'info, TokenAccount>,
    #[account(mut)]
    pub fees_account: UncheckedAccount<'info>,
}*/

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
    pub swap_mode: u8,                  // Swap mode
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
            swap_mode: 0,
        }
    }
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
    pub merchant_token: Pubkey,
    pub dest_account: Pubkey,
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
    pub program_name: String,   // Max len 60
    pub developer_name: String, // Max len 60
    pub developer_url: String,  // Max len 124
    pub source_url: String,     // Max len 124
    pub verify_url: String,     // Max len 124
}
// 8 + (4 * 3) + (4 * 5) + (64 * 2) + (128 * 3) + 32
// Data length (with discrim): 584 bytes

#[error_code]
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
    InvalidSwapMode,
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
