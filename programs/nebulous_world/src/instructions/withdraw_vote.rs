use anchor_lang::prelude::*;
use anchor_spl::token::{Token, TokenAccount};

use crate::constants::{APP_SEED, CONFIG_SEED, VOTE_POSITION_SEED};
use crate::error::ErrorCode;
use crate::reward_math::{reward_debt_for, settle_pending, transfer_from_vault};
use crate::state::{AppAccount, Config, VotePosition};
use crate::unstake_fee::{linear_decay_fee_bps, unstake_fee};

#[derive(Accounts)]
pub struct WithdrawVote<'info> {
    #[account(
        mut,
        seeds = [
            APP_SEED,
            app.app_id.as_bytes()
        ],
        bump = app.bump,
    )]
    pub app: Account<'info, AppAccount>,
    // Unlike `Vote`'s `position` (which is `init_if_needed` — a vote may be
    // the very first one for this user/app pair), a withdrawal can only ever
    // target a position that already exists: you cannot withdraw stake from
    // a position you never opened. Hence a plain `mut` + seeds/bump
    // constraint here instead of `init_if_needed`.
    //
    // Only a single `user: Signer` is used to authorize the withdrawal: the
    // `position` PDA is derived from `user.key()` (seeds below), exactly as
    // it was when `vote()` first created it, so requiring this specific
    // signer's signature to re-derive that same PDA *is* the ownership
    // check.
    #[account(
        mut,
        seeds = [
            VOTE_POSITION_SEED,
            app.key().as_ref(),
            user.key().as_ref(),
        ],
        bump = position.bump,
    )]
    pub position: Account<'info, VotePosition>,
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
    /// The admin's own token account (an ATA for `config.authority`) —
    /// where the unstake fee is paid directly, see the handler's doc
    /// comment. `token::mint =` is the same defense-in-depth as every other
    /// caller-supplied token account here (see `Vote::user_token_account`'s
    /// doc comment); `address =` additionally pins the OWNER to
    /// `config.authority` specifically, so a withdrawer can't redirect the
    /// fee to an ATA they control themselves.
    ///
    /// Boxed (heap, not stack): this is the 7th `Account<'info, _>` field on
    /// this struct, and `WithdrawTagStake`'s equivalent (one more account —
    /// `app_tag_stake` — than this struct) overflows SBF's 4096-byte stack
    /// frame limit in `try_accounts` by a handful of bytes without it. Boxed
    /// here too for consistency between the two mirrored instructions, not
    /// because `WithdrawVote` alone is currently over the limit.
    #[account(
        mut,
        associated_token::mint = config.vote_mint,
        associated_token::authority = config.authority,
    )]
    pub admin_token_account: Box<Account<'info, TokenAccount>>,
    pub user: Signer<'info>,
    pub token_program: Program<'info, Token>,
}

/// Moves `amount` of previously-locked vote-stake principal back out of the
/// global vault to the caller, always settling (and paying out) any pending
/// vote-reward accrued on the position first — mirrors `vote()`'s
/// checkpoint-before-size-change requirement, except here settlement is
/// unconditional rather than guarded by `position.amount > 0`: a
/// `WithdrawVote` can only ever run against an existing position, and an
/// existing position always has `amount > 0`.
///
/// Charges a linearly-decaying unstake fee (see `unstake_fee.rs`) on the
/// withdrawn `amount` — 1% right after staking, decaying to 0% over the
/// following week — deducted from what's paid out, not from the `amount`
/// recorded as unstaked (`position.amount`/`app.total_vote_stake` both still
/// move by the full `amount`, keeping the accumulator math exactly as
/// before). The fee is paid directly to `admin_token_account`, a straight
/// treasury skim — unlike the reward pools, it never touches
/// `vote_acc_reward_per_share`, so there's no "pool is empty, nobody to pay"
/// edge case to special-case: the fee is owed to the admin regardless of
/// how many other stakers remain.
pub fn handler(ctx: Context<WithdrawVote>, amount: u64) -> Result<()> {
    require!(amount > 0, ErrorCode::ZeroAmount);
    require!(
        ctx.accounts.position.amount >= amount,
        ErrorCode::InsufficientStake
    );

    let pending = settle_pending(
        ctx.accounts.position.amount,
        ctx.accounts.position.reward_debt,
        ctx.accounts.app.vote_acc_reward_per_share,
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
    let position = &mut ctx.accounts.position;

    position.amount -= amount; // safe: checked `position.amount >= amount` above
    app.total_vote_stake = app
        .total_vote_stake
        .checked_sub(amount)
        .ok_or(ErrorCode::MathOverflow)?;
    position.reward_debt = reward_debt_for(position.amount, app.vote_acc_reward_per_share)?;

    // Pay the fee straight to the admin, signed by `config` — the vault's
    // only authority. `transfer_from_vault` no-ops when `fee` is 0 (the
    // common case once a position has fully decayed past the fee window).
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
