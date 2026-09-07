# Invictus

**Invictus** is a high-performance, asynchronous Solana trading bot written in Rust. It is designed for low-latency token discovery, scoring, execution, and automated position management, with a focus on newly graduated and high-volatility tokens.

The system monitors on-chain activity, enriches discovered tokens with market and holder data, evaluates them through a deterministic scoring engine, and can automatically execute and manage trades.

> **Status:** Experimental / research project
> **Version:** 0.1.0

---

## Overview

Invictus is built as an asynchronous event-driven trading pipeline:

```text
Solana / Helius
      │
      ▼
┌───────────────┐
│   Discovery   │
│ Helius Events │
└───────┬───────┘
        │
        ▼
┌───────────────┐
│  Enrichment   │
│ Market + Meta │
└───────┬───────┘
        │
        ▼
┌───────────────┐
│    Scoring    │
│   0 - 150     │
└───────┬───────┘
        │
        ├───────────────┐
        │               │
   High Score      Watchlist
        │               │
        ▼               ▼
┌────────────────────────────┐
│       Trade Engine         │
│ Buy → Monitor → Sell       │
└─────────────┬──────────────┘
              │
              ▼
       Position Tracking
              │
              ▼
       SQLite + Telegram
```

The core runtime is built around Tokio and uses asynchronous tasks to handle discovery, enrichment, trading, monitoring, notifications, and background services concurrently.

---

## Features

### Token Discovery

* Real-time Solana event monitoring through Helius
* Detection of relevant pool creation and token graduation events
* Asynchronous event pipeline
* Low-latency processing from discovery to trade decision

### Token Enrichment

Discovered tokens are enriched with additional market and blockchain data, including:

* Liquidity
* Price movement
* Trading volume
* Buy/sell pressure
* Unique wallet activity
* Holder distribution
* Token metadata
* Twitter / Telegram presence

External data sources include Helius, Birdeye, and Moralis.

### Scoring Engine

Tokens are evaluated on a **0-150 point scale**.

The scoring model considers:

| Category              |  Maximum |
| --------------------- | -------: |
| Momentum              |       50 |
| Activity              |       40 |
| Buy pressure          |       30 |
| Ignition bonus        |       15 |
| Distribution / safety | Variable |
| Social presence       |       10 |

The scorer also applies safety gates and penalties before a token can become eligible for trading.

### Safety Filters

Invictus rejects tokens that fail configured safety conditions, including:

* Active mint authority
* Active freeze authority
* Insufficient liquidity
* Insufficient holder count
* Excessive whale concentration
* Extreme short-term volatility
* Poor price displacement relative to trading volume
* Momentum decay
* Buy-pressure absorption patterns

These checks are intended to filter obvious low-quality or dangerous setups before execution.

### Automated Trading

The trade engine manages the complete trade lifecycle:

```text
Signal
  ↓
Buy
  ↓
Position Created
  ↓
Real-Time Monitoring
  ↓
Take Profit / Stop Loss / Trailing Stop / Timeout
  ↓
Sell
  ↓
Record PnL
```

The system supports both standard Solana transaction execution and Jito-based transaction routing.

### Position Management

Open positions can be managed using:

* Fixed take-profit targets
* Stop losses
* Trailing stops
* Partial exits
* Dynamic timeouts
* Multiple concurrent positions
* Exposure limits

The system can also resume monitoring existing positions after startup.

### Watchlist / Dip Strategy

Tokens that are interesting but do not immediately meet the buy threshold can be placed on a watchlist.

The watchlist can subsequently trigger a buy when additional conditions are satisfied.

This allows the bot to distinguish between immediate high-conviction entries and setups that require additional confirmation.

### Telegram Control

Telegram is integrated for notifications and runtime control.

The bot can also enter a kill-switch / "comatose" mode and wait for a Telegram command before resuming operation.

### Native GUI

Invictus includes a native desktop GUI built with **egui / eframe**.

The GUI provides a visual interface for the bot and its runtime logs.

---

## Architecture

The main components are organized roughly as follows:

