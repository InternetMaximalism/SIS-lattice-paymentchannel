# SIS Lattice Multi-Party Payment Channel Design Spec

## 1. 目的

このリポジトリでは、`SIS-lattice-private-balance` の残高コミットメントと range proof を部品として使い、各参加者の残高を秘匿したまま更新できる多人数ペイメントチャネルを設計する。

この文書の対象は以下の 3 つである。

- オフチェーンで全員署名により更新されるチャネル状態
- オンチェーンの close / challenge / finalize / withdraw フロー
- `sis_amount_stark` を import してどの責務に使うか

## 2. 前提とスコープ

### 2.1 前提

- 参照実装は `InternetMaximalism/SIS-lattice-private-balance` の `sis_amount_stark` を使う
- `sis_amount_stark` は「1 つの隠し残高コミットメント opening の知識証明 + amount range proof + randomness bound proof」を行う
- チャネル全体の実残高を表す外部システムが存在し、close 時にその proof をオンチェーン提出できる
- 最終状態は「参加者全員が署名したもっとも新しい state」とする

### 2.2 MVP スコープ

- 固定メンバーの多人数チャネル
- 単一資産
- オフチェーン更新は毎回全員署名
- challenge では部分署名 state を採用しない
- withdraw 時に各参加者は自分の残高を公開して引き出す

### 2.3 非スコープ

- 参加者集合の動的変更
- 複数資産同時管理
- sender / receiver 以外への metadata 完全秘匿
- production-secure な lattice parameter 選定
- on-chain での native lattice STARK verifier 最適化

## 3. 参照実装の import 方針

`SIS-lattice-private-balance` の現状実装は、コミットメント `C` に対して以下を証明する。

- `C = g * a + B * r (mod q)`
- `0 <= a < 2^64`
- `-BETA <= r_i <= BETA`

ここで `a` は hidden balance、`r` は randomness vector、`k` は modular equality 用の quotient witness である。

このリポジトリでは、この実装をローカルにコピーせず import する。

Rust 側の想定は pinned git dependency である。

```toml
[dependencies]
sis_amount_stark = { git = "https://github.com/InternetMaximalism/SIS-lattice-private-balance.git", rev = "8ab35b5bbb58666fca7fd56e21d33fed3e66fcea" }
```

この import 先を以下の責務に使う。

- 各参加者の残高コミットメント生成
- 送金 amount コミットメント生成
- 各参加者の残高 opening 証明
- sender の post-state balance range proof
- withdraw 時の個別残高証明

一方で、以下は参照実装の外側で新規実装が必要である。

- state hash、署名、challenge ルール
- amount commitment を使ったオフチェーン state 遷移ロジック
- receiver への witness share 伝達
- on-chain settlement contract

## 4. 設計上の重要な判断

### 4.1 可視性モデル

MVP の可視性は以下とする。

- sender と receiver の participant index は公開
- sender と receiver のアドレスは公開
- 各参加者の balance は秘匿
- 送金額 `delta` は秘匿
- 更新後 state の participant commitment 群は全参加者が見る
- receiver は自分の更新 witness を維持するために送金 amount の witness share を受け取る

### 4.2 支払い時に必要な proof と共有情報

ユーザー要件どおり、支払い更新で全参加者が受け取る cryptographic proof は sender の残高 range proof を基本とする。

ただし state 遷移の correctness は追加 ZKP ではなく、前 state の commitment 群と amount commitment の加減算で検証する。

更新 bundle に最低限含めるものは以下である。

- amount commitment `A = Commit(delta, r_amount)`
- sender の更新後残高 commitment に対する range proof
- 更新後 state 全体
- その state に対する全参加者署名

receiver にはこれに加えて、receiver が自分の新残高 witness を計算できるだけの私的情報共有が必要である。

- `delta`
- `r_amount`

receiver がこれを受け取れない場合、receiver は更新 state に署名しない。

### 4.3 送金額の秘匿

`delta` は公開しない。

全参加者は amount commitment `A` を見るが、その opening `(delta, r_amount)` は原則として sender と receiver だけが知る。

### 4.4 メンバー固定

MVP の参加者集合は固定とする。join / leave を許すと、署名集合・state root・withdraw 権限の定義が一気に複雑になるためである。

## 5. アーキテクチャ概要

コンポーネントは 4 層に分ける。

