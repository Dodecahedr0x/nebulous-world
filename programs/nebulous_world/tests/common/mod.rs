//! Shared fixtures, PDA derivations, and instruction builders for the
//! `nebulous_world` LiteSVM integration tests.
//!
//! Each `tests/test_*.rs` file is compiled as its own crate, so anything they
//! share has to live in a module every one of them declares with `mod common;`
//! — and no single test binary uses all of it. Hence the blanket
//! `allow(dead_code)`: without it the unused half of this module would warn in
//! every test binary that doesn't happen to touch it.

#![allow(dead_code)]

use {
    anchor_lang::{
        solana_program::{
            bpf_loader_upgradeable::{self, UpgradeableLoaderState},
            instruction::Instruction,
            program_option::COption,
            program_pack::Pack,
            pubkey::Pubkey,
            system_program,
        },
        AccountDeserialize, AccountSerialize, InstructionData, ToAccountMetas,
    },
    anchor_spl::{
        associated_token::{get_associated_token_address, ID as ASSOCIATED_TOKEN_PROGRAM_ID},
        token::ID as TOKEN_PROGRAM_ID,
    },
    litesvm::{
        types::{FailedTransactionMetadata, TransactionMetadata},
        LiteSVM,
    },
    nebulous_world::{
        constants::{
            APP_SEED, APP_TAG_STAKE_SEED, CONFIG_SEED, STAKE_POSITION_SEED, TAG_SEED,
            VOTE_POSITION_SEED,
        },
        RewardPool,
    },
    solana_account::Account,
    solana_clock::Clock,
    solana_keypair::Keypair,
    solana_message::{Message, VersionedMessage},
    solana_signer::Signer,
    solana_transaction::versioned::VersionedTransaction,
    spl_token_interface::state::{Account as SplTokenAccount, AccountState, Mint},
};

/// SOL airdropped to every keypair these fixtures create — enough to cover
/// rent for any account an instruction under test might open.
pub const AIRDROP_LAMPORTS: u64 = 1_000_000_000;

/// `Config.protocol_fee_bps` every fixture initializes with. Tests that care
/// about the value itself build their own `initialize_ix` instead.
pub const PROTOCOL_FEE_BPS: u16 = 250;

/// `AppAccount.url` every fixture registers apps with.
pub const APP_URL: &str = "example.com";

/// Decimals on the fabricated vote mint.
pub const MINT_DECIMALS: u8 = 6;

/// The program-wide singletons every instruction needs: `Config`'s own PDA and
/// the single global vault derived from it (an ATA of `config` for
/// `vote_mint` — see the design note on `Config`), plus `vote_mint` /
/// `program_id` for convenience, and the admin's own ATA, where `withdraw_vote`
/// and `withdraw_tag_stake` pay the unstake fee directly. Produced once by
/// [`setup`] via a real `initialize()` call.
#[derive(Clone, Copy)]
pub struct Env {
    pub program_id: Pubkey,
    pub config: Pubkey,
    pub vault: Pubkey,
    pub vote_mint: Pubkey,
    /// The admin's (`deployer`'s) ATA, pre-created holding 0. It is a plain
    /// `Account<'info, TokenAccount>` in the program, not `init_if_needed`, so
    /// it has to exist before the first withdrawal.
    pub admin_token_account: Pubkey,
}

/// The two PDAs `suggest_tag` creates for one (app, tag_id) pair: the GLOBAL
/// `Tag` identity (shared across every app that suggests the same tag_id,
/// seeded only by `tag_id` — no `app`) and the per-(app, tag) `AppTagStake`
/// stake-accounting link (seeded by `[app, tag]`).
#[derive(Clone, Copy)]
pub struct TagPdas {
    pub tag: Pubkey,
    pub app_tag_stake: Pubkey,
}

// ---------------------------------------------------------------------------
// SVM setup
// ---------------------------------------------------------------------------

/// A fresh LiteSVM with the nebulous_world program loaded and a funded
/// deployer/payer — nothing else. `init_app` and `suggest_tag` reference
/// neither `Config` nor a vote mint nor any vault (see `init_app.rs` and
/// `suggest_tag.rs`), so tests covering only those can skip everything
/// [`setup`] layers on top.
pub fn setup_svm() -> (LiteSVM, Keypair) {
    let deployer = Keypair::new();
    let mut svm = LiteSVM::new();
    let bytes = include_bytes!("../../../../target/deploy/nebulous_world.so");
    svm.add_program(nebulous_world::id(), bytes).unwrap();
    svm.airdrop(&deployer.pubkey(), AIRDROP_LAMPORTS).unwrap();
    (svm, deployer)
}

