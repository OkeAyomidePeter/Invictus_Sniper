# 🛠️ Invictus: Technical Architecture & Strategy Guide

Invictus is a high-performance, asynchronous Solana trading bot written in Rust. It is engineered for low-latency token discovery and execution, specifically targeting high-volatility "graduated" tokens.

---

## 🏗️ System Architecture

The software is structured as a multi-stage asynchronous pipeline, utilizing Rust's `tokio` runtime for concurrent execution.

### 1. Discovery Layer (`helius_listener.rs`)

- **Function**: Monitor the Solana blockchain for specific transaction types.
- **Mechanism**: Subscribes to Helius Webhook/WebSocket streams.
- **Filtering**: Specifically identifies `PoolCreation` (Raydium) and `Graduated` (Pump.fun to Raydium) events.
- **Output**: Dispatches `ClassifiedEvent` objects into the enrichment pipeline.

### 2. Enrichment Layer (`enrichment.rs`)

- **Function**: Transform raw blockchain events into high-fidelity data models.
- **Mechanism**: Concurrently fetches data from multiple APIs:
  - **Birdeye**: Provides OHLCV price action, liquidity depth, and volume in multiple timeframes (1m, 5m, 30m).
  - **Helius DAS**: Resolves token metadata (Twitter/Telegram links) and performs holder distribution analysis.
- **Caching**: Implements a thread-safe `MintAccountCache` to minimize redundant RPC calls and stay within API rate limits.

### 3. Scoring Engine (`scoring.rs`)

- **Function**: Deterministic evaluation of token potential and safety.
- **Scale**: 0 to 150 points.
- **Metrics**:
  - **Momentum (50 pts)**: Rate of price change in the last 5 minutes.
  - **Activity (40 pts)**: Measuring viral growth (new wallet counts and transaction density).
  - **Buy Pressure (30 pts)**: The ratio of buy volume vs. sell volume.
  - **Security (15 pts)**: Distribution of supply (Whale detection) and verified social presence.

### 4. Trade Engine (`trade_engine.rs`)

- **Function**: Orchestrate the full lifecycle of a trade (Buy → Monitor → Sell).
- **Phases**:
  - **Execution**: Coordinates with the `TransactionManager` to land transactions.
  - **Monitoring**: Spawns a dedicated tokio task for every open position to monitor real-time PnL.
  - **Automated Exit**: Triggers sells based on Profit Targets, Stop Losses, Trailing Stops, or Timeouts.

### 5. Execution Layer (`presigner.rs` & `tx.rs`)

- **Standard Routing**: Direct transactions to Solana RPC nodes.
- **Jito Integration**: Builds and sends Jito Bundles (multiple transactions grouped together with a tip) to bypass public congestion and prevent front-running.
- **Simulation**: Can simulate transactions pre-flight to check for failure conditions.

---

## 🧠 Trading Strategy & Logic

### Survival Gates (Risk Management)

Invictus applies "Survival Gates" before any trade to avoid common scams and low-quality tokens:

- **Authority Check**: Rejects any token with "Freeze Authority" or "Mint Authority" enabled.
- **Ghost Filter**: Requires a minimum count of unique holders (preventing early-stage bot-only pools).
- **Liquidity Floor**: Rejects tokens with liquidity below a configurable USD threshold (e.g., $2500).
- **Thin Pool Filter**: Low liquidity tokens are subjected to stricter volatility checks to avoid "wicking" out.

### Entry Conditions

- **Direct Buy**: Triggered when a token score exceeds the `buy_threshold` (e.g., 100/150).
- **Watchlist (Dip Strategy)**: High-potential tokens that don't meet the immediate buy threshold are added to a watchlist. A "Buy Signal" is triggered if the price corrects to a certain level while volume remains healthy.

### Exit Algorithms

1. **Take Profit (TP)**: Fixed percentage gain targets.
2. **Stop Loss (SL)**: Hard floor to protect capital from rugs or sharp dumps.
3. **Trailing Stop**: Activates after a minimum profit threshold is met. It "trails" the peak price by a configurable distance (e.g., 15%), locking in gains while allowing upward runs.
4. **Partial Exit**: Sells a portion of the position (e.g., 50%) at an early target to secure the initial investment, letting the remainder run.
5. **Timeout**: Exits the position if target/stop isn't hit within a specific window (e.g., 10 minutes) to minimize capital exposure time.

---

## 📊 Technical Stack

- **Runtime**: Rust / Tokio (Async/Await)
- **Networking**: Reqwest (JSON/REST), Tungstenite (WebSockets)
- **Database**: SQLite (via SQLx) for persistent trade logs and state.
- **Blockchain**: Solana Client, SPL Token, Helius (DAS/Webhooks), Jito (MEV).
- **Frontend/Dashboard**: Eframe/Egui (Rust-native immediate mode GUI).
- **Notifications**: Teloxide (Telegram Bot API).

---

## 🔄 Data Lifecycle Flow

1. **Blockchain Event** → Detected by Helius Listener.
2. **Raw Mint** → Enriched with Market Metrics from Birdeye/Helius.
3. **Enriched Token** → Scored by the Scoring Engine.
4. **High Score** → Trade Engine builds a Jito Bundle for the Buy.
5. **Buy Landed** → Position Tracker starts "Optimistic" price monitoring.
6. **Target/Stop Hit** → Trade Engine executes the Sell.
7. **Final PnL** → Recorded in SQLite and reported via Telegram/GUI.
