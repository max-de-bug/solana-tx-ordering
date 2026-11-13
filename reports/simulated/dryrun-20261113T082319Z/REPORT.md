# Solana Transaction Ordering: Same-Block Campaign Report

> **Campaign:** `dryrun-20261113T082319Z`  
> **Mode:** `dry-run`  
> **Design version:** 2  
> **Generated:** 2026-11-13T12:00:58.038839+00:00  
> **Measured:** 2026-11-13T08:23:20.014157400+00:00 → (not finished)

> **DRY-RUN — SYNTHETIC PLACEMENTS.** Pipeline validation only. 320 target-block probes; none are mainnet evidence.

## 1. What this report can and cannot establish

The primary inferential unit is a **block**, not a transaction. A probe is used only if it landed in the validator's target block. Probes from different blocks are never correlated. This is the main correction over the v1 report, which pooled cross-block placements and therefore could not support its stated conclusions.

The headline test is a **block-clustered, tie-aware permutation test**: fee labels (or send positions) are shuffled within each block, the mean within-block Kendall τb is recomputed, and the observed statistic is compared with the null distribution. A 95% interval comes from bootstrapping whole blocks. The exact sign test over block-level τ values is reported as a low-power cross-check. No per-burst p-value is interpreted as significance.

## 2. Provenance and measurement dates

| Field | Value |
|---|---|
| Campaign ID | `dryrun-20261113T082319Z` |
| Execution mode | `dry-run` |
| Measurement started (UTC) | `2026-11-13T08:23:20.014157400+00:00` |
| Measurement finished (UTC) | `not recorded` |
| RPC endpoint | `https://api.mainnet-beta.solana.com` |
| Transport | `tpu-quic` |
| Payer mode | `shared` |
| Send lead | 2 slot(s) before target window |
| Target window | 4 slot(s) |
| Burst size | 8 probes |
| Repetitions | 1 per validator/pattern |
| Patterns | ascending-fee, descending-fee, alternating-fee, uniform-fee, randomized-fee |
| Seed | `42` |
| Memo prefix | `solana-tx-order:v2` |
| Spend | 0.005565728 / 0.050000000 SOL cap |

### Validator roster actually used

| Identity | Client | Version | Stake (SOL) | tpuQuic |
|---|---|---:|---:|---|
| `17EtdeMwcxWvuEN3yCfYAowtnyHhRSRxhuhAeTEEhZa` | Agave (Anza) | `synthetic-Agave (Anza)` | 0 | `none` |
| `29jGKkZXzh1NJSUzU3kHV3vv3VYi9W7NTeErA7DQwN3b` | Agave (Anza) | `synthetic-Agave (Anza)` | 0 | `none` |
| `36bQKy5gGsAYFhz6aEK9ZJjbTiVXaPirHMa2yRqoQ2ZV` | Jito-Solana | `synthetic-Jito-Solana` | 0 | `none` |
| `4FDRm5zrKwDPdFEizJrmWBrZcR4x2UPnnHuBxtbz6MVA` | Jito-Solana | `synthetic-Jito-Solana` | 0 | `none` |
| `5C5ZmJWzc7NZaWjq6VRdaSfF2e1mTN1Gc1ENnD7oYCaS` | Firedancer (Jump Crypto) | `synthetic-Firedancer (Jump Crypto)` | 0 | `none` |
| `6LhbCRSAfBRQx3zTWZyFXKnDBLbBuSgD6wZVZh8LLTNw` | Firedancer (Jump Crypto) | `synthetic-Firedancer (Jump Crypto)` | 0 | `none` |
| `7HZjCdxJwMaauKVZckY7baatbZY1LLHguuuTWSJH2r8u` | Agave-BAM | `synthetic-Agave-BAM` | 0 | `none` |
| `8SBkdksUzRdSGrkC2q5jYThrkG7RmKEFwUUJ3mAQg1AN` | Agave-BAM | `synthetic-Agave-BAM` | 0 | `none` |

## 3. Sample accounting

| Quantity | Count |
|---|---:|
| Bursts attempted | 40 |
| Bursts with target-block probes | 40 |
| Bursts with target-window probes | 40 |
| Probes sent | 320 |
| Probes landed | 320 |
| **Target-block probes (primary)** | **320** |
| Target-window probes (secondary) | 320 |
| Probes outside target window | 0 |
| Late bursts | 0 |
| Bursts with no target-block probe | 0 |

## 4. Primary result: target-block ordering

| Comparison | Usable blocks | Usable pairs | Mean τb (95% CI) | Permutation p | Sign-test p | Verdict |
|---|---:|---:|---|---:|---:|---|
| Priority fee vs index | 32 | 896 | +0.040 (-0.058..+0.136) | 0.440 | 0.572 | No statistically robust within-block association at the configured threshold. |
| Send order vs index | 40 | 1120 | -0.046 (-0.143..+0.055) | 1.00e-4 | 1.000 | Nominally significant permutation result, but the block-bootstrap CI includes zero; treat as inconclusive. |

**Primary conclusion:** No statistically robust within-block association at the configured threshold.  

## 5. Secondary result: target-window same-block pairs (exploratory)

This view uses all same-block pairs inside the leader's window, including blocks after the first slot. It is *not* the primary estimand: a probe that arrives after the window starts can be affected by the send delay itself. It is reported for diagnostic continuity with v1.

