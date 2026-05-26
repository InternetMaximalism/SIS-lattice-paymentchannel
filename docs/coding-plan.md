# SIS Lattice Multi-Party Payment Channel Coding Plan

## 1. 実装方針

このリポジトリは空なので、最初から workspace を切る。`SIS-lattice-private-balance` はコピーせず、pinned git dependency として import する。

推奨レイアウト:

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

## 2. フェーズ別計画

### Phase 0: Bootstrap

目的:

- workspace の骨格を作る
- upstream proof crate を git import で pin して再現可能にする
- 共通 lint / format / CI を入れる

作業:

- root `Cargo.toml` を workspace 化
- `sis_amount_stark = { git = \"https://github.com/InternetMaximalism/SIS-lattice-private-balance.git\", rev = \"8ab35b5bbb58666fca7fd56e21d33fed3e66fcea\" }` を設定
- `rustfmt`, `clippy`, `cargo-nextest` の設定
- Solidity 側を使うなら `foundry.toml` を追加

完了条件:

- import した `sis_amount_stark` を使う最小テストが通る
- 新規 crate を 1 つ追加して workspace 解決できる

### Phase 1: Shared Types and State Hashing

目的:

- チャネル状態と署名対象の型を固定する

作業:

- `crates/channel-types` に以下を定義
- `ChannelParams`
- `Participant`
- `ParticipantLeaf`
- `OffchainState`
- `SignedState`
- `UpdateProposalBundle`
- deterministic hash 関数
- Merkle root 生成
- state hash の EIP-712 互換エンコード

テスト:

- hash が順序依存で安定している
- 同じ state から常に同一 hash が出る
- leaf 改ざんで root が変わる

完了条件:

- state hash / leaf hash / root hash の golden test が揃う

### Phase 2: Proof Adapter

目的:

- `sis_amount_stark` を channel 用 API に包む

作業:

- `crates/proof-adapter` を作成
- `BalanceCommitment` 型
- `AmountCommitment` 型
- `BalanceOpeningProof` の serialize / verify wrapper
- witness 生成 helper
- proof format version 管理
- upstream crate の API 差分を局所化

テスト:

- balance opening proof の round-trip
- invalid commitment / invalid proof rejection

完了条件:

- channel code から upstream 型を直接触らずに済む

### Phase 3: Homomorphic Transfer Update

目的:

- amount commitment を使う state update ロジックを実装する

作業:

- `A = Commit(delta, r_amount)` を扱う API を追加
- `C_sender_new = C_sender_old - A`
- `C_receiver_new = C_receiver_old + A`
- sender の更新後残高 proof 生成
- `ReceiverWitnessShare(delta, r_amount)` の型と整合性チェック
- receiver が witness を更新できる helper を追加

テスト:

- happy path
- sender 残高不足で sender proof が失敗
- amount commitment と更新後 commitment の加減算整合性
- receiver witness share 不一致なら拒否
- `delta = 0` と最大近傍値の境界テスト

完了条件:

- `UpdateProposalBundle` と `ReceiverWitnessShare` を使ってローカル検証できる

### Phase 4: Off-Chain State Machine

目的:

- 全員署名ベースの state 更新ロジックを作る

作業:

- `crates/channel-state` に state machine 実装
- `propose_update`
- `verify_update_bundle`
- `apply_signed_state`
- version 競合検出
- sender / receiver 以外 unchanged チェック
- latest fully-signed state 保存

テスト:

- 3 人 channel で A -> B 更新
- 同時 proposal 競合
- 旧 state 参照 proposal 拒否
- sender proof が正しくても `prev_state_hash` 不一致なら拒否

完了条件:

- ローカルだけで 3 人チャネル更新の E2E が通る

### Phase 5: Coordinator / Node API

目的:

- proposal 配布と署名回収を行うノード層を作る

作業:

- `crates/channel-node` に API を実装
- `create_channel`
- `propose_payment`
- `receive_proposal`
- `sign_state`
- `finalize_state`
- storage interface
- 最新 signed state / 自分の witness 保管

MVP 判断:

- まずは single-process integration test で十分
- 実ネットワーク transport は後回しでもよい

完了条件:

- 擬似ノード 3 体で state 更新フローを再現できる

### Phase 6: Settlement Contract

目的:

- close / challenge / finalize / withdraw をオンチェーン化する

作業:

- `contracts/src/MultiPartyLatticeChannel.sol`
- `startClose(channelId, cap, totalBalanceProof)`
- `challengeState(signedState, signatures)`
- `finalize(channelId)`
- `withdraw(withdrawArgs)`
- `ITotalBalanceVerifier` interface
- `IBalanceOpeningVerifier` interface

MVP では contract 内で本物の lattice verifier を直接実装せず、外部 verifier interface を叩く設計に分離する。

テスト:

- stale state より新 state が勝つ
- challenge period 終了前 finalize 不可
- 二重 withdraw 不可
- `withdrawn_total > channel_cap` を防止

完了条件:

- Solidity テストで close から全員 withdraw まで通る

### Phase 7: End-to-End Integration

目的:

- オフチェーン state machine と settlement を接続する

作業:

- Rust 側から signed state export
- contract へ challenge 用 payload 生成
- withdraw 用 Merkle proof 生成
- final state から各 participant の claim package 生成

テスト:

- 3 人チャネルの full lifecycle
- close 後に古い state を出されても challenge で勝てる
- 1 人が withdraw しても他人の claim に影響しない

完了条件:

- `open -> multiple payments -> close -> challenge -> finalize -> withdraw` の統合テストが通る

## 3. 優先順位

優先順位は以下の順に置く。

1. state hash と署名対象の固定
2. amount commitment update ロジック
3. オフチェーン state machine
4. settlement contract
5. transport / coordinator 改良

amount commitment の表現と state hash が固まる前にノード層を広げると、型や署名仕様の手戻りが大きくなる。

## 4. 初期実装で固定してよい仮定

- 参加者は固定長配列で管理する
- 署名は ECDSA/secp256k1
- 送金額は sender と receiver だけが知る
- single asset
- partial signature state は捨てる
- challenge で受理するのは全員署名 state のみ

## 5. 先に作るべきテスト

実装前半で最重要なのは以下である。

- amount commitment update の境界テスト
- state hash の golden test
- 3 人以上での unchanged participant 検証
- stale state challenge テスト
- `channel_cap` 超過防止テスト

## 6. mainnet 前に必要な追加作業

MVP 後の必須課題:

- toy lattice parameter の置き換え
- proof system の cryptographic review
- verifier gas / performance の見積もり
- aggregate signature の検討
- receiver witness share の安全な配送
- watchtower 運用設計

## 7. 次の実装ステップ

この文書に沿ってすぐ着手するなら、次の順が最も安全である。

1. workspace bootstrap
2. `channel-types` の state / hash 実装
3. `proof-adapter` で upstream proof を包む
4. amount commitment update の最小版を実装
5. 3 人チャネルのローカル更新テストを書く
