mod common;

use {
    anchor_lang::solana_program::pubkey::Pubkey,
    common::{
        claim_tag_reward_ix, create_funded_user, derive_stake_position, fetch_app,
        fetch_app_tag_stake, fetch_stake_position, fetch_token_amount, fund_rewards,
        register_app_and_tag, register_tag, send, setup_with_tag, stake_tag_ix, Env, TagPdas,
    },
    litesvm::LiteSVM,
    nebulous_world::RewardPool,
    solana_keypair::Keypair,
    solana_signer::Signer,
};

/// `setup()`'s app_id, hoisted to a shared constant so the shared-accumulator
/// test below can suggest an ADDITIONAL tag onto that same app — `suggest_tag`
/// takes the app_id string, not just the derived PDA.
const APP_ID: &str = "cid_ctag_test_app_000001";
const TAG_ID: &str = "defi";

fn setup() -> (LiteSVM, Keypair, Env, Pubkey, TagPdas) {
    setup_with_tag(APP_ID, TAG_ID)
}

/// Common fixture for every test below: registers an app + tag, funds a
/// fresh user's wallet, stakes `stake` in to create a `StakePosition`, then
/// funds the TAGS pool for real via `fund_app_rewards` with `fund_amount` —
/// so the accumulator and the global vault's balance are both genuinely
/// produced by the two instructions under test, not hand-poked into the
/// account.
fn setup_staked_and_funded(
    stake: u64,
    wallet_amount: u64,
    fund_amount: u64,
) -> (LiteSVM, Env, Pubkey, TagPdas, Keypair, Pubkey, Pubkey) {
    let (mut svm, deployer, env, app, tag_pdas) = setup();

    let (user, user_token_account) = create_funded_user(&mut svm, &env, wallet_amount);
    let position = derive_stake_position(&env.program_id, &tag_pdas.app_tag_stake, &user.pubkey());
    let ix = stake_tag_ix(
        &env,
        &app,
        &tag_pdas,
        &position,
        &user_token_account,
        &user.pubkey(),
        stake,
    );
    send(&mut svm, ix, &user.pubkey(), &[&user]).expect("setup stake_tag must succeed");

    fund_rewards(
        &mut svm,
        &env,
        &deployer,
        &app,
        RewardPool::Tags,
        fund_amount,
    );

    (svm, env, app, tag_pdas, user, user_token_account, position)
}

#[test]
fn test_claim_tag_reward_pays_out_pending_and_leaves_principal_untouched() {
    let stake = 1_000u64;
    let wallet_amount = 10_000u64;
    let fund_amount = 2_000u64;
    let (mut svm, env, app, tag_pdas, user, user_token_account, position) =
        setup_staked_and_funded(stake, wallet_amount, fund_amount);

    // acc = 2_000 * PRECISION / 1_000 = 2 * PRECISION.
    // pending = settle_pending(1_000, reward_debt=0, acc=2*PRECISION) = 2_000.
    let expected_pending = 2_000u64;

    let ix = claim_tag_reward_ix(
        &env,
        &app,
        &tag_pdas,
        &position,
        &user_token_account,
        &user.pubkey(),
    );
    send(&mut svm, ix, &user.pubkey(), &[&user]).expect("claim_tag_reward transaction failed");

    // Principal is untouched.
    let position_account = fetch_stake_position(&svm, position);
    assert_eq!(position_account.amount, stake);
    // reward_debt re-checkpointed to reward_debt_for(1_000, 2*PRECISION) = 2_000.
    assert_eq!(position_account.reward_debt, expected_pending as u128);

    // Reward actually landed: user started with (wallet_amount - stake)
    // after staking, then received `expected_pending`.
    assert_eq!(
        fetch_token_amount(&svm, user_token_account),
        wallet_amount - stake + expected_pending
    );
    // The single global vault holds the staked principal (`stake`) plus the
    // funded round (`fund_amount`), minus whatever was just claimed out of
    // it — unlike the old dedicated `tags_reward_vault`, this vault also
    // custodies the stake principal, so the balance check must account for
    // both.
    assert_eq!(
        fetch_token_amount(&svm, env.vault),
        stake + fund_amount - expected_pending
    );
    // app_tag_stake completely untouched by a claim.
    assert_eq!(
        fetch_app_tag_stake(&svm, tag_pdas.app_tag_stake).stake_amount,
        stake
    );
}