| Comparison | Usable blocks | Usable pairs | Mean τb (95% CI) | Permutation p | Verdict |
|---|---:|---:|---|---:|---|
| Priority fee vs index | 32 | 896 | +0.040 (-0.058..+0.136) | 0.440 | No statistically robust within-block association at the configured threshold. |
| Send order vs index | 40 | 1120 | -0.046 (-0.143..+0.055) | 1.00e-4 | Nominally significant permutation result, but the block-bootstrap CI includes zero; treat as inconclusive. |

## 6. Per-validator same-block results (target block)

| Validator | Client | Version | Bursts | Target-block probes | Fee τb | 95% CI | p | Verdict |
|---|---|---|---:|---:|---:|---|---:|---|
| `17EtdeMwcxWvuEN3yCfYAowtnyHhRSRxhuhAeTEEhZa` | Agave (Anza) | `synthetic-Agave (Anza)` | 5 | 40 | -0.071 | -0.268..+0.125 | 0.664 | No statistically robust within-block association at the configured threshold. |
| `29jGKkZXzh1NJSUzU3kHV3vv3VYi9W7NTeErA7DQwN3b` | Agave (Anza) | `synthetic-Agave (Anza)` | 5 | 40 | +0.143 | -0.036..+0.321 | 0.355 | No statistically robust within-block association at the configured threshold. |
| `36bQKy5gGsAYFhz6aEK9ZJjbTiVXaPirHMa2yRqoQ2ZV` | Jito-Solana | `synthetic-Jito-Solana` | 5 | 40 | +0.143 | -0.054..+0.482 | 0.361 | No statistically robust within-block association at the configured threshold. |
| `4FDRm5zrKwDPdFEizJrmWBrZcR4x2UPnnHuBxtbz6MVA` | Jito-Solana | `synthetic-Jito-Solana` | 5 | 40 | -0.018 | -0.179..+0.143 | 0.950 | No statistically robust within-block association at the configured threshold. |
| `5C5ZmJWzc7NZaWjq6VRdaSfF2e1mTN1Gc1ENnD7oYCaS` | Firedancer (Jump Crypto) | `synthetic-Firedancer (Jump Crypto)` | 5 | 40 | +0.018 | -0.250..+0.339 | 0.953 | No statistically robust within-block association at the configured threshold. |
| `6LhbCRSAfBRQx3zTWZyFXKnDBLbBuSgD6wZVZh8LLTNw` | Firedancer (Jump Crypto) | `synthetic-Firedancer (Jump Crypto)` | 5 | 40 | +0.071 | -0.143..+0.339 | 0.664 | No statistically robust within-block association at the configured threshold. |
| `7HZjCdxJwMaauKVZckY7baatbZY1LLHguuuTWSJH2r8u` | Agave-BAM | `synthetic-Agave-BAM` | 5 | 40 | -0.071 | -0.482..+0.321 | 0.668 | No statistically robust within-block association at the configured threshold. |
| `8SBkdksUzRdSGrkC2q5jYThrkG7RmKEFwUUJ3mAQg1AN` | Agave-BAM | `synthetic-Agave-BAM` | 5 | 40 | +0.107 | -0.054..+0.250 | 0.499 | No statistically robust within-block association at the configured threshold. |

## 7. Per-pattern results (target block, fee comparison)

| Pattern | Bursts | Target-block probes | Usable blocks | Mean τb | p | Verdict |
|---|---:|---:|---:|---:|---:|---|
| ascending-fee | 8 | 64 | 8 | +0.036 | 0.761 | No statistically robust within-block association at the configured threshold. |
| descending-fee | 8 | 64 | 8 | +0.009 | 0.969 | No statistically robust within-block association at the configured threshold. |
| alternating-fee | 8 | 64 | 8 | +0.116 | 0.272 | No statistically robust within-block association at the configured threshold. |
| uniform-fee | 8 | 64 | 0 | n/a | n/a | Insufficient: no block has two or more probes with distinct comparison values and block indexes. |
| randomized-fee | 8 | 64 | 8 | +0.000 | 1.000 | No statistically robust within-block association at the configured threshold. |

## 8. On-chain verification

No verification file was found. Run `cargo run -- --verify-only --campaign-dir <dir>` before citing the campaign.

## 9. Limitations and what went wrong

**Dry-run.** Block placements are synthetic. This run validates the pipeline, report generation, spend accounting, and statistics; it says nothing about mainnet validators. No mainnet transaction was broadcast.

### Machine-generated warnings

- This campaign is a dry-run simulation. It validates the pipeline, not validator behaviour.

## 10. Reproduction and data layout

```bash
# Inspect the committed legacy data without sending anything
cargo run --release -- --analyze-only --campaign-dir data/legacy-v1

# Re-run a pipeline-validating dry-run (no network broadcast)
cargo run --release -- --dry-run --patterns randomized,uniform --repetitions 1

# Verify a saved campaign against mainnet (read-only)
cargo run --release -- --verify-only --campaign-dir data/campaigns/<id>
```

Raw campaign data: `./data\simulated\dryrun-20261113T082319Z`  
Raw blocks: `./data\simulated\dryrun-20261113T082319Z\raw_blocks` (40 block files)
Spend journal: `./data\simulated\dryrun-20261113T082319Z\spend_journal.jsonl`

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
| Spent (this campaign) | 5565728 lamports |
| Reserved | 0 lamports |
| Remaining under cap | 44434272 lamports |
| Transactions sent | 320 |
| Transactions confirmed | 320 |

The spend tracker reserves worst-case fees before dispatch, commits the charged fee after dispatch, and appends an audit line to the campaign or global live journal. The 0.05 SOL hard cap is enforced before dispatch and is replayed from the journal on restart; it is never reset silently.
