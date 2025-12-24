# 🎯 Invictus Sniper Bot: Strategy & Configuration Guide

This document provides a comprehensive breakdown of the Invictus Sniper Bot's configuration, scoring logic, and advanced trading features. Use this guide to optimize your bot for different market conditions.

---

## 🛠️ Core Configuration (`.env`)

These settings control the fundamental behavior of the bot.

### 💰 Trading Limits

- `MAX_TRADE_SIZE_SOL`: The amount of SOL used for each buy transaction (e.g., `0.1`).
- `MAX_DAILY_EXPOSURE_SOL`: Cumulative limit for trades in a 24-hour window.
- `TOTAL_EXPOSURE_LIMIT_SOL`: Maximum SOL allowed across all currently open positions.
- `MAX_OPEN_POSITIONS`: Maximum number of tokens the bot can hold simultaneously.

### ⏱️ Position Lifecycle (Hold Time)

- `AUTO_SELL_TIMEOUT_SECONDS`: The base "Exposure Time". If no profit target or stop loss is hit, the bot will sell after this duration (default: `120s`).
- `DYNAMIC_TIMEOUT_ENABLED`: Allows the bot to extend the hold time if the token is performing well but hasn't hit targets.
- `MAX_TIMEOUT_EXTENSIONS`: How many times the timeout can be extended (e.g., `2`).
- `TIMEOUT_EXTENSION_SECONDS`: Duration of each extension (e.g., `60s`).

---

## ⚖️ Token Scoring Strategy

The bot uses a "Growth Trend" scoring model (0-150 points). High scores trigger immediate buys, while moderate scores trigger the Watchlist (Dip Strategy).

### 💀 Instant Fail Conditions (Score: 0)

The bot WILL NOT buy if any of these are true:

- **Freeze Authority**: Is active (token can be frozen).
- **Mint Authority**: Is active (more tokens can be printed).
- **Low Liquidity**: Below `MIN_LIQUIDITY_USD` (default: `$2500`).
- **Ghost Token**: Fewer than 10 unique holders.

### 📈 Point Distribution (Max 150)

1. **Momentum (50 pts)**: Based on 30-minute price change.
   - `Excellent (+50%)`: 50 points.
   - `Good (+20%)`: Linear scaling.
   - `Negative`: Penalty points.
2. **Activity (40 pts)**: Based on unique wallets and volume in 30m.
   - High unique wallet counts and total volume over `$10k` boost this score.
3. **Buy Pressure (30 pts)**: Based on Buy/Sell volume ratio.
   - Pure buying pressure (no sells) or a ratio > 2.0 grants max points.
4. **Distribution (20 pts)**: Holder concentration.
   - **Penalty**: Top 1 holder > 30% or Top 10 > 60%.
   - **Bonus**: Top 10 < 30% (well-distributed).
5. **Socials (10 pts)**: Presence of Twitter, Telegram, or Website.

---

## 🚀 Advanced Trading Features

### 📉 Dip Strategy (Watchlist)

When a token scores between **70 and 99 points**, it enters the **Watchlist** instead of an immediate buy.

- `DIP_ENTRY_PCT`: The % drop from discovery price required to trigger a buy (e.g., `30.0` for a -30% dip).
- `VOLUME_TREND_ENABLED`: Validates that the dip isn't a "dead" drop by ensuring volume is stable or increasing during the dip.
- `MIN_VOLUME_USD_5M`: Minimum 5-minute volume required to trigger a watchlist buy.

### 📈 Exit Strategies (The "Money Makers")

- **Profit Target (`AUTO_SELL_PROFIT_TARGET_PCT`)**: Sells the entire position at a fixed % gain (e.g., `+50%`).
- **Stop Loss (`AUTO_SELL_STOP_LOSS_PCT`)**: Sells to protect capital at a fixed % loss (e.g., `-20%`).
- **Trailing Stop Loss**:
  - `TRAILING_STOP_ENABLED`: Tracks the peak price reached after the buy.
  - `TRAILING_STOP_DISTANCE_PCT`: Sells if the price drops by this % from the **peak** (e.g., `15.0`). This "locks in" profits during a run.
- **Partial Exits**:
  - `PARTIAL_EXIT_ENABLED`: Sells a portion of the position early.
  - `PARTIAL_EXIT_TARGET_PCT`: Gain % to trigger the partial sell (e.g., `+30%`).
  - `PARTIAL_EXIT_AMOUNT_PCT`: What % of the bag to sell (e.g., `50%`). The rest continues to run until a full target or stop is hit.

---

## 💡 Optimization Tips

| Market Vibe          | Strategy Adjustment                                                                 |
| :------------------- | :---------------------------------------------------------------------------------- |
| **Bullish/Moonshot** | Increase `AUTO_SELL_PROFIT_TARGET_PCT` to `100%+`, enable `TRAILING_STOP` at `20%`. |
| **Volatile/Choppy**  | Enable `PARTIAL_EXIT` at `+25%` to secure initial investment quickly.               |
| **Conservative**     | Increase `MIN_LIQUIDITY_USD` to `$5000` and `MIN_HOLDERS` to `50`.                  |
| **Sniper Mode**      | Set `DIP_STRATEGY_ENABLED=false` for instant execution on high scores.              |

---

> [!TIP]
> Always check your `invictus.log` to see the **Scoring Breakdown**. If the bot is rejecting tokens you like, adjust the thresholds in `src/main.rs` (default: 100 for Buy, 70 for Watchlist).
