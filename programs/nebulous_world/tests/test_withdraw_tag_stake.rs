mod common;

use {
    anchor_lang::solana_program::pubkey::Pubkey,
    common::{
        create_funded_user, credit_token_account, derive_stake_position, fetch_app,
        fetch_app_tag_stake, fetch_stake_position, fetch_token_amount, register_app_and_tag, send,
        set_app_tags_accumulator, setup_with_tag, stake_tag_ix, warp_forward,
        withdraw_tag_stake_ix, Env, TagPdas,
    },
    litesvm::LiteSVM,
    nebulous_world::{
        constants::{REWARD_PRECISION, UNSTAKE_FEE_DECAY_SECONDS},
        unstake_fee::{linear_decay_fee_bps, unstake_fee},
    },
    solana_keypair::Keypair,
    solana_signer::Signer,
};

const APP_ID: &str = "cid_wtag_test_app_0000001";
const TAG_ID: &str = "defi";

/// The fee charged on withdrawing `amount` at elapsed=0 — i.e. when the
/// withdrawal lands in the same LiteSVM instance as the `stake_tag` that
/// opened the position, with no explicit warp, so the full 1% (100 bps)
/// applies.
fn fee_at_elapsed_zero(amount: u64) -> u64 {
    unstake_fee(amount, linear_decay_fee_bps(0)).unwrap()
}

/// Common fixture: registers an app + tag, funds a fresh user's wallet with
/// vote tokens, and stakes `initial_stake` in to create a `StakePosition`.
/// Returns everything a `withdraw_tag_stake` test needs.
fn setup_with_position(
    initial_stake: u64,
    wallet_amount: u64,
) -> (LiteSVM, Env, Pubkey, TagPdas, Keypair, Pubkey, Pubkey) {
    let (mut svm, _deployer, env, app, tag_pdas) = setup_with_tag(APP_ID, TAG_ID);
    let (user, user_token_account) = create_funded_user(&mut svm, &env, wallet_amount);
    let position = derive_stake_position(&env.program_id, &tag_pdas.app_tag_stake, &user.pubkey());

    let ix = stake_tag_ix(
        &env,
        &app,
        &tag_pdas,
        &position,
        &user_token_account,
        &user.pubkey(),
        initial_stake,
    );
    send(&mut svm, ix, &user.pubkey(), &[&user])
        .expect("initial stake_tag must succeed in test setup");

    (svm, env, app, tag_pdas, user, user_token_account, position)
}

/// Even on a full withdrawal (the last stake this `user` holds on this tag),
/// the elapsed=0 1% unstake fee is still charged and paid straight to the
/// admin's token account — the tag-staking mirror of
/// `test_withdraw_vote_full_withdrawal_returns_principal_and_zeroes_position`,
/// same reasoning: no "pool would be empty" waiver.
#[test]
fn test_withdraw_tag_stake_full_withdrawal_returns_principal_and_zeroes_position() {
    let initial_stake = 4_000u64;
    let wallet_amount = 10_000u64;
    let (mut svm, env, app, tag_pdas, user, user_token_account, position) =
        setup_with_position(initial_stake, wallet_amount);

    let ix = withdraw_tag_stake_ix(
        &env,
        &app,
        &tag_pdas,
        &position,
        &user_token_account,
        &user.pubkey(),
        initial_stake,
    );
    send(&mut svm, ix, &user.pubkey(), &[&user]).expect("withdraw_tag_stake transaction failed");

    let fee = fee_at_elapsed_zero(initial_stake);
    assert!(
        fee > 0,
        "test is only meaningful if a nonzero fee was actually charged"
    );

    let position_account = fetch_stake_position(&svm, position);
    assert_eq!(position_account.amount, 0);
    assert_eq!(position_account.reward_debt, 0);

    assert_eq!(
        fetch_app_tag_stake(&svm, tag_pdas.app_tag_stake).stake_amount,
        0
    );
    let app_account = fetch_app(&svm, app);
    assert_eq!(app_account.total_tag_stake, 0);
    // The fee no longer touches the accumulator — it went straight to the
    // admin instead.
    assert_eq!(app_account.tags_acc_reward_per_share, 0);

    assert_eq!(fetch_token_amount(&svm, env.vault), 0);
    assert_eq!(fetch_token_amount(&svm, env.admin_token_account), fee);
    assert_eq!(
        fetch_token_amount(&svm, user_token_account),
        wallet_amount - fee
    );
}

