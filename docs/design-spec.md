# SIS Lattice Multi-Party Payment Channel Design Spec

## 1. Purpose

This document defines a multi-party payment channel design that uses the `sis_amount_stark` crate from `SIS-lattice-private-balance` as a building block for private balances.

The scope of this document is:

- the off-chain channel state updated by unanimous signatures
- the on-chain close / challenge / finalize / withdraw flow
- how `sis_amount_stark` is imported and what responsibilities it covers

## 2. Assumptions and Scope

### 2.1 Assumptions

- The reference implementation is `sis_amount_stark` from `InternetMaximalism/SIS-lattice-private-balance`
- `sis_amount_stark` proves knowledge of one hidden balance commitment opening together with an amount range proof and a bounded-randomness proof
- An external system exists that can prove the total channel balance at close time on-chain
- The final state is the newest state signed by every participant

### 2.2 MVP Scope

- fixed membership multi-party channel
- single asset
- every off-chain update requires signatures from all participants
- challenge does not accept partially signed states
- each participant reveals and withdraws only their own final balance

### 2.3 Out of Scope

- dynamic membership changes
- multi-asset support
- full metadata privacy against parties other than sender and receiver
- production-secure lattice parameter selection
- optimized native on-chain lattice STARK verification

## 3. Import Strategy for the Reference Implementation

The current `SIS-lattice-private-balance` implementation proves the following for a commitment `C`:

- `C = g * a + B * r (mod q)`
- `0 <= a < 2^64`
- `-BETA <= r_i <= BETA`

Here:

- `a` is the hidden balance
- `r` is the randomness vector
- `k` is the quotient witness used for modular equalities

This repository imports that implementation directly rather than copying it locally.

The Rust dependency is intended to be pinned as a git dependency:

```toml
[dependencies]
sis_amount_stark = { git = "https://github.com/InternetMaximalism/SIS-lattice-private-balance.git", rev = "8eedf2d01cf8b0295bebcb6a330ef79eac6bd95f" }
```

The imported crate is used for:

- per-participant balance commitment generation
- transfer amount commitment generation
- individual balance opening proofs
- post-state sender balance proofs
- withdraw-time balance proofs

New implementation outside the reference crate is still required for:

- state hashing, signatures, and challenge rules
- off-chain state transition logic using amount commitments
- witness-share delivery to the receiver
- the on-chain settlement contract

## 4. Key Design Decisions

### 4.1 Visibility Model

The MVP visibility model is:

- sender and receiver participant indices are public
- sender and receiver addresses are public
- each participant balance is private
- transfer amount `delta` is private
- all participants see the updated participant commitment set
- the receiver receives the private witness share needed to maintain their updated witness

### 4.2 Required Proofs and Shared Data During a Payment

Per the current requirements, the cryptographic proof shared with channel participants during a payment is primarily the sender’s balance proof.

State-transition correctness is not proven by an additional transition ZKP. Instead, it is checked from:

- the prior state commitments
- the transfer amount commitment
- commitment addition and subtraction

The minimum update bundle contains:

- amount commitment `A = Commit(delta, r_amount)`
- a range proof for the sender’s post-update balance commitment
- the full updated state
- unanimous signatures on that updated state

For same-channel updates, the receiver additionally needs private data sufficient to update their witness:

- `delta`
- `r_amount`

If the receiver does not obtain that data, they do not sign the updated state.

### 4.3 Transfer Amount Privacy

`delta` is not public.

All participants see the amount commitment `A`, but the opening `(delta, r_amount)` is known only to the sender and receiver in the same-channel flow.

### 4.4 Fixed Membership

The MVP assumes a fixed participant set. Allowing join / leave would significantly complicate the signature set, state root rules, and withdraw authorization model.

## 5. Architecture Overview

The design is split into four layers:

- `proof layer`: lattice balance proofs
- `state layer`: off-chain state, hashes, signed payloads, and validation rules
- `coordination layer`: proposal distribution, signature collection, and latest fully signed state storage
- `settlement layer`: contract logic for close / challenge / finalize / withdraw

## 6. Data Model

### 6.1 ChannelParams

```text
ChannelParams {
  channel_id: bytes32,
  asset_id: bytes32,
  participant_count: u16,
  participants: [Participant],
  challenge_period: uint64,
  settlement_contract: address,
  total_balance_verifier: address,
  proof_system_id: bytes32,
}
```

