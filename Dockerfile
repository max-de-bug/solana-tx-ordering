# Multi-stage Docker build for Solana Validator Ordering Experiment Engine
FROM rust:1.82-bookworm AS builder

WORKDIR /usr/src/solana-validator-ordering

# Copy dependency definitions
COPY Cargo.toml Cargo.lock ./

# Cache dependencies
RUN mkdir src && echo "fn main() {}" > src/main.rs && cargo build --release && rm -rf src

# Copy source code
COPY src ./src

# Build release binary
RUN touch src/main.rs && cargo build --release

# Minimal runtime image
FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    curl \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# Copy binary from builder
COPY --from=builder /usr/src/solana-validator-ordering/target/release/solana-validator-ordering /usr/local/bin/solana-validator-ordering

# Copy existing raw data and report assets
COPY data ./data
COPY reports ./reports

# Create volumes for persistent outputs and secure key mounting
VOLUME ["/app/data", "/app/reports", "/keys"]

# Expose Windows 95 Dashboard port
EXPOSE 1995

# Default execution is DRY-RUN with dashboard server enabled
ENV SOLANA_RPC_URL="https://api.mainnet-beta.solana.com"
ENV SPEND_CAP_SOL="0.05"

ENTRYPOINT ["solana-validator-ordering"]
CMD ["--dry-run", "--serve-dashboard", "--port", "1995"]