/// A partial withdrawal (`user` still holds stake afterward) at elapsed=0
/// charges the full 1% unstake fee — paid straight to the admin's token
/// account, not redistributed back into `user`'s own remaining position or
/// anyone else's on this tag.
#[test]
fn test_withdraw_tag_stake_partial_withdrawal_leaves_remaining_stake() {
    let initial_stake = 4_000u64;
    let wallet_amount = 10_000u64;
    let (mut svm, env, app, tag_pdas, user, user_token_account, position) =
        setup_with_position(initial_stake, wallet_amount);

    let withdraw_amount = 1_500u64;
    let ix = withdraw_tag_stake_ix(
        &env,
        &app,
        &tag_pdas,
        &position,
        &user_token_account,
        &user.pubkey(),
        withdraw_amount,
    );
    send(&mut svm, ix, &user.pubkey(), &[&user]).expect("withdraw_tag_stake transaction failed");

    let remaining = initial_stake - withdraw_amount;
    let fee = fee_at_elapsed_zero(withdraw_amount);
    let net_withdraw_amount = withdraw_amount - fee;

    assert_eq!(fetch_stake_position(&svm, position).amount, remaining);

    // Both counters stayed in lockstep.
    assert_eq!(
        fetch_app_tag_stake(&svm, tag_pdas.app_tag_stake).stake_amount,
        remaining
    );
    let app_account = fetch_app(&svm, app);
    assert_eq!(app_account.total_tag_stake, remaining);
    // The accumulator is untouched — the fee never goes through it anymore.
    assert_eq!(app_account.tags_acc_reward_per_share, 0);

    // The fee portion of `withdraw_amount` left the vault for the admin,
    // same as the rest of the withdrawal — nothing stays behind.
    assert_eq!(fetch_token_amount(&svm, env.vault), remaining);
    assert_eq!(fetch_token_amount(&svm, env.admin_token_account), fee);
    assert_eq!(
        fetch_token_amount(&svm, user_token_account),
        wallet_amount - initial_stake + net_withdraw_amount
    );
}

#[test]
fn test_withdraw_tag_stake_rejects_zero_amount() {
    let (mut svm, env, app, tag_pdas, user, user_token_account, position) =
        setup_with_position(4_000, 10_000);

    let ix = withdraw_tag_stake_ix(
        &env,
        &app,
        &tag_pdas,
        &position,
        &user_token_account,
        &user.pubkey(),
        0,
    );
    assert!(
        send(&mut svm, ix, &user.pubkey(), &[&user]).is_err(),
        "expected withdraw_tag_stake to reject a zero amount, but it succeeded"
    );
}

#[test]
fn test_withdraw_tag_stake_rejects_amount_exceeding_stake() {
    let initial_stake = 4_000u64;
    let (mut svm, env, app, tag_pdas, user, user_token_account, position) =
        setup_with_position(initial_stake, 10_000);

    let ix = withdraw_tag_stake_ix(
        &env,
        &app,
        &tag_pdas,
        &position,
        &user_token_account,
        &user.pubkey(),
        initial_stake + 1,
    );
    assert!(
        send(&mut svm, ix, &user.pubkey(), &[&user]).is_err(),
        "expected withdraw_tag_stake to reject an over-withdrawal, but it succeeded"
    );

    // Nothing moved: the position, app_tag_stake, and vault are untouched.
    assert_eq!(fetch_stake_position(&svm, position).amount, initial_stake);
    assert_eq!(
        fetch_app_tag_stake(&svm, tag_pdas.app_tag_stake).stake_amount,
        initial_stake
    );
    assert_eq!(fetch_token_amount(&svm, env.vault), initial_stake);
}