/// [`setup_svm`] plus a fabricated SPL mint whose mint authority is the
/// deployer. Returns the SVM, the deployer, and the mint — for tests that need
/// a mint but must drive `initialize` themselves.
pub fn setup_svm_with_mint() -> (LiteSVM, Keypair, Pubkey) {
    let (mut svm, deployer) = setup_svm();
    let vote_mint = create_mint(&mut svm, deployer.pubkey());
    (svm, deployer, vote_mint)
}

/// The common baseline: program loaded, mint fabricated, `Config` + the single
/// global vault initialized via a real `initialize()` call (authority =
/// `deployer`, who is also the program's upgrade authority), and the admin's
/// ATA pre-created empty.
pub fn setup() -> (LiteSVM, Keypair, Env) {
    let program_id = nebulous_world::id();
    let (mut svm, deployer, vote_mint) = setup_svm_with_mint();

    let program_data = set_upgrade_authority(&mut svm, &program_id, deployer.pubkey());
    let config = derive_config(&program_id);
    let vault = get_associated_token_address(&config, &vote_mint);
    let ix = initialize_ix(
        &program_id,
        &deployer.pubkey(),
        &vote_mint,
        &program_data,
        PROTOCOL_FEE_BPS,
    );
    send(&mut svm, ix, &deployer.pubkey(), &[&deployer]).expect("initialize must succeed in setup");

    let admin_token_account = get_associated_token_address(&deployer.pubkey(), &vote_mint);
    fund_token_account(
        &mut svm,
        admin_token_account,
        vote_mint,
        deployer.pubkey(),
        0,
    );

    (
        svm,
        deployer,
        Env {
            program_id,
            config,
            vault,
            vote_mint,
            admin_token_account,
        },
    )
}

/// [`setup`] plus one `AppAccount` registered via a real `init_app` call.
pub fn setup_with_app(app_id: &str) -> (LiteSVM, Keypair, Env, Pubkey) {
    let (mut svm, deployer, env) = setup();
    let app = register_app(&mut svm, &env.program_id, &deployer, app_id);
    (svm, deployer, env, app)
}

/// [`setup_with_app`] plus one tag suggested onto it via a real `suggest_tag`
/// call (creating both the global `Tag` and its `AppTagStake`).
pub fn setup_with_tag(app_id: &str, tag_id: &str) -> (LiteSVM, Keypair, Env, Pubkey, TagPdas) {
    let (mut svm, deployer, env, app) = setup_with_app(app_id);
    let tag_pdas = register_tag(&mut svm, &env.program_id, &deployer, &app, app_id, tag_id);
    (svm, deployer, env, app, tag_pdas)
}

/// Overwrites the nebulous_world program's `ProgramData` account (created by
/// `svm.add_program`, which defaults to `upgrade_authority_address: None`) so
/// that `upgrade_authority` is its recorded upgrade authority — `initialize`
/// is gated on that signer. Returns the programdata account's address.
pub fn set_upgrade_authority(
    svm: &mut LiteSVM,
    program_id: &Pubkey,
    upgrade_authority: Pubkey,
) -> Pubkey {
    let program_data_address = bpf_loader_upgradeable::get_program_data_address(program_id);
    let mut account = svm
        .get_account(&program_data_address)
        .expect("programdata account must exist (call after add_program)");

    let header = bincode::serialize(&UpgradeableLoaderState::ProgramData {
        slot: 0,
        upgrade_authority_address: Some(upgrade_authority),
    })
    .unwrap();
    account.data[..header.len()].copy_from_slice(&header);

    svm.set_account(program_data_address, account).unwrap();
    program_data_address
}

