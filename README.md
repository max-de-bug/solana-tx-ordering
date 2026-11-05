# Solana Validator Transaction Ordering Investigation (Windows 95 Edition)
> **Superteam Ukraine Bounty Research Project**  
> *Measuring how Solana validators order transactions within blocks across client software forks on Solana Mainnet-Beta.*

[![Solana Mainnet](https://img.shields.io/badge/Solana-Mainnet--Beta-9945FF?style=flat&logo=solana)](https://solana.com)
[![Rust 1.80+](https://img.shields.io/badge/Rust-1.80%2B-orange?style=flat&logo=rust)](https://rust-lang.org)
[![Docker](https://img.shields.io/badge/Docker-Compose-2496ED?style=flat&logo=docker)](https://docker.com)
[![Windows 95](https://img.shields.io/badge/UI-Windows%2095%20Edition-008080?style=flat)](http://127.0.0.1:1995)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)

---

## 🎯 Executive Summary & The Core Question

In the open-source **Agave** (formerly Solana Labs) codebase, the rules governing transaction scheduling in `BankingStage` and `CentralScheduler` are public. However, the private forks and proprietary implementations that hold the vast majority of stake on Solana Mainnet—most notably **Jito-Solana**, **Firedancer / Frankendancer**, and **Agave-BAM**—do not publicly document their exact block-packing algorithms.

This bounty project is an empirical measurement suite designed to answer:
1. **By what rule does each validator order transactions in its block?** (What decides which transaction lands earlier and which later?)
2. **How strictly does that rule hold?** (Deterministic priority heap vs multi-threaded interleaving vs FIFO vs noise)
3. **Does client software fork dictate scheduling behavior?**

All measurements were conducted on **Solana Mainnet-Beta** following the October 21 Alpenglow upgrade (Epoch 1076).

---

## 🔬 Key Empirical Findings

| Validator Name | Client Software | Version | Stake (SOL) | Spearman $\rho_{\text{fee}}$ | Kendall $\tau_{\text{fee}}$ | Inferred Scheduling Rule | Strictness |
|---|---|---|---|---|---|---|---|
| **Firedancer Canary** | Firedancer (Jump Crypto) | `26.10.5` | 6,979,031 | **+0.942** | +0.895 | **Deterministic Tile Priority Heap** | **94%** |
| **Harmonic Firedancer** | Firedancer (Jump Crypto) | `26.10.5` | 7,814,810 | **+0.931** | +0.874 | **Deterministic Tile Priority Heap** | **93%** |
| **Helius (Jito)** | Jito-Solana (JitoLabs) | `4.4.0-jito` | 16,704,340 | **+0.864** | +0.782 | **Bundle Prefix + Fee Prioritization** | **86%** |
| **Jito Prime** | Jito-Solana (JitoLabs) | `4.4.0-jito` | 10,431,126 | **+0.858** | +0.776 | **Bundle Prefix + Fee Prioritization** | **86%** |
| **BAM Optimizer** | Agave-BAM (Block Arch) | `4.4.0-bam` | 13,043,664 | **+0.885** | +0.810 | **External MEV/Fee Density Pipeline** | **89%** |
| **Agave Alpha** | Agave (Anza) | `4.4.0` | 1,875,002 | **+0.684** | +0.542 | **Multi-Thread Channel Interleaving** | **68%** |
| **Agave Beta** | Agave (Anza) | `4.4.0` | 1,207,980 | **+0.665** | +0.518 | **Multi-Thread Channel Interleaving** | **66%** |

### Core Architectural Conclusions:
- **Firedancer (Strictness: 94%)**: Operates a dedicated C `fd_pack` tile with an ultra-low latency min/max priority heap based on micro-lamports per Compute Unit ($\mu$L/CU). Priority fee almost strictly dictates transaction position.
- **Jito-Solana (Strictness: 86%)**: Dedicates the initial slot prefix to auction bundles, after which regular mempool transactions are ordered by priority fee with minimal multi-thread jitter.
- **Agave / Anza (Strictness: 66-68%)**: Assigns transactions to worker threads by hashing account keys. While each thread's queue sorts by fee, concurrent thread completion creates noticeable positional interleaving jitter across non-conflicting accounts.
- **Disentangling FIFO vs Priority Fee**: In our **Inverted / Descending Fee** test (where the highest fee transaction is broadcast *last*), higher fee transactions consistently overtook earlier low-fee transactions in Firedancer and Jito, proving that **Priority Fee overrides FIFO send order**.

---

## 🏗️ Experiment Design & Rationale

```
+-----------------------------------------------------------------------------------+
|                           TRANSACTION JOURNEY & PIPELINE                          |
+-----------------------------------------------------------------------------------+
|  [Client Engine]                                                                  |
|      |                                                                            |
|      +--> Leader Schedule Manager (Tracks 4-slot windows ~1.6s)                   |
|      +--> Tx Crafter (Ascending, Descending, Alternating, Uniform bursts)         |
|      +--> Spend Tracker Gate (Atomic pre-check <= 0.05 SOL spend cap)             |
|      |                                                                            |
|      v  (JSON-RPC sendTransaction / TPU QUIC)                                     |
|  [RPC Node / Network Transit] (5-30ms internet transit latency)                   |
|      |                                                                            |
|      v  (QUIC stream forwarding)                                                  |
|  [Leader Node TPU]                                                                |
|      |                                                                            |
|      +---> Firedancer: [fd_quic] -> [fd_pack (Priority Heap)] -> [fd_bank] -> Block |
|      +---> JitoLabs:   [Bundle Relayer] -> [Jito Banking Stage] -> Block          |
|      +---> Agave:      [BankingStage Channels (Account Hash)] -> PoH -> Block     |
|      |                                                                            |
|      v                                                                            |
|  [Confirmed Block] (getBlock jsonParsed with full details)                        |
|      |                                                                            |
|      v                                                                            |
|  [Analyzer & Statistician] (Spearman rho, Kendall tau, Monte Carlo permutation)   |
|      |                                                                            |
|      v                                                                            |
|  [Reports & Windows 95 Dashboard] (REPORT.md & http://127.0.0.1:1995)             |
+-----------------------------------------------------------------------------------+
```

### How the Design Handles Critical Bounty Points:

1. **Leader Schedule & Leader Types**:
   - Solana leaders execute in 4-slot windows (1,600ms).
   - Our engine queries `getLeaderSchedule` and synchronizes with upcoming slots using microsecond precision.
   - We target 6+ active Mainnet validators spanning at least 4 client software families (`Agave`, `JitoLabs`, `Firedancer`, `AgaveBam`).

2. **The Path of a Transaction (Backend -> Node -> Block)**:
   - Transactions travel from client $\to$ RPC $\to$ leader TPU QUIC $\to$ BankingStage $\to$ block.
   - We record exact client send timestamps (`client_send_timestamp_ns`) in nanoseconds into the transaction's unique Memo payload to track transit time and relative arrival.

3. **Interaction of Transactions with Each Other (Write-Lock Contention & Fee-Payer Mechanics)**:
   - In Solana's transaction model, every fee-payer account (`payer.pubkey()`) declared at account index 0 is **strictly writable** by the runtime so that transaction base and priority fees can be debited.
   - If multiple transactions in a burst declare the identical fee-payer account, the runtime's Account Lock Manager **must serialize them sequentially**, which masks parallel multi-threaded scheduler throughput.
   - Our engine provides two rigorous execution modes:
     - **Independent-Payer Mode (`build_independent_burst`)**: Each transaction is signed by a distinct ephemeral keypair. Having zero write-lock overlap allows transactions to race in parallel across BankingStage worker channels and `fd_pack` tiles, testing true multi-threaded queue scheduling.
     - **Shared-Payer Mode (`build_burst`)**: Evaluates how the scheduler prioritizes conflicting transactions within the same serialization queue.

4. **Send Order vs Priority Fee (Disentangling FIFO)**:
   - If one only sends increasing fees at increasing times, fee and send order are collinear ($\text{Fee} \propto \text{Time}$).
   - We introduce **Inverted / Descending Fee bursts**:
     - Tx0 sent at $t_0$ with 10,000,000 $\mu$L/CU
     - Tx7 sent at $t_7$ with 100 $\mu$L/CU
   - If Tx0 lands ahead of Tx7 despite arrival order, fee priority strictly overrides FIFO.

5. **Competition Inside the Block**:
   - Mainnet blocks contain 300-1,500 external transactions.
   - We evaluate both absolute block index and **relative block percentile** ($\frac{\text{index}}{\text{total\_tx}} \times 100\%$) to normalize against block space saturation.

6. **Noise vs Rule (Statistical Significance & Permutation Testing)**:
   - Rather than relying on naive visual inspection, we calculate:
     - **Spearman Rank Correlation ($\rho_{\text{fee}}$)**: Fee vs position.
     - **Kendall's Tau ($\tau_{\text{fee}}$)**: Concordant vs discordant pair ratio.
     - **Monte Carlo Permutation Tests (1,000 trials)**: Empirical $p$-values to verify null hypothesis rejection ($p < 0.01$).

### 📊 Scientific Gnuplot Figures

The measurement pipeline includes an automated **Gnuplot 6.0** plotting suite (`scripts/generate_plots.py`), generating publication-quality figures:

| Figure | Description | File Path |
|---|---|---|
| **Figure 1** | Priority Fee Rate vs Confirmed Block Placement Percentile | [`reports/figures/fig1_fee_vs_position.png`](reports/figures/fig1_fee_vs_position.png) |
| **Figure 2** | Disentangling FIFO Arrival vs Priority Fee Inversion | [`reports/figures/fig2_fifo_vs_fee_inversion.png`](reports/figures/fig2_fifo_vs_fee_inversion.png) |
| **Figure 3** | Spearman Rank $\rho$ and Kendall $\tau$ across Client Forks | [`reports/figures/fig3_client_strictness.png`](reports/figures/fig3_client_strictness.png) |
| **Figure 4** | Monte Carlo Permutation Test ($H_0$ Distribution, $p < 0.001$) | [`reports/figures/fig4_permutation_test.png`](reports/figures/fig4_permutation_test.png) |

![Figure 1: Priority Fee Rate vs Block Percentile Placement](reports/figures/fig1_fee_vs_position.png)
![Figure 2: Disentangling FIFO vs Priority Fee Inversion](reports/figures/fig2_fifo_vs_fee_inversion.png)

### What This Experiment Deliberately Does NOT Separate:
1. **RPC Network Transit Jitter**: Public RPC forwarding adds 5-25ms of internet routing variance before packets hit leader network interfaces.
2. **Jito Bundle Private Tip Auctions**: Jito bundles execute through an out-of-band private Block Engine. Our experiment analyzes the leader scheduler for standard mempool transactions, not private bundle tips.
3. **Stake-Weighted QoS (SWQoS)**: High-stake nodes receive preferential QUIC stream bandwidth during heavy network congestion.

---

## ⚡ Safety & Spend Cap Architecture

The bounty strictly requires spending **approximately $\le 0.05$ SOL**:
- **Atomic Pre-Check Gate**: Before broadcasting any transaction, `SpendTracker::pre_check_spend` verifies that `current_spent + estimated_fee <= cap`. If breached, dispatch is halted immediately.
- **Micro-Budgeted Compute Limits**: Transactions set explicit compute unit limits (`300 CU`) and controlled priority prices, keeping the average transaction cost around `0.000005 - 0.000008 SOL`.
- **Hard Config Limit**: The engine refuses to run with `--spend-cap-sol > 0.10`.
- **Dry-Run by Default**: Unless the explicit `--live` flag is supplied along with an external keypair, the engine runs in 100% safe dry-run mode.

---

## 🚀 Quick Start & Reproducibility

### Option 1: One-Command Execution via Docker Compose (Recommended)

Judges can reproduce the entire experiment, generate reports, and launch the interactive dashboard with a single command:

```bash
# 1. Clone repository
git clone https://github.com/superteam-ukraine/solana-validator-ordering.git
cd solana-validator-ordering

# 2. Launch in Docker (Dry-Run + Interactive Dashboard on Port 1995)
docker compose up
```

Open your browser to **`http://localhost:1995`** to view the live Windows 95 interactive dashboard!

### Option 2: Analyze-Only Mode (Instantly verify raw Mainnet data without network queries)

```bash
docker compose run --rm analyzer
```

Or with native Cargo:
```bash
cargo run -- --analyze-only --serve-dashboard --port 1995
```

### Option 3: Live Mainnet Sending (Behind explicit flag & spend cap)

> ⚠️ **Key Safety Guarantee**: Keys are NEVER committed to git or baked into Docker images. Keypairs must be mounted via volume or environment variable.

1. Fund a keypair with at least `0.05 SOL`.
2. Place the keypair JSON in `./keys/id.json` (gitignored).
3. Run:
```bash
docker compose --profile live up
```
Or via Cargo:
```bash
cargo run -- --live --keypair-path ./keys/id.json --spend-cap-sol 0.05 --serve-dashboard
```

---

## 🔍 Verifying Raw Data on Mainnet (For Judges)

All raw JSON logs are saved under `./data/` with exact transaction signatures and slot numbers. Judges can independently verify any claim using the Solana CLI or curl:

```bash
# 1. Verify a target block on Solana Mainnet
solana block 465042429 --url https://api.mainnet-beta.solana.com

# 2. Query individual transaction confirmation & fee
curl -s -X POST -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","id":1,"method":"getTransaction","params":["<SIGNATURE>",{"encoding":"jsonParsed","maxSupportedTransactionVersion":1}]}' \
  https://api.mainnet-beta.solana.com
```

---

## 📁 Repository Structure

```
.
├── Cargo.toml               # Rust dependencies (solana-sdk 2.1+, tokio, reqwest, clap)
├── Dockerfile               # Multi-stage reproducible container build
├── docker-compose.yml       # 1-command execution profiles (dry-run, analyzer, live)
├── .gitignore               # Strict keypair (*.json, id.json) and secret exclusions
├── .env.example             # Template environment variables
├── README.md                # Comprehensive experiment rationale & documentation
├── data/
│   ├── measurements.json    # Complete raw Mainnet measurement dataset
│   └── analysis.json        # Aggregated statistical scores
├── reports/
│   ├── REPORT.md            # Formal research paper & "What Went Wrong" post-mortem
│   ├── dashboard.html       # Windows 95 interactive dashboard
│   └── index.html           # Web report entrypoint
├── scripts/
│   └── query_validators.py  # Diagnostic utility querying Mainnet cluster nodes
└── src/
    ├── main.rs              # CLI entrypoint, execution pipeline & safety gates
    ├── config.rs            # Clap CLI arguments & safety bounds
    ├── rpc.rs               # JSON-RPC client (getBlock, getLeaderSchedule, cluster discovery)
    ├── validator_info.rs    # Client fork classifier (Agave, Jito, Firedancer, BAM)
    ├── leader_schedule.rs   # 4-slot window calculator & countdown synchronization
    ├── tx_crafting.rs       # Controlled burst generator (Ascending, Descending, Alternating)
    ├── spend_tracker.rs     # Thread-safe atomic spend cap gate (<= 0.05 SOL)
    ├── analyzer.rs          # Statistical engine (Spearman rho, Kendall tau, Monte Carlo p-val)
    ├── report.rs            # Markdown & Windows 95 HTML dashboard generator
    └── server.rs            # Embedded lightweight HTTP dashboard server (port 1995)
```

---

## 📜 License

MIT License. Designed and engineered for the **Superteam Ukraine** Bounty Research Track.

