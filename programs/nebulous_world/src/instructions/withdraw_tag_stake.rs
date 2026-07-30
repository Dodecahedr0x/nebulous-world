use anchor_lang::prelude::*;
use anchor_spl::token::{Token, TokenAccount};

use crate::constants::{APP_SEED, APP_TAG_STAKE_SEED, CONFIG_SEED, STAKE_POSITION_SEED};
use crate::error::ErrorCode;
use crate::reward_math::{reward_debt_for, settle_pending, transfer_from_vault};
use crate::state::{AppAccount, AppTagStake, Config, StakePosition};
use crate::unstake_fee::{linear_decay_fee_bps, unstake_fee};

/// The tag-staking mirror of `WithdrawVote`. Unlike the pre-global-vault
/// design, there is only ONE signing authority involved here (`config`, for
/// both the reward payout and the principal return) — no more juggling two
/// different PDA signers for two different vaults.
#[derive(Accounts)]
pub struct WithdrawTagStake<'info> {
    #[account(
        mut,
        seeds = [
            APP_SEED,
            app.app_id.as_bytes()
        ],
        bump = app.bump,
    )]
    pub app: Account<'info, AppAccount>,
    // See `StakeTag::app_tag_stake`'s doc comment for why the trailing
    // `constraint =` is required: the seeds/bump constraint alone only
    // proves `app_tag_stake` is internally self-consistent, not that it
    // belongs to the specific `app` passed alongside it in this instruction.
    #[account(
        mut,
        seeds = [
            APP_TAG_STAKE_SEED,
            app_tag_stake.app.as_ref(),
            app_tag_stake.tag.as_ref(),
        ],
        bump = app_tag_stake.bump,
        has_one = app @ ErrorCode::AppTagStakeMismatch,
    )]
    pub app_tag_stake: Account<'info, AppTagStake>,
    // As in `WithdrawVote`, a withdrawal can only ever target a position
    // that already exists, so this is a plain `mut` + seeds/bump constraint
    // rather than `init_if_needed`. The single `user: Signer` re-derivation
    // of this PDA *is* the ownership check.
    #[account(
        mut,
        seeds = [
            STAKE_POSITION_SEED,
            app_tag_stake.key().as_ref(),
            user.key().as_ref(),
        ],
        bump = position.bump,
    )]
    pub position: Account<'info, StakePosition>,
    #[account(seeds = [CONFIG_SEED], bump = config.bump)]
    pub config: Account<'info, Config>,
    #[account(
        mut,
        associated_token::mint = config.vote_mint,
        associated_token::authority = config,
    )]
    pub vault: Account<'info, TokenAccount>,
    #[account(mut,
        associated_token::mint = config.vote_mint,
        associated_token::authority = user,
    )]
    pub user_token_account: Account<'info, TokenAccount>,
    /// The admin's own token account — see `WithdrawVote::admin_token_account`'s
    /// doc comment, same constraints and same reasoning. Boxed for the same
    /// reason too: this struct has one more `Account<'info, _>` field than
    /// `WithdrawVote` (`app_tag_stake`), which is what actually pushes
    /// `try_accounts` over SBF's 4096-byte stack frame limit without it.
    #[account(
        mut,
        associated_token::mint = config.vote_mint,
        associated_token::authority = config.authority,
    )]
    pub admin_token_account: Box<Account<'info, TokenAccount>>,
    pub user: Signer<'info>,
    pub token_program: Program<'info, Token>,
}

/// Moves `amount` of previously-locked tag-stake principal back out of the
/// global vault to the caller, always settling (and paying out) any pending
/// tags-pool reward accrued on the position first — mirrors
/// `withdraw_vote()`'s checkpoint-before-size-change requirement,
/// unconditional here for the same reason documented there (an existing
/// position always has `amount > 0`).
///
/// Charges the same linearly-decaying unstake fee as `withdraw_vote`'s (1%
/// -> 0% over a week — see `unstake_fee.rs`), paid directly to
/// `admin_token_account` — see that field's doc comment and the matching
/// note on `withdraw_vote`'s handler for why this is a straight treasury
/// skim with no "pool is empty" edge case, unlike the reward pools this
/// instruction otherwise interacts with.
pub fn handler(ctx: Context<WithdrawTagStake>, amount: u64) -> Result<()> {
    require!(amount > 0, ErrorCode::ZeroAmount);
    require!(
        ctx.accounts.position.amount >= amount,
        ErrorCode::InsufficientStake
    );

    let pending = settle_pending(
        ctx.accounts.position.amount,
        ctx.accounts.position.reward_debt,
        ctx.accounts.app.tags_acc_reward_per_share,
    )?;
    transfer_from_vault(
        &ctx.accounts.vault,
        &ctx.accounts.user_token_account,
        &ctx.accounts.config.to_account_info(),
        ctx.accounts.config.bump,
        &ctx.accounts.token_program,
        pending,
    )?;

    let now = Clock::get()?.unix_timestamp;
    let elapsed = now.saturating_sub(ctx.accounts.position.staked_at);
    let fee = unstake_fee(amount, linear_decay_fee_bps(elapsed))?;

    let app = &mut ctx.accounts.app;
    let app_tag_stake = &mut ctx.accounts.app_tag_stake;
    let position = &mut ctx.accounts.position;

    position.amount -= amount; // safe: checked `position.amount >= amount` above
    app_tag_stake.stake_amount = app_tag_stake
        .stake_amount
        .checked_sub(amount)
        .ok_or(ErrorCode::MathOverflow)?;
    app.total_tag_stake = app
        .total_tag_stake
        .checked_sub(amount)
        .ok_or(ErrorCode::MathOverflow)?;
    position.reward_debt = reward_debt_for(position.amount, app.tags_acc_reward_per_share)?;

    // Pay the fee straight to the admin, signed by `config`.
    // `transfer_from_vault` no-ops when `fee` is 0.
    transfer_from_vault(
        &ctx.accounts.vault,
        &ctx.accounts.admin_token_account,
        &ctx.accounts.config.to_account_info(),
        ctx.accounts.config.bump,
        &ctx.accounts.token_program,
        fee,
    )?;

    // Return principal (net of the unstake fee), signed by `config`.
    transfer_from_vault(
        &ctx.accounts.vault,
        &ctx.accounts.user_token_account,
        &ctx.accounts.config.to_account_info(),
        ctx.accounts.config.bump,
        &ctx.accounts.token_program,
        amount - fee,
    )?;

    Ok(())
}