/// Writes a fake SPL mint account directly, so it satisfies
/// `Account<'info, Mint>` deserialization without running the token program's
/// `InitializeMint`.
pub fn create_mint(svm: &mut LiteSVM, mint_authority: Pubkey) -> Pubkey {
    let vote_mint = Pubkey::new_unique();
    let mint = Mint {
        mint_authority: COption::Some(mint_authority),
        supply: 0,
        decimals: MINT_DECIMALS,
        is_initialized: true,
        freeze_authority: COption::None,
    };
    let mut data = vec![0u8; Mint::LEN];
    Mint::pack(mint, &mut data).unwrap();
    svm.set_account(
        vote_mint,
        Account {
            lamports: svm.minimum_balance_for_rent_exemption(Mint::LEN),
            data,
            owner: spl_token_interface::ID,
            executable: false,
            rent_epoch: 0,
        },
    )
    .unwrap();
    vote_mint
}

/// Directly writes a funded, initialized SPL token account owned by `owner`
/// for `mint`, holding exactly `amount` — bypassing the token program's
/// `InitializeAccount`/`MintTo` since only the end state matters, mirroring how
/// [`create_mint`] fabricates the mint itself. Also used to top up the single
/// global vault (owner = `config`), standing in for a real `fund_app_rewards`.
///
/// `pubkey` must be the owner's canonical ATA for `mint`: every token account
/// the program touches is constrained with `associated_token::authority`, so an
/// arbitrary address is rejected during account resolution.
pub fn fund_token_account(
    svm: &mut LiteSVM,
    pubkey: Pubkey,
    mint: Pubkey,
    owner: Pubkey,
    amount: u64,
) {
    let token_account = SplTokenAccount {
        mint,
        owner,
        amount,
        delegate: COption::None,
        state: AccountState::Initialized,
        is_native: COption::None,
        delegated_amount: 0,
        close_authority: COption::None,
    };
    let mut data = vec![0u8; SplTokenAccount::LEN];
    SplTokenAccount::pack(token_account, &mut data).unwrap();
    svm.set_account(
        pubkey,
        Account {
            lamports: svm.minimum_balance_for_rent_exemption(SplTokenAccount::LEN),
            data,
            owner: spl_token_interface::ID,
            executable: false,
            rent_epoch: 0,
        },
    )
    .unwrap();
}

/// Adds `additional` on top of an account's CURRENT balance rather than
/// overwriting it the way [`fund_token_account`] does. Necessary for the single
/// global vault (see the design note on `Config`): by the time a test wants to
/// stand in for a `fund_app_rewards` payout round, that vault may already hold
/// real staked principal from an earlier step of the same test, and clobbering
/// it outright would silently corrupt the balance.
pub fn credit_token_account(
    svm: &mut LiteSVM,
    pubkey: Pubkey,
    mint: Pubkey,
    owner: Pubkey,
    additional: u64,
) {
    let current = fetch_token_amount(svm, pubkey);
    fund_token_account(svm, pubkey, mint, owner, current + additional);
}

/// A fresh airdropped keypair plus its ATA for `env.vote_mint`, funded with
/// `wallet_amount` — the starting point of nearly every staking test.
pub fn create_funded_user(svm: &mut LiteSVM, env: &Env, wallet_amount: u64) -> (Keypair, Pubkey) {
    let user = Keypair::new();
    svm.airdrop(&user.pubkey(), AIRDROP_LAMPORTS).unwrap();
    let token_account = get_associated_token_address(&user.pubkey(), &env.vote_mint);
    fund_token_account(
        svm,
        token_account,
        env.vote_mint,
        user.pubkey(),
        wallet_amount,
    );
    (user, token_account)
}

/// Advances the SVM's on-chain clock by `seconds`. LiteSVM's clock is
/// otherwise frozen at its initial value, which would never exercise the
/// linearly-decaying unstake fee's time dependency (see `unstake_fee.rs`) or
/// `staked_at`'s weighted-average top-up behavior.
pub fn warp_forward(svm: &mut LiteSVM, seconds: i64) {
    let mut clock = svm.get_sysvar::<Clock>();
    clock.unix_timestamp += seconds;
    svm.set_sysvar::<Clock>(&clock);
}

/// Signs and submits `ix` as a single-instruction transaction. Returning the
/// `Result` (rather than a bool) keeps the on-chain error and logs available:
/// success paths `.expect("...")` it, rejection paths assert on `is_err()` or
/// `.expect_err(...).meta.pretty_logs()` for a specific `ErrorCode`.
///
/// `FailedTransactionMetadata` is a large struct (>=200 bytes); box it so this
/// `Result`'s error variant doesn't bloat every caller's stack frame
/// (clippy::result_large_err).
pub fn send(
    svm: &mut LiteSVM,
    ix: Instruction,
    payer: &Pubkey,
    signers: &[&Keypair],
) -> Result<TransactionMetadata, Box<FailedTransactionMetadata>> {
    let blockhash = svm.latest_blockhash();
    let msg = Message::new_with_blockhash(&[ix], Some(payer), &blockhash);
    let tx = VersionedTransaction::try_new(VersionedMessage::Legacy(msg), signers).unwrap();
    svm.send_transaction(tx).map_err(Box::new)
}

