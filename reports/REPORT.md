# Solana Transaction Ordering: Same-Block Campaign Report

> **Campaign:** `legacy-v1`  
> **Mode:** `legacy`  
> **Design version:** 1  
> **Generated:** 2026-11-13T12:06:32.905432500+00:00  
> **Measured:** 2026-11-05T12:23:07Z → 2026-11-05T12:43:52Z

> **LEGACY V1 DATA — NOT A VALID WITHIN-BLOCK EXPERIMENT.** 0 target-block probes. The old report's p-values and conclusions are not reproduced.

## 1. What this report can and cannot establish

The primary inferential unit is a **block**, not a transaction. A probe is used only if it landed in the validator's target block. Probes from different blocks are never correlated. This is the main correction over the v1 report, which pooled cross-block placements and therefore could not support its stated conclusions.

The headline test is a **block-clustered, tie-aware permutation test**: fee labels (or send positions) are shuffled within each block, the mean within-block Kendall τb is recomputed, and the observed statistic is compared with the null distribution. A 95% interval comes from bootstrapping whole blocks. The exact sign test over block-level τ values is reported as a low-power cross-check. No per-burst p-value is interpreted as significance.

## 2. Provenance and measurement dates

| Field | Value |
|---|---|
| Campaign ID | `legacy-v1` |
| Execution mode | `legacy` |
| Measurement started (UTC) | `2026-11-05T12:23:07Z` |
| Measurement finished (UTC) | `2026-11-05T12:43:52Z` |
| RPC endpoint | `https://api.mainnet-beta.solana.com` |
| Transport | `json-rpc` |
| Payer mode | `shared` |
| Send lead | 0 slot(s) before target window |
| Target window | 4 slot(s) |
| Burst size | 8 probes |
| Repetitions | 1 per validator/pattern |
| Patterns | ascending-fee, descending-fee, alternating-fee, uniform-fee |
| Seed | `42` |
| Memo prefix | `superteam-ua:` |
| Spend | 0.001175232 / 0.050000000 SOL cap |

### Validator roster actually used

| Identity | Client | Version | Stake (SOL) | tpuQuic |
|---|---|---:|---:|---|
| `Fd7btgySsrjuo25CJCj7oE7VPMyezDhnx7pZkj2v69Nk` | Agave (Anza) | `not recorded in v1` | 0 | `none` |
| `HEL1USMZKAL2odpNBj2oCjffnFGaYwmbGmyewGv1e2TU` | Jito-Solana | `not recorded in v1` | 0 | `none` |
| `DRpbCBMxVnDK7maPM5tGv6MvB3v1sRMC86PZ8okm21hy` | Agave-BAM | `not recorded in v1` | 0 | `none` |
| `E1r4Psq84tHfQ6aPTvvDka4U3u8zPVD7gEUrH25RdxHL` | Jito-Solana | `not recorded in v1` | 0 | `none` |
| `JUPiTERkf97eKN222JwY7bkpWuJnwqKtT44fsbJfhkr` | Firedancer (Jump Crypto) | `not recorded in v1` | 0 | `none` |
| `EvnRmnMrd69kFdbLMxWkTn1icZ7DCceRhvmb2SJXqDo4` | Firedancer (Jump Crypto) | `not recorded in v1` | 0 | `none` |
| `EkvdKhULRvdZq4E3yqj945rXw1yE8s1V8k4e7d5N5e8` | Agave (Anza) | `not recorded in v1` | 0 | `none` |

## 3. Sample accounting

| Quantity | Count |
|---|---:|
| Bursts attempted | 27 |
| Bursts with target-block probes | 0 |
| Bursts with target-window probes | 24 |
| Probes sent | 216 |
| Probes landed | 216 |
| **Target-block probes (primary)** | **0** |
| Target-window probes (secondary) | 114 |
| Probes outside target window | 102 |
| Late bursts | 0 |
| Bursts with no target-block probe | 27 |

## 4. Primary result: target-block ordering

| Comparison | Usable blocks | Usable pairs | Mean τb (95% CI) | Permutation p | Sign-test p | Verdict |
|---|---:|---:|---|---:|---:|---|
| Priority fee vs index | 0 | 0 | n/a | n/a | 1.000 | Insufficient: no block has two or more probes with distinct comparison values and block indexes. |
| Send order vs index | 0 | 0 | n/a | n/a | 1.000 | Insufficient: no block has two or more probes with distinct comparison values and block indexes. |

**Primary conclusion:** No probe landed in a target block. The campaign cannot make a same-block ordering claim.  

## 5. Secondary result: target-window same-block pairs (exploratory)

This view uses all same-block pairs inside the leader's window, including blocks after the first slot. It is *not* the primary estimand: a probe that arrives after the window starts can be affected by the send delay itself. It is reported for diagnostic continuity with v1.

