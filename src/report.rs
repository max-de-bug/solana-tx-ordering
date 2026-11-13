use crate::analyzer::{CampaignAnalysis, SameBlockStats, ValidatorSameBlock};
use crate::campaign::Campaign;
use crate::config::Config;
use crate::spend_tracker::SpendTracker;
use crate::tx_crafting::BurstPattern;
use crate::verify::VerificationReport;
use anyhow::{Context, Result};
use serde::Serialize;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct ReportOutput {
    pub markdown: PathBuf,
    pub dashboard: PathBuf,
    pub analysis_json: PathBuf,
}

#[derive(Debug, Serialize)]
struct ReportBundle<'a> {
    campaign: &'a crate::campaign::CampaignMeta,
    analysis: &'a CampaignAnalysis,
    verification: Option<&'a VerificationReport>,
    spend: crate::spend_tracker::SpendSummary,
}

pub struct ReportGenerator;

impl ReportGenerator {
    pub fn generate_all(
        config: &Config,
        campaign: &Campaign,
        analysis: &CampaignAnalysis,
        spend: &SpendTracker,
    ) -> Result<ReportOutput> {
        let out_dir = output_dir(config, campaign);
        fs::create_dir_all(&out_dir)
            .with_context(|| format!("create report directory {}", out_dir.display()))?;

        let verification = load_verification(config, campaign);
        let spend_summary = spend.summary();
        let markdown = render_markdown(campaign, analysis, verification.as_ref(), &spend_summary);
        let dashboard = render_html(campaign, analysis, verification.as_ref(), &spend_summary);

        let markdown_path = out_dir.join("REPORT.md");
        let dashboard_path = out_dir.join("dashboard.html");
        let index_path = out_dir.join("index.html");
        let analysis_path = out_dir.join("analysis.json");

        fs::write(&markdown_path, markdown)?;
        fs::write(&dashboard_path, dashboard.clone())?;
        fs::write(&index_path, dashboard)?;
        let bundle = ReportBundle {
            campaign: &campaign.meta,
            analysis,
            verification: verification.as_ref(),
            spend: spend_summary,
        };
        fs::write(&analysis_path, serde_json::to_string_pretty(&bundle)?)?;

        Ok(ReportOutput {
            markdown: markdown_path,
            dashboard: dashboard_path,
            analysis_json: analysis_path,
        })
    }
}

fn output_dir(config: &Config, campaign: &Campaign) -> PathBuf {
    match campaign.meta.execution_mode {
        crate::analyzer::ExecutionMode::DryRun => config
            .reports_dir
            .join("simulated")
            .join(&campaign.meta.campaign_id),
        crate::analyzer::ExecutionMode::Legacy => config.reports_dir.join("legacy-v1"),
        crate::analyzer::ExecutionMode::Live => config
            .reports_dir
            .join("live")
            .join(&campaign.meta.campaign_id),
        crate::analyzer::ExecutionMode::Mixed => config
            .reports_dir
            .join("mixed")
            .join(&campaign.meta.campaign_id),
    }
}

fn load_verification(config: &Config, campaign: &Campaign) -> Option<VerificationReport> {
    let candidates = [
        campaign.dir.as_ref().map(|d| d.join("verification.json")),
        Some(config.data_dir.join("verification.json")),
        Some(config.data_dir.join("verification_legacy.json")),
    ];
    for candidate in candidates.into_iter().flatten() {
        if let Ok(raw) = fs::read_to_string(&candidate) {
            if let Ok(report) = serde_json::from_str::<VerificationReport>(&raw) {
                return Some(report);
            }
        }
    }
    None
}