// ---------------------------------------------------------------------------
// Account reads
// ---------------------------------------------------------------------------

fn fetch<T: AccountDeserialize>(svm: &LiteSVM, pubkey: Pubkey, what: &str) -> T {
    let raw = svm
        .get_account(&pubkey)
        .unwrap_or_else(|| panic!("{what} account must exist"));
    T::try_deserialize(&mut raw.data.as_slice()).unwrap()
}

pub fn fetch_config(svm: &LiteSVM, config: Pubkey) -> nebulous_world::Config {
    fetch(svm, config, "config")
}

pub fn fetch_app(svm: &LiteSVM, app: Pubkey) -> nebulous_world::AppAccount {
    fetch(svm, app, "app")
}

pub fn fetch_app_tag_stake(svm: &LiteSVM, app_tag_stake: Pubkey) -> nebulous_world::AppTagStake {
    fetch(svm, app_tag_stake, "app_tag_stake")
}

pub fn fetch_tag(svm: &LiteSVM, tag: Pubkey) -> nebulous_world::Tag {
    fetch(svm, tag, "tag")
}

pub fn fetch_vote_position(svm: &LiteSVM, position: Pubkey) -> nebulous_world::VotePosition {
    fetch(svm, position, "position")
}

pub fn fetch_stake_position(svm: &LiteSVM, position: Pubkey) -> nebulous_world::StakePosition {
    fetch(svm, position, "position")
}

pub fn fetch_token_amount(svm: &LiteSVM, pubkey: Pubkey) -> u64 {
    let raw = svm.get_account(&pubkey).expect("token account must exist");
    SplTokenAccount::unpack(&raw.data).unwrap().amount
}

/// Directly overwrites an already-created `AppAccount`'s
/// `vote_acc_reward_per_share`, so tests can exercise the reward-payout leg of
/// `vote()`/`withdraw_vote()` (normally only nonzero once `fund_app_rewards`
/// has run) without depending on that instruction. Re-serializes through
/// `AccountSerialize` so the Anchor discriminator is preserved, and keeps the
/// account's existing lamports/owner.
pub fn set_app_vote_accumulator(svm: &mut LiteSVM, app: Pubkey, acc_reward_per_share: u128) {
    let mut raw = svm.get_account(&app).expect("app account must exist");
    let mut app_account: nebulous_world::AppAccount =
        AccountDeserialize::try_deserialize(&mut raw.data.as_slice()).unwrap();
    app_account.vote_acc_reward_per_share = acc_reward_per_share;

    let mut data = Vec::new();
    AccountSerialize::try_serialize(&app_account, &mut data).unwrap();
    raw.data = data;
    svm.set_account(app, raw).unwrap();
}

/// The Tags-pool counterpart to [`set_app_vote_accumulator`].
pub fn set_app_tags_accumulator(svm: &mut LiteSVM, app: Pubkey, acc_reward_per_share: u128) {
    let mut raw = svm.get_account(&app).expect("app account must exist");
    let mut app_account: nebulous_world::AppAccount =
        AccountDeserialize::try_deserialize(&mut raw.data.as_slice()).unwrap();
    app_account.tags_acc_reward_per_share = acc_reward_per_share;

    let mut data = Vec::new();
    AccountSerialize::try_serialize(&app_account, &mut data).unwrap();
    raw.data = data;
    svm.set_account(app, raw).unwrap();
}

// ---------------------------------------------------------------------------
// PDA derivation
// ---------------------------------------------------------------------------

pub fn derive_config(program_id: &Pubkey) -> Pubkey {
    Pubkey::find_program_address(&[CONFIG_SEED], program_id).0
}

pub fn derive_app(program_id: &Pubkey, app_id: &str) -> Pubkey {
    Pubkey::find_program_address(&[APP_SEED, app_id.as_bytes()], program_id).0
}