#[test]
fn test_claim_tag_reward_twice_pays_nothing_extra_second_time() {
    let stake = 1_000u64;
    let wallet_amount = 10_000u64;
    let fund_amount = 2_000u64;
    let (mut svm, env, app, tag_pdas, user, user_token_account, position) =
        setup_staked_and_funded(stake, wallet_amount, fund_amount);

    let ix = claim_tag_reward_ix(
        &env,
        &app,
        &tag_pdas,
        &position,
        &user_token_account,
        &user.pubkey(),
    );
    send(&mut svm, ix, &user.pubkey(), &[&user]).expect("first claim must succeed");

    let balance_after_first_claim = fetch_token_amount(&svm, user_token_account);
    let vault_after_first_claim = fetch_token_amount(&svm, env.vault);
    let position_after_first_claim = fetch_stake_position(&svm, position);

    // Claim again immediately, with no intervening stake_tag()/
    // fund_app_rewards() call — there is genuinely nothing new to pay out.
    // `expire_blockhash` only forces a distinct transaction signature (the
    // first claim's tx would otherwise be byte-for-byte identical and get
    // rejected by litesvm as an `AlreadyProcessed` duplicate).
    svm.expire_blockhash();
    let ix = claim_tag_reward_ix(
        &env,
        &app,
        &tag_pdas,
        &position,
        &user_token_account,
        &user.pubkey(),
    );
    send(&mut svm, ix, &user.pubkey(), &[&user])
        .expect("second claim_tag_reward transaction failed");

    // Nothing extra moved: user balance, vault balance, and position are all
    // byte-for-byte identical to right after the first claim.
    assert_eq!(
        fetch_token_amount(&svm, user_token_account),
        balance_after_first_claim
    );
    assert_eq!(fetch_token_amount(&svm, env.vault), vault_after_first_claim);
    let position_after_second_claim = fetch_stake_position(&svm, position);
    assert_eq!(
        position_after_second_claim.amount,
        position_after_first_claim.amount
    );
    assert_eq!(
        position_after_second_claim.reward_debt,
        position_after_first_claim.reward_debt
    );
}

#[test]
fn test_claim_tag_reward_zero_pending_is_a_harmless_no_op() {
    // Stake in, but never fund the tags pool: pending is genuinely 0.
    let stake = 1_000u64;
    let wallet_amount = 10_000u64;
    let (mut svm, _deployer, env, app, tag_pdas) = setup();

    let (user, user_token_account) = create_funded_user(&mut svm, &env, wallet_amount);
    let position = derive_stake_position(&env.program_id, &tag_pdas.app_tag_stake, &user.pubkey());
    let ix = stake_tag_ix(
        &env,
        &app,
        &tag_pdas,
        &position,
        &user_token_account,
        &user.pubkey(),
        stake,
    );
    send(&mut svm, ix, &user.pubkey(), &[&user]).expect("setup stake_tag must succeed");

    let ix = claim_tag_reward_ix(
        &env,
        &app,
        &tag_pdas,
        &position,
        &user_token_account,
        &user.pubkey(),
    );
    send(&mut svm, ix, &user.pubkey(), &[&user])
        .expect("zero-pending claim_tag_reward must succeed as a no-op");

    let position_account = fetch_stake_position(&svm, position);
    assert_eq!(position_account.amount, stake);
    assert_eq!(position_account.reward_debt, 0);
    assert_eq!(
        fetch_token_amount(&svm, user_token_account),
        wallet_amount - stake
    );
}