fn render_markdown(
    campaign: &Campaign,
    analysis: &CampaignAnalysis,
    verification: Option<&VerificationReport>,
    spend: &crate::spend_tracker::SpendSummary,
) -> String {
    let mode = campaign.meta.execution_mode;
    let mut md = String::new();
    md.push_str("# Solana Transaction Ordering: Same-Block Campaign Report\n\n");
    md.push_str(&format!(
        "> **Campaign:** `{}`  \n> **Mode:** `{}`  \n> **Design version:** {}  \n> **Generated:** {}  \n> **Measured:** {} → {}\n\n",
        campaign.meta.campaign_id,
        mode,
        campaign.meta.design_version,
        analysis.generated_at_utc,
        campaign.meta.created_at_utc,
        campaign.meta.finished_at_utc.as_deref().unwrap_or("(not finished)"),
    ));

    md.push_str(&status_banner(campaign, analysis));
    md.push_str("\n## 1. What this report can and cannot establish\n\n");
    md.push_str(
        "The primary inferential unit is a **block**, not a transaction. A probe is used only if it \
         landed in the validator's target block. Probes from different blocks are never correlated. \
         This is the main correction over the v1 report, which pooled cross-block placements and \
         therefore could not support its stated conclusions.\n\n",
    );
    md.push_str(
        "The headline test is a **block-clustered, tie-aware permutation test**: fee labels (or send \
         positions) are shuffled within each block, the mean within-block Kendall τb is recomputed, \
         and the observed statistic is compared with the null distribution. A 95% interval comes from \
         bootstrapping whole blocks. The exact sign test over block-level τ values is reported as a \
         low-power cross-check. No per-burst p-value is interpreted as significance.\n\n",
    );

    md.push_str("## 2. Provenance and measurement dates\n\n");
    md.push_str("| Field | Value |\n|---|---|\n");
    md.push_str(&format!(
        "| Campaign ID | `{}` |\n",
        campaign.meta.campaign_id
    ));
    md.push_str(&format!("| Execution mode | `{}` |\n", mode));
    md.push_str(&format!(
        "| Measurement started (UTC) | `{}` |\n",
        campaign.meta.created_at_utc
    ));
    md.push_str(&format!(
        "| Measurement finished (UTC) | `{}` |\n",
        campaign
            .meta
            .finished_at_utc
            .as_deref()
            .unwrap_or("not recorded")
    ));
    md.push_str(&format!("| RPC endpoint | `{}` |\n", campaign.meta.rpc_url));
    md.push_str(&format!("| Transport | `{}` |\n", campaign.meta.transport));
    md.push_str(&format!(
        "| Payer mode | `{}` |\n",
        campaign.meta.payer_mode
    ));
    md.push_str(&format!(
        "| Send lead | {} slot(s) before target window |\n",
        campaign.meta.send_lead_slots
    ));
    md.push_str(&format!(
        "| Target window | {} slot(s) |\n",
        campaign.meta.target_window_slots
    ));
    md.push_str(&format!(
        "| Burst size | {} probes |\n",
        campaign.meta.burst_size
    ));
    md.push_str(&format!(
        "| Repetitions | {} per validator/pattern |\n",
        campaign.meta.repetitions
    ));
    md.push_str(&format!(
        "| Patterns | {} |\n",
        campaign
            .meta
            .patterns
            .iter()
            .map(|p| p.to_string())
            .collect::<Vec<_>>()
            .join(", ")
    ));
    md.push_str(&format!("| Seed | `{}` |\n", campaign.meta.seed));
    md.push_str(&format!(
        "| Memo prefix | `{}` |\n",
        campaign.meta.memo_prefix
    ));
    md.push_str(&format!(
        "| Spend | {:.9} / {:.9} SOL cap |\n\n",
        campaign.meta.total_spent_lamports as f64 / 1_000_000_000.0,
        campaign.meta.spend_cap_lamports as f64 / 1_000_000_000.0
    ));

    md.push_str("### Validator roster actually used\n\n");
    md.push_str(
        "| Identity | Client | Version | Stake (SOL) | tpuQuic |\n|---|---|---:|---:|---|\n",
    );
    for validator in &campaign.meta.validators {
        md.push_str(&format!(
            "| `{}` | {} | `{}` | {:.0} | `{}` |\n",
            validator.identity,
            validator.client_type,
            validator.version_string,
            validator.stake_sol,
            validator.tpu_ip.as_deref().unwrap_or("none")
        ));
    }
    md.push('\n');

    md.push_str("## 3. Sample accounting\n\n");
    md.push_str("| Quantity | Count |\n|---|---:|\n");
    md.push_str(&format!(
        "| Bursts attempted | {} |\n",
        analysis.sample.total_bursts
    ));
    md.push_str(&format!(
        "| Bursts with target-block probes | {} |\n",
        analysis.sample.bursts_with_target_block_probes
    ));
    md.push_str(&format!(
        "| Bursts with target-window probes | {} |\n",
        analysis.sample.bursts_with_target_window_probes
    ));
    md.push_str(&format!(
        "| Probes sent | {} |\n",
        analysis.sample.total_probes_sent
    ));
    md.push_str(&format!(
        "| Probes landed | {} |\n",
        analysis.sample.total_probes_landed
    ));
    md.push_str(&format!(
        "| **Target-block probes (primary)** | **{}** |\n",
        analysis.sample.target_block_probes
    ));
    md.push_str(&format!(
        "| Target-window probes (secondary) | {} |\n",
        analysis.sample.target_window_probes
    ));
    md.push_str(&format!(
        "| Probes outside target window | {} |\n",
        analysis.sample.out_of_window_probes
    ));
    md.push_str(&format!(
        "| Late bursts | {} |\n",
        analysis.sample.late_bursts
    ));
    md.push_str(&format!(
        "| Bursts with no target-block probe | {} |\n\n",
        analysis.sample.missing_target_block_bursts
    ));

    md.push_str("## 4. Primary result: target-block ordering\n\n");
    md.push_str("| Comparison | Usable blocks | Usable pairs | Mean τb (95% CI) | Permutation p | Sign-test p | Verdict |\n");
    md.push_str("|---|---:|---:|---|---:|---:|---|\n");
    md.push_str(&format!(
        "| Priority fee vs index | {} | {} | {} | {} | {} | {} |\n",
        analysis.fee_target_block.blocks_used,
        analysis.fee_target_block.pairs_usable,
        effect_cell(&analysis.fee_target_block),
        p_cell(analysis.fee_target_block.permutation_p),
        p_cell(analysis.fee_target_block.sign_test_p),
        analysis.fee_target_block.verdict
    ));
    md.push_str(&format!(
        "| Send order vs index | {} | {} | {} | {} | {} | {} |\n\n",
        analysis.send_target_block.blocks_used,
        analysis.send_target_block.pairs_usable,
        effect_cell(&analysis.send_target_block),
        p_cell(analysis.send_target_block.permutation_p),
        p_cell(analysis.send_target_block.sign_test_p),
        analysis.send_target_block.verdict
    ));
    md.push_str(&format!(
        "**Primary conclusion:** {}  \n\n",
        if analysis.sample.target_block_probes == 0 {
            "No probe landed in a target block. The campaign cannot make a same-block ordering claim."
                .to_string()
        } else if analysis.fee_target_block.blocks_used < analysis.fee_target_block.min_blocks {
            format!(
                "Only {} usable target block(s); the pre-registered minimum is {}. No ordering claim is made.",
                analysis.fee_target_block.blocks_used, analysis.fee_target_block.min_blocks
            )
        } else {
            analysis.fee_target_block.verdict.clone()
        }
    ));

    md.push_str("## 5. Secondary result: target-window same-block pairs (exploratory)\n\n");
    md.push_str(
        "This view uses all same-block pairs inside the leader's window, including blocks after the \
         first slot. It is *not* the primary estimand: a probe that arrives after the window starts can \
         be affected by the send delay itself. It is reported for diagnostic continuity with v1.\n\n",
    );
    md.push_str("| Comparison | Usable blocks | Usable pairs | Mean τb (95% CI) | Permutation p | Verdict |\n");
    md.push_str("|---|---:|---:|---|---:|---|\n");
    md.push_str(&format!(
        "| Priority fee vs index | {} | {} | {} | {} | {} |\n",
        analysis.fee_target_window.blocks_used,
        analysis.fee_target_window.pairs_usable,
        effect_cell(&analysis.fee_target_window),
        p_cell(analysis.fee_target_window.permutation_p),
        analysis.fee_target_window.verdict
    ));
    md.push_str(&format!(
        "| Send order vs index | {} | {} | {} | {} | {} |\n\n",
        analysis.send_target_window.blocks_used,
        analysis.send_target_window.pairs_usable,
        if mode == crate::analyzer::ExecutionMode::Legacy {
            "suppressed (synthetic timestamps)".to_string()
        } else {
            effect_cell(&analysis.send_target_window)
        },
        if mode == crate::analyzer::ExecutionMode::Legacy {
            "n/a".to_string()
        } else {
            p_cell(analysis.send_target_window.permutation_p)
        },
        if mode == crate::analyzer::ExecutionMode::Legacy {
            "Not interpretable: v1 send timestamps were synthesized at fixed intervals".to_string()
        } else {
            analysis.send_target_window.verdict.clone()
        }
    ));

    md.push_str("## 6. Per-validator same-block results (target block)\n\n");
    md.push_str("| Validator | Client | Version | Bursts | Target-block probes | Fee τb | 95% CI | p | Verdict |\n");
    md.push_str("|---|---|---|---:|---:|---:|---|---:|---|\n");
    for validator in &analysis.per_validator {
        md.push_str(&format!(
            "| `{}` | {} | `{}` | {} | {} | {} | {} | {} | {} |\n",
            validator.validator_identity,
            validator.client_type,
            validator.version_string,
            validator.total_bursts,
            validator.target_block_probes,
            opt_num(validator.fee.mean_block_tau_b, 3),
            ci_cell(&validator.fee),
            p_cell(validator.fee.permutation_p),
            validator.fee.verdict
        ));
    }
    md.push('\n');

    md.push_str("## 7. Per-pattern results (target block, fee comparison)\n\n");
    md.push_str(
        "| Pattern | Bursts | Target-block probes | Usable blocks | Mean τb | p | Verdict |\n",
    );
    md.push_str("|---|---:|---:|---:|---:|---:|---|\n");
    for pattern in &analysis.per_pattern {
        md.push_str(&format!(
            "| {} | {} | {} | {} | {} | {} | {} |\n",
            pattern.pattern,
            pattern.bursts,
            pattern.target_block_probes,
            pattern.fee.blocks_used,
            opt_num(pattern.fee.mean_block_tau_b, 3),
            p_cell(pattern.fee.permutation_p),
            pattern.fee.verdict
        ));
    }
    md.push('\n');

    md.push_str("## 8. On-chain verification\n\n");
    match verification {
        Some(report) => {
            md.push_str(&format!(
                "**Status:** {}  \n**Confirmed signatures:** {}/{}  \n**Slot matches:** {}/{}  \n**Index matches:** {}/{}  \n**Fee matches:** {}/{}  \n**Target-block members:** {}  \n**Leader checks:** {}/{} matched  \n**Transaction errors:** {}\n\n",
                if report.passed() { "PASS" } else { "FAIL / INCOMPLETE" },
                report.confirmed,
                report.total_signatures,
                report.slot_matched,
                report.total_signatures,
                report.index_matched,
                report.total_signatures,
                report.fee_matched,
                report.total_signatures,
                report.target_block_members,
                report.leaders_matched,
                report.leaders_checked,
                report.tx_errors
            ));
            if !report.failures.is_empty() {
                md.push_str(
                    "| Burst | Signature | Kind | Expected | Actual |\n|---|---|---|---|---|\n",
                );
                for failure in report.failures.iter().take(20) {
                    md.push_str(&format!(
                        "| `{}` | `{}` | {} | `{}` | `{}` |\n",
                        failure.burst_id,
                        failure.signature.as_deref().unwrap_or("-"),
                        failure.kind,
                        failure.expected,
                        failure.actual
                    ));
                }
                md.push('\n');
            }
            for note in &report.notes {
                md.push_str(&format!("- {note}\n"));
            }
            if report.tx_errors > 0 {
                md.push_str(&format!(
                    "- **{} of {} transactions failed at execution.** Signatures, slots, block indexes, \
                     and charged fees still match the chain. This is an execution failure, not a \
                     verification mismatch, but it must not be hidden.\n",
                    report.tx_errors, report.total_signatures
                ));
            }
            if report.target_block_members == 0 {
                md.push_str(
                    "- **No transaction landed in a target block.** Target-block ordering cannot be \
                     inferred from this campaign.\n",
                );
            }
            md.push('\n');
        }
        None => {
            md.push_str(
                "No verification file was found. Run `cargo run -- --verify-only --campaign-dir <dir>` \
                 before citing the campaign.\n\n",
            );
        }
    }

    md.push_str("## 9. Limitations and what went wrong\n\n");
    if mode == crate::analyzer::ExecutionMode::Legacy {
        md.push_str(
            "This is the **v1 legacy dataset**, not a v2 campaign. Its limitations are material:\n\n\
             - **Zero target-block probes.** All 216 transactions landed in slots after the first target \
               slot; the original report's cross-block correlations cannot establish within-block ordering.\n\
             - **All 216 transactions failed at runtime.** On-chain status reports \
               `InstructionError: ProgramFailedToComplete`; the memo program exceeded its compute meter. \
               Their signatures, slots, block indexes, and charged fees are verifiable, so the ordering \
               sample is structurally real, but these were not successful probes and must not be described \
               as such.\n\
             - **On-chain fee ties.** The v1 fee tiers (100/1,000/5,000/10,000 uL/CU at a low CU limit) \
               collapsed into only a few distinct charged fees (e.g., 5001, 5002, 5003). Tied fees cannot \
               identify a fee-priority rule.\n\
             - **Synthesized timing.** The v1 `send_timestamp_ns` values were generated at fixed 80 ms \
               intervals and are not the on-chain memo timestamps.\n\
             - **Memo mismatch.** On-chain memos use `superteam-ua:` while the v1 code emitted \
               `solana-tx-order:`; the old verifier did not check this.\n\
             - **Synthetic auxiliary data.** `contention_benchmark.json` and `saturation_benchmark.json` \
               were dry-run simulator outputs presented as empirical data.\n\
             - **Validator identity error.** The v1 roster contained `EkvdKhULRvdZq4E3yqj945rXw1yE8s1V8k4e7d5N5e8`, \
               which is not the on-chain leader identity `EkvdKhULbMFqjKBKotAzGi3kwMvMpYNDKJXXQQmi6C1f`.\n\
             - **Reporting errors.** The old report cited p < 0.001 and \"conclusive\" scheduling claims \
               that its own data did not support; it also stated the wrong measurement time and epoch.\n\n\
             The target-window fee statistic above is diagnostic negative evidence, not a validator comparison.\n\n",
        );
    } else if mode == crate::analyzer::ExecutionMode::DryRun {
        md.push_str(
            "**Dry-run.** Block placements are synthetic. This run validates the pipeline, report \
             generation, spend accounting, and statistics; it says nothing about mainnet validators. \
             No mainnet transaction was broadcast.\n\n",
        );
    } else {
        md.push_str(
            "- The campaign only observes inclusion and within-block order; it does not observe the \
               validator's internal scheduler or Jito bundle auctions.\n\
             - The target block may be skipped or unavailable; those bursts remain in the sample \
               accounting but contribute no target-block pairs.\n\
             - With a shared payer, the fee-payer write lock can serialize probes before the scheduler. \
               Use `--payer-mode independent` for a cleaner design.\n\
             - Public RPC `getSlot` polling is not a precision clock. Recorded send slots are the best \
               available estimate; the raw dispatch timestamps are included in the campaign data.\n\
             - The randomization test is conditional on the observed blocks; it cannot recover probes \
               that never landed.\n\n",
        );
    }

    if !analysis.warnings.is_empty() {
        md.push_str("### Machine-generated warnings\n\n");
        for warning in &analysis.warnings {
            md.push_str(&format!("- {warning}\n"));
        }
        md.push('\n');
    }

    md.push_str("## 10. Reproduction and data layout\n\n");
    md.push_str("```bash\n");
    md.push_str("# Inspect the committed legacy data without sending anything\n");
    md.push_str("cargo run --release -- --analyze-only --campaign-dir data/legacy-v1\n\n");
    md.push_str("# Re-run a pipeline-validating dry-run (no network broadcast)\n");
    md.push_str(
        "cargo run --release -- --dry-run --patterns randomized,uniform --repetitions 1\n\n",
    );
    md.push_str("# Verify a saved campaign against mainnet (read-only)\n");
    md.push_str("cargo run --release -- --verify-only --campaign-dir data/campaigns/<id>\n");
    md.push_str("```\n\n");
    md.push_str(&format!(
        "Raw campaign data: `{}`  \n{}\nSpend journal: `{}`\n\n",
        campaign
            .dir
            .as_deref()
            .map(|d| d.display().to_string())
            .unwrap_or_else(|| "(legacy JSON)".to_string()),
        raw_blocks_note(campaign),
        spend_path(campaign)
    ));

    md.push_str("## 11. Data dictionary\n\n");
    md.push_str(
        "- `campaign.json`: immutable campaign metadata, validator roster, timestamps, spend, notes.\n\
         - `bursts.json`: one record per burst, including every sent probe, every miss, raw signatures, \
           landed slot, block index, charged fee, send order, fee rank, and provenance.\n\
         - `raw_blocks/<slot>.json`: full parsed block used for index/fee/leader verification.\n\
         - `verification.json`: signature status, slot, index, fee, memo, and leader-schedule checks.\n\
         - `analysis.json`: machine-readable statistics and report bundle.\n\n",
    );

    md.push_str("## 12. Statistical definitions\n\n");
    md.push_str(
        "- **τb**: Kendall's tau-b computed per block with ties in fee/send order and block index \
           handled by the standard tie correction.\n\
         - **Mean τb**: unweighted mean of block-level τb; each block contributes one cluster.\n\
         - **95% CI**: percentile interval from bootstrapping whole blocks (2,000 resamples).\n\
         - **Permutation p**: two-sided Monte Carlo p-value from shuffling the comparison values within \
           each block (10,000 trials by default), with a +1 correction.\n\
         - **Sign-test p**: exact two-sided binomial test over the signs of non-zero block τb values; \
           low power, reported for transparency.\n\n",
    );

    md.push_str("## 13. Spend accounting\n\n");
    let (cap, spent, sent, confirmed) = effective_spend(campaign, spend);
    let remaining = cap.saturating_sub(spent);
    md.push_str(&format!(
        "| Field | Value |\n|---|---:|\n| Cap | {} lamports |\n| Spent (this campaign) | {} lamports |\n| Reserved | {} lamports |\n| Remaining under cap | {} lamports |\n| Transactions sent | {} |\n| Transactions confirmed | {} |\n\n",
        cap,
        spent,
        if campaign.meta.execution_mode == crate::analyzer::ExecutionMode::Legacy { 0 } else { spend.reserved_lamports },
        remaining,
        sent,
        confirmed
    ));
    if campaign.meta.execution_mode == crate::analyzer::ExecutionMode::Legacy {
        md.push_str(
            "Legacy totals were reconstructed from the charged fees stored in `measurements.json`; no \
             v2 spend tracker state existed for that campaign. The v1 transactions did pay these fees \
             even though their memo instructions failed.\n",
        );
    } else {
        md.push_str(
            "The spend tracker reserves worst-case fees before dispatch, commits the charged fee after \
             dispatch, and appends an audit line to the campaign or global live journal. The 0.05 SOL \
             hard cap is enforced before dispatch and is replayed from the journal on restart; it is \
             never reset silently.\n",
        );
    }

    md
}