/// The GLOBAL `Tag` PDA: seeded ONLY by `tag_id`, with no `app` in the
/// derivation — every app that suggests the same `tag_id` string resolves to
/// this exact same address.
pub fn derive_tag(program_id: &Pubkey, tag_id: &str) -> Pubkey {
    Pubkey::find_program_address(&[TAG_SEED, tag_id.as_bytes()], program_id).0
}

/// The per-(app, tag) stake-accounting PDA: seeded by `app.key()` and
/// `tag.key()` (the `Tag` account's pubkey, not the raw `tag_id` string).
pub fn derive_app_tag_stake(program_id: &Pubkey, app: &Pubkey, tag: &Pubkey) -> Pubkey {
    Pubkey::find_program_address(
        &[APP_TAG_STAKE_SEED, app.as_ref(), tag.as_ref()],
        program_id,
    )
    .0
}

pub fn derive_tag_pdas(program_id: &Pubkey, app: &Pubkey, tag_id: &str) -> TagPdas {
    let tag = derive_tag(program_id, tag_id);
    TagPdas {
        tag,
        app_tag_stake: derive_app_tag_stake(program_id, app, &tag),
    }
}

pub fn derive_vote_position(program_id: &Pubkey, app: &Pubkey, user: &Pubkey) -> Pubkey {
    Pubkey::find_program_address(
        &[VOTE_POSITION_SEED, app.as_ref(), user.as_ref()],
        program_id,
    )
    .0
}

/// Note the seed is the `AppTagStake` PDA, not the app — a user holds one tag
/// stake position per (app, tag) pair.
pub fn derive_stake_position(program_id: &Pubkey, app_tag_stake: &Pubkey, user: &Pubkey) -> Pubkey {
    Pubkey::find_program_address(
        &[STAKE_POSITION_SEED, app_tag_stake.as_ref(), user.as_ref()],
        program_id,
    )
    .0
}

// ---------------------------------------------------------------------------
// Instruction builders
// ---------------------------------------------------------------------------

pub fn initialize_ix(
    program_id: &Pubkey,
    authority: &Pubkey,
    vote_mint: &Pubkey,
    program_data: &Pubkey,
    protocol_fee_bps: u16,
) -> Instruction {
    let config = derive_config(program_id);
    Instruction::new_with_bytes(
        *program_id,
        &nebulous_world::instruction::Initialize { protocol_fee_bps }.data(),
        nebulous_world::accounts::Initialize {
            config,
            vault: get_associated_token_address(&config, vote_mint),
            authority: *authority,
            vote_mint: *vote_mint,
            program: *program_id,
            program_data: *program_data,
            token_program: TOKEN_PROGRAM_ID,
            associated_token_program: ASSOCIATED_TOKEN_PROGRAM_ID,
            system_program: system_program::ID,
        }
        .to_account_metas(None),
    )
}

pub fn init_app_ix(
    program_id: &Pubkey,
    payer: &Pubkey,
    app: &Pubkey,
    app_id: &str,
    url: &str,
) -> Instruction {
    Instruction::new_with_bytes(
        *program_id,
        &nebulous_world::instruction::InitApp {
            app_id: app_id.to_string(),
            url: url.to_string(),
        }
        .data(),
        nebulous_world::accounts::InitApp {
            app: *app,
            payer: *payer,
            system_program: system_program::ID,
        }
        .to_account_metas(None),
    )
}

pub fn suggest_tag_ix(
    program_id: &Pubkey,
    payer: &Pubkey,
    app: &Pubkey,
    app_id: &str,
    tag_pdas: &TagPdas,
    tag_id: &str,
) -> Instruction {
    Instruction::new_with_bytes(
        *program_id,
        &nebulous_world::instruction::SuggestTag {
            app_id: app_id.to_string(),
            tag_id: tag_id.to_string(),
        }
        .data(),
        nebulous_world::accounts::SuggestTag {
            app: *app,
            tag: tag_pdas.tag,
            app_tag_stake: tag_pdas.app_tag_stake,
            payer: *payer,
            system_program: system_program::ID,
        }
        .to_account_metas(None),
    )
}

