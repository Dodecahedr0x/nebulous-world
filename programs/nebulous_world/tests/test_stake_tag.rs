mod common;

use {
    common::{
        create_funded_user, credit_token_account, derive_stake_position, fetch_app,
        fetch_app_tag_stake, fetch_stake_position, fetch_token_amount, register_app_and_tag, send,
        set_app_tags_accumulator, setup_with_tag, stake_tag_ix, warp_forward,
    },
    nebulous_world::constants::REWARD_PRECISION,
    solana_clock::Clock,
    solana_signer::Signer,
};

const APP_ID: &str = "cid_stake_test_app_000001";
const TAG_ID: &str = "defi";

#[test]
fn test_stake_tag_locks_principal_and_creates_position() {
    let (mut svm, _deployer, env, app, tag_pdas) = setup_with_tag(APP_ID, TAG_ID);
    let (user, user_token_account) = create_funded_user(&mut svm, &env, 10_000);
    let position = derive_stake_position(&env.program_id, &tag_pdas.app_tag_stake, &user.pubkey());

    let amount = 4_000u64;
    let ix = stake_tag_ix(
        &env,
        &app,
        &tag_pdas,
        &position,
        &user_token_account,
        &user.pubkey(),
        amount,
    );
    send(&mut svm, ix, &user.pubkey(), &[&user]).expect("stake_tag transaction failed");

    let position_account = fetch_stake_position(&svm, position);
    assert_eq!(position_account.owner, user.pubkey());
    assert_eq!(position_account.amount, amount);
    assert_eq!(position_account.reward_debt, 0);
    // New field: the position now records its own derivation seed.
    assert_eq!(position_account.app_tag_stake, tag_pdas.app_tag_stake);
    // A brand-new position's staked_at is exactly `now` (weighted_avg_timestamp
    // collapses to `now` when the old amount is 0 — see unstake_fee.rs).
    assert_eq!(
        position_account.staked_at,
        svm.get_sysvar::<Clock>().unix_timestamp
    );

    // Both counters moved in lockstep.
    let app_tag_stake_account = fetch_app_tag_stake(&svm, tag_pdas.app_tag_stake);
    assert_eq!(app_tag_stake_account.stake_amount, amount);
    assert_eq!(app_tag_stake_account.app, app);
    assert_eq!(app_tag_stake_account.tag, tag_pdas.tag);
    assert_eq!(fetch_app(&svm, app).total_tag_stake, amount);

    assert_eq!(fetch_token_amount(&svm, env.vault), amount);
    assert_eq!(
        fetch_token_amount(&svm, user_token_account),
        10_000 - amount
    );
}

#[test]
fn test_stake_tag_rejects_zero_amount() {
    let (mut svm, _deployer, env, app, tag_pdas) = setup_with_tag(APP_ID, TAG_ID);
    let (user, user_token_account) = create_funded_user(&mut svm, &env, 10_000);
    let position = derive_stake_position(&env.program_id, &tag_pdas.app_tag_stake, &user.pubkey());

    let ix = stake_tag_ix(
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
        "expected stake_tag to reject a zero amount, but it succeeded"
    );
}

#[test]
fn test_stake_tag_accumulates_across_two_deposits() {
    let (mut svm, _deployer, env, app, tag_pdas) = setup_with_tag(APP_ID, TAG_ID);
    let (user, user_token_account) = create_funded_user(&mut svm, &env, 10_000);
    let position = derive_stake_position(&env.program_id, &tag_pdas.app_tag_stake, &user.pubkey());

    for amount in [1_000u64, 2_500u64] {
        let ix = stake_tag_ix(
            &env,
            &app,
            &tag_pdas,
            &position,
            &user_token_account,
            &user.pubkey(),
            amount,
        );
        send(&mut svm, ix, &user.pubkey(), &[&user]).expect("stake_tag transaction failed");
    }

    assert_eq!(fetch_stake_position(&svm, position).amount, 3_500);
    assert_eq!(
        fetch_app_tag_stake(&svm, tag_pdas.app_tag_stake).stake_amount,
        3_500
    );
    assert_eq!(fetch_app(&svm, app).total_tag_stake, 3_500);
}