`proof_system_id` identifies the lattice parameter set and proof format combination.

### 6.2 Participant

```text
Participant {
  index: u16,
  signing_key: address,
  withdraw_key: address,
}
```

The MVP allows `signing_key` and `withdraw_key` to differ so that state signing authority and withdraw destination can be separated.

### 6.3 ParticipantLeaf

Each participant entry in a state is represented as a Merkle leaf:

```text
ParticipantLeaf {
  channel_id: bytes32,
  state_version: uint64,
  participant_index: u16,
  withdraw_key: address,
  balance_commitment: [u64; M],
}
```

The leaf hash includes `channel_id` and `state_version` to prevent reuse across channels or versions.

### 6.4 OffchainState

```text
OffchainState {
  channel_id: bytes32,
  version: uint64,
  prev_state_hash: bytes32,
  participant_root: bytes32,
  participants: Vec<ParticipantLeaf>,
  transition_meta_hash: bytes32,
}
```

The full `participants` vector is kept off-chain. On-chain, only `participant_root` is required.

### 6.5 SignedState

```text
SignedState {
  state: OffchainState,
  signatures: Vec<ParticipantSignature>,
}
```

In the MVP, only states signed by every participant are valid.

### 6.6 UpdateProposalBundle

```text
UpdateProposalBundle {
  channel_id: bytes32,
  next_version: uint64,
  prev_state_hash: bytes32,
  sender_index: u16,
  receiver_index: u16,
  amount_commitment: [u64; M],
  next_participants: Vec<ParticipantLeaf>,
  sender_post_balance_proof: bytes,
  proof_format_version: u32,
}
```

### 6.7 ReceiverWitnessShare

For same-channel updates, the receiver-only private payload is:

```text
ReceiverWitnessShare {
  channel_id: bytes32,
  next_version: uint64,
  sender_index: u16,
  receiver_index: u16,
  delta: u64,
  r_amount: [i64; N],
}
```

This can be delivered as an encrypted payload from sender to receiver.

## 7. Proof Interface

### 7.1 Existing Primitive: BalanceOpeningProof

Uses:

- individual balance opening proofs
- sender post-state balance proofs
- withdraw-time balance proofs

Statement:

```text
I know (balance, r, k) such that
  C = Commit(balance, r)
  0 <= balance < 2^64
  -BETA <= r_i <= BETA
```

### 7.2 State Transition Representation

The MVP does not use an additional ZKP for the state transition itself.

Given the transfer amount commitment `A = Commit(delta, r_amount)`, the two affected participants are updated by commitment arithmetic:

```text
C_sender_new   = C_sender_old   - A
C_receiver_new = C_receiver_old + A
```

For every other participant:

```text
C_i_new = C_i_old
```

All participants inspect the previous state and next state and verify this commitment arithmetic before signing.

### 7.3 Sender Solvency Check

Overspend prevention for the sender is enforced through a `BalanceOpeningProof` on the sender’s post-state balance commitment.

Statement:

```text
I know (balance_after, r_after, k_after) such that
  C_sender_new = Commit(balance_after, r_after)
  0 <= balance_after < 2^64
```

Since `C_sender_new = C_sender_old - A` is publicly verifiable, a valid proof implies that the sender’s balance did not become negative.

## 8. Channel Lifecycle

### 8.1 Open

1. Fix the participant set and challenge period
2. Compute each initial balance commitment from `balance_i` and `r_i`
3. Build the initial `participant_root` from all participant leaves
4. Construct `OffchainState(version = 0)`
5. Have every participant sign the state hash and store `SignedState(version = 0)`

In the MVP, open does not publish the full state on-chain. The settlement contract stores only channel metadata.

### 8.2 Payment Update

For a payment where participant `p` sends `delta` to participant `q`:

1. The proposer loads the latest fully signed state `S_t`
2. The sender selects transfer witness `(delta, r_amount)` and constructs `A = Commit(delta, r_amount)`
3. The sender computes `C_p_new = C_p_old - A`
4. The receiver computes `C_q_new = C_q_old + A`
5. The sender generates a range proof for the updated sender balance commitment `C_p_new`
6. The sender privately sends `ReceiverWitnessShare(delta, r_amount)` to the receiver
7. The proposer assembles the updated participant leaves and distributes `UpdateProposalBundle`
8. Every participant validates the bundle
9. The receiver updates their witness from the received witness share
10. If validation succeeds, every participant signs the new state hash `S_{t+1}`
11. Once all signatures are collected, `SignedState(version = t + 1)` is finalized