pub fn vote_ix(
    env: &Env,
    app: &Pubkey,
    position: &Pubkey,
    user_token_account: &Pubkey,
    user: &Pubkey,
    amount: u64,
) -> Instruction {
    Instruction::new_with_bytes(
        env.program_id,
        &nebulous_world::instruction::Vote { amount }.data(),
        nebulous_world::accounts::Vote {
            app: *app,
            position: *position,
            config: env.config,
            vault: env.vault,
            user_token_account: *user_token_account,
            user: *user,
            token_program: TOKEN_PROGRAM_ID,
            system_program: system_program::ID,
        }
        .to_account_metas(None),
    )
}

pub fn withdraw_vote_ix(
    env: &Env,
    app: &Pubkey,
    position: &Pubkey,
    user_token_account: &Pubkey,
    user: &Pubkey,
    amount: u64,
) -> Instruction {
    Instruction::new_with_bytes(
        env.program_id,
        &nebulous_world::instruction::WithdrawVote { amount }.data(),
        nebulous_world::accounts::WithdrawVote {
            app: *app,
            position: *position,
            config: env.config,
            vault: env.vault,
            user_token_account: *user_token_account,
            admin_token_account: env.admin_token_account,
            user: *user,
            token_program: TOKEN_PROGRAM_ID,
        }
        .to_account_metas(None),
    )
}

pub fn close_vote_position_ix(
    program_id: &Pubkey,
    position: &Pubkey,
    payer: &Pubkey,
    user: &Pubkey,
) -> Instruction {
    Instruction::new_with_bytes(
        *program_id,
        &nebulous_world::instruction::CloseVotePosition {}.data(),
        nebulous_world::accounts::CloseVotePosition {
            position: *position,
            payer: *payer,
            user: *user,
        }
        .to_account_metas(None),
    )
}

pub fn stake_tag_ix(
    env: &Env,
    app: &Pubkey,
    tag_pdas: &TagPdas,
    position: &Pubkey,
    user_token_account: &Pubkey,
    user: &Pubkey,
    amount: u64,
) -> Instruction {
    Instruction::new_with_bytes(
        env.program_id,
        &nebulous_world::instruction::StakeTag { amount }.data(),
        nebulous_world::accounts::StakeTag {
            app: *app,
            app_tag_stake: tag_pdas.app_tag_stake,
            position: *position,
            config: env.config,
            vault: env.vault,
            user_token_account: *user_token_account,
            user: *user,
            token_program: TOKEN_PROGRAM_ID,
            system_program: system_program::ID,
        }
        .to_account_metas(None),
    )
}

pub fn withdraw_tag_stake_ix(
    env: &Env,
    app: &Pubkey,
    tag_pdas: &TagPdas,
    position: &Pubkey,
    user_token_account: &Pubkey,
    user: &Pubkey,
    amount: u64,
) -> Instruction {
    Instruction::new_with_bytes(
        env.program_id,
        &nebulous_world::instruction::WithdrawTagStake { amount }.data(),
        nebulous_world::accounts::WithdrawTagStake {
            app: *app,
            app_tag_stake: tag_pdas.app_tag_stake,
            position: *position,
            config: env.config,
            vault: env.vault,
            user_token_account: *user_token_account,
            admin_token_account: env.admin_token_account,
            user: *user,
            token_program: TOKEN_PROGRAM_ID,
        }
        .to_account_metas(None),
    )
}

pub fn close_tag_stake_position_ix(
    program_id: &Pubkey,
    position: &Pubkey,
    payer: &Pubkey,
    user: &Pubkey,
) -> Instruction {
    Instruction::new_with_bytes(
        *program_id,
        &nebulous_world::instruction::CloseTagStakePosition {}.data(),
        nebulous_world::accounts::CloseTagStakePosition {
            position: *position,
            payer: *payer,
            user: *user,
        }
        .to_account_metas(None),
    )
}

pub fn fund_app_rewards_ix(
    env: &Env,
    app: &Pubkey,
    funder_token_account: &Pubkey,
    authority: &Pubkey,
    pool: RewardPool,
    amount: u64,
) -> Instruction {
    Instruction::new_with_bytes(
        env.program_id,
        &nebulous_world::instruction::FundAppRewards { pool, amount }.data(),
        nebulous_world::accounts::FundAppRewards {
            app: *app,
            config: env.config,
            vault: env.vault,
            funder_token_account: *funder_token_account,
            authority: *authority,
            token_program: TOKEN_PROGRAM_ID,
        }
        .to_account_metas(None),
    )
}