/// Regression test for a critical fund-drain vulnerability (see the matching
/// tests in `test_stake_tag.rs`/`test_withdraw_tag_stake.rs` for the full
/// exploit writeup): without the `has_one = app` check on
/// `ClaimTagReward::app_tag_stake`, an attacker with their OWN legitimate
/// (app, app_tag_stake, position) could call `claim_tag_reward` passing their
/// own `app_tag_stake`/`position` alongside a victim's well-funded `app`. The
/// claim would then settle against the VICTIM's real
/// `tags_acc_reward_per_share` and pay out of the single shared vault, which
/// also custodies the victim's real funded reward round. Asserts the call is
/// rejected with `AppTagStakeMismatch` specifically.
#[test]
fn test_claim_tag_reward_rejects_mismatched_app_and_app_tag_stake() {
    let (mut svm, deployer, env, victim_app, victim_tag_pdas) = setup();

    // Fund the VICTIM's tags reward pool for real, so there's something
    // juicy to try to steal. Needs a staker on the victim's own tag first,
    // since `fund_app_rewards` rejects funding an empty pool.
    let (victim_staker, victim_staker_token_account) = create_funded_user(&mut svm, &env, 10_000);
    let victim_position = derive_stake_position(
        &env.program_id,
        &victim_tag_pdas.app_tag_stake,
        &victim_staker.pubkey(),
    );
    let ix = stake_tag_ix(
        &env,
        &victim_app,
        &victim_tag_pdas,
        &victim_position,
        &victim_staker_token_account,
        &victim_staker.pubkey(),
        1_000,
    );
    send(&mut svm, ix, &victim_staker.pubkey(), &[&victim_staker])
        .expect("victim's own stake_tag must succeed in test setup");

    fund_rewards(
        &mut svm,
        &env,
        &deployer,
        &victim_app,
        RewardPool::Tags,
        50_000,
    );

    // The attacker's own, entirely independent app + tag.
    let (attacker_app, attacker_tag_pdas) = register_app_and_tag(
        &mut svm,
        &env.program_id,
        &deployer,
        "cid_attacker_app_0000003",
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
    // claiming against.
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

    // Capture the shared vault's balance right before the attack attempt,
    // so "nothing moved" can be asserted as a before/after delta rather than
    // an absolute number — the vault is shared by every app in the program,
    // so its absolute balance at this point already includes the victim's
    // stake+funding AND the attacker's own legitimate stake above.
    let vault_before_attack = fetch_token_amount(&svm, env.vault);

    // Now attempt to claim, but pass the VICTIM's `app` alongside the
    // attacker's own `app_tag_stake`/`position`.
    let ix = claim_tag_reward_ix(
        &env,
        &victim_app,
        &attacker_tag_pdas,
        &position,
        &user_token_account,
        &user.pubkey(),
    );
    let err = send(&mut svm, ix, &user.pubkey(), &[&user]).expect_err(
        "expected claim_tag_reward to reject a mismatched (app, app_tag_stake) pair, but it succeeded",
    );
    let logs = err.meta.pretty_logs();
    assert!(
        logs.contains("AppTagStakeMismatch"),
        "expected the rejection to be AppTagStakeMismatch specifically, got logs: {logs}"
    );

    // Nothing moved: the shared vault and the attacker's own position are
    // both untouched.
    assert_eq!(fetch_token_amount(&svm, env.vault), vault_before_attack);
    assert_eq!(
        fetch_token_amount(&svm, user_token_account),
        10_000 - stake_amount
    );
    assert_eq!(fetch_stake_position(&svm, position).amount, stake_amount);
}

/// Coverage for the tags pool's defining behavior, never exercised
/// end-to-end elsewhere in this task set: `app.tags_acc_reward_per_share`
/// and `app.total_tag_stake` are SHARED across every tag of the same app,
/// even though each tag has its own `AppTagStake` accounting record. Two
/// different stakers, staked into two DIFFERENT tags of the SAME app, must
/// fairly split a single `fund_app_rewards(Tags)` round proportional to
/// their stake relative to the COMBINED total across both tags — not
/// proportional to either tag's own stake in isolation (there is no such
/// per-tag quantity). Every other test in this file only ever involves one
/// tag per app (`test_stake_tag_accumulates_across_two_deposits` in
/// `test_stake_tag.rs` covers two deposits into the *same* tag, which is a
/// different property).
#[test]
fn test_claim_tag_reward_splits_shared_accumulator_proportionally_across_two_tags() {
    let (mut svm, deployer, env, app, tag_a_pdas) = setup(); // tag A = "defi"

    // A second, independent tag on the SAME app as `setup()`'s "defi" tag —
    // the crux of this test: two different `app_tag_stake` PDAs, one shared
    // `app`.
    let tag_b_pdas = register_tag(&mut svm, &env.program_id, &deployer, &app, APP_ID, "gaming");

    // Staker A stakes 1_000 into tag A ("defi").
    let (staker_a, staker_a_token_account) = create_funded_user(&mut svm, &env, 10_000);
    let position_a = derive_stake_position(
        &env.program_id,
        &tag_a_pdas.app_tag_stake,
        &staker_a.pubkey(),
    );
    let stake_a = 1_000u64;
    let ix = stake_tag_ix(
        &env,
        &app,
        &tag_a_pdas,
        &position_a,
        &staker_a_token_account,
        &staker_a.pubkey(),
        stake_a,
    );
    send(&mut svm, ix, &staker_a.pubkey(), &[&staker_a])
        .expect("staker A's stake_tag into tag A must succeed");

    // Staker B stakes 3_000 into tag B ("gaming") — a DIFFERENT tag, SAME
    // app.
    let (staker_b, staker_b_token_account) = create_funded_user(&mut svm, &env, 10_000);
    let position_b = derive_stake_position(
        &env.program_id,
        &tag_b_pdas.app_tag_stake,
        &staker_b.pubkey(),
    );
    let stake_b = 3_000u64;
    let ix = stake_tag_ix(
        &env,
        &app,
        &tag_b_pdas,
        &position_b,
        &staker_b_token_account,
        &staker_b.pubkey(),
        stake_b,
    );
    send(&mut svm, ix, &staker_b.pubkey(), &[&staker_b])
        .expect("staker B's stake_tag into tag B must succeed");

    // total_tag_stake is now 4_000, shared across both tags — not tracked
    // per-tag anywhere.
    assert_eq!(fetch_app(&svm, app).total_tag_stake, stake_a + stake_b);

    // Fund the Tags pool ONCE, for the whole app — not per-tag.
    let fund_amount = 4_000u64;
    fund_rewards(
        &mut svm,
        &env,
        &deployer,
        &app,
        RewardPool::Tags,
        fund_amount,
    );

    // acc = 4_000 * PRECISION / 4_000 = 1 * PRECISION.
    // Staker A: settle_pending(1_000, 0, 1*PRECISION) = 1_000 (1/4 of the pool).
    // Staker B: settle_pending(3_000, 0, 1*PRECISION) = 3_000 (3/4 of the pool).
    let expected_a = 1_000u64;
    let expected_b = 3_000u64;
    assert_eq!(
        expected_a + expected_b,
        fund_amount,
        "the two shares must exhaust the whole funded round"
    );

    let vault_before_claims = fetch_token_amount(&svm, env.vault);

    let ix = claim_tag_reward_ix(
        &env,
        &app,
        &tag_a_pdas,
        &position_a,
        &staker_a_token_account,
        &staker_a.pubkey(),
    );
    send(&mut svm, ix, &staker_a.pubkey(), &[&staker_a])
        .expect("staker A's claim_tag_reward must succeed");

    let ix = claim_tag_reward_ix(
        &env,
        &app,
        &tag_b_pdas,
        &position_b,
        &staker_b_token_account,
        &staker_b.pubkey(),
    );
    send(&mut svm, ix, &staker_b.pubkey(), &[&staker_b])
        .expect("staker B's claim_tag_reward must succeed");

    // Each staker received exactly their proportional share of the SAME
    // funding round, purely as a function of their stake relative to the
    // combined total — regardless of which specific tag they staked into.
    assert_eq!(
        fetch_token_amount(&svm, staker_a_token_account),
        10_000 - stake_a + expected_a
    );
    assert_eq!(
        fetch_token_amount(&svm, staker_b_token_account),
        10_000 - stake_b + expected_b
    );

    // The shared vault paid out exactly the whole funded round (both claims
    // combined), on top of whatever it held going into the claims (the two
    // stakers' still-locked principal).
    assert_eq!(
        fetch_token_amount(&svm, env.vault),
        vault_before_claims - expected_a - expected_b
    );
    // Both tags' principal is still fully accounted for inside the shared
    // vault — a claim never touches stake_amount/principal.
    assert_eq!(
        fetch_app_tag_stake(&svm, tag_a_pdas.app_tag_stake).stake_amount,
        stake_a
    );
    assert_eq!(
        fetch_app_tag_stake(&svm, tag_b_pdas.app_tag_stake).stake_amount,
        stake_b
    );
}
