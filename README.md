# soroban-community-treasury

A DAO community treasury built on [Stellar's Soroban](https://soroban.stellar.org) platform. Members deposit tokens into a shared pool, propose spending requests, and vote with token-weighted governance. Passed proposals enter a veto window before funds are released. Designed as the on-chain treasury and governance primitive for DAOs, community funds, and protocol treasuries on Stellar.

---

## Features

- **Collective treasury** — any address may deposit treasury tokens into the shared pool; there are no individual balances or refund rights
- **Token-weighted voting** — vote weight equals the voter's governance token balance at cast time; whales have more say, not equal to 1-member-1-vote (see `soroban-mutual-aid` for that)
- **Three-direction votes** — `For`, `Against`, `Abstain`; abstains count toward the quorum denominator, making large abstain blocs harder to reach quorum
- **Configurable quorum** — minimum `for_votes / total_votes` in basis points (e.g. 5100 = 51%); both a majority AND quorum threshold must be met
- **Voting window** — proposals accept votes for a configurable duration; no votes accepted after the deadline
- **Veto period** — between queue and execution, the admin may cancel a proposal; an on-chain safety layer for malicious or erroneous approvals
- **Permissionless queue and execute** — anyone may call `queue_proposal` after the window closes, and `execute_proposal` after the veto period; no privileged executor required
- **Fund reservation** — queued proposals immediately reserve their amount from the available balance, preventing over-commitment of the treasury
- **Spending cap** — optional per-proposal maximum; 0 = no cap
- **Proposer cancel** — a proposer may cancel their own Active proposal before the voting window closes
- **Admin cancel (veto)** — admin may cancel any non-executed proposal at any lifecycle stage
- **Full governance token requirement** — both proposers and voters must hold at least 1 governance token unit at call time

---

## How It Works

```
Any address ──► deposit(amount)
                    │  treasury_balance += amount
                    ▼
Gov holder  ──► submit_proposal(title, description, recipient, amount)
                    │  requires gov_balance > 0
                    │  requires amount ≤ available_balance (and ≤ cap if set)
                    ▼
             ProposalStatus::Active
             voting_deadline = now + voting_window
                    │
Gov holders ──► vote(For | Against | Abstain)
                    │  weight = governance_token_balance
                    │
            [voting_deadline passes]
                    │
Anyone      ──► queue_proposal()
                    │
            ┌─── PASS: for > against AND for/total >= quorum_bps ───────────┐
            │                                                                  │
            │                                               ProposalStatus::Queued
            │                                               reserved_balance += amount
            │                                               executable_at = now + veto_period
            │
            └─── FAIL: quorum not met OR against wins ───────────────────────┐
                                                                               │
                                                              ProposalStatus::Defeated
                                                              (reserved_balance unchanged)
                                          │
                              [veto period — admin may cancel]
                                          │
Anyone      ──► execute_proposal()  (after executable_at)
                    │
             treasury_balance -= amount
             reserved_balance -= amount
             treasury_token transferred to proposal.recipient
             ProposalStatus::Executed
```

---

## Quorum Formula

```
total_votes  =  for_votes + against_votes + abstain_votes
quorum_met   =  for_votes × 10_000 / total_votes  >=  quorum_bps
majority_met =  for_votes > against_votes

passes = quorum_met AND majority_met
```

**Example — quorum_bps = 5100 (51%):**

| For | Against | Abstain | for% | Result |
|---|---|---|---|---|
| 900 | 100 | 0 | 90% | ✅ Pass |
| 510 | 490 | 0 | 51% | ✅ Pass |
| 500 | 500 | 0 | 50% | ❌ Defeat (majority not met) |
| 51 | 0 | 100 | 33.7% | ❌ Defeat (abstain dilutes) |
| 0 | 0 | 0 | — | ❌ Defeat (no votes) |

---

## Fund Accounting

```
treasury_balance  =  Σ deposits  −  Σ executed payouts
reserved_balance  =  Σ amounts of Queued proposals (not yet executed)
available_balance =  treasury_balance − reserved_balance
```

New proposals may only be submitted if `available_balance >= requested_amount`. This prevents over-committing funds across multiple concurrent proposals.

---

## Proposal Lifecycle

| Status | Description |
|---|---|
| `Active` | Voting window open; votes accepted; can be cancelled by proposer or admin |
| `Defeated` | Window closed; quorum not met or against won; no funds reserved |
| `Queued` | Passed vote; funds reserved; inside veto window |
| `Executed` | Funds transferred to recipient; irreversible |
| `Cancelled` | Cancelled before execution; reserved funds (if any) released |

---

## Storage Layout

| Key | Type | Scope | Description |
|---|---|---|---|
| `INIT` | `bool` | Instance | Initialization guard |
| `Config` | `Config` | Instance | Admin, tokens, quorum, windows, cap |
| `TreasuryBalance` | `i128` | Instance | Total deposited minus executed payouts |
| `ReservedBalance` | `i128` | Instance | Sum of Queued proposal amounts |
| `NextProposalId` | `u64` | Instance | Proposal ID counter |
| `ProposalIndex` | `Vec<u64>` | Instance | All proposal IDs in submission order |
| `Proposal(id)` | `Proposal` | Persistent | Full proposal record |
| `ProposerProposals(addr)` | `Vec<u64>` | Persistent | Proposal IDs per proposer |
| `Vote(key)` | `VoteDirection` | Persistent | Per-(proposal, voter) vote direction |

---

## Public Interface

### Setup

#### `initialize`
```rust
pub fn initialize(
    env: Env,
    admin: Address,
    treasury_token: Address,    // token held in treasury and paid out
    governance_token: Address,  // token determining vote weight
    quorum_bps: u32,            // 1–10000; e.g. 5100 = 51%
    voting_window: u64,         // seconds votes are accepted
    veto_period: u64,           // seconds between queue and earliest execution
    spending_cap: i128,         // max per-proposal amount; 0 = no cap
)
```
Deploy the treasury. Can only be called once.

---

### Admin

#### `update_config`
```rust
pub fn update_config(
    env: Env,
    admin: Address,
    quorum_bps: u32,
    voting_window: u64,
    veto_period: u64,
    spending_cap: i128,
)
```
Update governance parameters. Admin only. Takes effect on the next proposal.

#### `transfer_admin`
```rust
pub fn transfer_admin(env: Env, admin: Address, new_admin: Address)
```
Transfer admin rights. Admin only.

---

### Treasury Funding

#### `deposit`
```rust
pub fn deposit(env: Env, depositor: Address, amount: i128)
```
Deposit treasury tokens into the collective pool. Any address may call. Tokens cannot be individually withdrawn.

---

### Proposals

#### `submit_proposal`
```rust
pub fn submit_proposal(
    env: Env,
    proposer: Address,
    title: String,
    description: String,
    recipient: Address,   // receives funds on execution
    amount: i128,         // treasury tokens requested
) -> u64                  // returns proposal ID
```
Submit a spending proposal. Caller must hold ≥ 1 governance token.

**Validations:** `amount > 0`; title and description non-empty; `amount ≤ available_balance`; `amount ≤ spending_cap` if cap set.

---

### Voting

#### `vote`
```rust
pub fn vote(env: Env, voter: Address, proposal_id: u64, direction: VoteDirection)
```
Cast a weighted vote. Weight = caller's governance token balance at call time. Caller must hold ≥ 1 governance token. One vote per address per proposal.

`VoteDirection`: `For` | `Against` | `Abstain`

---

### Queue and Execute

#### `queue_proposal`
```rust
pub fn queue_proposal(env: Env, proposal_id: u64)
```
Resolve an Active proposal after its voting window closes. **Permissionless.** Transitions to `Queued` (if passed) or `Defeated`. Reserves funds on queue.

**Requires:** `now > voting_deadline`.

#### `execute_proposal`
```rust
pub fn execute_proposal(env: Env, proposal_id: u64)
```
Execute a Queued proposal after the veto period expires. **Permissionless.** Transfers funds to recipient.

**Requires:** `now >= executable_at`.

---

### Cancel / Veto

#### `cancel_proposal`
```rust
pub fn cancel_proposal(env: Env, caller: Address, proposal_id: u64)
```
Cancel a non-executed proposal.

- **Admin** — may cancel any `Active`, `Queued`, or `Defeated` proposal at any time.
- **Proposer** — may only cancel their own `Active` proposal before the voting window closes.

Cancelling a `Queued` proposal releases its reserved funds.

---

### Queries

| Function | Returns |
|---|---|
| `get_config(env)` | `Config` — all governance parameters |
| `get_treasury_balance(env)` | `i128` — total deposited minus executed |
| `get_reserved_balance(env)` | `i128` — sum of Queued proposal amounts |
| `get_available_balance(env)` | `i128` — treasury minus reserved |
| `get_proposal(env, id)` | `Proposal` — full record |
| `get_all_proposals(env)` | `Vec<u64>` — all proposal IDs |
| `get_proposer_proposals(env, addr)` | `Vec<u64>` — proposals per proposer |
| `get_vote(env, id, voter)` | `Option<VoteDirection>` — vote direction if cast |
| `has_voted(env, id, voter)` | `bool` — whether address has voted |

---

## Data Types

### `Config`
```rust
pub struct Config {
    pub admin: Address,
    pub treasury_token: Address,   // held in pool; paid out on execution
    pub governance_token: Address, // determines voting weight
    pub quorum_bps: u32,           // min for% in basis points (1–10000)
    pub voting_window: u64,        // seconds votes accepted after submission
    pub veto_period: u64,          // seconds between queue and execution
    pub spending_cap: i128,        // 0 = no cap
}
```

### `Proposal`
```rust
pub struct Proposal {
    pub id: u64,
    pub proposer: Address,
    pub title: String,
    pub description: String,
    pub recipient: Address,         // funds go here on execution
    pub amount: i128,
    pub status: ProposalStatus,     // Active | Defeated | Queued | Executed | Cancelled
    pub for_votes: u64,
    pub against_votes: u64,
    pub abstain_votes: u64,         // dilutes quorum denominator
    pub voting_deadline: u64,       // submitted_at + voting_window
    pub executable_at: u64,         // set on queue; 0 while Active/Defeated
    pub submitted_at: u64,
    pub executed_at: u64,           // 0 until Executed
}
```

### `VoteKey`
```rust
pub struct VoteKey {
    pub proposal_id: u64,
    pub voter: Address,
}
```
Composite key for vote storage. A dedicated struct is required because `#[contracttype]` does not support multi-field tuple enum variants.

---

## Getting Started

### Prerequisites

- [Rust](https://www.rust-lang.org/tools/install) with `wasm32-unknown-unknown` target
- [Stellar CLI](https://developers.stellar.org/docs/tools/developer-tools/cli/install-cli)

```bash
rustup target add wasm32-unknown-unknown
```
### Build

```bash
cargo build --target wasm32-unknown-unknown --release
```

Output:
```
target/wasm32-unknown-unknown/release/soroban_community_treasury.wasm
```

### Test

```bash
cargo test
```

Expected output:
```
running 50 tests
test test::test_initialize_stores_config ... ok
test test::test_full_proposal_lifecycle_pass ... ok
...
test result: ok. 50 passed; 0 failed
```

---

## Example Walkthrough

### 1. Deploy and initialize

```bash
stellar contract deploy \
  --wasm target/wasm32-unknown-unknown/release/soroban_community_treasury.wasm \
  --source deployer --network testnet

# 51% quorum, 7-day voting window, 2-day veto period, no spending cap
stellar contract invoke \
  --id <CONTRACT_ID> --source deployer --network testnet \
  -- initialize \
  --admin <ADMIN_ADDRESS> \
  --treasury_token <TREASURY_TOKEN_ADDRESS> \
  --governance_token <GOV_TOKEN_ADDRESS> \
  --quorum_bps 5100 \
  --voting_window 604800 \
  --veto_period 172800 \
  --spending_cap 0
```

### 2. Fund the treasury

```bash
stellar contract invoke \
  --id <CONTRACT_ID> --source donor --network testnet \
  -- deposit --depositor <DONOR_ADDRESS> --amount 100000
```

### 3. Submit a spending proposal

```bash
stellar contract invoke \
  --id <CONTRACT_ID> --source proposer --network testnet \
  -- submit_proposal \
  --proposer <PROPOSER_ADDRESS> \
  --title "Q1 Developer Grants" \
  --description "Fund 3 developer grants for ecosystem tools" \
  --recipient <RECIPIENT_ADDRESS> \
  --amount 30000
```

### 4. Community votes

```bash
stellar contract invoke \
  --id <CONTRACT_ID> --source voter1 --network testnet \
  -- vote --voter <VOTER_1> --proposal_id 1 --direction '{"For":{"_":0}}'

stellar contract invoke \
  --id <CONTRACT_ID> --source voter2 --network testnet \
  -- vote --voter <VOTER_2> --proposal_id 1 --direction '{"Against":{"_":0}}'
```

### 5. Queue after voting window (permissionless)

```bash
stellar contract invoke \
  --id <CONTRACT_ID> --source anyone --network testnet \
  -- queue_proposal --proposal_id 1
```

### 6. Admin veto (optional — within veto period)

```bash
stellar contract invoke \
  --id <CONTRACT_ID> --source admin --network testnet \
  -- cancel_proposal --caller <ADMIN_ADDRESS> --proposal_id 1
```

### 7. Execute after veto period (permissionless)

```bash
stellar contract invoke \
  --id <CONTRACT_ID> --source anyone --network testnet \
  -- execute_proposal --proposal_id 1
```

---

## Test Coverage

| Category | Tests |
|---|---|
| Initialization | config stored, balances start at zero, double-init, zero quorum, quorum > 10000, zero voting window, zero veto period |
| Admin config | update config, transfer admin, non-admin blocked |
| Deposits | increases balance, multiple deposits accumulate, tokens transferred to contract, zero deposit panics |
| Proposal submission | record stored, sequential IDs, global index, proposer index, no gov tokens, above available, above spending cap, empty title, zero amount |
| Voting | For increments for_votes, Against increments against_votes, Abstain increments abstain_votes, weight proportional to balance, has_voted tracked, get_vote returns direction, double vote, voter without tokens, vote after window, vote on cancelled, |
| Queue / defeat | queued status set, reserved on queue, defeated when quorum not met, defeated when against wins, defeated with no votes, permissionless, queue before window panics |
| Execution | transfers funds to recipient, deducts treasury balance, clears reserved, sets status + timestamp, permissionless, execute during veto panics, execute active panics, execute defeated panics |
| Cancel / veto | proposer cancels active, admin cancels active, admin veto releases reserve, admin cancels defeated, proposer cannot cancel queued, third party cannot cancel, cancel executed panics, double cancel, proposer cancel after window |
| Balance accounting | two queued proposals both reserved, cannot over-commit treasury |
| End-to-end | full lifecycle pass (deposit → propose → vote → queue → execute), full lifecycle veto, two proposals one passes one fails, abstain counts toward quorum denominator |

---

## License

MIT