/// Exercises the reward-payout CPI leg of `withdraw_tag_stake()` end-to-end
/// on a PARTIAL withdrawal. Unlike the old per-(app, tag) vault design (where
/// TWO DIFFERENT PDAs signed two separate transfers out of two separate
/// vaults in the same instruction), `config` now signs BOTH the
/// pending-reward payout and the returned principal out of the SAME single
/// global vault — so a single before/after vault-balance delta covers both
/// legs at once.
#[test]
fn test_withdraw_tag_stake_pays_out_pending_reward_on_partial_withdrawal() {
    let initial_stake = 1_000u64;
    let wallet_amount = 10_000u64;
    let (mut svm, env, app, tag_pdas, user, user_token_account, position) =
        setup_with_position(initial_stake, wallet_amount);

    // Stand in for `fund_app_rewards` (Tags pool): bump the shared
    // accumulator to 1 reward token per staked token, and add reward funds
    // on top of the vault's existing principal balance so the payout CPI
    // (signed by `config`) has something to actually transfer.
    let acc_reward_per_share = REWARD_PRECISION; // 1.0 reward token per staked token
    set_app_tags_accumulator(&mut svm, app, acc_reward_per_share);
    credit_token_account(&mut svm, env.vault, env.vote_mint, env.config, 50_000);
    let vault_before_withdraw = fetch_token_amount(&svm, env.vault);

    let withdraw_amount = 400u64;

    // settle_pending(1_000, reward_debt=0, acc=1*PRECISION) = 1_000
    let expected_pending = 1_000u64;

    let ix = withdraw_tag_stake_ix(
        &env,
        &app,
        &tag_pdas,
        &position,
        &user_token_account,
        &user.pubkey(),
        withdraw_amount,
    );
    send(&mut svm, ix, &user.pubkey(), &[&user]).expect("withdraw_tag_stake transaction failed");

    let remaining = initial_stake - withdraw_amount;
    let position_account = fetch_stake_position(&svm, position);
    assert_eq!(position_account.amount, remaining);
    // reward_debt_for(remaining, 1*PRECISION) = remaining — checkpointed
    // against the accumulator's value BEFORE this withdrawal's own
    // fee-funding bump (see withdraw_vote's doc comment on why that ordering
    // is correct).
    assert_eq!(position_account.reward_debt, remaining as u128);

    let fee = fee_at_elapsed_zero(withdraw_amount);
    let net_withdraw_amount = withdraw_amount - fee;

    assert_eq!(
        fetch_app_tag_stake(&svm, tag_pdas.app_tag_stake).stake_amount,
        remaining
    );
    let app_account = fetch_app(&svm, app);
    assert_eq!(app_account.total_tag_stake, remaining);
    // The manually-set accumulator is untouched — the fee no longer bumps it.
    assert_eq!(app_account.tags_acc_reward_per_share, acc_reward_per_share);

    // User received the withdrawn principal (net of the unstake fee) and the
    // pending reward, both paid by `config` in the same instruction.
    assert_eq!(
        fetch_token_amount(&svm, user_token_account),
        wallet_amount - initial_stake + net_withdraw_amount + expected_pending
    );
    // The admin received the fee directly.
    assert_eq!(fetch_token_amount(&svm, env.admin_token_account), fee);

    // The single shared vault paid out the reward, the fee (now to the
    // admin), and the net returned principal — nothing stays behind.
    assert_eq!(
        fetch_token_amount(&svm, env.vault),
        vault_before_withdraw - expected_pending - withdraw_amount
    );
}

