mod common;

use {
    anchor_lang::solana_program::pubkey::Pubkey,
    common::{derive_app, fetch_app, init_app_ix, send, setup_svm, AIRDROP_LAMPORTS, APP_URL},
    solana_keypair::Keypair,
    solana_signer::Signer,
};

#[test]
fn test_init_app() {
    let program_id = nebulous_world::id();
    let (mut svm, _deployer) = setup_svm();

    // Registering an app is permissionless: use a payer that is *not* the
    // program's upgrade authority (unlike `initialize`, which requires it),
    // to prove no authority/signer-identity gating crept in.
    let stranger = Keypair::new();
    svm.airdrop(&stranger.pubkey(), AIRDROP_LAMPORTS).unwrap();

    let app_id = "cid_test_app_0000000001";
    let app = derive_app(&program_id, app_id);
    let ix = init_app_ix(&program_id, &stranger.pubkey(), &app, app_id, APP_URL);

    send(&mut svm, ix, &stranger.pubkey(), &[&stranger]).expect("init_app transaction failed");

    // The `AppAccount` was created with zeroed counters and no vault
    // pubkeys of its own — every app shares the single global vault.
    let app_account = fetch_app(&svm, app);
    assert_eq!(app_account.app_id, app_id);
    assert_eq!(app_account.url, APP_URL);
    assert_eq!(app_account.total_vote_stake, 0);
    assert_eq!(app_account.vote_acc_reward_per_share, 0);
    assert_eq!(app_account.total_tag_stake, 0);
    assert_eq!(app_account.tags_acc_reward_per_share, 0);
}

#[test]
fn test_init_app_rejects_app_id_over_32_bytes() {
    let program_id = nebulous_world::id();
    let (mut svm, deployer) = setup_svm();

    // 33 bytes exceeds Solana's 32-byte-per-seed limit. `Pubkey::find_program_address`
    // panics on an oversized seed on *any* target (not just on-chain), so we
    // can't even derive the "real" `app` PDA for this app_id here — the same
    // way a client can't either (`PublicKey.findProgramAddressSync` throws
    // client-side too, see `tests/nebulous_world.ts`). That's fine: we only
    // need *some* pubkey in the `app` slot, because the program's own
    // `find_program_address` call (during account resolution, before the
    // handler body or any other constraint runs — see the comment in
    // `init_app.rs`) panics on the oversized seed regardless of what key we
    // pass. What we're asserting is that the transaction is rejected either
    // way.
    let app_id = "a".repeat(33);
    let app = Pubkey::new_unique();
    let ix = init_app_ix(&program_id, &deployer.pubkey(), &app, &app_id, APP_URL);

    assert!(
        send(&mut svm, ix, &deployer.pubkey(), &[&deployer]).is_err(),
        "expected init_app to reject an app_id longer than 32 bytes, but it succeeded"
    );
}

#[test]
fn test_init_app_rejects_url_over_max_len() {
    let program_id = nebulous_world::id();
    let (mut svm, deployer) = setup_svm();

    let app_id = "cid_test_app_0000000002";
    let app = derive_app(&program_id, app_id);
    // 201 bytes exceeds MAX_URL_LEN (200); unlike app_id, url isn't a PDA
    // seed, so this is rejected by the handler's `require!`, not a panic
    // during account resolution.
    let url = "a".repeat(201);
    let ix = init_app_ix(&program_id, &deployer.pubkey(), &app, app_id, &url);

    assert!(
        send(&mut svm, ix, &deployer.pubkey(), &[&deployer]).is_err(),
        "expected init_app to reject a url longer than MAX_URL_LEN, but it succeeded"
    );
}