/// `staked_at` is a size-weighted average across deposits (see
/// `unstake_fee::weighted_avg_timestamp`) — the tag-staking mirror of
/// `test_vote_staked_at_is_a_weighted_average_across_deposits` in
/// `test_vote.rs`; see that test's doc comment for the full rationale
/// (a "first deposit only" checkpoint would let a stale, fully-decayed
/// timestamp cover an arbitrarily large later top-up fee-free).
#[test]
fn test_stake_tag_staked_at_is_a_weighted_average_across_deposits() {
    let (mut svm, _deployer, env, app, tag_pdas) = setup_with_tag(APP_ID, TAG_ID);
    let (user, user_token_account) = create_funded_user(&mut svm, &env, 1_000_100);
    let position = derive_stake_position(&env.program_id, &tag_pdas.app_tag_stake, &user.pubkey());

    let first_amount = 100u64;
    let ix = stake_tag_ix(
        &env,
        &app,
        &tag_pdas,
        &position,
        &user_token_account,
        &user.pubkey(),
        first_amount,
    );
    send(&mut svm, ix, &user.pubkey(), &[&user]).expect("first stake_tag must succeed");
    let staked_at_after_first = fetch_stake_position(&svm, position).staked_at;

    let elapsed = 7 * 24 * 60 * 60;
    warp_forward(&mut svm, elapsed);
    let second_amount = 1_000_000u64;
    let ix = stake_tag_ix(
        &env,
        &app,
        &tag_pdas,
        &position,
        &user_token_account,
        &user.pubkey(),
        second_amount,
    );
    send(&mut svm, ix, &user.pubkey(), &[&user]).expect("second stake_tag must succeed");

    let position_account = fetch_stake_position(&svm, position);
    assert_eq!(position_account.amount, first_amount + second_amount);

    let expected_staked_at = nebulous_world::unstake_fee::weighted_avg_timestamp(
        staked_at_after_first,
        first_amount,
        staked_at_after_first + elapsed,
        second_amount,
    );
    assert_eq!(position_account.staked_at, expected_staked_at);
    let moved = position_account.staked_at - staked_at_after_first;
    assert!(
        moved >= elapsed * 99 / 100,
        "a 10_000x top-up should move staked_at at least 99% of the way to the top-up time, \
         moved {moved}s of {elapsed}s"
    );
}

/// Exercises the reward-payout CPI leg of `stake_tag()` end-to-end — the
/// highest-risk path (`config`, the single authority for the whole shared
/// vault, signing a transfer out of it), which every other test above never
/// touches since they all run with `tags_acc_reward_per_share == 0`. This
/// test stakes once to create a nonzero position, manually bumps the app's
/// tags accumulator (standing in for `fund_app_rewards` targeting the Tags
/// pool) and adds reward funds on top of the vault's existing principal
/// balance, then stakes again and asserts the pending reward actually lands
/// in the user's wallet and the position's `reward_debt` checkpoints to the
/// new accumulator value.
#[test]
fn test_stake_tag_pays_out_pending_reward_on_second_stake() {
    let (mut svm, _deployer, env, app, tag_pdas) = setup_with_tag(APP_ID, TAG_ID);
    let (user, user_token_account) = create_funded_user(&mut svm, &env, 10_000);
    let position = derive_stake_position(&env.program_id, &tag_pdas.app_tag_stake, &user.pubkey());

    let first_amount = 1_000u64;
    let ix = stake_tag_ix(
        &env,
        &app,
        &tag_pdas,
        &position,
        &user_token_account,
        &user.pubkey(),
        first_amount,
    );
    send(&mut svm, ix, &user.pubkey(), &[&user]).expect("first stake_tag must succeed in setup");

    // Stand in for `fund_app_rewards` (Tags pool): bump the shared
    // accumulator to 1 reward token per staked token, and add reward funds
    // on top of whatever the SHARED global vault already holds (the first
    // deposit's principal) so the payout CPI has something to transfer.
    let acc_reward_per_share = REWARD_PRECISION; // 1.0 reward token per staked token
    set_app_tags_accumulator(&mut svm, app, acc_reward_per_share);
    credit_token_account(&mut svm, env.vault, env.vote_mint, env.config, 50_000);
    let vault_before_second_stake = fetch_token_amount(&svm, env.vault);

    // settle_pending(1_000, reward_debt=0, acc=1*PRECISION) = 1_000.
    let expected_pending = 1_000u64;

    let second_amount = 500u64;
    let ix = stake_tag_ix(
        &env,
        &app,
        &tag_pdas,
        &position,
        &user_token_account,
        &user.pubkey(),
        second_amount,
    );
    send(&mut svm, ix, &user.pubkey(), &[&user]).expect("second stake_tag transaction failed");

    let position_account = fetch_stake_position(&svm, position);
    assert_eq!(position_account.amount, first_amount + second_amount);
    // reward_debt_for(1_500, 1*PRECISION) = 1_500.
    assert_eq!(position_account.reward_debt, 1_500);

    assert_eq!(
        fetch_app_tag_stake(&svm, tag_pdas.app_tag_stake).stake_amount,
        first_amount + second_amount
    );
    assert_eq!(
        fetch_app(&svm, app).total_tag_stake,
        first_amount + second_amount
    );

    // The reward actually landed in the user's wallet, signed by `config` —
    // started with 10_000, paid principal deposits, received
    // `expected_pending` back as reward.
    assert_eq!(
        fetch_token_amount(&svm, user_token_account),
        10_000 - first_amount - second_amount + expected_pending
    );

    // The single shared vault moved by exactly (principal in) - (reward
    // out) on top of whatever it held going into this transaction.
    assert_eq!(
        fetch_token_amount(&svm, env.vault),
        vault_before_second_stake - expected_pending + second_amount
    );
}