/// Regression test for a critical fund-drain vulnerability (see the matching
/// test in `test_stake_tag.rs` for the full exploit writeup): without the
/// `has_one = app` check on `WithdrawTagStake::app_tag_stake`, an attacker
/// with their OWN legitimate (app, app_tag_stake, position) could call
/// `withdraw_tag_stake` passing their own `app_tag_stake`/`position` alongside
/// a victim's well-funded `app`. The pending-reward leg would then settle
/// against the VICTIM's real `tags_acc_reward_per_share`, and BOTH the reward
/// payout and the returned "principal" would be signed by `config` out of the
/// single global vault — so with the constraint removed, a successful attack
/// would pay the attacker their own already-legitimate principal back a second
/// time PLUS the victim's reward, out of funds that were never theirs to draw
/// against. Asserts the call is rejected with `AppTagStakeMismatch`
/// specifically.
#[test]
fn test_withdraw_tag_stake_rejects_mismatched_app_and_app_tag_stake() {
    let (mut svm, deployer, env, victim_app, _victim_tag_pdas) = setup_with_tag(APP_ID, TAG_ID);

    // The attacker's own, entirely independent app + tag.
    let (attacker_app, attacker_tag_pdas) = register_app_and_tag(
        &mut svm,
        &env.program_id,
        &deployer,
        "cid_attacker_app_0000002",
        "attacker_tag",
    );

    let (user, user_token_account) = create_funded_user(&mut svm, &env, 10_000);
    let position = derive_stake_position(
        &env.program_id,
        &attacker_tag_pdas.app_tag_stake,
        &user.pubkey(),
    );

    // A legitimate stake under the attacker's OWN, correctly-matched
    // (app, app_tag_stake) pair — establishes a real position to attempt
    // withdrawing against.
    let stake_amount = 1_000u64;
    let ix = stake_tag_ix(
        &env,
        &attacker_app,
        &attacker_tag_pdas,
        &position,
        &user_token_account,
        &user.pubkey(),
        stake_amount,
    );
    send(&mut svm, ix, &user.pubkey(), &[&user])
        .expect("legitimate stake_tag under the attacker's own app must succeed in test setup");

    // Now attempt to withdraw, but pass the VICTIM's `app` alongside the
    // attacker's own `app_tag_stake`/`position`.
    let ix = withdraw_tag_stake_ix(
        &env,
        &victim_app,
        &attacker_tag_pdas,
        &position,
        &user_token_account,
        &user.pubkey(),
        stake_amount,
    );
    let err = send(&mut svm, ix, &user.pubkey(), &[&user]).expect_err(
        "expected withdraw_tag_stake to reject a mismatched (app, app_tag_stake) pair, but it succeeded",
    );
    let logs = err.meta.pretty_logs();
    assert!(
        logs.contains("AppTagStakeMismatch"),
        "expected the rejection to be AppTagStakeMismatch specifically, got logs: {logs}"
    );

    // Nothing moved: the victim's pool and the attacker's own position are
    // both untouched.
    assert_eq!(fetch_app(&svm, victim_app).total_tag_stake, 0);
    assert_eq!(fetch_stake_position(&svm, position).amount, stake_amount);
}

/// Once `UNSTAKE_FEE_DECAY_SECONDS` (a week) has elapsed since a position's
/// `staked_at` checkpoint, the fee is exactly 0 — the tag-staking mirror of
/// `test_withdraw_vote_fee_decays_to_zero_after_the_decay_window`. A PARTIAL
/// withdrawal (leaves stake behind, so this is genuinely the time-decay
/// path, not the "last staker" waiver).
#[test]
fn test_withdraw_tag_stake_fee_decays_to_zero_after_the_decay_window() {
    let initial_stake = 4_000u64;
    let wallet_amount = 10_000u64;
    let (mut svm, env, app, tag_pdas, user, user_token_account, position) =
        setup_with_position(initial_stake, wallet_amount);

    warp_forward(&mut svm, UNSTAKE_FEE_DECAY_SECONDS);

    let withdraw_amount = 1_500u64;
    let ix = withdraw_tag_stake_ix(
        &env,
        &app,
        &tag_pdas,
        &position,
        &user_token_account,
        &user.pubkey(),
        withdraw_amount,
    );
    send(&mut svm, ix, &user.pubkey(), &[&user]).expect("withdraw_tag_stake transaction failed");

    let remaining = initial_stake - withdraw_amount;
    assert_eq!(fetch_stake_position(&svm, position).amount, remaining);

    let app_account = fetch_app(&svm, app);
    assert_eq!(app_account.total_tag_stake, remaining);
    assert_eq!(app_account.tags_acc_reward_per_share, 0);

    // Full withdraw_amount returned, fee-free — nothing paid to the admin.
    assert_eq!(fetch_token_amount(&svm, env.vault), remaining);
    assert_eq!(fetch_token_amount(&svm, env.admin_token_account), 0);
    assert_eq!(
        fetch_token_amount(&svm, user_token_account),
        wallet_amount - initial_stake + withdraw_amount
    );
}