fn status_banner(campaign: &Campaign, analysis: &CampaignAnalysis) -> String {
    let mode = campaign.meta.execution_mode;
    let body = match mode {
        crate::analyzer::ExecutionMode::Live => format!(
            "**LIVE CAMPAIGN.** {} target-block probes across {} usable block clusters. {}",
            analysis.sample.target_block_probes, analysis.fee_target_block.blocks_used, analysis.fee_target_block.verdict
        ),
        crate::analyzer::ExecutionMode::DryRun => format!(
            "**DRY-RUN — SYNTHETIC PLACEMENTS.** Pipeline validation only. {} target-block probes; none are mainnet evidence.",
            analysis.sample.target_block_probes
        ),
        crate::analyzer::ExecutionMode::Legacy => format!(
            "**LEGACY V1 DATA — NOT A VALID WITHIN-BLOCK EXPERIMENT.** {} target-block probes. The old report's p-values and conclusions are not reproduced.",
            analysis.sample.target_block_probes
        ),
        crate::analyzer::ExecutionMode::Mixed => format!(
            "**MIXED PROVENANCE.** {} target-block probes; inspect per-burst provenance before citing.",
            analysis.sample.target_block_probes
        ),
    };
    format!("> {body}\n")
}

fn effect_cell(stats: &SameBlockStats) -> String {
    match stats.mean_block_tau_b {
        Some(tau) => format!(
            "{:+.3} ({:+.3}..{:+.3})",
            tau,
            stats.ci95_low.unwrap_or(f64::NAN),
            stats.ci95_high.unwrap_or(f64::NAN)
        ),
        None => "n/a".to_string(),
    }
}