/// Regression test for a critical fund-drain vulnerability: without the
/// `has_one = app` check on `StakeTag::app_tag_stake`, each of
/// `app`/`app_tag_stake`'s seeds/bump constraints only proves internal
/// self-consistency — NEITHER proves the two accounts belong together. An
/// attacker could permissionlessly create their OWN (app, app_tag_stake) pair
/// via `init_app`/`suggest_tag`, then call `stake_tag` passing THEIR
/// `app_tag_stake` alongside a victim's well-funded `app`, crediting the
/// attacker's position against the victim's
/// `total_tag_stake`/`tags_acc_reward_per_share` — a permissionless,
/// capital-light path to draining the single global vault (shared by every app
/// in the program, not just the victim's own) once its accumulator advances
/// from legitimate funding.
///
/// This test builds exactly that mismatched pair (a second, independent
/// app+tag standing in for the "attacker's own") and asserts `stake_tag`
/// rejects it with `AppTagStakeMismatch`, not merely "some error".
#[test]
fn test_stake_tag_rejects_mismatched_app_and_app_tag_stake() {
    let (mut svm, deployer, env, victim_app, _victim_tag_pdas) = setup_with_tag(APP_ID, TAG_ID);

    // The attacker's own, entirely independent app + tag.
    let (_attacker_app, attacker_tag_pdas) = register_app_and_tag(
        &mut svm,
        &env.program_id,
        &deployer,
        "cid_attacker_app_0000001",
        "attacker_tag",
    );

    let (user, user_token_account) = create_funded_user(&mut svm, &env, 10_000);

    // Position PDA derived off the ATTACKER's app_tag_stake (matching what
    // `stake_tag`'s own `position` seeds constraint expects for this
    // `app_tag_stake`), but the instruction passes the VICTIM's `app`.
    let position = derive_stake_position(
        &env.program_id,
        &attacker_tag_pdas.app_tag_stake,
        &user.pubkey(),
    );

    let ix = stake_tag_ix(
        &env,
        &victim_app,
        &attacker_tag_pdas,
        &position,
        &user_token_account,
        &user.pubkey(),
        1_000,
    );
    let err = send(&mut svm, ix, &user.pubkey(), &[&user]).expect_err(
        "expected stake_tag to reject a mismatched (app, app_tag_stake) pair, but it succeeded",
    );
    let logs = err.meta.pretty_logs();
    assert!(
        logs.contains("AppTagStakeMismatch"),
        "expected the rejection to be AppTagStakeMismatch specifically, got logs: {logs}"
    );

    // Nothing moved on the victim's side.
    assert_eq!(fetch_app(&svm, victim_app).total_tag_stake, 0);
}
