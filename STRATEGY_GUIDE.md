# 🎯 Invictus Sniper Bot: Detailed Strategy Guide

This guide provides a comprehensive "mental picture" of how Invictus operates—from the second it discovers a token to the final cleanup after a successful sell.

---

## 🔍 Phase 1: High-Velocity Discovery & Scoring

Invictus is built for **speed** and **momentum**. It focuses specifically on graduated tokens (Pump.fun to Raydium/PumpSwap migration) where the "real" volume and volatility live.

### The Scoring Model (0-150 Points)

Every token is graded against a "Growth Trend" model using high-frequency data (1-minute and 5-minute intervals).

| Category                | Max Points | Logic / Goal                                                                    |
| :---------------------- | :--------- | :------------------------------------------------------------------------------ |
| **Momentum (5m)**       | 50         | Detects tokens with verified price action (+15% in 5m is max score).            |
| **Activity (5m)**       | 40         | Measures "Virality." Looks for 30+ new wallets and high transaction volume.     |
| **Buy Pressure (5m)**   | 30         | Favors tokens where the buy/sell ratio is > 2.0 (Double the buyers vs sellers). |
| **Ignition Bonus (1m)** | 15         | Detects a "Micro-Pump" (+3% in 60 seconds) to catch entries early.              |
| **Socials/Safety**      | 15         | Rewards tokens with distributed holders and active Twitter/TG links.            |

### The "Survival Gates" (Instant Rejection)

The bot acts as its own auditor. It will **never** buy if:

- **Freeze/Mint Authority** is ON (Scam protection).
- **Liquidity** is under `$2500` (Safety floor).
- **Ghost Tokens**: Fewer than 10 unique holders.
- **Liquidity Volatility**: High-risk filter. If liquidity is low (< $10k), it rejects tokens with extreme volatility (> 25% moves) to avoid "thin" rug-pulls.

---

## ⚡ Phase 2: High-Response Execution

Once a token scores **100+**, the execution engine kicks in with an "Optimistic" architecture.

1. **Jito Bundle Routing**: The bot builds a bundle with your swap and a "Jito Tip" (min 500,000 lamports). It cycles through **Global, NY, Amsterdam, Frankfurt, and Tokyo** block engines to ensure your transaction lands even during peak congestion.
2. **Optimistic Monitoring**: Standard bots wait for the blockchain to "confirm" your balance (taking 10-15 seconds). **Invictus doesn't wait.**
   - It assumes the buy worked and starts monitoring the price **immediately** using the "Expected Amount" from the swap quote.
   - This removes the "13-second blind spot" where you could lose money before the bot even starts tracking.
3. **Background Verification**: While the price tracker is already running, a background task quietly polls the node every 500ms. Once the real balance is confirmed, it updates the "Optimistic" position with the exact numbers.

---

## 📊 Phase 3: Real-Time Monitoring & Profit Tracking

The Monitoring Loop is the heartbeat of the bot. It runs a dedicated task for every open position with a default check interval of **5 seconds** to optimize API usage.

- **Price Aggregation**: It queries **Moralis** first (for credit efficiency) and falls back to **Birdeye** if needed. This prioritization can be toggled via `PRICE_SOURCE_PRIORITY`.
- **P/L Calculation**: It tracks Profit/Loss relative to your **Entry Price (SOL per Token)**. Every move is recorded in the `invictus.db`.
- **The Highest Price (Peak)**: The bot continuously updates the `highest_price_reached`. This is the reference point for the Trailing Stop.

---

## 💰 Phase 4: Dynamic Exit Strategies

Invictus uses a multi-layered exit system to secure profits and protect capital.

### 1. The Safety Floor (Fixed Stop Loss)

If the token drops to your `AUTO_SELL_STOP_LOSS_PCT` (e.g., -20%), the bot exits immediately. This is your insurance policy.

### 2. securing the Initial (Partial Exit)

Once the token hits a target (e.g., +30% gain), the bot can sell a portion (e.g., 50%) of the position.

- **Goal**: secures your initial SOL investment so the remaining bag is "risk-free" profit.

### 3. The Smart Trailing Stop (Gain Protection)

This is the most advanced part of the strategy.

- **Activation Threshold**: The trailing stop stays **OFF** until you hit a certain profit (e.g., +10%). This survives initial "chop" and noisy price action.
- **Trailing Distance**: Once active, if the price drops by `TRAILING_STOP_DISTANCE_PCT` from the absolute peak, it sells.
- **Tightening Logic**: If the trade lasts more than 90 seconds without a new peak, the bot tightens the trail distance (e.g., from 15% to 12.5%) to lock in whatever is left.

### 4. Timeouts (Exposure Time)

- **Exposure Limit**: If a token doesn't hit a target or stop within your `AUTO_SELL_TIMEOUT_SECONDS`, the bot exits anyway to free up SOL for better plays.
- **Dynamic Extensions**: If the token is currently at its **Peak Price** when the timer runs out, the bot grants an "Extension" because the momentum is still alive.

---

## 🏁 Phase 5: Cleanup (Zombie Protection)

Once a sell is confirmed, the position is **purged** from the tracker. This prevents "Zombie Positions"—broken loops that try to sell tokens you no longer own.

---

## 💡 How to Read the Logs

- `🛡️ OPTIMISTIC`: Monitoring started before the balance was even confirmed.
- `🎯 BUY ATTEMPT`: The bot has found a winner and is hitting the "buy" button.
- `🚨 SELL SIGNAL`: A target (Profit/Stop/Trail) was hit, and the exit is being sent.
- `🏁 Position tracker: Removed`: The trade lifecycle is 100% complete.