fn ci_cell(stats: &SameBlockStats) -> String {
    match (stats.ci95_low, stats.ci95_high) {
        (Some(lo), Some(hi)) => format!("{:+.3}..{:+.3}", lo, hi),
        _ => "n/a".to_string(),
    }
}

fn p_cell(p: Option<f64>) -> String {
    match p {
        Some(value) if value < 0.001 => format!("{value:.2e}"),
        Some(value) => format!("{value:.3}"),
        None => "n/a".to_string(),
    }
}

fn opt_num(value: Option<f64>, decimals: usize) -> String {
    match value {
        Some(value) => format!("{value:+.*}", decimals),
        None => "n/a".to_string(),
    }
}

fn effective_spend(
    campaign: &Campaign,
    _spend: &crate::spend_tracker::SpendSummary,
) -> (u64, u64, u64, u64) {
    // The campaign metadata is the authoritative per-campaign total. The live
    // journal may include earlier campaigns, and an analyze-only run has no
    // live tracker state at all.
    (
        campaign.meta.spend_cap_lamports,
        campaign.meta.total_spent_lamports,
        campaign.meta.total_tx_sent as u64,
        campaign.meta.total_tx_landed as u64,
    )
}

fn raw_blocks_note(campaign: &Campaign) -> String {
    let Some(dir) = &campaign.dir else {
        return "Raw blocks: not saved; legacy data is the original measurements JSON.".to_string();
    };
    let raw_dir = dir.join(crate::campaign::RAW_BLOCKS_DIR);
    let count = fs::read_dir(&raw_dir)
        .map(|entries| entries.filter_map(|e| e.ok()).count())
        .unwrap_or(0);
    if count > 0 {
        format!(
            "Raw blocks: `{}` ({} block files)",
            raw_dir.display(),
            count
        )
    } else {
        format!(
            "Raw blocks: not saved in `{}`; per-transaction slot/index/fee/memo evidence is in `verification.json`.",
            raw_dir.display()
        )
    }
}

