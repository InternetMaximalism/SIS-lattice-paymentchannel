# SIS Lattice Multi-Party Payment Channel Coding Plan

## 1. Implementation Strategy

This repository started empty, so the implementation is organized as a workspace from the beginning. `SIS-lattice-private-balance` is imported as a pinned git dependency rather than copied into the repository.

Recommended layout:

```text
Cargo.toml
crates/
  proof-adapter/
  channel-types/
  channel-state/
  channel-node/
contracts/
  src/
  test/
docs/
```

## 2. Phased Plan

### Phase 0: Bootstrap

Goals:

- create the workspace skeleton
- pin the upstream proof crate through git import for reproducibility
- add shared linting, formatting, and CI basics

Work:

- turn the root `Cargo.toml` into a workspace manifest
- configure `sis_amount_stark = { git = "https://github.com/InternetMaximalism/SIS-lattice-private-balance.git", rev = "8eedf2d01cf8b0295bebcb6a330ef79eac6bd95f" }`
- add `rustfmt`, `clippy`, and `cargo-nextest` settings
- add `foundry.toml` if Solidity work starts

Done when:

- a minimal test using imported `sis_amount_stark` passes
- a new local crate can be added and resolved by the workspace

### Phase 1: Shared Types and State Hashing

Goals:

- fix the core channel state types and signed payload structure

Work:

- define the following in `crates/channel-types`
- `ChannelParams`
- `Participant`
- `ParticipantLeaf`
- `OffchainState`
- `SignedState`
- `UpdateProposalBundle`
- deterministic hashing functions
- Merkle root generation
- EIP-712-compatible state-hash encoding

Tests:

- hash stability with deterministic ordering
- identical states always produce identical hashes
- leaf mutation changes the root

Done when:

- golden tests exist for state hash, leaf hash, and root hash

### Phase 2: Proof Adapter

Goals:

- wrap `sis_amount_stark` behind a channel-focused API

Work:

- create `crates/proof-adapter`
- define `BalanceCommitment`
- define `AmountCommitment`
- add serialization / verification wrappers for `BalanceOpeningProof`
- add witness construction helpers
- manage proof format versions
- isolate upstream API differences in one place

Tests:

- balance opening proof round-trip
- invalid commitment rejection
- invalid proof rejection

Done when:

- channel code no longer touches upstream proof types directly

### Phase 3: Homomorphic Transfer Update

Goals:

- implement state updates based on amount commitments

Work:

- add API support for `A = Commit(delta, r_amount)`
- implement `C_sender_new = C_sender_old - A`
- implement `C_receiver_new = C_receiver_old + A`
- generate the sender post-update balance proof
- define `ReceiverWitnessShare(delta, r_amount)` and validate its consistency
- add helpers for receiver witness updates

Tests:

- happy path
- sender proof fails on insufficient sender balance
- amount commitment arithmetic matches updated commitments
- receiver witness-share mismatch is rejected
- boundary tests around `delta = 0` and near-maximum values

Done when:

- local verification works using `UpdateProposalBundle` and `ReceiverWitnessShare`

### Phase 4: Off-Chain State Machine

Goals:

- implement the state-update logic around unanimous signatures

Work:

- implement the state machine in `crates/channel-state`
- `propose_update`
- `verify_update_bundle`
- `apply_signed_state`
- version conflict detection
- unchanged checks for all non-sender / non-receiver participants
- storage of the latest fully signed state

Tests:

- three-party channel with an A -> B update
- concurrent proposal conflict
- stale proposal rejection
- `prev_state_hash` mismatch rejection even if the sender proof is otherwise valid

Done when:

- a three-party end-to-end local update test passes

### Phase 5: Coordinator / Node API

Goals:

- implement proposal distribution and signature collection

Work:

- implement APIs in `crates/channel-node`
- `create_channel`
- `propose_payment`
- `receive_proposal`
- `sign_state`
- `finalize_state`
- storage abstraction
- persistence for the latest signed state and each party’s witness

MVP guidance:

- a single-process integration test is enough at first
- real network transport can be deferred

Done when:

- three simulated nodes can execute the update flow

### Phase 6: Settlement Contract

Goals:

- implement close / challenge / finalize / withdraw on-chain

Work:

- `contracts/src/MultiPartyLatticeChannel.sol`
- `startClose(channelId, cap, totalBalanceProof)`
- `challengeState(signedState, signatures)`
- `finalize(channelId)`
- `withdraw(withdrawArgs)`
- `ITotalBalanceVerifier`
- `IBalanceOpeningVerifier`

The MVP keeps the actual lattice verification logic outside the contract and calls external verifier interfaces.

Tests:

- a newer state beats a stale state
- finalize is blocked before the challenge period ends
- double withdraw is prevented
- `withdrawn_total > channel_cap` is rejected

Done when:

- Solidity tests cover close through full withdrawal

### Phase 7: End-to-End Integration

Goals:

- connect the off-chain state machine to settlement

Work:

- export signed states from Rust
- generate challenge payloads for the contract
- generate Merkle proofs for withdraw
- generate claim packages for each participant from the final state

Tests:

- full lifecycle for a three-party channel
- successful challenge against a stale close submission
- one participant withdrawing does not interfere with others

Done when:

- an integration test covers `open -> multiple payments -> close -> challenge -> finalize -> withdraw`

## 3. Priorities

Priority order:

1. fix the state hash and signed payload shape
2. implement amount-commitment update logic
3. implement the off-chain state machine
4. implement settlement contracts
5. improve transport and coordination

Expanding the node layer before the amount-commitment representation and state hash are stable would create unnecessary rework in types and signature rules.

## 4. Assumptions That Are Safe to Freeze Early

- participants are tracked in fixed-length collections
- signatures use ECDSA/secp256k1
- transfer amounts are known only to sender and receiver
- the MVP is single-asset
- partially signed states are discarded
- only unanimously signed states are accepted during challenge

## 5. Tests to Build Early

The most important tests in the early implementation are:

- amount-commitment update boundary tests
- state-hash golden tests
- unchanged participant checks for channels with three or more parties
- stale-state challenge tests
- `channel_cap` overflow-prevention tests

## 6. Additional Work Required Before Mainnet

Required after the MVP:

- replace toy lattice parameters
- cryptographic review of the proof system
- verifier gas and performance analysis
- evaluate aggregated signatures
- secure receiver witness-share delivery
- watchtower operations design

## 7. Immediate Next Steps

If development continues from this plan, the safest order is:

1. workspace bootstrap
2. implement state / hash types in `channel-types`
3. wrap the upstream proof crate in `proof-adapter`
4. implement the minimum amount-commitment update path
5. write a three-party local update test
