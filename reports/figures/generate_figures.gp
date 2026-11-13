# Gnuplot Master Script for Solana Validator Ordering Research
set encoding utf8
set terminal pngcairo size 1200,800 font 'Helvetica,12' linewidth 2 rounded

# Color palette: Solarized / Modern Scientific
C_FD = "#268BD2"      # Blue (Firedancer)
C_JITO = "#D33682"    # Magenta (JitoLabs)
C_AGAVE = "#859900"   # Green (Agave)
C_BAM = "#CB4B16"     # Orange (Agave-BAM)
C_GRID = "#E0E0E0"

# ----------------------------------------------------------------------
# FIGURE 1: Priority Fee vs Block Position Percentile
# ----------------------------------------------------------------------
set output 'D:/projects_d/solana-tx-ordering/reports/figures/fig1_fee_vs_position.png'
set title "Figure 1: Priority Fee Rate vs Block Percentile Placement\n(Controlled Bursts on Solana Mainnet-Beta Post-Alpenglow)" font 'Helvetica-Bold,14'
set xlabel "Priority Fee Rate (micro-lamports / Compute Unit) [Log Scale]" font 'Helvetica,12'
set ylabel "Block Placement Percentile (0% = Head of Block, 100% = Tail)" font 'Helvetica,12'
set logscale x
set xrange [70:15000000]
set xtics ("100" 100, "1K" 1000, "5K" 5000, "20K" 20000, "100K" 100000, "500K" 500000, "2M" 2000000, "10M" 10000000) font 'Helvetica,10'
set yrange [0:100]
set ytics 10 font 'Helvetica,10'
set grid lc rgb C_GRID lt 1 lw 1
set key top right box opaque

plot 'D:/projects_d/solana-tx-ordering/reports/figures/data/fig1_fee_vs_position.dat' using 1:2 with linespoints pt 7 ps 1.5 lc rgb C_FD lw 3 title "Firedancer (fd\\_pack Min/Max Heap)", \
     'D:/projects_d/solana-tx-ordering/reports/figures/data/fig1_fee_vs_position.dat' using 1:3 with linespoints pt 9 ps 1.5 lc rgb C_JITO lw 3 title "Jito-Solana (Post-Bundle Fee Queue)", \
     'D:/projects_d/solana-tx-ordering/reports/figures/data/fig1_fee_vs_position.dat' using 1:5 with linespoints pt 5 ps 1.5 lc rgb C_BAM lw 3 title "Agave-BAM (Fee Density Optimizer)", \
     'D:/projects_d/solana-tx-ordering/reports/figures/data/fig1_fee_vs_position.dat' using 1:4 with linespoints pt 11 ps 1.5 lc rgb C_AGAVE lw 3 title "Agave (BankingStage Worker Channel Interleaving)"

# ----------------------------------------------------------------------
# FIGURE 2: FIFO vs Priority Fee Disentanglement (Inverted Descending Burst)
# ----------------------------------------------------------------------
set output 'D:/projects_d/solana-tx-ordering/reports/figures/fig2_fifo_vs_fee_inversion.png'
set title "Figure 2: Disentangling FIFO Arrival vs Priority Fee Ordering\n(Descending Fee Pattern: Tx0=10M uL/CU, Tx7=100 uL/CU)" font 'Helvetica-Bold,14'
unset logscale x
set xrange [-0.5:7.5]
set yrange [0:260]
set ytics 25 font 'Helvetica,10'
set xtics 1 font 'Helvetica,10'
set xlabel "Transmission Sequence Order (Tx 0 sent first with 10M fee, Tx 7 sent last with 100 fee)" font 'Helvetica,12'
set ylabel "Confirmed Block Index Position" font 'Helvetica,12'
set key top left box opaque

plot 'D:/projects_d/solana-tx-ordering/reports/figures/data/fig2_fifo_inversion.dat' using 1:3 with linespoints pt 7 ps 1.5 lc rgb C_FD lw 3 title "Firedancer (fd\\_pack)", \
     'D:/projects_d/solana-tx-ordering/reports/figures/data/fig2_fifo_inversion.dat' using 1:4 with linespoints pt 9 ps 1.5 lc rgb C_JITO lw 3 title "Jito-Solana (Post-Bundle Mempool)", \
     'D:/projects_d/solana-tx-ordering/reports/figures/data/fig2_fifo_inversion.dat' using 1:5 with linespoints pt 11 ps 1.5 lc rgb C_AGAVE lw 3 title "Agave (BankingStage Worker Interleaving)", \
     'D:/projects_d/solana-tx-ordering/reports/figures/data/fig2_fifo_inversion.dat' using 1:6 with linespoints pt 5 ps 1.5 lc rgb C_BAM lw 3 title "Agave-BAM (Fee Density Pipeline)"

# ----------------------------------------------------------------------
# FIGURE 3: Comparative Client Matrix (Spearman rho vs Kendall tau)
# ----------------------------------------------------------------------
set output 'D:/projects_d/solana-tx-ordering/reports/figures/fig3_client_strictness.png'
set title "Figure 3: Empirical Strictness & Correlation Coefficients Across Client Forks" font 'Helvetica-Bold,14'
set xlabel "Target Mainnet Validator" font 'Helvetica,12'
set ylabel "Correlation Coefficient (-1.0 to +1.0)" font 'Helvetica,12'
set yrange [0.0:0.45]
set ytics 0.05 font 'Helvetica,10'
set style data histogram
set style histogram cluster gap 1
set style fill solid 0.8 border -1
set boxwidth 0.9
set xtics rotate by -20 font 'Helvetica,10'
set key top right box opaque

plot 'D:/projects_d/solana-tx-ordering/reports/figures/data/fig3_validator_metrics.dat' using 3:xtic(1) title "Spearman Rank Correlation (rho_fee)" lc rgb C_FD, \
     'D:/projects_d/solana-tx-ordering/reports/figures/data/fig3_validator_metrics.dat' using 4 title "Kendall Tau (tau_fee)" lc rgb C_JITO

# ----------------------------------------------------------------------
# FIGURE 4: Monte Carlo Permutation Test (H0 vs Observed Statistic)
# ----------------------------------------------------------------------
set output 'D:/projects_d/solana-tx-ordering/reports/figures/fig4_permutation_test.png'
set title "Figure 4: Monte Carlo Permutation Test (1,000 Permutations)\nNull Hypothesis H0 (Random Ordering) vs Observed Mainnet Statistic" font 'Helvetica-Bold,14'
set xlabel "Permutation Rank Correlation (rho)" font 'Helvetica,12'
set ylabel "Permutation Frequency Count" font 'Helvetica,12'
set xrange [-0.8:1.0]
set yrange [0:80]
set boxwidth 0.03
set style fill solid 0.5 border -1

# Arrow pointing to observed statistic
set arrow from 0.342,60 to 0.342,5 lc rgb "red" lw 3
set label "Observed rho = +0.342\n(p < 0.001, Reject H0)" at 0.16,65 font 'Helvetica-Bold,11' tc rgb "red"

plot 'D:/projects_d/solana-tx-ordering/reports/figures/data/fig4_permutation_null.dat' using 1:2 with boxes lc rgb "#6C71C4" title "Empirical Null Distribution H0"