### 8.3 Validation Rules During an Update

Before signing, each participant must check at least:

- `next_version = current_version + 1`
- `prev_state_hash` matches the latest fully signed local state
- sender and receiver indices are valid and distinct
- `C_sender_new = C_sender_old - A`
- `C_receiver_new = C_receiver_old + A`
- the sender’s post-update balance proof is valid
- every non-sender, non-receiver participant leaf is unchanged
- the updated `participant_root` matches the updated leaf set
- the signed state hash is deterministic

The receiver additionally checks:

- `ReceiverWitnessShare` was received
- recomputing `A` from `(delta, r_amount)` matches the bundle’s `amount_commitment`
- the receiver can derive their new witness

### 8.4 Concurrent Updates

The MVP allows only one proposal per state version. Conflicting proposals are rejected as version conflicts.

## 9. Close / Challenge / Finalize / Withdraw

### 9.1 Start Close

Anyone may call `startClose`.

Inputs:

- `channel_id`
- external system `total_balance_cap`
- `total_balance_proof` justifying that cap

The contract verifies the proof and fixes the total withdraw ceiling as `channel_cap = total_balance_cap`.

### 9.2 Challenge

After close starts, anyone may submit a newer fully signed state during the challenge period.

Inputs:

- `SignedState`
- signatures from all participants

Acceptance conditions:

- `channel_id` matches
- all participant signatures are valid
- `version` is greater than the currently stored `best_version`

The contract stores the greatest valid version as `best_state`.

### 9.3 Finalize

After the challenge period ends, `best_state` becomes the final state.

Minimum on-chain state:

- `channel_cap`
- `final_version`
- `final_participant_root`
- `withdrawn_total`
- `withdrawn_bitmap` or `claimed[index]`

### 9.4 Withdraw

Each participant submits their own leaf and opening proof against the final state in order to withdraw.

Inputs:

- `participant_index`
- `withdraw_key`
- `claimed_amount`
- `balance_commitment`
- `merkle_proof` for `final_participant_root`
- `BalanceOpeningProof`

Contract checks:

- the leaf belongs to `final_participant_root`
- `withdraw_key` matches the caller
- the opening proof is valid for `balance_commitment` and `claimed_amount`
- the participant index has not been claimed yet
- `withdrawn_total + claimed_amount <= channel_cap`

On success:

- `claimed[index] = true`
- `withdrawn_total += claimed_amount`
- transfer `claimed_amount`

### 9.5 Why `channel_cap` Is Needed

`final_state` alone does not let the chain directly know the real total channel funds, so withdrawals need a hard ceiling. `channel_cap` provides that bound.

## 10. Signature Format

State signatures use an EIP-712-style typed hash.

The signed payload should include at least:

- `channel_id`
- `version`
- `prev_state_hash`
- `participant_root`
- `transition_meta_hash`

The MVP assumes individual ECDSA/secp256k1 signatures. Aggregated signatures such as BLS can be explored later.

## 11. Security Conditions

### 11.1 Off-Chain Conditions

- participants must always keep the latest fully signed state locally
- proofs and roots must be validated before signing
- watcher or watchtower support is recommended during challenge monitoring

### 11.2 On-Chain Conditions

- a newer fully signed state must be able to beat a stale state during challenge
- partially signed states must be rejected
- double withdraws must be prevented
- `withdrawn_total` must never exceed `channel_cap`

### 11.3 Cryptographic Assumptions

The current `sis_amount_stark` implementation is still a research prototype with non-production assumptions. Mainnet use would still require:

- lattice parameter review
- cryptographic review of amount-commitment handling and witness-share flow
- audit of the on-chain verifier path
- DoS and side-channel analysis

## 12. Known Open Questions

- whether lattice STARK verification should happen directly on L1 or through another succinct wrapper
- how receiver witness shares should be encrypted and transported
- whether aggregated signatures should be used to reduce close-time signature verification cost

## 13. MVP Completion Criteria

The MVP is complete when all of the following are available:

- a channel with three or more fixed members can be opened
- state updates can be validated from amount commitment arithmetic plus the sender range proof
- only unanimously signed states can become the final candidate
- close and challenge can determine the latest final state
- each participant can prove and withdraw their own final balance