fn spend_path(campaign: &Campaign) -> String {
    if campaign.meta.execution_mode == crate::analyzer::ExecutionMode::Legacy {
        return "(no v2 journal; spent fees reconstructed from measurements.json)".to_string();
    }
    campaign
        .dir
        .as_ref()
        .map(|d| {
            let local = d.join("spend_journal.jsonl");
            if local.exists() {
                local.display().to_string()
            } else if campaign.meta.execution_mode == crate::analyzer::ExecutionMode::Live {
                format!(
                    "data/live/spend_journal.jsonl (global live journal; campaign dir {})",
                    d.display()
                )
            } else {
                local.display().to_string()
            }
        })
        .unwrap_or_else(|| "data/live/spend_journal.jsonl".to_string())
}

fn render_html(
    campaign: &Campaign,
    analysis: &CampaignAnalysis,
    verification: Option<&VerificationReport>,
    spend: &crate::spend_tracker::SpendSummary,
) -> String {
    let mut html = String::new();
    html.push_str("<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\">");
    html.push_str("<meta name=\"viewport\" content=\"width=device-width,initial-scale=1\">");
    html.push_str("<title>Solana Transaction Ordering — Same-Block Campaign</title>");
    html.push_str("<style>");
    html.push_str(
        ":root{--bg:#f4f1e8;--ink:#141414;--muted:#5b5b55;--line:#c9c2b2;--accent:#0b6b5d;--warn:#8a3b12;--bad:#8b1e1e;--card:#fffdf6}*{box-sizing:border-box}body{margin:0;background:var(--bg);color:var(--ink);font:15px/1.5 Georgia,'Times New Roman',serif}header{background:#0b3b35;color:#f9f5e8;padding:28px 5vw 24px;border-bottom:6px double #d7b46a}h1{font-size:clamp(24px,4vw,44px);line-height:1.05;margin:0 0 10px;letter-spacing:-.02em}main{max-width:1280px;margin:0 auto;padding:28px 5vw 80px}.banner{border:2px solid var(--line);background:var(--card);padding:16px 18px;margin:0 0 24px;border-left:8px solid var(--accent)}.banner.legacy{border-left-color:var(--bad)}.banner.dryrun{border-left-color:var(--warn)}h2{margin:38px 0 10px;font-size:24px;border-bottom:1px solid var(--line);padding-bottom:6px}h3{margin:24px 0 8px}.meta{display:grid;grid-template-columns:repeat(auto-fit,minmax(220px,1fr));gap:10px;margin:16px 0 8px}.meta div{border:1px solid var(--line);background:var(--card);padding:10px 12px}.meta b{display:block;font:11px/1.2 ui-monospace,monospace;letter-spacing:.08em;text-transform:uppercase;color:var(--muted)}table{width:100%;border-collapse:collapse;margin:12px 0 24px;font-size:13px;background:var(--card)}th,td{border:1px solid var(--line);padding:8px 9px;text-align:left;vertical-align:top}th{background:#e9e3d4;font:11px/1.2 ui-monospace,monospace;letter-spacing:.06em;text-transform:uppercase}td.num{text-align:right;font-variant-numeric:tabular-nums}code{font:12px/1.4 ui-monospace,monospace;background:#e9e3d4;padding:1px 4px;border-radius:2px}.ok{color:var(--accent);font-weight:bold}.bad{color:var(--bad);font-weight:bold}.warn{color:var(--warn);font-weight:bold}.muted{color:var(--muted)}.grid{display:grid;grid-template-columns:repeat(auto-fit,minmax(280px,1fr));gap:16px}.card{border:1px solid var(--line);background:var(--card);padding:16px}.card h3{margin-top:0}.footer{margin-top:48px;border-top:1px solid var(--line);padding-top:16px;color:var(--muted);font-size:12px}@media(max-width:700px){th,td{font-size:12px;padding:6px}header{padding:20px 5vw}}"
    );
    html.push_str("</style></head><body><header>");
    html.push_str("<h1>Solana Transaction Ordering</h1>");
    html.push_str("<div>Same-block campaign report · v2 methodology</div></header><main>");

    let banner_class = match campaign.meta.execution_mode {
        crate::analyzer::ExecutionMode::Legacy => "banner legacy",
        crate::analyzer::ExecutionMode::DryRun => "banner dryrun",
        _ => "banner",
    };
    html.push_str(&format!("<div class=\"{banner_class}\">"));
    html.push_str(&format!(
        "<b>Campaign:</b> <code>{}</code><br><b>Mode:</b> <code>{}</code><br>",
        esc(&campaign.meta.campaign_id),
        campaign.meta.execution_mode
    ));
    html.push_str(&format!(
        "<b>Target-block probes:</b> {} &nbsp; <b>Usable clusters:</b> {}<br>",
        analysis.sample.target_block_probes, analysis.fee_target_block.blocks_used
    ));
    html.push_str(&format!(
        "<b>Primary verdict:</b> {}</div>",
        esc(&analysis.fee_target_block.verdict)
    ));

    html.push_str("<div class=\"meta\">");
    html.push_str(&meta_card("Measured start", &campaign.meta.created_at_utc));
    html.push_str(&meta_card(
        "Measured finish",
        campaign
            .meta
            .finished_at_utc
            .as_deref()
            .unwrap_or("not recorded"),
    ));
    html.push_str(&meta_card(
        "Transport",
        &campaign.meta.transport.to_string(),
    ));
    html.push_str(&meta_card(
        "Payer mode",
        &campaign.meta.payer_mode.to_string(),
    ));
    html.push_str(&meta_card(
        "Send lead",
        &format!("{} slot(s)", campaign.meta.send_lead_slots),
    ));
    html.push_str(&meta_card(
        "Spend",
        &format!(
            "{:.9} / {:.9} SOL",
            campaign.meta.total_spent_lamports as f64 / 1e9,
            campaign.meta.spend_cap_lamports as f64 / 1e9
        ),
    ));
    html.push_str("</div>");

    html.push_str("<h2>1. Primary result: target-block ordering</h2>");
    html.push_str("<p>The unit of inference is a block. Cross-block pairs are excluded. Ties in fee or send order are not counted as concordant. Intervals bootstrap whole blocks; p-values come from within-block label permutation.</p>");
    html.push_str("<table><thead><tr><th>Comparison</th><th>Usable blocks</th><th>Usable pairs</th><th>Mean τb (95% CI)</th><th>Permutation p</th><th>Sign-test p</th><th>Verdict</th></tr></thead><tbody>");
    for (label, stats) in [
        ("Priority fee vs index", &analysis.fee_target_block),
        ("Send order vs index", &analysis.send_target_block),
    ] {
        html.push_str(&format!(
            "<tr><td>{}</td><td class=\"num\">{}</td><td class=\"num\">{}</td><td>{}</td><td class=\"num\">{}</td><td class=\"num\">{}</td><td>{}</td></tr>",
            label,
            stats.blocks_used,
            stats.pairs_usable,
            esc(&effect_cell(stats)),
            p_cell(stats.permutation_p),
            p_cell(stats.sign_test_p),
            esc(&stats.verdict)
        ));
    }
    html.push_str("</tbody></table>");

    html.push_str("<h2>2. Sample accounting</h2>");
    html.push_str("<table><thead><tr><th>Quantity</th><th>Count</th></tr></thead><tbody>");
    for (label, value) in [
        ("Bursts attempted", analysis.sample.total_bursts),
        (
            "Bursts with target-block probes",
            analysis.sample.bursts_with_target_block_probes,
        ),
        (
            "Bursts with target-window probes",
            analysis.sample.bursts_with_target_window_probes,
        ),
        ("Probes sent", analysis.sample.total_probes_sent),
        ("Probes landed", analysis.sample.total_probes_landed),
        (
            "Target-block probes (primary)",
            analysis.sample.target_block_probes,
        ),
        (
            "Target-window probes (secondary)",
            analysis.sample.target_window_probes,
        ),
        (
            "Probes outside target window",
            analysis.sample.out_of_window_probes,
        ),
        ("Late bursts", analysis.sample.late_bursts),
    ] {
        html.push_str(&format!(
            "<tr><td>{}</td><td class=\"num\">{}</td></tr>",
            label, value
        ));
    }
    html.push_str("</tbody></table>");

    html.push_str("<h2>3. Per-validator results</h2>");
    html.push_str("<table><thead><tr><th>Validator</th><th>Client</th><th>Version</th><th>Bursts</th><th>Target probes</th><th>Fee τb</th><th>95% CI</th><th>p</th><th>Verdict</th></tr></thead><tbody>");
    for validator in &analysis.per_validator {
        html.push_str(&validator_row(validator));
    }
    html.push_str("</tbody></table>");

    html.push_str("<h2>4. On-chain verification</h2>");
    match verification {
        Some(report) => {
            html.push_str(&format!(
                "<p class=\"{}\"><b>{}</b> — confirmed {}/{}, index {}/{}, fee {}/{}, leader {}/{}, failures {}</p>",
                if report.passed() { "ok" } else { "warn" },
                if report.passed() { "PASS" } else { "INCOMPLETE" },
                report.confirmed,
                report.total_signatures,
                report.index_matched,
                report.total_signatures,
                report.fee_matched,
                report.total_signatures,
                report.leaders_matched,
                report.leaders_checked,
                report.failures.len()
            ));
            if !report.failures.is_empty() {
                html.push_str("<table><thead><tr><th>Burst</th><th>Signature</th><th>Kind</th><th>Expected</th><th>Actual</th></tr></thead><tbody>");
                for failure in report.failures.iter().take(20) {
                    html.push_str(&format!(
                        "<tr><td><code>{}</code></td><td><code>{}</code></td><td>{}</td><td>{}</td><td>{}</td></tr>",
                        esc(&failure.burst_id),
                        esc(failure.signature.as_deref().unwrap_or("-")),
                        esc(&failure.kind),
                        esc(&failure.expected),
                        esc(&failure.actual)
                    ));
                }
                html.push_str("</tbody></table>");
            }
        }
        None => html.push_str("<p class=\"warn\">No verification file. Run <code>--verify-only</code> before citing this campaign.</p>"),
    }

    html.push_str("<h2>5. Limitations / what went wrong</h2>");
    if campaign.meta.execution_mode == crate::analyzer::ExecutionMode::Legacy {
        html.push_str("<div class=\"card\"><p><b>Legacy v1 is not a valid within-block experiment.</b></p><ul><li>0 target-block probes; all probes landed after the first target slot.</li><li>Fee tiers collapsed into on-chain ties (e.g., 5001/5002/5003 lamports).</li><li>Timestamps were synthesized at 80 ms intervals; on-chain memo times differ.</li><li>Contention/saturation JSON were dry-run simulator output presented as empirical.</li><li>The roster contained an invalid validator identity.</li><li>The old report claimed p &lt; 0.001 and conclusive scheduling; this analysis does not reproduce that.</li></ul></div>");
    } else if campaign.meta.execution_mode == crate::analyzer::ExecutionMode::DryRun {
        html.push_str("<div class=\"card\"><p><b>Dry-run only.</b> Placements are synthetic. This validates the pipeline; it is not evidence about mainnet validators. No mainnet transaction was sent.</p></div>");
    } else {
        html.push_str("<div class=\"card\"><p>Observation is limited to inclusion and within-block order. The campaign does not observe scheduler internals, Jito bundles, or private order flow. A shared payer can add a fee-payer write-lock confound. Public RPC slot polling is not a precision clock.</p></div>");
    }

    if !analysis.warnings.is_empty() {
        html.push_str("<h2>6. Machine-generated warnings</h2><ul>");
        for warning in &analysis.warnings {
            html.push_str(&format!("<li>{}</li>", esc(warning)));
        }
        html.push_str("</ul>");
    }

    html.push_str("<h2>7. Reproduction</h2>");
    html.push_str("<pre><code>cargo run --release -- --analyze-only --campaign-dir data/legacy-v1\ncargo run --release -- --dry-run --patterns randomized,uniform\ncargo run --release -- --verify-only --campaign-dir data/campaigns/&lt;id&gt;</code></pre>");
    let (cap, spent, sent, confirmed) = effective_spend(campaign, spend);
    html.push_str(&format!(
        "<h2>8. Spend</h2><table><tbody><tr><td>Cap</td><td class=\"num\">{} lamports</td></tr><tr><td>Spent (this campaign)</td><td class=\"num\">{} lamports</td></tr><tr><td>Reserved</td><td class=\"num\">{} lamports</td></tr><tr><td>Remaining under cap</td><td class=\"num\">{} lamports</td></tr><tr><td>Sent / confirmed</td><td class=\"num\">{} / {}</td></tr></tbody></table>",
        cap,
        spent,
        if campaign.meta.execution_mode == crate::analyzer::ExecutionMode::Legacy { 0 } else { spend.reserved_lamports },
        cap.saturating_sub(spent),
        sent,
        confirmed
    ));
    html.push_str("<div class=\"footer\">Generated by solana-validator-ordering v2. Data, raw blocks, and verification files are linked from the campaign directory. No claim in this report is stronger than the sample accounting supports.</div>");
    html.push_str("</main></body></html>");
    html
}

fn meta_card(label: &str, value: &str) -> String {
    format!("<div><b>{}</b>{}</div>", esc(label), esc(value))
}

fn validator_row(validator: &ValidatorSameBlock) -> String {
    format!(
        "<tr><td><code>{}</code></td><td>{}</td><td><code>{}</code></td><td class=\"num\">{}</td><td class=\"num\">{}</td><td class=\"num\">{}</td><td>{}</td><td class=\"num\">{}</td><td>{}</td></tr>",
        esc(&validator.validator_identity),
        esc(&validator.client_type.to_string()),
        esc(&validator.version_string),
        validator.total_bursts,
        validator.target_block_probes,
        opt_num(validator.fee.mean_block_tau_b, 3),
        esc(&ci_cell(&validator.fee)),
        p_cell(validator.fee.permutation_p),
        esc(&validator.fee.verdict)
    )
}

fn esc(input: &str) -> String {
    input
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

#[allow(dead_code)]
fn pattern_label(pattern: BurstPattern) -> String {
    pattern.to_string()
}

#[allow(dead_code)]
fn report_path_label(path: &Path) -> String {
    path.display().to_string()
}