/// The unstake fee is a straight treasury skim, paid directly to the admin's
/// token account — NOT redistributed to whoever else remains staked on the
/// same tag. The tag-staking mirror of
/// `test_withdraw_vote_fee_is_paid_directly_to_admin_not_other_stakers`: user
/// A fully exits and pays a fee that lands in `admin_token_account`; user B,
/// who never withdraws, is completely unaffected.
#[test]
fn test_withdraw_tag_stake_fee_is_paid_directly_to_admin_not_other_stakers() {
    let (mut svm, _deployer, env, app, tag_pdas) = setup_with_tag(APP_ID, TAG_ID);

    let (user_a, a_token_account) = create_funded_user(&mut svm, &env, 10_000);
    let a_position =
        derive_stake_position(&env.program_id, &tag_pdas.app_tag_stake, &user_a.pubkey());
    let (user_b, b_token_account) = create_funded_user(&mut svm, &env, 10_000);
    let b_position =
        derive_stake_position(&env.program_id, &tag_pdas.app_tag_stake, &user_b.pubkey());

    let a_amount = 4_000u64;
    let b_amount = 5_000u64;
    for (position, token_account, user, amount) in [
        (a_position, a_token_account, &user_a, a_amount),
        (b_position, b_token_account, &user_b, b_amount),
    ] {
        let ix = stake_tag_ix(
            &env,
            &app,
            &tag_pdas,
            &position,
            &token_account,
            &user.pubkey(),
            amount,
        );
        send(&mut svm, ix, &user.pubkey(), &[user]).expect("stake_tag must succeed in test setup");
    }

    // User A fully exits at elapsed=0 (full 1% fee); User B stays staked
    // throughout and never touches their own position.
    let ix = withdraw_tag_stake_ix(
        &env,
        &app,
        &tag_pdas,
        &a_position,
        &a_token_account,
        &user_a.pubkey(),
        a_amount,
    );
    send(&mut svm, ix, &user_a.pubkey(), &[&user_a]).expect("withdraw_tag_stake must succeed");

    let fee = fee_at_elapsed_zero(a_amount);
    assert!(
        fee > 0,
        "test is only meaningful if a nonzero fee was actually charged"
    );

    // The fee landed directly in the admin's token account.
    assert_eq!(fetch_token_amount(&svm, env.admin_token_account), fee);

    let app_account = fetch_app(&svm, app);
    assert_eq!(app_account.total_tag_stake, b_amount);
    // The shared accumulator never moved — A's fee never touched it.
    assert_eq!(app_account.tags_acc_reward_per_share, 0);

    // User B's position is byte-for-byte what it was after their own stake:
    // no pending reward accrued from A's fee, because there is none.
    let b_position_account = fetch_stake_position(&svm, b_position);
    assert_eq!(b_position_account.amount, b_amount);
    assert_eq!(b_position_account.reward_debt, 0);
    assert_eq!(
        fetch_token_amount(&svm, b_token_account),
        10_000 - b_amount,
        "B's balance is untouched by A's withdrawal"
    );
}