pub fn claim_vote_reward_ix(
    env: &Env,
    app: &Pubkey,
    position: &Pubkey,
    user_token_account: &Pubkey,
    user: &Pubkey,
) -> Instruction {
    Instruction::new_with_bytes(
        env.program_id,
        &nebulous_world::instruction::ClaimVoteReward {}.data(),
        nebulous_world::accounts::ClaimVoteReward {
            app: *app,
            position: *position,
            config: env.config,
            vault: env.vault,
            user_token_account: *user_token_account,
            user: *user,
            token_program: TOKEN_PROGRAM_ID,
        }
        .to_account_metas(None),
    )
}

pub fn claim_tag_reward_ix(
    env: &Env,
    app: &Pubkey,
    tag_pdas: &TagPdas,
    position: &Pubkey,
    user_token_account: &Pubkey,
    user: &Pubkey,
) -> Instruction {
    Instruction::new_with_bytes(
        env.program_id,
        &nebulous_world::instruction::ClaimTagReward {}.data(),
        nebulous_world::accounts::ClaimTagReward {
            app: *app,
            app_tag_stake: tag_pdas.app_tag_stake,
            position: *position,
            config: env.config,
            vault: env.vault,
            user_token_account: *user_token_account,
            user: *user,
            token_program: TOKEN_PROGRAM_ID,
        }
        .to_account_metas(None),
    )
}

// ---------------------------------------------------------------------------
// Composite fixtures
// ---------------------------------------------------------------------------

/// Registers an app via a real `init_app` call and returns its PDA.
pub fn register_app(
    svm: &mut LiteSVM,
    program_id: &Pubkey,
    payer: &Keypair,
    app_id: &str,
) -> Pubkey {
    let app = derive_app(program_id, app_id);
    let ix = init_app_ix(program_id, &payer.pubkey(), &app, app_id, APP_URL);
    send(svm, ix, &payer.pubkey(), &[payer]).expect("init_app must succeed in setup");
    app
}

/// Suggests a tag onto an already-registered app via a real `suggest_tag`
/// call, creating both the global `Tag` and the (app, tag) `AppTagStake`.
pub fn register_tag(
    svm: &mut LiteSVM,
    program_id: &Pubkey,
    payer: &Keypair,
    app: &Pubkey,
    app_id: &str,
    tag_id: &str,
) -> TagPdas {
    let tag_pdas = derive_tag_pdas(program_id, app, tag_id);
    let ix = suggest_tag_ix(program_id, &payer.pubkey(), app, app_id, &tag_pdas, tag_id);
    send(svm, ix, &payer.pubkey(), &[payer]).expect("suggest_tag must succeed in setup");
    tag_pdas
}

/// Funds `pool` for `app` for real, through a genuine `fund_app_rewards` call
/// paid out of the admin's own ATA (`env.admin_token_account`, owned by
/// `Config.authority` — the only signer `fund_app_rewards` accepts). The
/// accumulator and the vault balance therefore both end up genuinely produced
/// by the instruction, rather than hand-poked into the accounts the way
/// [`set_app_vote_accumulator`] has to do.
pub fn fund_rewards(
    svm: &mut LiteSVM,
    env: &Env,
    deployer: &Keypair,
    app: &Pubkey,
    pool: RewardPool,
    amount: u64,
) {
    fund_token_account(
        svm,
        env.admin_token_account,
        env.vote_mint,
        deployer.pubkey(),
        amount,
    );
    let ix = fund_app_rewards_ix(
        env,
        app,
        &env.admin_token_account,
        &deployer.pubkey(),
        pool,
        amount,
    );
    send(svm, ix, &deployer.pubkey(), &[deployer]).expect("fund_app_rewards must succeed in setup");
}

/// Registers an ADDITIONAL app + tag pair against the same already-initialized
/// `Config`/vault/mint. Used by the cross-app mismatch regression tests, which
/// need two independent (app, app_tag_stake) pairs to build a mismatched call.
pub fn register_app_and_tag(
    svm: &mut LiteSVM,
    program_id: &Pubkey,
    payer: &Keypair,
    app_id: &str,
    tag_id: &str,
) -> (Pubkey, TagPdas) {
    let app = register_app(svm, program_id, payer, app_id);
    let tag_pdas = register_tag(svm, program_id, payer, &app, app_id, tag_id);
    (app, tag_pdas)
}
