# Deployment guide

Every step to take `nebulous.world` from a checkout to a running deployment,
on **devnet** (cheap, disposable, for testing) or **mainnet-beta** (real SOL,
real users, effectively permanent once `initialize` is called — see
"Deployment ordering matters" below). Read this straight through once before
running anything on mainnet; the two paths share almost every command, and
the differences are called out inline and summarized in the table below.

This repo has three deployable pieces, in dependency order:

| # | Piece | What | Where |
| - | --- | --- | --- |
| 1 | `programs/nebulous_world` | The on-chain Anchor program | Solana cluster (devnet/mainnet-beta) |
| 2 | NEB token + DLMM pool | `app/scripts/launch-neb/` — mints the vote token, seeds a Meteora pool | Same Solana cluster |
| 3 | App + indexer + docs + cron + Postgres | `render.yaml` Blueprint | Render |

The indexer is the app's *only* path to Solana RPC (see `indexer/README.md`)
— nothing in `app/` opens a `Connection`. That means the deploy order below
matters: the chain-side pieces (1–2) must exist before the infra pieces (3)
have anything real to point at.

**Automating this:** `app/scripts/deploy/` runs phases 1–4 below (program
deploy, NEB launch, Config init, app seeding) plus 5b's env var sync end to
end from one JSONC config, instead of by hand — see `app/README.md`'s
"Production deploy" section. It still leaves 5a (the one-time Blueprint
launch) and 5c/hardening to you. Useful for redeploys or once you've already
read through this guide once; read the phases below first regardless, since
the script's config fields assume the same context this document covers
(ordering, the bricking warning in phase 3, mainnet vs devnet differences).

## At a glance: devnet vs mainnet