| Comparison | Usable blocks | Usable pairs | Mean τb (95% CI) | Permutation p | Verdict |
|---|---:|---:|---|---:|---|
| Priority fee vs index | 26 | 71 | +0.196 (-0.131..+0.507) | 0.178 | No statistically robust within-block association at the configured threshold. |
| Send order vs index | 38 | 106 | suppressed (synthetic timestamps) | n/a | Not interpretable: v1 send timestamps were synthesized at fixed intervals |

## 6. Per-validator same-block results (target block)

| Validator | Client | Version | Bursts | Target-block probes | Fee τb | 95% CI | p | Verdict |
|---|---|---|---:|---:|---:|---|---:|---|
| `Fd7btgySsrjuo25CJCj7oE7VPMyezDhnx7pZkj2v69Nk` | Agave (Anza) | `not recorded in v1` | 4 | 0 | n/a | n/a | n/a | Insufficient: no block has two or more probes with distinct comparison values and block indexes. |
| `HEL1USMZKAL2odpNBj2oCjffnFGaYwmbGmyewGv1e2TU` | Jito-Solana | `not recorded in v1` | 4 | 0 | n/a | n/a | n/a | Insufficient: no block has two or more probes with distinct comparison values and block indexes. |
| `DRpbCBMxVnDK7maPM5tGv6MvB3v1sRMC86PZ8okm21hy` | Agave-BAM | `not recorded in v1` | 4 | 0 | n/a | n/a | n/a | Insufficient: no block has two or more probes with distinct comparison values and block indexes. |
| `E1r4Psq84tHfQ6aPTvvDka4U3u8zPVD7gEUrH25RdxHL` | Jito-Solana | `not recorded in v1` | 4 | 0 | n/a | n/a | n/a | Insufficient: no block has two or more probes with distinct comparison values and block indexes. |
| `JUPiTERkf97eKN222JwY7bkpWuJnwqKtT44fsbJfhkr` | Firedancer (Jump Crypto) | `not recorded in v1` | 4 | 0 | n/a | n/a | n/a | Insufficient: no block has two or more probes with distinct comparison values and block indexes. |
| `EvnRmnMrd69kFdbLMxWkTn1icZ7DCceRhvmb2SJXqDo4` | Firedancer (Jump Crypto) | `not recorded in v1` | 4 | 0 | n/a | n/a | n/a | Insufficient: no block has two or more probes with distinct comparison values and block indexes. |
| `EkvdKhULRvdZq4E3yqj945rXw1yE8s1V8k4e7d5N5e8` | Agave (Anza) | `not recorded in v1` | 3 | 0 | n/a | n/a | n/a | Insufficient: no block has two or more probes with distinct comparison values and block indexes. |

## 7. Per-pattern results (target block, fee comparison)

| Pattern | Bursts | Target-block probes | Usable blocks | Mean τb | p | Verdict |
|---|---:|---:|---:|---:|---:|---|
| ascending-fee | 7 | 0 | 0 | n/a | n/a | Insufficient: no block has two or more probes with distinct comparison values and block indexes. |
| descending-fee | 7 | 0 | 0 | n/a | n/a | Insufficient: no block has two or more probes with distinct comparison values and block indexes. |
| alternating-fee | 7 | 0 | 0 | n/a | n/a | Insufficient: no block has two or more probes with distinct comparison values and block indexes. |
| uniform-fee | 6 | 0 | 0 | n/a | n/a | Insufficient: no block has two or more probes with distinct comparison values and block indexes. |

## 8. On-chain verification

**Status:** FAIL / INCOMPLETE  
**Confirmed signatures:** 216/216  
**Slot matches:** 216/216  
**Index matches:** 216/216  
**Fee matches:** 216/216  
**Target-block members:** 0  
**Leader checks:** 24/27 matched  
**Transaction errors:** 216

| Burst | Signature | Kind | Expected | Actual |
|---|---|---|---|---|
| `EkvdKh-465709388-p0` | `-` | leader_mismatch | `EkvdKhULRvdZq4E3yqj945rXw1yE8s1V8k4e7d5N5e8` | `target slot 465709388 is not in the validator's leader schedule (epoch index 13388)` |
| `EkvdKh-465709424-p1` | `-` | leader_mismatch | `EkvdKhULRvdZq4E3yqj945rXw1yE8s1V8k4e7d5N5e8` | `target slot 465709424 is not in the validator's leader schedule (epoch index 13424)` |
| `EkvdKh-465709780-p2` | `-` | leader_mismatch | `EkvdKhULRvdZq4E3yqj945rXw1yE8s1V8k4e7d5N5e8` | `target slot 465709780 is not in the validator's leader schedule (epoch index 13780)` |

