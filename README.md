# finnanssce_plus

A Soroban smart contract that brings transparent, auditable treasury governance to Student Supreme Council (SSC) funds on Stellar testnet.

`finnanssce_plus` lets SSC-administered organizations submit funding proposals, enables one-wallet-one-vote community voting, and finalizes outcomes on-chain with verifiable disbursement records. Every critical action (initialize, whitelist, proposal creation, voting, execution) is permanently recorded, creating a trust-minimized governance trail for campus finance decisions.

---

## Current Status

- Smart contract: implemented
- Testnet deployment: live
- Unit tests: passing
- Frontend: scaffold only (not connected yet)

Note: this repository has a `frontend/` scaffold, but there is currently no connected user interface for the contract.

---

## Features

- Admin initialization with one-time protection
- Whitelist-based org access for proposal creation
- Proposal lifecycle tracking (`Pending`, `Executed`, `Rejected`)
- One-address-one-vote per proposal
- Quorum-triggered finalization (`QUORUM = 3`)
- Auto-execution on `YES` majority at quorum
- Admin fallback execution path
- Event emission on init, whitelist, proposal, vote, and disbursement

---

## How It Works

```text
Initialize admin + token contract
                |
                v
Whitelist org wallet
                |
                v
Create proposal (campus, amount, description)
                |
                v
Students vote yes/no
                |
                +--> if total votes < 3: keep Pending
                |
                +--> if total votes >= 3 and yes > no:
                |        transfer tokens to org wallet
                |        mark Executed
                |
                +--> else:
                         mark Rejected
```

---

## Contract Interface

### `initialize`

```rust
pub fn initialize(env: Env, admin: Address, token_contract: Address) -> Result<(), Error>
```

Initializes contract once with admin and token contract.

### `whitelist_org`

```rust
pub fn whitelist_org(env: Env, admin: Address, org: Address) -> Result<(), Error>
```

Whitelists an org wallet for proposal creation.

### `create_proposal`

```rust
pub fn create_proposal(
    env: Env,
    org_wallet: Address,
    campus: String,
    amount: i128,
    description: String,
) -> Result<u64, Error>
```

Creates one pending proposal for a whitelisted org.

### `vote_proposal`

```rust
pub fn vote_proposal(
    env: Env,
    voter: Address,
    proposal_id: u64,
    vote_yes: bool,
) -> Result<ProposalStatus, Error>
```

Casts one `YES/NO` vote for a proposal.

### `execute_proposal`

```rust
pub fn execute_proposal(env: Env, admin: Address, proposal_id: u64) -> Result<(), Error>
```

Admin fallback execution for pending proposals.

### `get_total`-style reads

```rust
pub fn get_proposal(env: Env, proposal_id: u64) -> Result<Proposal, Error>
pub fn get_treasury_balance(env: Env) -> Result<i128, Error>
```

Returns proposal details and treasury token balance.

---

## Data Model

### `Proposal`

```rust
pub struct Proposal {
    pub id: u64,
    pub org_wallet: Address,
    pub campus: String,
    pub amount: i128,
    pub description: String,
    pub status: ProposalStatus,
    pub yes_votes: u64,
    pub no_votes: u64,
}
```

### Persistent Keys

- `Admin`
- `TokenContract`
- `NextProposalId`
- `Proposal(u64)`
- `Voted(Address, u64)`
- `Whitelist(Address)`

---

## Key Invariants

- Contract cannot be initialized twice
- Only whitelisted orgs can create proposals
- A voter cannot vote twice on the same proposal
- Proposal executes only when quorum is reached and `yes_votes > no_votes`
- Treasury must have sufficient token balance before disbursement

---

## Testnet Deployment

- Treasury Contract ID: `CC3SML4J6DVEFZO5DHTCMEPBBTECLPWPFWJQ722625HRWW6ZZ5XQPQGR`
- Token Contract ID: `CDUJVDSWXR2KB6Z2G5SBCRB2H5OQIEHRSWEXQD636NPQC3WD2XGHTCAJ`
- Contract page:
  - https://lab.stellar.org/r/testnet/contract/CC3SML4J6DVEFZO5DHTCMEPBBTECLPWPFWJQ722625HRWW6ZZ5XQPQGR
- Deploy tx:
  - https://stellar.expert/explorer/testnet/tx/28e04be8078bf3327ec4b2b260034af0a37348046ebab6c0eb8dc2818f2eaf12
- Initialize tx:
  - https://stellar.expert/explorer/testnet/tx/130e7e71339d20cb8d3c7268f0848199630984e59a831ad5b9fec34e303face5

---

## Getting Started

### Prerequisites

- Rust toolchain
- Stellar CLI
- Soroban-compatible build target

```bash
rustup target add wasm32v1-none
```

### Build

```bash
stellar contract build
```

Expected artifact:

```text
target/wasm32v1-none/release/finnanssce_plus.wasm
```

### Test

```bash
cargo test
```

### Deploy (example)

```bash
stellar keys generate allen --overwrite --fund -n testnet

stellar contract deploy \
  --wasm target/wasm32v1-none/release/finnanssce_plus.wasm \
  --source-account allen \
  --network testnet \
  --alias finnanssce_plus
```

---

## Quick CLI Usage

Initialize:

```bash
stellar contract invoke \
  --id finnanssce_plus \
  --source-account allen \
  --network testnet \
  -- initialize \
  --admin <ADMIN_G_ADDRESS> \
  --token_contract <TOKEN_CONTRACT_C_ADDRESS>
```

Whitelist org:

```bash
stellar contract invoke \
  --id finnanssce_plus \
  --source-account allen \
  --network testnet \
  -- whitelist_org \
  --admin <ADMIN_G_ADDRESS> \
  --org <ORG_G_ADDRESS>
```

Create proposal:

```bash
stellar contract invoke \
  --id finnanssce_plus \
  --source-account <ORG_ALIAS> \
  --network testnet \
  -- create_proposal \
  --org_wallet <ORG_G_ADDRESS> \
  --campus ALANGILAN \
  --amount 10000000 \
  --description "Test proposal"
```

Vote:

```bash
stellar contract invoke \
  --id finnanssce_plus \
  --source-account <VOTER_ALIAS> \
  --network testnet \
  -- vote_proposal \
  --voter <VOTER_G_ADDRESS> \
  --proposal_id 0 \
  --vote_yes true
```

Get proposal:

```bash
stellar contract invoke \
  --id finnanssce_plus \
  --source-account allen \
  --network testnet \
  -- get_proposal \
  --proposal_id 0
```

---

## Project Structure

- `src/lib.rs` - contract logic
- `src/test.rs` - unit tests
- `frontend/` - frontend scaffold (not connected yet)

---

## Roadmap

- Connect scaffolded frontend to contract calls
- Add clearer role model for who can vote
- Add terminal scripts for repeatable integration flows
- Add richer contract-level query endpoints for analytics

---

## License

MIT
