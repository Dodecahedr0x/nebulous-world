mod common;

use {
    anchor_lang::solana_program::pubkey::Pubkey,
    common::{
        derive_app_tag_stake, derive_tag, derive_tag_pdas, fetch_app_tag_stake, fetch_tag,
        register_app, send, setup_svm, suggest_tag_ix, TagPdas, AIRDROP_LAMPORTS,
    },
    solana_keypair::Keypair,
    solana_signer::Signer,
};

// `init_app` and `suggest_tag` reference neither `Config` nor a vote mint nor
// any vault (see `init_app.rs` and `suggest_tag.rs`), so every test here runs
// on the bare `setup_svm()` — spinning up a fake mint and an
// upgrade-authority-gated `initialize` call would be unused overhead.

#[test]
fn test_suggest_tag_happy_path() {
    let program_id = nebulous_world::id();
    let (mut svm, deployer) = setup_svm();

    let app_id = "cid_test_app_0000000001";
    let app = register_app(&mut svm, &program_id, &deployer, app_id);

    let tag_id = "defi";
    let tag_pdas = derive_tag_pdas(&program_id, &app, tag_id);
    let ix = suggest_tag_ix(
        &program_id,
        &deployer.pubkey(),
        &app,
        app_id,
        &tag_pdas,
        tag_id,
    );
    send(&mut svm, ix, &deployer.pubkey(), &[&deployer]).expect("suggest_tag failed");

    assert_eq!(fetch_tag(&svm, tag_pdas.tag).tag_id, tag_id);

    let stake_account = fetch_app_tag_stake(&svm, tag_pdas.app_tag_stake);
    assert_eq!(stake_account.app, app);
    assert_eq!(stake_account.tag, tag_pdas.tag);
    assert_eq!(stake_account.stake_amount, 0);
}

#[test]
fn test_suggest_tag_is_permissionless() {
    let program_id = nebulous_world::id();
    let (mut svm, deployer) = setup_svm();

    let app_id = "cid_test_app_0000000002";
    let app = register_app(&mut svm, &program_id, &deployer, app_id);

    // A stranger (not the deployer/upgrade authority) can suggest a tag.
    let stranger = Keypair::new();
    svm.airdrop(&stranger.pubkey(), AIRDROP_LAMPORTS).unwrap();

    let tag_id = "gaming";
    let tag_pdas = derive_tag_pdas(&program_id, &app, tag_id);
    let ix = suggest_tag_ix(
        &program_id,
        &stranger.pubkey(),
        &app,
        app_id,
        &tag_pdas,
        tag_id,
    );
    send(&mut svm, ix, &stranger.pubkey(), &[&stranger])
        .expect("suggest_tag failed for a stranger payer");
}

#[test]
fn test_suggest_tag_rejects_duplicate_tag_for_same_app() {
    let program_id = nebulous_world::id();
    let (mut svm, deployer) = setup_svm();

    let app_id = "cid_test_app_0000000003";
    let app = register_app(&mut svm, &program_id, &deployer, app_id);

    let tag_id = "defi";
    let tag_pdas = derive_tag_pdas(&program_id, &app, tag_id);
    let ix = suggest_tag_ix(
        &program_id,
        &deployer.pubkey(),
        &app,
        app_id,
        &tag_pdas,
        tag_id,
    );
    send(&mut svm, ix, &deployer.pubkey(), &[&deployer]).expect("first suggest_tag must succeed");

    // Suggesting the exact same (app, tag_id) pair again must fail cleanly —
    // Anchor's plain `init` constraint on `app_tag_stake` requires the
    // account not already exist. (`tag` itself is `init_if_needed` and would
    // happily be reused; it's `app_tag_stake` that blocks the duplicate.)
    let ix = suggest_tag_ix(
        &program_id,
        &deployer.pubkey(),
        &app,
        app_id,
        &tag_pdas,
        tag_id,
    );
    assert!(
        send(&mut svm, ix, &deployer.pubkey(), &[&deployer]).is_err(),
        "expected a duplicate suggest_tag for the same (app, tag_id) to fail"
    );
}

