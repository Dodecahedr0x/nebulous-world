use anchor_lang::prelude::*;
use anchor_spl::token::{self, Token, TokenAccount, Transfer};

use crate::constants::{APP_SEED, CONFIG_SEED, VOTE_POSITION_SEED};
use crate::error::ErrorCode;
use crate::reward_math::{reward_debt_for, settle_pending, transfer_from_vault};
use crate::state::{AppAccount, Config, VotePosition};
use crate::unstake_fee::weighted_avg_timestamp;

#[derive(Accounts)]
pub struct Vote<'info> {
    #[account(mut, seeds = [APP_SEED, app.app_id.as_bytes()], bump = app.bump)]
    pub app: Account<'info, AppAccount>,
    #[account(
        init_if_needed,
        payer = user,
        space = 8 + VotePosition::SPACE,
        seeds = [VOTE_POSITION_SEED, app.key().as_ref(), user.key().as_ref()],
        bump,
    )]
    pub position: Account<'info, VotePosition>,
    #[account(seeds = [CONFIG_SEED], bump = config.bump)]
    pub config: Account<'info, Config>,
    /// The single global vault (see the design note on `Config`). Verified
    /// by re-deriving its ATA address from `config`/`config.vote_mint`
    /// directly, so no separate `vote_mint` account needs to be passed in.
    #[account(
        mut,
        associated_token::mint = config.vote_mint,
        associated_token::authority = config,
    )]
    pub vault: Account<'info, TokenAccount>,
    /// `token::mint =` isn't independently load-bearing — the SPL Token
    /// program's own `Transfer` instruction already rejects any cross-mint
    /// transfer against `vault`, on both legs, so a wrong-mint account can
    /// only ever fail closed. It's here for a clear, typed Anchor error
    /// instead of an opaque SPL-level one on the same class of mistake. Same
    /// constraint, same reasoning, on every other caller-supplied token
    /// account in this program (`withdraw_vote`, `stake_tag`,
    /// `withdraw_tag_stake`, `claim_vote_reward`, `claim_tag_reward`,
    /// `fund_app_rewards`).
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
/// pending vote-reward accrued on the caller's existing position (if any)
/// before the position's size changes — the standard accumulator-pattern
/// requirement: rewards must be checkpointed *before* `amount` moves, or the
/// old stake would retroactively "earn" rewards accrued only after the top-up.
///
/// On the very first vote for a fresh `VotePosition`, `position.amount` is
/// 0 (zero-initialized by `init_if_needed`), so the settle-and-pay-out step
/// below is skipped entirely — there is nothing to settle yet.
pub fn handler(ctx: Context<Vote>, amount: u64) -> Result<()> {
    require!(amount > 0, ErrorCode::ZeroAmount);

    let position = &mut ctx.accounts.position;

    if position.amount > 0 {
        let pending = settle_pending(
            position.amount,
            position.reward_debt,
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
    }

    // Transfer principal IN, signed by the user (not a PDA) — this leg needs
    // no PDA signer seeds at all.
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
    let app_key = ctx.accounts.app.key();
    let app = &mut ctx.accounts.app;
    let position = &mut ctx.accounts.position;

    // Must run BEFORE `position.amount` is updated below — the weighted
    // average needs the OLD amount as the weight for the OLD checkpoint.
    position.staked_at = weighted_avg_timestamp(position.staked_at, position.amount, now, amount);
    position.amount = position
        .amount
        .checked_add(amount)
        .ok_or(ErrorCode::MathOverflow)?;
    // Idempotent: `app`/`owner` are exactly the seeds that derived this PDA,
    // so re-writing them on a top-up of an existing position is a no-op.
    // `payer` isn't a seed, but is idempotent here too — only the account's
    // ORIGINAL creator can ever reach this line for a given position, since
    // that's exactly the signer `app`/`owner`'s seeds already require.
    position.app = app_key;
    position.owner = ctx.accounts.user.key();
    position.payer = ctx.accounts.user.key();
    position.bump = ctx.bumps.position;
    app.total_vote_stake = app
        .total_vote_stake
        .checked_add(amount)
        .ok_or(ErrorCode::MathOverflow)?;
    position.reward_debt = reward_debt_for(position.amount, app.vote_acc_reward_per_share)?;

    Ok(())
}