| | Devnet | Mainnet-beta |
| --- | --- | --- |
| Cost | Free (airdropped SOL) | Real SOL — program deploy alone is typically a few SOL in rent |
| Quote token for NEB/USDC pool | Devnet USDC: `4zMMC9srt5Ri5X14GAgXhaHii3GnPAEERYPJgZJDncDU` (Circle devnet faucet) | Real USDC: `EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v` |
| RPC | Public `https://api.devnet.solana.com` is fine to start, but is noted in this repo (`app/.env.example`) as unreliable for deploys/indexer polling under load | Use a **paid** RPC provider (Helius, QuickNode, Triton, …) from day one — the free public endpoint's rate limit is easily tripped by the indexer's crawler + this app's own transaction bursts, see `app/.env.example`'s `SURFPOOL_DATASOURCE_RPC_URL` comment for the exact failure mode |
| Program upgrade authority | Your local deploy keypair is fine | Move it to a multisig (e.g. [Squads](https://squads.so)) once launch is stable — see "Hardening" below |
| Deployer keypair | Any throwaway keypair, airdrop as needed | A keypair you control custody of carefully; consider a hardware wallet for the upgrade authority specifically |
| `protocol_fee_bps` / vote mint | Pick anything, redo freely (just redeploy) | **Permanent** — there's no `update_config` instruction (see `programs/nebulous_world/src/lib.rs`); decide these once, correctly, before calling `initialize` |
| Turnstile / ad revenue | Can stay unset (page views just aren't revenue-eligible) | Set it up if ad revenue actually needs to accrue for real |
| Blast radius of a mistake | None — redeploy under a fresh program id any time | Real user funds move through `AppAccount::vote_vault` / `principal_vault`; get a security review first |

## Prerequisites

Install once, works for both environments:

```bash
# Rust (pinned to 1.89.0 by rust-toolchain.toml — rustup will pick it up automatically)
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# Solana CLI
sh -c "$(curl -sSfL https://release.anza.xyz/stable/install)"

# Anchor CLI (via avm), pinned to 1.0.2 to match Cargo.toml's anchor-lang version
cargo install --git https://github.com/coral-xyz/anchor avm --locked
avm install 1.0.2 && avm use 1.0.2

# Node.js (LTS) + this repo's own deps
node --version   # 20 or 22 LTS; docs-site's Mintlify build additionally requires <25
npm install                      # Anchor workspace deps (root)
cd app && npm install && cd ..   # app deps (separate install — see README's "Getting started")
```

Confirm the toolchain matches what this repo was built against:

```bash
solana --version   # this repo was last verified against solana-cli 3.1.7 (Agave)
anchor --version    # anchor-cli 1.0.2 — must match, IDL/type generation is version-sensitive
```

You'll also need:
- A funded Solana keypair to deploy from. `Anchor.toml`'s `[provider].wallet`
  defaults to `~/.config/solana/id.json`; generate one if you don't have it:
  `solana-keygen new --outfile ~/.config/solana/id.json`.
- For mainnet specifically: real SOL in that wallet (buy on an exchange,
  withdraw to the wallet's pubkey — `solana-keygen pubkey ~/.config/solana/id.json`)
  and a small amount of real USDC (for the NEB/USDC pool's anti-rug check,
  see phase 2).
- A [Render](https://render.com) account with this repo connected, for phase 3.

---

## Phase 1 — Deploy the Anchor program

The program id is pinned two places that must agree: `declare_id!(...)` in
`programs/nebulous_world/src/lib.rs`, and `[programs.<cluster>]` in
`Anchor.toml`. Today `Anchor.toml` only has entries for `localnet` and
`devnet` — mainnet needs a new one.

### 1a. Point the Solana CLI at your target cluster

```bash
solana config set --url https://api.devnet.solana.com      # devnet
# or
solana config set --url https://your-paid-rpc.example.com  # mainnet — use your paid RPC, not the public endpoint
solana balance   # confirm funds are there before continuing
```

Devnet only: airdrop yourself SOL if the balance is low —
`solana airdrop 2` (public devnet faucet, rate-limited; use
https://faucet.solana.com in a browser if the CLI faucet is exhausted).

### 1b. Get a program keypair for this cluster

**Devnet:** `Anchor.toml` already has a devnet program id
(`EkQRRgRFd2FUedJnPVs2Xs6N7U2Jef5GrfwJ62UJZUXx`) from a prior deploy. If you
have that exact keypair file (normally at
`target/deploy/nebulous_world-keypair.json`, gitignored so it won't be in a
fresh clone), drop it in place and skip to 1c. If you don't have it, treat
this like a fresh launch under a new id — proceed as below and expect
`Anchor.toml`'s devnet entry to change.

**Mainnet (or a fresh devnet id):** generate a new program keypair, add a
cluster entry for it, then let Anchor sync everything to match:

```bash
solana-keygen new --outfile target/deploy/nebulous_world-keypair.json
solana-keygen pubkey target/deploy/nebulous_world-keypair.json
```

Add the printed pubkey to `Anchor.toml`:

```toml
[programs.mainnet]
nebulous_world = "<paste the pubkey here>"
```

Then sync `declare_id!()` in `lib.rs` and every `Anchor.toml` entry to match
the keypair file:

```bash
anchor keys sync
```

Commit the `Anchor.toml` change (the keypair file itself stays gitignored —
back it up somewhere safe instead, especially for mainnet: losing it means
losing the ability to upgrade the program unless you've already moved
upgrade authority elsewhere).

### 1c. Build and deploy

```bash
anchor build
```

Verify the build picked up the right id — `anchor build` warns loudly on a
program-id mismatch between `declare_id!()` and the active
`[provider].cluster` in `Anchor.toml`. Then set `[provider].cluster` in
`Anchor.toml` (or pass `--provider.cluster` explicitly) and deploy:

```bash
anchor deploy --provider.cluster devnet    # or: --provider.cluster mainnet
```

This is the expensive step on mainnet — the CLI prints the SOL cost before
sending (roughly proportional to the compiled program's `.so` size, paid as
rent for the program's on-chain storage). Check it landed:

```bash
solana program show <program id> --url <cluster rpc url>
```

`anchor build` also generates `target/idl/nebulous_world.json` and
`target/types/nebulous_world.ts` — required by `app/scripts/launch-neb/`,
`scripts/settleEpoch.ts`, and `scripts/ensureConfigInitialized.ts` in the
next phases, but not by the deployed app itself (it never imports the
program directly).

---

## Phase 2 — Launch the NEB token

NEB isn't minted by the Anchor program — a separate script mints the full
supply with Metaplex metadata, then creates a NEB/USDC Meteora DLMM pool and
seeds it single-sided with that entire supply. Buying NEB is a swap against
this public pool, not an instruction on `nebulous_world`.

```bash
cp app/scripts/launch-neb/launch-neb.config.example.jsonc \
   app/scripts/launch-neb/launch-neb.config.json
```

Edit `launch-neb.config.json`:

| Field | Devnet | Mainnet |
| --- | --- | --- |
| `rpcUrl` | `https://api.devnet.solana.com` | your paid RPC |
| `cluster` | `"devnet"` | `"mainnet-beta"` |
| `keypairFilePath` | your dev keypair | your funded deployer keypair — **becomes the mint/update authority and DLMM pool creator** |
| `quoteMint` | `4zMMC9srt5Ri5X14GAgXhaHii3GnPAEERYPJgZJDncDU` (devnet USDC) | `EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v` (real USDC) |
| `dryRun` | `true` first, then `false` | `true` first — review the printed plan carefully, then `false` |
| `token.uri` | any placeholder JSON URL | real, permanently-hosted metadata JSON (Arweave/IPFS recommended over a URL you might stop paying for) |
| `token.totalSupply`, `pool.initialPrice`, `pool.binStep`, `pool.feeBps`, `pool.maxPriceMultiplier` | whatever's convenient for testing | **decide deliberately** — this is the real, permanent supply and starting price |
| `token.revokeMintAuthority` | `true` | `true` — this is what makes the supply provably fixed; double check `totalSupply` before running, there's no minting more after |

The deployer wallet needs, before running for real:
- Enough SOL to cover the mint, metadata account, pool, and position rent
  (a few tenths of a SOL is comfortable).
- A **nonzero balance of the quote token** (USDC) — any amount, even a
  fraction of a cent. The DLMM program rejects pool creation from a wallet
  holding only the freshly-minted base token
  (`MissingTokenAmountAsTokenLaunchProof`, an anti-rug check). Devnet USDC:
  https://faucet.circle.com. Mainnet: buy or swap for a small amount of real
  USDC first.

Run it:

```bash
npm run launch:neb -- --config=./scripts/launch-neb/launch-neb.config.json
```

Leave `dryRun: true` until you've reviewed the printed plan (computed active
bin, mint params, pool params) — it sends nothing in dry-run mode. Once it
runs for real, it prints the resulting mint and pool addresses as ready-to-paste
`KEY="VALUE"` lines. Set those as `NEXT_PUBLIC_VOTE_TOKEN_MINT` and
`NEXT_PUBLIC_NEB_DLMM_POOL` in `app/.env` (for the next phase) and — critically
— in the Render service env vars for both `nebulous-world` and
`nebulous-world-indexer` (phase 3). They must match on both services; the
indexer derives token accounts against whatever mint the app tells it to use
when it builds vote/stake/claim transactions.

Leaving `NEXT_PUBLIC_VOTE_TOKEN_MINT` unset instead runs the whole product in
**simulation mode** — votes/stakes recorded off-chain, no real token
required. That's a legitimate way to run either environment if you're not
ready to launch NEB yet; app/tag creation (`init_app`/`suggest_tag`) is
always real on-chain regardless of this setting. See README's "Simulation vs
on-chain mode".

---

## Phase 3 — Initialize the program's `Config`

`Config` is the program's one global singleton (vote mint, protocol fee) and
only ever gets created by a single, one-time `initialize` call — every
vote/stake instruction fails with `AccountNotInitialized` until it runs.

**Deployment ordering matters, and this is the one truly irreversible step:**
`initialize` checks that its signer is the program's *current* upgrade
authority (closing a front-running window where anyone could otherwise race
you to seize `Config.authority` — see
`programs/nebulous_world/src/instructions/initialize.rs`). If you ever
finalize the program (`solana program set-upgrade-authority --final`, or
otherwise revoke upgrade authority) **before** calling `initialize`, that
check can never be satisfied again and `Config` — a fixed-address PDA — can
never be created. **The deployment is permanently bricked.** Always call
`initialize` before touching upgrade authority.

```bash
NEXT_PUBLIC_SOLANA_RPC=<cluster rpc url> \
NEXT_PUBLIC_NEBULOUS_WORLD_PROGRAM_ID=<deployed program id> \
NEXT_PUBLIC_VOTE_TOKEN_MINT=<NEB mint from phase 2> \
npm run ensure:config
```

This runs `app/scripts/ensureConfigInitialized.ts`, signed by whatever
keypair is at `~/.config/solana/id.json` (must be the program's current
upgrade authority — normally whoever ran `anchor deploy` in phase 1). It's
idempotent — safe to re-run, it no-ops if `Config` already exists.

The script currently hardcodes `protocol_fee_bps = 250` (2.5%) for local dev
convenience. **For mainnet, decide this number deliberately before running**
— open `app/scripts/ensureConfigInitialized.ts` and change the
`.initialize(250)` argument first. There is no `update_config` instruction
in this program (see the instruction table in
`programs/nebulous_world/AGENTS.md`), so both the vote mint and the protocol
fee are effectively permanent for the life of this deployment; changing
either later means shipping a program upgrade that adds an update
instruction, not just re-running this script.

---

## Phase 4 — Populate apps on-chain (optional, either environment)

There's no database seed script anywhere in this repo — `App`/`Tag` rows
only exist once a real `init_app`/`suggest_tag` transaction confirms
on-chain and the indexer picks it up (see "Database ownership" in the root
`README.md`). To bootstrap a non-empty product instead of waiting for
organic "Create app" submissions:

```bash
DEPLOYER_KEYPAIR_PATH=/path/to/funded-keypair.json \
NEXT_PUBLIC_SOLANA_RPC=<cluster rpc url> \
NEXT_PUBLIC_NEBULOUS_WORLD_PROGRAM_ID=<deployed program id> \
npm run apps:create-onchain
```

This sends one real `init_app` (+ `suggest_tag` per initial tag) transaction
per entry in `app/scripts/appData/apps.json` — the same instructions the
app's own "Create app" UI builds, just scripted, and idempotent (a second
run only creates whatever's still missing since each `app_id` is derived
deterministically from its URL). There's deliberately no automated step for
this in `render.yaml` — it needs a funded keypair, which isn't something to
hand to an automatic Render build. Run it by hand against each environment
once its program is deployed and initialized.

---

## Phase 5 — Deploy the infra (Render)

`render.yaml` at the repo root is a [Render Blueprint](https://render.com/docs/blueprint-spec)
defining every service: the Next.js app, the indexer (private service), the
Mintlify docs static site, a daily-snapshot cron job, and a managed Postgres
instance. One Blueprint launch creates all of them wired together.

### 5a. Launch the Blueprint

In the Render dashboard: **New +** → **Blueprint** → connect this GitHub repo
→ Render reads `render.yaml` and shows every service it's about to create →
confirm.

### 5b. Set the cluster-specific env vars

`render.yaml` as checked in defaults every Solana-related var to **devnet**.
For a devnet deployment these need no changes. For **mainnet**, update these
on both the `nebulous-world` (app) and `nebulous-world-indexer` services in
the Render dashboard (Environment tab) — they're not meant to be edited in
`render.yaml` and committed, since some are per-deployment secrets:

| Var | Service(s) | Devnet value (in `render.yaml`) | Mainnet value |
| --- | --- | --- | --- |
| `NEXT_PUBLIC_SOLANA_RPC` | app, indexer | `https://api.devnet.solana.com` | your paid RPC's HTTPS URL |
| `NEXT_PUBLIC_SOLANA_CLUSTER` | app, indexer | `devnet` | `mainnet-beta` |
| `NEXT_PUBLIC_NEBULOUS_WORLD_PROGRAM_ID` | app, indexer | devnet program id | mainnet program id from phase 1 |
| `NEXT_PUBLIC_VOTE_TOKEN_MINT` | app, indexer | unset (`sync: false`) | NEB mint from phase 2 — **must match on both services** |
| `NEXT_PUBLIC_TREASURY_ADDRESS` | app | unset | a wallet you control (ideally a multisig) — also the x402 `payTo` for `/api/data/*` |
| `NEXT_PUBLIC_NEB_DLMM_POOL` | indexer | unset | DLMM pool address from phase 2 |
| `RPC_WS_URL` | indexer | unset (derived from the RPC URL) | only set this if your paid RPC's WebSocket endpoint uses a different host/port than the HTTPS one — see `indexer/src/config.rs`'s `default_ws_url` |

Everything else in `render.yaml` (build/start commands, service topology,
the Postgres instance, `TRACKING_SECRET` auto-generation, plan/region) is
identical between environments — only the Solana-facing values change.

### 5c. Production hardening (recommended for a real mainnet launch, optional on devnet)

- **Turnstile**: set `NEXT_PUBLIC_TURNSTILE_SITE_KEY` / `TURNSTILE_SECRET_KEY`
  (Cloudflare Turnstile) on the app service — without them, page views are
  recorded but never marked revenue-eligible (see `app/src/lib/turnstile.ts`).
- **Custom domains**: set `NEXT_PUBLIC_SITE_URL` (app) and optionally
  `NEXT_PUBLIC_DOCS_URL` once a custom docs domain is attached (defaults to
  the `nebulous-world-docs` service's own `onrender.com` URL otherwise).
- **`nebulous-world-daily-snapshot`** (the cron service) and
  `npm run settle:epoch` (revenue settlement — not currently scheduled in
  `render.yaml`, run manually or add your own cron entry) both need the same
  `INDEXER_API_URL` wiring already present for the app; nothing extra to
  configure beyond what the Blueprint sets up.

### 5d. Verify

- App: hit the deployed URL, confirm the homepage loads and shows apps
  (empty until phase 4's `apps:create-onchain` runs, or until real
  "Create app" submissions land).
- Indexer: check its Render logs for `sqlx::migrate!()` running cleanly at
  startup (it owns and applies the whole Postgres schema itself — see
  `indexer/README.md`) and for the backfill/crawler picking up program
  accounts.
- Try a real vote or stake against a live app on the deployed site with a
  wallet holding the vote-token mint, and confirm it lands on-chain
  (`solana confirm -v <signature>` or a block explorer) and shows up in the
  UI shortly after (indexer polling latency, not instant).

---

## Hardening for mainnet (do this once launch is stable)

- **Move the upgrade authority off your local hot-wallet keypair** once
  you're confident in the deployed program — a [Squads](https://squads.so)
  (or similar) multisig is the common Solana pattern:
  `solana program set-upgrade-authority <program id> --new-upgrade-authority <multisig address>`.
  Do this *after* phase 3's `initialize` has already succeeded — see the
  bricking warning there. Do **not** run `--final` (which revokes upgrade
  authority entirely, making the program permanently immutable) unless you
  are certain you'll never need to ship a fix.
- **Security review before real funds are at risk.** `SECURITY_REVIEW.md` at
  the repo root documents the existing review of `programs/nebulous_world`
  (automated `solana-security-standard` scan + manual review, both findings
  already fixed). Re-run a scan after any program change before a mainnet
  deploy:
  ```bash
  # via the solana-security-standard plugin, if available in your environment
  # scan_solana_code({ path: "programs/nebulous_world/src" })
  ```
- **Back up every keypair that matters** — the program keypair (if upgrade
  authority is still a direct keypair rather than a multisig) and the NEB
  mint/pool deployer keypair (mint authority is revoked after launch, so
  losing this key afterward isn't catastrophic, but you'll want it for any
  pool-admin actions first).
- **Treasury custody**: `NEXT_PUBLIC_TREASURY_ADDRESS` collects real x402
  payments once configured — use a wallet with real operational custody
  practices behind it (multisig, hardware wallet), not a throwaway keypair.

## Rolling out a program upgrade later

Once mainnet is live and the upgrade authority is a multisig, a program
change is: `anchor build` → get the multisig to sign
`solana program deploy --program-id <id> --upgrade-authority <multisig> target/deploy/nebulous_world.so`
(via whatever your multisig's tooling provides, e.g. Squads' UI) → done. No
change to `Config` or existing on-chain accounts unless the upgrade's own
instruction logic migrates them — Anchor upgrades preserve account data by
default.

## Troubleshooting

| Symptom | Likely cause |
| --- | --- |
| `anchor build` warns about a program id mismatch | `declare_id!()` in `lib.rs` doesn't match `Anchor.toml`'s entry for the active `[provider].cluster` — run `anchor keys sync` |
| Every vote/stake instruction fails `AccountNotInitialized` | Phase 3 (`ensure:config`) hasn't run yet against this program deployment |
| `initialize` fails `Unauthorized` | The signer isn't the program's current upgrade authority — check `solana program show <program id>` |
| App loads but shows no apps | Normal on a fresh deployment — either run phase 4 or wait for real "Create app" submissions; also check the indexer's logs actually connected to Postgres and finished its backfill |
| Votes/stakes don't show up in the UI after confirming on-chain | Indexer polling latency (`CRAWLER_POLL_INTERVAL_SECS`, default 15s) — not a bug, give it a few seconds; if it never appears, check the indexer's crawler logs for RPC errors (rate limiting on a free/public endpoint is the usual cause) |
| DLMM pool creation fails `MissingTokenAmountAsTokenLaunchProof` | Deployer wallet holds zero of the quote token (USDC) — fund it with any nonzero amount first |