#[test]
fn test_suggest_tag_rejects_tag_id_over_32_bytes() {
    let program_id = nebulous_world::id();
    let (mut svm, deployer) = setup_svm();

    let app_id = "cid_test_app_0000000004";
    let app = register_app(&mut svm, &program_id, &deployer, app_id);

    // 33 bytes exceeds Solana's 32-byte-per-seed limit — mirrors
    // `test_init_app.rs`'s oversized app_id test. We can't derive the "real"
    // PDAs (find_program_address panics client-side too), so pass unrelated
    // pubkeys in those slots; the program's own seed derivation during
    // account resolution panics on the oversized seed regardless.
    let tag_id = "a".repeat(33);
    let tag_pdas = TagPdas {
        tag: Pubkey::new_unique(),
        app_tag_stake: Pubkey::new_unique(),
    };
    let ix = suggest_tag_ix(
        &program_id,
        &deployer.pubkey(),
        &app,
        app_id,
        &tag_pdas,
        &tag_id,
    );
    assert!(
        send(&mut svm, ix, &deployer.pubkey(), &[&deployer]).is_err(),
        "expected suggest_tag to reject a tag_id longer than 32 bytes"
    );
}

/// The core new behavior of the two-account split: the SAME `tag_id` string
/// suggested by two DIFFERENT apps now resolves to the exact SAME global
/// `Tag` account (since its seeds are `[TAG_SEED, tag_id]`, with no `app`),
/// while each app still gets its OWN `app_tag_stake` account (since those
/// seeds include `app.key()`). This replaces the old (pre-refactor)
/// `..._no_collision` test, which asserted the opposite — that the two apps'
/// tag accounts were different — back when `AppTagAccount` was seeded by
/// both `app` and `tag_id` together.
#[test]
fn test_suggest_tag_same_tag_id_shared_across_apps() {
    let program_id = nebulous_world::id();
    let (mut svm, deployer) = setup_svm();

    let app_id_a = "cid_test_app_aaaaaaaaaaa";
    let app_id_b = "cid_test_app_bbbbbbbbbbb";
    let app_a = register_app(&mut svm, &program_id, &deployer, app_id_a);
    let app_b = register_app(&mut svm, &program_id, &deployer, app_id_b);
    assert_ne!(app_a, app_b);

    let tag_id = "defi";
    // Same tag_id -> same global Tag PDA, regardless of which app suggests it.
    let tag = derive_tag(&program_id, tag_id);
    let pdas_a = derive_tag_pdas(&program_id, &app_a, tag_id);
    let pdas_b = derive_tag_pdas(&program_id, &app_b, tag_id);
    assert_eq!(pdas_a.tag, tag);
    assert_eq!(pdas_b.tag, tag);
    // But the per-(app, tag) stake-accounting PDAs differ, since their seeds
    // include `app.key()`.
    assert_ne!(pdas_a.app_tag_stake, pdas_b.app_tag_stake);
    assert_eq!(
        pdas_a.app_tag_stake,
        derive_app_tag_stake(&program_id, &app_a, &tag)
    );

    let ix_a = suggest_tag_ix(
        &program_id,
        &deployer.pubkey(),
        &app_a,
        app_id_a,
        &pdas_a,
        tag_id,
    );
    send(&mut svm, ix_a, &deployer.pubkey(), &[&deployer]).expect("suggest_tag for app A failed");

    let ix_b = suggest_tag_ix(
        &program_id,
        &deployer.pubkey(),
        &app_b,
        app_id_b,
        &pdas_b,
        tag_id,
    );
    send(&mut svm, ix_b, &deployer.pubkey(), &[&deployer]).expect("suggest_tag for app B failed");

    // Exactly one `Tag` account was ever created (app B's suggestion reused
    // it via `init_if_needed`), and both apps' `app_tag_stake` accounts
    // point at that identical `Tag` pubkey.
    assert_eq!(fetch_tag(&svm, tag).tag_id, tag_id);

    let stake_a = fetch_app_tag_stake(&svm, pdas_a.app_tag_stake);
    let stake_b = fetch_app_tag_stake(&svm, pdas_b.app_tag_stake);
    assert_eq!(stake_a.app, app_a);
    assert_eq!(stake_b.app, app_b);
    assert_eq!(stake_a.tag, tag);
    assert_eq!(stake_b.tag, tag);
    assert_ne!(stake_a.app, stake_b.app);
}