- `proof layer`: lattice balance proof
- `state layer`: オフチェーン状態、ハッシュ、署名対象、検証ルール
- `coordination layer`: proposal 配布、署名回収、最新 fully-signed state 保持
- `settlement layer`: close / challenge / finalize / withdraw を扱うコントラクト

## 6. データモデル

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

`proof_system_id` は lattice parameter と proof format の組を識別する。

### 6.2 Participant

```text
Participant {
  index: u16,
  signing_key: address,
  withdraw_key: address,
}
```

MVP では `signing_key` と `withdraw_key` を分離可能にする。state 署名鍵と、withdraw 受取先を分けたいケースに備えるためである。

### 6.3 ParticipantLeaf

各 state の participant entry は Merkle leaf として表す。

```text
ParticipantLeaf {
  channel_id: bytes32,
  state_version: uint64,
  participant_index: u16,
  withdraw_key: address,
  balance_commitment: [u64; M],
}
```

leaf hash は `channel_id` と `state_version` を含め、別 channel / 別 version への使い回しを防ぐ。

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

`participants` 全体はオフチェーン保存し、オンチェーンには `participant_root` だけを置く。

### 6.5 SignedState

```text
SignedState {
  state: OffchainState,
  signatures: Vec<ParticipantSignature>,
}
```

MVP では「全参加者署名が揃った state だけが有効」である。

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

receiver にだけ渡す私的ペイロードを定義する。

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

必要なら sender から receiver への encrypted payload として運ぶ。

## 7. Proof インターフェース

### 7.1 既存 primitive: BalanceOpeningProof

用途:

- 個別残高の opening 証明
- sender の更新後残高 range proof
- withdraw 時の残高証明

statement:

```text
I know (balance, r, k) such that
  C = Commit(balance, r)
  0 <= balance < 2^64
  -BETA <= r_i <= BETA
```

### 7.2 State Transition の表現

MVP では state transition 自体に ZKP は使わない。

送金 amount の commitment を `A = Commit(delta, r_amount)` とすると、影響を受ける 2 人の更新は commitment の加減算だけで表される。

```text
C_sender_new   = C_sender_old   - A
C_receiver_new = C_receiver_old + A
```

その他の participant については:

```text
C_i_new = C_i_old
```

全参加者は前 state と更新後 state を見て、この commitment arithmetic を検証してから署名する。

### 7.3 sender の支払い可能性チェック

送金者の overspend 防止は sender の更新後残高 commitment に対する `BalanceOpeningProof` で行う。

statement:

```text
I know (balance_after, r_after, k_after) such that
  C_sender_new = Commit(balance_after, r_after)
  0 <= balance_after < 2^64
```

`C_sender_new = C_sender_old - A` が全員に検証可能なので、この proof が通れば sender の残高は負になっていないと判断できる。

## 8. チャネルライフサイクル

### 8.1 Open

1. 参加者集合と challenge period を固定して channel を作成する
2. 初期各残高 `balance_i` と randomness `r_i` を使って `balance_commitment_i` を作る
3. 全 participant leaf から初期 `participant_root` を作る
4. `version = 0` の `OffchainState` を構成する
5. 全員が state hash に署名し、`SignedState(version=0)` を各自保存する

MVP では open 時にオンチェーンへ full state を載せず、settlement contract には channel metadata だけを登録する想定とする。

### 8.2 支払い更新

participant `p` が `q` に `delta` を送るときの手順を定義する。

1. proposer が最新 fully-signed state `S_t` を取得する
2. sender は送金 amount の witness `(delta, r_amount)` を選び、`A = Commit(delta, r_amount)` を作る
3. sender は `C_p_new = C_p_old - A` を計算する
4. receiver は `C_q_new = C_q_old + A` を計算する
5. sender は更新後 sender 残高 commitment `C_p_new` に対する range proof を生成する
6. sender は receiver に `ReceiverWitnessShare(delta, r_amount)` を私的に渡す
7. proposer は更新後 participant leaf 群を組み立て、`UpdateProposalBundle` を全員へ配布する
8. 各参加者は bundle を検証する
9. receiver は受け取った witness share で自分の新残高 witness を更新する
10. 問題がなければ全員が `S_{t+1}` の state hash に署名する
11. 全署名が揃ったら `SignedState(version=t+1)` が確定する

### 8.3 更新時の検証ルール

各参加者は署名前に少なくとも以下を検証しなければならない。