```text
src/
├── bin/
│   └── invictus-gui.rs       # Native GUI entry point
│
├── config.rs                 # Configuration
├── db.rs                     # SQLite persistence
├── enrichment.rs             # Market/token enrichment
├── health.rs                 # Health monitoring
├── helius_listener.rs        # Solana event discovery
├── lib.rs                    # Library exports
├── main.rs                   # Main bot runtime
├── moralis_client.rs         # Moralis integration
├── pnl_tracker.rs            # PnL tracking
├── position_tracker.rs       # Open position management
├── presigner.rs              # Transaction preparation/signing
├── rate_limiter.rs           # API rate limiting
├── scoring.rs                # Token scoring
├── trade_engine.rs           # Trade execution lifecycle
├── tx.rs                     # Transaction management
├── watchlist.rs              # Watchlist/dip strategy
└── ...
```

The project exposes both a command-line bot binary and a GUI binary through Cargo.

---

## Technology Stack

### Core

* Rust 2021
* Tokio
* Async/Await
* SQLx
* SQLite
* Serde / Serde JSON
* Reqwest
* WebSockets

### Solana

* Solana SDK
* Solana Client
* SPL Token
* SPL Token 2022
* SPL Associated Token Account
* Jito transaction routing

### Market Data

* Helius
* Birdeye
* Moralis
* Jupiter

### Interface & Monitoring

* egui
* eframe
* Telegram Bot API via Teloxide
* Prometheus
* `env_logger`
* `tracing-subscriber`

The dependency configuration is defined in `Cargo.toml`.

---

## Requirements

You will need:

* Rust / Cargo
* A Solana wallet
* Helius API access
* Birdeye API access
* Jupiter API access
* Moralis API access
* Telegram bot credentials if Telegram functionality is enabled

The project currently targets Solana mainnet by default.

---

## Installation

Clone the repository:

```bash
git clone https://github.com/OkeAyomidePeter/Invictus_Sniper.git
cd Invictus_Sniper
```

Build the project:

```bash
cargo build --release
```

---

## Configuration

Create a `.env` file based on the provided example:

```bash
cp .env.example .env
```

At minimum, configure the required API credentials and wallet settings.

```env
HELIUS_API_KEY=your_helius_api_key
BIRDEYE_API_KEY=your_birdeye_api_key
JUPITER_API_KEY=your_jupiter_api_key
MORALIS_API_KEY=your_moralis_api_key

SOLANA_PRIVATE_KEY=your_private_key

TELEGRAM_BOT_TOKEN=your_telegram_bot_token
TELEGRAM_CHAT_ID=your_telegram_chat_id

SOLANA_RPC_URL=https://api.mainnet-beta.solana.com
DATABASE_URL=sqlite://invictus.db
```

The repository's `.env.example` contains the complete set of available configuration options, including trading limits, transaction settings, rate limits, position management, dip strategy, and trailing-stop configuration.

**Never commit your real `.env` file or private wallet keys to the repository.**

---

## Running

Run the trading bot:

```bash
cargo run --release
```

Or run the compiled binary:

```bash
./target/release/invictus
```

The project also provides a native GUI binary:

```bash
cargo run --release --bin invictus-gui
```

The GUI is built with eframe/egui and starts with a 1400×900 default viewport.

---

## Trading Configuration

Some of the primary risk parameters include:

```env
MIN_LIQUIDITY_USD=2500.0
MIN_HOLDERS=10
MAX_TRADE_SIZE_SOL=1.0
MAX_DAILY_EXPOSURE_SOL=10.0
MAX_CONCURRENT_TRADES=3
MAX_OPEN_POSITIONS=10
TOTAL_EXPOSURE_LIMIT_SOL=10.0
```

Automated exits can be configured through:

```env
AUTO_SELL_ENABLED=true
AUTO_SELL_PROFIT_TARGET_PCT=50.0
AUTO_SELL_STOP_LOSS_PCT=20.0
AUTO_SELL_TIMEOUT_SECONDS=120
AUTO_SELL_SLIPPAGE_BPS=300
```

Trailing stops and partial exits are also configurable:

```env
TRAILING_STOP_ENABLED=true
TRAILING_STOP_DISTANCE_PCT=15.0

PARTIAL_EXIT_ENABLED=true
PARTIAL_EXIT_TARGET_PCT=30.0
PARTIAL_EXIT_AMOUNT_PCT=50.0
```

These values are examples from the repository's current configuration template and should be reviewed carefully before using the system with real funds.

---

## Transaction Modes

Invictus supports different transaction execution modes:

```env
TRANSACTION_MODE=Standard
SELL_TRANSACTION_MODE=Standard
```

Jito can be enabled for transaction routing:

```env
TRANSACTION_MODE=Jito
```

Jito tip behavior can be configured through:

```env
JITO_BASE_TIP_LAMPORTS=1000000
JITO_MIN_TIP_LAMPORTS=500000
JITO_MAX_TIP_LAMPORTS=5000000
JITO_DYNAMIC_TIPS_ENABLED=true
```

The system also supports separate transaction modes for buys and sells, including fallback behavior when Jito execution fails.

---

## Risk Management

Invictus is designed around several layers of risk control.

### Position Limits

The bot limits:

* Maximum trade size
* Maximum open positions
* Total exposure
* Daily exposure

### Token Safety

Before scoring and execution, tokens can be rejected for:

* Mint authority
* Freeze authority
* Low liquidity
* Low holder count
* Whale concentration
* Excessive volatility

### Circuit Breaker

The trading loop raises its score thresholds after consecutive losses.

By default, three consecutive losses activate the circuit breaker, increasing the buy and watchlist thresholds.

---

## Data & Persistence

Invictus uses SQLite through SQLx to persist trading state and analytics.

The default database configuration is:

```env
DATABASE_URL=sqlite://invictus.db
```

The database is used for persistent trade information, token data, monitoring state, and PnL-related information.

---

## Operational Flow

A simplified runtime flow looks like this:

1. Start the application.
2. Load environment configuration.
3. Start the Helius listener.
4. Receive relevant Solana events.
5. Enrich discovered tokens with external market and blockchain data.
6. Run the token through the scoring engine.
7. Reject unsafe or low-quality tokens.
8. Immediately trade high-scoring tokens or place moderate candidates on the watchlist.
9. Monitor open positions.
10. Trigger exits according to the configured strategy.
11. Persist trade and PnL information.
12. Send operational information through Telegram and the GUI.

The main runtime coordinates these components through Tokio tasks and asynchronous channels.

---

## Strategy

The original strategy is focused on detecting momentum in newly graduated tokens.

The primary signal components include:

* 5-minute price momentum
* 5-minute wallet activity
* 5-minute buy/sell pressure
* 1-minute momentum ignition
* Holder distribution
* Social presence
* Liquidity
* Short-term volatility

The strategy also contains additional survival penalties designed to detect momentum decay, choppy price action, and buy-pressure absorption.

For more detail, see:

* [`STRATEGY_GUIDE.md`](./STRATEGY_GUIDE.md)
* [`tech.md`](./tech.md)
* [`arch.md`](./arch.md)

---

## Project Documentation

| Document                                   | Description                                              |
| ------------------------------------------ | -------------------------------------------------------- |
| [`STRATEGY_GUIDE.md`](./STRATEGY_GUIDE.md) | Detailed trading strategy and lifecycle                  |
| [`tech.md`](./tech.md)                     | Technical architecture and system design                 |
| [`arch.md`](./arch.md)                     | Architecture analysis and proposed strategy improvements |
| [`.env.example`](./.env.example)           | Configuration reference                                  |

---

## Disclaimer

Invictus is an experimental software project for automated cryptocurrency trading.

Automated trading involves significant financial risk. The scoring system, safety filters, transaction execution, and exit strategies do not guarantee profitable trades or protection against losses.

Use appropriate risk limits and thoroughly test the system before deploying it with real funds.

The authors are not responsible for financial losses resulting from the use or modification of this software.

---

## License

No license is currently specified for this repository.

Until a license is added, the default copyright rules apply and others should not assume that the code can be freely reused, modified, or redistributed.
