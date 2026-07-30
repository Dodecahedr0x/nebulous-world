use anchor_lang::prelude::*;
use anchor_spl::token::{self, Token, TokenAccount, Transfer};

use crate::constants::{APP_SEED, APP_TAG_STAKE_SEED, CONFIG_SEED, STAKE_POSITION_SEED};
use crate::error::ErrorCode;
use crate::reward_math::{reward_debt_for, settle_pending, transfer_from_vault};
use crate::state::{AppAccount, AppTagStake, Config, StakePosition};
use crate::unstake_fee::weighted_avg_timestamp;

/// The tag-staking mirror of `Vote`. Both the pending-reward payout and the
/// principal-in leg move through the single global vault, signed (for the
/// payout leg) by `config` — the vault's only authority now, not `app` or
/// `app_tag_stake`.
#[derive(Accounts)]
pub struct StakeTag<'info> {
    #[account(mut, seeds = [APP_SEED, app.app_id.as_bytes()], bump = app.bump)]
    pub app: Account<'info, AppAccount>,
    // No instruction-arg `tag_id`/`tag` account needed to derive/validate
    // this PDA: once `app_tag_stake` is deserialized (a plain `mut`, not
    // `init`, since a stake can only ever target an already-suggested tag),
    // its seeds constraint can reference its OWN already-deserialized fields
    // (`app_tag_stake.app`/`app_tag_stake.tag`).
    //
    // The seeds/bump constraint alone only proves `app_tag_stake` is
    // internally self-consistent — it does NOT prove it belongs to the
    // `app` account passed alongside it in this same instruction. Without
    // the `constraint =` below, an attacker could permissionlessly create
    // their OWN `app`/`app_tag_stake` pair and then call `stake_tag` with
    // THEIR `app_tag_stake` but a victim's well-funded `app`, crediting the
    // attacker's position against the victim's
    // `total_tag_stake`/`tags_acc_reward_per_share` and draining the shared
    // vault via the corrupted stake denominator. This explicit cross-check
    // closes that gap.
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
    #[account(
        init_if_needed,
        payer = user,
        space = 8 + StakePosition::SPACE,
        seeds = [
            STAKE_POSITION_SEED,
            app_tag_stake.key().as_ref(),
            user.key().as_ref(),
        ],
        bump,
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
    #[account(
        mut,
        associated_token::mint = config.vote_mint,
        associated_token::authority = user,
    )]
    pub user_token_account: Account<'info, TokenAccount>,
    #[account(mut)]
    pub user: Signer<'info>,
    pub token_program: Program<'info, Token>,
    pub system_program: Program<'info, System>,
}

/// Locks `amount` of the vote token into the global vault, auto-settling any
/// pending tags-pool reward accrued on the caller's existing position first
/// — same checkpoint-before-size-change requirement as `vote()`. The key
/// difference from `vote()`: the accumulator checked against is
/// `app.tags_acc_reward_per_share` (shared across ALL of this app's tags),
/// NOT anything stored per-(app, tag) — see the design note on
/// `AppTagStake`.
///
/// `app_tag_stake.stake_amount` and `app.total_tag_stake` are two
/// independent counters that must move in lockstep on every mutation: the
/// former is this (app, tag) pair's own principal total, the latter is the
/// shared pool's total (the denominator `fund_app_rewards`'s Tags-pool
/// `bump_accumulator` divides by). Letting them drift would either starve or
/// over-pay every tag's stakers.
pub fn handler(ctx: Context<StakeTag>, amount: u64) -> Result<()> {
    require!(amount > 0, ErrorCode::ZeroAmount);

    let position = &mut ctx.accounts.position;

    if position.amount > 0 {
        let pending = settle_pending(
            position.amount,
            position.reward_debt,
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
    }

    // Transfer principal IN, signed by the user (not a PDA) — this leg
    // needs no PDA signer seeds at all.
    token::transfer(
        CpiContext::new(
            ctx.accounts.token_program.key(),
            Transfer {
                from: ctx.accounts.user_token_account.to_account_info(),
                to: ctx.accounts.vault.to_account_info(),
                authority: ctx.accounts.user.to_account_info(),
            },
        ),
        amount,
    )?;

    let now = Clock::get()?.unix_timestamp;
    let app_tag_stake_key = ctx.accounts.app_tag_stake.key();
    let app = &mut ctx.accounts.app;
    let app_tag_stake = &mut ctx.accounts.app_tag_stake;
    let position = &mut ctx.accounts.position;

    // Must run BEFORE `position.amount` is updated below — see the matching
    // comment in vote.rs.
    position.staked_at = weighted_avg_timestamp(position.staked_at, position.amount, now, amount);
    position.amount = position
        .amount
        .checked_add(amount)
        .ok_or(ErrorCode::MathOverflow)?;
    // Idempotent: `app_tag_stake`/`owner` are exactly the seeds that derived
    // this PDA, so re-writing them on a top-up of an existing position is a
    // no-op. `payer` isn't a seed, but is idempotent here too — see the
    // matching comment in vote.rs.
    position.app_tag_stake = app_tag_stake_key;
    position.owner = ctx.accounts.user.key();
    position.payer = ctx.accounts.user.key();
    position.bump = ctx.bumps.position;
    app_tag_stake.stake_amount = app_tag_stake
        .stake_amount
        .checked_add(amount)
        .ok_or(ErrorCode::MathOverflow)?;
    app.total_tag_stake = app
        .total_tag_stake
        .checked_add(amount)
        .ok_or(ErrorCode::MathOverflow)?;
    position.reward_debt = reward_debt_for(position.amount, app.tags_acc_reward_per_share)?;

    Ok(())
}