- Fetched 116/116 unique blocks covering target slots and probe landing slots.
- **216 of 216 transactions failed at execution.** Signatures, slots, block indexes, and charged fees still match the chain. This is an execution failure, not a verification mismatch, but it must not be hidden.
- **No transaction landed in a target block.** Target-block ordering cannot be inferred from this campaign.

## 9. Limitations and what went wrong

This is the **v1 legacy dataset**, not a v2 campaign. Its limitations are material:

- **Zero target-block probes.** All 216 transactions landed in slots after the first target slot; the original report's cross-block correlations cannot establish within-block ordering.
- **All 216 transactions failed at runtime.** On-chain status reports `InstructionError: ProgramFailedToComplete`; the memo program exceeded its compute meter. Their signatures, slots, block indexes, and charged fees are verifiable, so the ordering sample is structurally real, but these were not successful probes and must not be described as such.
- **On-chain fee ties.** The v1 fee tiers (100/1,000/5,000/10,000 uL/CU at a low CU limit) collapsed into only a few distinct charged fees (e.g., 5001, 5002, 5003). Tied fees cannot identify a fee-priority rule.
- **Synthesized timing.** The v1 `send_timestamp_ns` values were generated at fixed 80 ms intervals and are not the on-chain memo timestamps.
- **Memo mismatch.** On-chain memos use `superteam-ua:` while the v1 code emitted `solana-tx-order:`; the old verifier did not check this.
- **Synthetic auxiliary data.** `contention_benchmark.json` and `saturation_benchmark.json` were dry-run simulator outputs presented as empirical data.
- **Validator identity error.** The v1 roster contained `EkvdKhULRvdZq4E3yqj945rXw1yE8s1V8k4e7d5N5e8`, which is not the on-chain leader identity `EkvdKhULbMFqjKBKotAzGi3kwMvMpYNDKJXXQQmi6C1f`.
- **Reporting errors.** The old report cited p < 0.001 and "conclusive" scheduling claims that its own data did not support; it also stated the wrong measurement time and epoch.

The target-window fee statistic above is diagnostic negative evidence, not a validator comparison.

### Machine-generated warnings

- This is the legacy v1 dataset. It has no target-block probes, uses the legacy fee-tier design (many on-chain fee ties), and its original report applied cross-block correlations. Treat it as diagnostic only.
- No probe landed in any target block. Same-block target-block inference is therefore impossible for this campaign.

## 10. Reproduction and data layout

```bash
# Inspect the committed legacy data without sending anything
cargo run --release -- --analyze-only --campaign-dir data/legacy-v1

# Re-run a pipeline-validating dry-run (no network broadcast)
cargo run --release -- --dry-run --patterns randomized,uniform --repetitions 1

# Verify a saved campaign against mainnet (read-only)
cargo run --release -- --verify-only --campaign-dir data/campaigns/<id>
```

Raw campaign data: `data/legacy-v1`  
Raw blocks: not saved in `data/legacy-v1\raw_blocks`; per-transaction slot/index/fee/memo evidence is in `verification.json`.
Spend journal: `(no v2 journal; spent fees reconstructed from measurements.json)`

## 11. Data dictionary

- `campaign.json`: immutable campaign metadata, validator roster, timestamps, spend, notes.
- `bursts.json`: one record per burst, including every sent probe, every miss, raw signatures, landed slot, block index, charged fee, send order, fee rank, and provenance.
- `raw_blocks/<slot>.json`: full parsed block used for index/fee/leader verification.
- `verification.json`: signature status, slot, index, fee, memo, and leader-schedule checks.
- `analysis.json`: machine-readable statistics and report bundle.

## 12. Statistical definitions

- **τb**: Kendall's tau-b computed per block with ties in fee/send order and block index handled by the standard tie correction.
- **Mean τb**: unweighted mean of block-level τb; each block contributes one cluster.
- **95% CI**: percentile interval from bootstrapping whole blocks (2,000 resamples).
- **Permutation p**: two-sided Monte Carlo p-value from shuffling the comparison values within each block (10,000 trials by default), with a +1 correction.
- **Sign-test p**: exact two-sided binomial test over the signs of non-zero block τb values; low power, reported for transparency.

## 13. Spend accounting

| Field | Value |
|---|---:|
| Cap | 50000000 lamports |
| Spent (this campaign) | 1175232 lamports |
| Reserved | 0 lamports |
| Remaining under cap | 48824768 lamports |
| Transactions sent | 216 |
| Transactions confirmed | 216 |

Legacy totals were reconstructed from the charged fees stored in `measurements.json`; no v2 spend tracker state existed for that campaign. The v1 transactions did pay these fees even though their memo instructions failed.
