# Profitability Rewire Plan (No-Code Analysis)

## Executive diagnosis

The bot architecture is strong on low-latency execution, but profitability is likely constrained by **signal quality**, **entry timing drift**, and **exit/PnL accounting feedback loops** rather than pure transaction landing speed.

### Why this diagnosis

1. The pipeline is optimized for speed (listener → enrichment → score → buy) and launches optimistic monitoring quickly.
2. Buy decisions are still threshold-driven and mostly static in `main.rs` (buy/watchlist thresholds and exposure checks are hardcoded in event loop logic).
3. The watchlist dip subsystem uses fallback values that can materially weaken quality checks (e.g., initial 5m volume currently defaulted to zero), reducing the quality of dip entries.
4. Position monitoring logic is advanced (trailing, partial, dynamic timeout), but trade outcome learning is limited to a simple consecutive-loss circuit breaker and does not feed a structured model back into entry criteria.

---

## Current architecture map (as implemented)

1. **Discovery**: Helius listener watches graduated-token style events and streams classified pool events.
2. **Enrichment**: Enrichment builds a rich token object (liquidity, holders, 1m/5m/30m metrics, socials).
3. **Scoring**: Scorer applies hard safety gates and momentum/activity/buy-pressure scoring with survival penalties.
4. **Decision loop**: Main loop maps score to buy/watchlist/ignore with static thresholds and basic risk caps.
5. **Execution**: Trade engine builds Jupiter transactions, signs once, broadcasts in parallel and starts optimistic monitoring.
6. **Monitoring/exits**: Position tracker handles partial TP, full TP, trailing stop, fixed stop, timeout/extension.
7. **Persistence/telemetry**: SQLite logs trades and analytics snapshots, but online decision adaptation remains shallow.

---

## Key architecture gaps hurting profitability

## 1) Decision policy is mostly static and globally applied

- Buy/watchlist thresholds are static defaults and only nudged by a coarse consecutive-loss counter.
- No regime-specific policy (e.g., high-volatility session vs mean-reversion session).
- No per-segment gating (liquidity buckets, holder distribution buckets, platform-specific behavior).

**Impact:** You likely overtrade noisy regimes and undertrade favorable regimes.

## 2) Weak feature-to-policy feedback loop

- You persist analytics, but there is no explicit “policy optimizer” stage that translates recent trade outcomes into updated thresholds/weights.
- Circuit-breaker reset/raise logic is too small-dimensional to capture why losses happen.

**Impact:** Same mistakes repeat across sessions.

## 3) Entry timing and quote quality are checked narrowly

- There is a two-sample stability check, but no broader slippage/route-quality confidence score before committing entry.
- No architecture to condition entry size on confidence.

**Impact:** You can enter at poor local microstructure moments even when macro score is high.

## 4) Watchlist dip quality controls are incomplete

- The watchlist currently seeds important validation fields with placeholders (e.g., initial volume = 0), and relies on external sampling later.
- This can make dip validation brittle and inconsistent.

**Impact:** Dip entries can become low-conviction or delayed in ways that degrade edge.

## 5) Exit engine is advanced but not “expectancy-aware”

- Exit triggers are configurable, but there is no per-trade expected-value adaptation based on entry archetype.
- The same stop/target structure can be suboptimal across different token archetypes.

**Impact:** Winners may be cut or losers held in patterns that hurt expectancy.

## 6) Metrics focus on outcomes, not counterfactuals

- You track PnL and triggers, but there is limited post-trade “what if” infrastructure (e.g., how would alternative exits/entry delays perform).

**Impact:** Hard to confidently rewire strategy knobs.

---

## Recommended rewire: add a Strategy Intelligence Layer

Insert a new conceptual layer between scoring and execution:

**`Discovery → Enrichment → Scoring → Strategy Intelligence → Execution → Monitoring → Learning`**

The new layer should perform:

1. **Regime classification** (market state tags per minute/session).
2. **Policy selection** (thresholds + sizing + allowed setups by regime).
3. **Confidence-adjusted sizing** (not just pass/fail buy).
4. **Entry vetoes** for poor quote quality / adverse short-window drift.

This is the highest leverage architectural change.

---

## Concrete rewiring priorities

## Priority 0 (Immediate, 1-2 days) — Observability-first

- Add a profitability dashboard dataset from existing DB tables:
  - PnL by trigger type
  - PnL by liquidity bucket
  - PnL by top_10 concentration bucket
  - PnL by entry momentum bucket (1m/5m)
- Define one north-star metric: **Expectancy per trade (SOL)** with confidence intervals.

## Priority 1 (Short term, 3-7 days) — Decision policy hardening

- Replace static global thresholds with **policy profiles**:
  - Aggressive, Balanced, Defensive profiles selected by regime.
- Convert “consecutive losses” from sole adaptation signal to a **multi-signal risk state**:
  - rolling win rate
  - rolling expectancy
  - drawdown depth
  - execution failure rate
- Add **no-trade windows** when execution quality degrades (high retries, poor fills).

## Priority 2 (Short term, 1-2 weeks) — Entry quality & sizing

- Add confidence score combining:
  - signal strength (score components)
  - quote stability quality
  - liquidity safety margin
  - holder concentration safety margin
- Map confidence bands to position size multipliers.
- Enforce stricter guardrails for thin-liquidity setups.

## Priority 3 (Mid term, 2-4 weeks) — Exit expectancy optimization

- Segment exits by setup archetype:
  - breakout continuation
  - dip-reclaim
  - late-momentum
- Run offline counterfactual replay from stored trade paths (where possible) to tune:
  - trailing activation
  - trail distance
  - partial exit size/trigger
  - timeout policy

## Priority 4 (Mid term, ongoing) — Learning loop

- Build periodic “policy refresh” job using trade_analytics + trades.
- Emit recommended parameter deltas, review them manually first.
- Move toward guarded auto-tuning once stable.

---

## Suggested target architecture (logical)

1. **Signal Factory**: Maintains normalized feature vectors from enrichment/scoring.
2. **Policy Engine**: Chooses profile + thresholds + sizing.
3. **Execution Gatekeeper**: Final veto on route/quote/slippage quality.
4. **Lifecycle Manager**: Existing trade engine + position tracker with setup-specific exits.
5. **Learning Service**: Batch evaluator producing next-cycle policy recommendations.

---

## Practical rollout plan

## Phase A: Stabilize measurement

- Freeze current strategy parameters for a baseline window.
- Produce daily expectancy report and segment breakdown.
- Identify top 3 loss archetypes.

## Phase B: Controlled experiments

- A/B policies by time slices (or alternating sessions).
- Change one dimension at a time (entry threshold, then sizing, then exits).
- Accept changes only if expectancy improves with stable drawdown.

## Phase C: Operational guardrails

- Hard stop trading when daily drawdown exceeds predefined risk budget.
- Auto-shift to Defensive profile on execution degradation.
- Keep manual override via Telegram/GUI for risk-off mode.

---

## What to rewire first (answer to your core question)

If you only rewire one part now, rewire the **decision architecture**:

- keep current discovery/execution speed,
- insert Strategy Intelligence to make entries selective and size-aware,
- connect trade analytics back into policy selection.

This should improve profitability more than further latency optimizations at this stage.
