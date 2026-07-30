mod common;

use {
    anchor_lang::solana_program::pubkey::Pubkey,
    common::{
        claim_vote_reward_ix, create_funded_user, derive_vote_position, fetch_token_amount,
        fetch_vote_position, fund_rewards, send, setup_with_app, vote_ix, Env,
    },
    litesvm::LiteSVM,
    nebulous_world::RewardPool,
    solana_keypair::Keypair,
    solana_signer::Signer,
};

const APP_ID: &str = "cid_cvote_test_app_000001";

/// Common fixture for every test below: registers an app, funds a fresh
/// user's wallet, votes `stake` in to create a `VotePosition`, then funds
/// the vote pool for real via `fund_app_rewards` with `fund_amount` — so the
/// accumulator and the single global vault's balance are both genuinely
/// produced by the two instructions under test, not hand-poked into the
/// account like the reward-payout tests in `test_vote.rs`/
/// `test_withdraw_vote.rs` have to do (those exist precisely so that at
/// least one test per instruction doesn't depend on `fund_app_rewards`
/// having already been migrated/working).
fn setup_voted_and_funded(
    stake: u64,
    wallet_amount: u64,
    fund_amount: u64,
) -> (LiteSVM, Env, Pubkey, Keypair, Pubkey, Pubkey) {
    let (mut svm, deployer, env, app) = setup_with_app(APP_ID);
    let (user, user_token_account) = create_funded_user(&mut svm, &env, wallet_amount);
    let position = derive_vote_position(&env.program_id, &app, &user.pubkey());

    let ix = vote_ix(
        &env,
        &app,
        &position,
        &user_token_account,
        &user.pubkey(),
        stake,
    );
    send(&mut svm, ix, &user.pubkey(), &[&user]).expect("setup vote must succeed");

    fund_rewards(
        &mut svm,
        &env,
        &deployer,
        &app,
        RewardPool::Vote,
        fund_amount,
    );

    (svm, env, app, user, user_token_account, position)
}

#[test]
fn test_claim_vote_reward_pays_out_pending_and_leaves_principal_untouched() {
    let stake = 1_000u64;
    let wallet_amount = 10_000u64;
    let fund_amount = 2_000u64;
    let (mut svm, env, app, user, user_token_account, position) =
        setup_voted_and_funded(stake, wallet_amount, fund_amount);

    // acc = 2_000 * PRECISION / 1_000 = 2 * PRECISION.
    // pending = settle_pending(1_000, reward_debt=0, acc=2*PRECISION) = 2_000.
    let expected_pending = 2_000u64;

    // The single global vault holds both the staked principal and the
    // freshly funded reward pool at this point.
    let vault_before_claim = stake + fund_amount;
    assert_eq!(fetch_token_amount(&svm, env.vault), vault_before_claim);

    let ix = claim_vote_reward_ix(&env, &app, &position, &user_token_account, &user.pubkey());
    send(&mut svm, ix, &user.pubkey(), &[&user]).expect("claim_vote_reward transaction failed");

    // Principal is untouched.
    let position_account = fetch_vote_position(&svm, position);
    assert_eq!(position_account.amount, stake);
    // reward_debt re-checkpointed to reward_debt_for(1_000, 2*PRECISION) = 2_000.
    assert_eq!(position_account.reward_debt, expected_pending as u128);

    // Reward actually landed: user started with (wallet_amount - stake)
    // after voting, then received `expected_pending`.
    assert_eq!(
        fetch_token_amount(&svm, user_token_account),
        wallet_amount - stake + expected_pending
    );
    // The vault paid out exactly `expected_pending`, leaving the staked
    // principal (a claim never touches principal).
    assert_eq!(
        fetch_token_amount(&svm, env.vault),
        vault_before_claim - expected_pending
    );
}

#[test]
fn test_claim_vote_reward_twice_pays_nothing_extra_second_time() {
    let stake = 1_000u64;
    let wallet_amount = 10_000u64;
    let fund_amount = 2_000u64;
    let (mut svm, env, app, user, user_token_account, position) =
        setup_voted_and_funded(stake, wallet_amount, fund_amount);

    let ix = claim_vote_reward_ix(&env, &app, &position, &user_token_account, &user.pubkey());
    send(&mut svm, ix, &user.pubkey(), &[&user]).expect("first claim must succeed");

    let balance_after_first_claim = fetch_token_amount(&svm, user_token_account);
    let vault_after_first_claim = fetch_token_amount(&svm, env.vault);
    let position_after_first_claim = fetch_vote_position(&svm, position);

    // Claim again immediately, with no intervening vote()/fund_app_rewards()
    // call — there is genuinely nothing new to pay out, since reward_debt
    // was already checkpointed against the current (unchanged) accumulator.
    // `expire_blockhash` only forces a distinct transaction signature (the
    // first claim's tx would otherwise be byte-for-byte identical and get
    // rejected by litesvm as an `AlreadyProcessed` duplicate) — it has no
    // bearing on the actual reward math being tested here.
    svm.expire_blockhash();
    let ix = claim_vote_reward_ix(&env, &app, &position, &user_token_account, &user.pubkey());
    send(&mut svm, ix, &user.pubkey(), &[&user])
        .expect("second claim_vote_reward transaction failed");

    // Nothing extra moved: user balance, vault balance, and position are all
    // byte-for-byte identical to right after the first claim.
    assert_eq!(
        fetch_token_amount(&svm, user_token_account),
        balance_after_first_claim
    );
    assert_eq!(fetch_token_amount(&svm, env.vault), vault_after_first_claim);
    let position_after_second_claim = fetch_vote_position(&svm, position);
    assert_eq!(
        position_after_second_claim.amount,
        position_after_first_claim.amount
    );
    assert_eq!(
        position_after_second_claim.reward_debt,
        position_after_first_claim.reward_debt
    );
}

/// End-to-end: vote -> fund_app_rewards -> claim_vote_reward, with
/// hand-verified numbers throughout.
///
/// stake = 2_500, fund_amount = 10_000
/// acc_reward_per_share = 10_000 * PRECISION / 2_500 = 4 * PRECISION
/// pending = settle_pending(2_500, 0, 4*PRECISION) = 2_500 * 4 = 10_000
/// (the entire funded amount, since this user holds 100% of the stake)
#[test]
fn test_vote_fund_claim_end_to_end_exact_payout() {
    let stake = 2_500u64;
    let wallet_amount = 50_000u64;
    let fund_amount = 10_000u64;
    let (mut svm, env, app, user, user_token_account, position) =
        setup_voted_and_funded(stake, wallet_amount, fund_amount);

    let expected_pending = 10_000u64;
    assert_eq!(
        expected_pending, fund_amount,
        "sole staker must receive the entire funded pool"
    );

    let ix = claim_vote_reward_ix(&env, &app, &position, &user_token_account, &user.pubkey());
    send(&mut svm, ix, &user.pubkey(), &[&user]).expect("claim must succeed");

    let position_account = fetch_vote_position(&svm, position);
    assert_eq!(position_account.amount, stake); // untouched
    assert_eq!(position_account.reward_debt, expected_pending as u128);

    assert_eq!(
        fetch_token_amount(&svm, user_token_account),
        wallet_amount - stake + expected_pending
    );
    // The single global vault held (stake + fund_amount) before this claim;
    // the sole staker claimed the entire funded pool, leaving only the
    // staked principal behind.
    assert_eq!(fetch_token_amount(&svm, env.vault), stake);
}