- `next_version = current_version + 1`
- `prev_state_hash` が自分の持つ最新 signed state と一致する
- sender / receiver index が有効で重複していない
- `C_sender_new = C_sender_old - A` が成り立つ
- `C_receiver_new = C_receiver_old + A` が成り立つ
- sender の更新後残高 proof が有効である
- sender / receiver 以外の participant leaf が前 state と完全一致する
- 更新後 `participant_root` が leaf 群と一致する
- 署名対象 hash が deterministic に計算できる

receiver は上記に加えて以下も確認する。

- `ReceiverWitnessShare` を受け取っている
- 受け取った `(delta, r_amount)` から `A` を再計算すると bundle の `amount_commitment` と一致する
- 自分の新残高 witness を更新できる

### 8.4 同時更新の扱い

MVP では 1 version あたり 1 proposal だけを許す。競合 proposal は version 衝突として破棄する。

## 9. Close / Challenge / Finalize / Withdraw

### 9.1 Close 開始

誰でも `startClose` を呼べる。

入力:

- `channel_id`
- 外部システムの `total_balance_cap`
- その cap を正当化する `total_balance_proof`

コントラクトは proof を検証し、以後の総出金上限を `channel_cap = total_balance_cap` に固定する。

### 9.2 Challenge

close 開始後、challenge period 中は誰でもより新しい fully-signed state を提出できる。

入力:

- `SignedState`
- 参加者全員分の署名

採用条件:

- `channel_id` が一致する
- 署名が全員分有効
- `version` が現在保持中の `best_version` より大きい

コントラクトはもっとも大きい `version` を `best_state` として保持する。

### 9.3 Finalize

challenge period 終了後、`best_state` を final state として固定する。

オンチェーンに保持する最低限の値:

- `channel_cap`
- `final_version`
- `final_participant_root`
- `withdrawn_total`
- `withdrawn_bitmap` または `claimed[index]`

### 9.4 Withdraw

各参加者は final state に対する自分の leaf と opening proof を提出して出金する。

入力:

- `participant_index`
- `withdraw_key`
- `claimed_amount`
- `balance_commitment`
- `merkle_proof` for `final_participant_root`
- `BalanceOpeningProof`

コントラクトの検証:

- leaf が `final_participant_root` に属する
- `withdraw_key` が caller と一致する
- opening proof が `balance_commitment` と `claimed_amount` に対して有効
- その index が未請求
- `withdrawn_total + claimed_amount <= channel_cap`

成功したら:

- `claimed[index] = true`
- `withdrawn_total += claimed_amount`
- `claimed_amount` を送金する

### 9.5 `channel_cap` を使う理由

`final_state` だけではチャネル全体の実残高をオンチェーンが直接知れないため、withdraw 総額に hard ceiling が必要である。`channel_cap` がそれを提供する。

## 10. 署名仕様

state 署名は EIP-712 風の typed hash を使う。

署名対象に最低限含める値:

- `channel_id`
- `version`
- `prev_state_hash`
- `participant_root`
- `transition_meta_hash`

MVP では ECDSA/secp256k1 の個別署名を想定する。BLS 等の集約署名は後段で検討する。

## 11. セキュリティ条件

### 11.1 オフチェーン条件

- 参加者は必ず最新 fully-signed state をローカルに保存する
- 署名前に proof と root を必ず検証する
- watcher または watchtower を使い challenge 監視することが望ましい

### 11.2 オンチェーン条件

- stale state より新しい fully-signed state を challenge できる
- partial signature state は受理しない
- withdraw 重複を防ぐ
- `withdrawn_total` が `channel_cap` を越えない

### 11.3 暗号前提

現時点の `sis_amount_stark` は toy parameter の研究プロトタイプであり、本仕様も mainnet-ready ではない。production に必要なのは以下である。

- lattice parameter review
- amount commitment 運用と witness share 受け渡し設計の cryptographic review
- on-chain verifier の監査
- DoS と side-channel の検証

## 12. 既知の未解決事項

- lattice STARK を L1 で直接検証するのか、別の succinct wrapper を使うのか
- receiver 向け witness share をどう暗号化・配送するか
- close 時に署名検証コストを下げるため集約署名へ移行するか

## 13. MVP 完了条件

以下が揃えば MVP とする。

- 3 人以上の固定メンバーで channel を開ける
- amount commitment の加減算と sender の range proof を検証して state 更新できる
- 全員署名 state だけが最終候補になる
- close 後に challenge で最新 state を確定できる
- 各参加者が final state から自分の残高を証明して withdraw できる
