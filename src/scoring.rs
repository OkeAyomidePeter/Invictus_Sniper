use crate::enrichment::EnrichedToken;
use log::{info, warn};
use crate::trade_logger::TradeLogger;

/// Scorer for enriched tokens - FRESH PUMP FOCUSED
/// GRADUATED TOKENS ONLY (Pump.fun/Bonk.fun)
/// 
/// NEW SCORING PHILOSOPHY (User Directed):
/// - Focus on HIGH FREQUENCY data (5m / 1m)
/// - "Jump on Pumps" strategy
/// - Liquidity is just a safety filter
/// 
/// MAX SCORE: 150 points
/// - Momentum (5m): 50 points (price change, velocity)
/// - Activity (5m): 40 points (wallets, volume)
/// - Buy Pressure (5m): 30 points (buy/sell ratio)
/// - Ignition Bonus (1m): 15 points (instant pump detection)
/// - Distribution: 15 points (safety)
/// 
/// PENALTIES:
/// - Authorities (freeze/mint): Instant fail
/// - Low liquidity: Instant fail
/// - Ghost town: Instant fail

// Liquidity thresholds (minimum requirements in USD)
const LIQUIDITY_MIN_THRESHOLD_USD: f64 = 2500.0; // Minimal filter
const LIQUIDITY_HEALTHY_THRESHOLD_USD: f64 = 5000.0;

// Momentum scoring (5m window - PRIMARY)
const PRICE_CHANGE_5M_EXCELLENT: f64 = 15.0; // +15% in 5m is massive
const PRICE_CHANGE_5M_GOOD: f64 = 5.0; // +5% in 5m is strong
const SCORE_MOMENTUM_MAX: f64 = 50.0;

// Activity scoring (5m window)
const UNIQUE_WALLETS_5M_HIGH: u64 = 30; // 30 new wallets in 5m is viral
const UNIQUE_WALLETS_5M_MED: u64 = 10; 
const SCORE_ACTIVITY_MAX: f64 = 40.0;

// Buy pressure scoring (5m window)
const BUY_SELL_RATIO_EXCELLENT: f64 = 2.0; // 2x more buying than selling
const BUY_SELL_RATIO_GOOD: f64 = 1.2; // 20% more buying
const SCORE_BUY_PRESSURE_MAX: f64 = 30.0;

// Ignition Bonus (1m window)
const PRICE_CHANGE_1M_IGNITION: f64 = 3.0; // +3% in 1m
const SCORE_BONUS_IGNITION: f64 = 15.0;

// Holder distribution
const HOLDER_TOP_1_PENALTY_THRESHOLD: f64 = 30.0;
const HOLDER_TOP_10_PENALTY_THRESHOLD: f64 = 60.0;
const HOLDER_TOP_10_BONUS_THRESHOLD: f64 = 30.0;
const SCORE_PENALTY_WHALE: f64 = 50.0;
const SCORE_PENALTY_TOP_10: f64 = 20.0;
const SCORE_BONUS_DISTRIBUTION: f64 = 15.0;

// Socials
const SCORE_SOCIAL_MAX: f64 = 10.0;

// Total holders (Safety)
const UNIQUE_HOLDERS_CRITICAL_LOW: u64 = 10;

// Survival Gate Thresholds
const DISPLACEMENT_RATIO_MIN: f64 = 0.5; // Min price movement per $1k volume
const MOMENTUM_DECAY_THRESHOLD_5M: f64 = 10.0;
const LOW_LIQUIDITY_VOLATILITY_MAX_5M: f64 = 25.0;
const LOW_LIQUIDITY_LIMIT_USD: f64 = 10000.0;
const BUY_PRESSURE_ABSORPTION_RATIO: f64 = 1.5;
const PENALTY_SURVIVAL_GATE: f64 = 50.0;

#[derive(Debug, Clone)]
pub struct TokenScorer;

impl TokenScorer {
    pub fn new() -> Self {
        Self
    }

    /// Calculate score for a token (0-150)
    /// FRESH PUMP FOCUSED (5m/1m data)
    pub fn score(&self, token: &EnrichedToken) -> f64 {
        let mut score = 0.0;
        let mut breakdown = ScoringBreakdown::default();

        // =====================================================================
        // 1. SAFETY CHECKS (CRITICAL) - INSTANT FAIL
        // =====================================================================
        
        if token.has_freeze_authority {
            warn!("💀 SCORING: {} has freeze authority -> SCORE 0", token.mint);
             TradeLogger::log(&format!("💀 SCORING REJECTED: {} has freeze authority", token.mint));
            return 0.0;
        }
        if token.has_mint_authority {
            warn!("💀 SCORING: {} has mint authority -> SCORE 0", token.mint);
            TradeLogger::log(&format!("💀 SCORING REJECTED: {} has mint authority", token.mint));
            return 0.0;
        }
        
        let liquidity_usd = token.liquidity_usd.unwrap_or(0.0);
        if liquidity_usd < LIQUIDITY_MIN_THRESHOLD_USD {
            warn!("💀 SCORING: {} has insufficient liquidity (${:.0} < ${})", 
                token.mint, liquidity_usd, LIQUIDITY_MIN_THRESHOLD_USD);
            TradeLogger::log(&format!("💀 SCORING REJECTED: {} insufficient liquidity", token.mint));
            return 0.0;
        }

        if let Some(holders) = &token.holders {
            if let Some(unique) = holders.unique_holders {
                 if unique < UNIQUE_HOLDERS_CRITICAL_LOW {
                    warn!("💀 SCORING: {} has only {} holders (ghost token)", token.mint, unique);
                    TradeLogger::log(&format!("💀 SCORING REJECTED: {} ghost token", token.mint));
                    return 0.0;
                }
            }
        }

        // --- NEW SURVIVAL GATES ---
        
        // Gate A: Liquidity-Aware Volatility Filter
        if liquidity_usd < LOW_LIQUIDITY_LIMIT_USD {
            if let Some(pct_5m) = token.price_change_5m_pct {
                if pct_5m > LOW_LIQUIDITY_VOLATILITY_MAX_5M {
                    warn!("💀 SCORING: {} REJECTED - Extreme volatility in low liquidity (+{:.1}% < $10k liq)", token.mint, pct_5m);
                    TradeLogger::log(&format!("💀 SCORING REJECTED: {} extreme vol/low liq", token.mint));
                    return 0.0;
                }
            }
        }

        // =====================================================================
        // 2. MOMENTUM SCORING (5m Window - PRIMARY)
        // =====================================================================
        // We prefer 5m over 30m for "fresh" pumps
        let momentum_score = if let Some(pct_5m) = token.price_change_5m_pct {
            let mut points = 0.0;
            if pct_5m >= PRICE_CHANGE_5M_EXCELLENT {
                points = SCORE_MOMENTUM_MAX;
                info!("    🚀 5m ROCKET: +{:.1}% -> {:.1} points", pct_5m, points);
            } else if pct_5m >= PRICE_CHANGE_5M_GOOD {
                points = (pct_5m / PRICE_CHANGE_5M_EXCELLENT) * SCORE_MOMENTUM_MAX;
                info!("    📈 5m CLIMB: +{:.1}% -> {:.1} points", pct_5m, points);
            } else if pct_5m > 0.0 {
                points = (pct_5m / PRICE_CHANGE_5M_GOOD) * 10.0;
            } else {
                points = -10.0; // Dumping in last 5m
            }
            points
        } else if let Some(pct_30m) = token.price_change_30m_pct {
            // Fallback to 30m if 5m missing (less weight)
            warn!("    ⚠️ 5m data missing, using 30m fallback");
            if pct_30m > 10.0 { 20.0 } else { 0.0 }
        } else {
            0.0
        };
        score += momentum_score;
        breakdown.momentum = momentum_score;

        // =====================================================================
        // 3. ACTIVITY SCORING (5m Window)
        // =====================================================================
        let mut activity_score = 0.0;
        if let Some(wallets_5m) = token.unique_wallets_5m {
            if wallets_5m >= UNIQUE_WALLETS_5M_HIGH {
                activity_score += 30.0;
                info!("    👥 5m VIRAL: {} new wallets -> +30 points", wallets_5m);
            } else if wallets_5m >= UNIQUE_WALLETS_5M_MED {
                activity_score += 15.0;
            }
        }
        
        // Volume check (5m)
        if let Some(vol_5m) = token.buy_volume_5m_usd {
             if let Some(sell_vol_5m) = token.sell_volume_5m_usd {
                 let total = vol_5m + sell_vol_5m;
                 if total > 5000.0 { activity_score += 10.0; } // >$5k in 5m is busy
             }
        }
        
        // Fallback to 30m wallets if 5m missing
        if token.unique_wallets_5m.is_none() {
            if let Some(wallets_30m) = token.unique_wallets_30m {
                 if wallets_30m > 50 { activity_score += 10.0; }
            }
        }
        
        score += activity_score;
        breakdown.activity = activity_score;

        // =====================================================================
        // 4. IGNITION BONUS (1m Window)
        // =====================================================================
        if let Some(pct_1m) = token.price_change_1m_pct {
            if pct_1m >= PRICE_CHANGE_1M_IGNITION {
                score += SCORE_BONUS_IGNITION;
                info!("    💥 1m IGNITION: +{:.1}% -> +{:.1} points", pct_1m, SCORE_BONUS_IGNITION);
            }
        }

        // =====================================================================
        // 5. BUY PRESSURE (5m)
        // =====================================================================
        let buy_pressure_score = if let (Some(buy_5m), Some(sell_5m)) = (token.buy_volume_5m_usd, token.sell_volume_5m_usd) {
            if sell_5m > 0.0 {
                let ratio = buy_5m / sell_5m;
                if ratio >= BUY_SELL_RATIO_EXCELLENT { 30.0 } 
                else if ratio >= BUY_SELL_RATIO_GOOD { 15.0 }
                else if ratio < 0.8 { -10.0 }
                else { 0.0 }
            } else { 30.0 }
        } else { 0.0 };
        score += buy_pressure_score;
        breakdown.buy_pressure = buy_pressure_score;

        // =====================================================================
        // 6. DISTRIBUTION (Safety)
        // =====================================================================
        let mut distribution_score = 0.0;
        if let Some(holders) = &token.holders {
             if holders.top_10_pct > HOLDER_TOP_10_PENALTY_THRESHOLD { distribution_score -= SCORE_PENALTY_TOP_10; }
             if holders.top_1_pct > HOLDER_TOP_1_PENALTY_THRESHOLD { distribution_score -= SCORE_PENALTY_WHALE; }
             if holders.top_10_pct < HOLDER_TOP_10_BONUS_THRESHOLD { distribution_score += SCORE_BONUS_DISTRIBUTION; }
        }
        score += distribution_score;
        breakdown.distribution = distribution_score;

        // =====================================================================
        // 7. SOCIALS
        // =====================================================================
        let mut social_score = 0.0;
        if let Some(meta) = &token.metadata {
            if let Some(socials) = &meta.socials {
                if socials.telegram.is_some() || socials.twitter.is_some() { social_score += 10.0; }
            }
        }
        score += social_score;
        breakdown.socials = social_score;

        // =====================================================================
        // 8. FINAL SURVIVAL PENALTIES (MOMENTUM / CHOP / ABSORPTION)
        // =====================================================================
        
        // Penalty 1: Momentum Health Gate (Late Entry Protection)
        if let (Some(p5m), Some(p1m)) = (token.price_change_5m_pct, token.price_change_1m_pct) {
            if p5m >= MOMENTUM_DECAY_THRESHOLD_5M && p1m <= 0.0 {
                warn!("⚠️ SCORING: {} Penalty - Momentum Health Gate (Momentum decayed: 5m={:.1}%, 1m={:.1}%)", token.mint, p5m, p1m);
                score -= PENALTY_SURVIVAL_GATE;
            }
        }

        // Penalty 2: Chop Detector (Displacement Ratio)
        if let (Some(p5m), Some(b5m), Some(s5m)) = (token.price_change_5m_pct, token.buy_volume_5m_usd, token.sell_volume_5m_usd) {
            let total_vol = b5m + s5m;
            let displacement_ratio = p5m.abs() / (total_vol / 1000.0).max(1.0);
            if displacement_ratio < DISPLACEMENT_RATIO_MIN {
                warn!("⚠️ SCORING: {} Penalty - Chop Detector (Low displacement: {:.2} ratio for ${:.0} vol)", token.mint, displacement_ratio, total_vol);
                score -= PENALTY_SURVIVAL_GATE;
            }
        }

        // Penalty 3: Buy Pressure Sanity (Absorption Check)
        if let (Some(b5m), Some(s5m), Some(p1m)) = (token.buy_volume_5m_usd, token.sell_volume_5m_usd, token.price_change_1m_pct) {
            if s5m > 0.0 {
                let ratio = b5m / s5m;
                if ratio > BUY_PRESSURE_ABSORPTION_RATIO && p1m <= 0.0 {
                    warn!("⚠️ SCORING: {} Penalty - Buy Pressure Sanity (Buys absorbed: Ratio={:.2}, 1m={:.1}%)", token.mint, ratio, p1m);
                    score -= (PENALTY_SURVIVAL_GATE / 2.0); // Moderate penalty
                }
            }
        }

        // Bounds
        if score > 150.0 { score = 150.0; }
        if score < 0.0 { score = 0.0; }

        TradeLogger::log_scoring_breakdown(
            &token.mint, breakdown.momentum, breakdown.activity + breakdown.buy_pressure, score, score // Passing score twice as placeholders
        );
        
        info!("📊 SCORING: {} -> {:.1}/150 (5m: {:.1}%, 1m: {:.1}%)", 
            token.mint, score, 
            token.price_change_5m_pct.unwrap_or(0.0),
            token.price_change_1m_pct.unwrap_or(0.0)
        );
        score
    }
}

#[derive(Debug, Default)]
struct ScoringBreakdown {
    momentum: f64,
    activity: f64,
    buy_pressure: f64,
    distribution: f64,
    socials: f64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::helius_listener::TokenPlatform;
    use crate::enrichment::HolderAnalysis;

    fn create_test_token(
        liquidity_sol: Option<f64>,
        has_freeze: bool,
        has_mint: bool,
    ) -> EnrichedToken {
        EnrichedToken {
            mint: "TestMint123".to_string(),
            signature: "TestSig123".to_string(),
            slot: 12345,
            timestamp: Some(1234567890),
            decimals: 9,
            supply: Some(1_000_000_000),
            has_liquidity: true,
            pool_address: "PoolAddr123".to_string(),
            pair_token: "So11111111111111111111111111111111111111112".to_string(),
            dex: "Raydium".to_string(),
            price_sol: None,
            price_usd: None,
            liquidity_usd: liquidity_sol.map(|s| s * 150.0), // Mock USD
            initial_liquidity_sol: liquidity_sol,
            fdv: None,
            market_cap: None,
            platform: TokenPlatform::PumpFun,
            has_freeze_authority: has_freeze,
            has_mint_authority: has_mint,
            enrichment_timestamp: 1234567890,
            enrichment_duration_ms: 100,
            metadata: None,
            holders: Some(HolderAnalysis {
                top_1_pct: 10.0,
                top_10_pct: 25.0,
                unique_holders: Some(50),
            }),
            unique_wallets_30m: None,
            buy_volume_30m_usd: None,
            sell_volume_30m_usd: None,
            price_change_30m_pct: None,
            
            // New fields set to None by default
            unique_wallets_5m: None,
            buy_volume_5m_usd: None,
            sell_volume_5m_usd: None,
            price_change_5m_pct: None,
            unique_wallets_1m: None,
            price_change_1m_pct: None,
        }
    }

    #[test]
    fn test_freeze_authority_returns_zero() {
        let scorer = TokenScorer::new();
        let token = create_test_token(Some(20.0), true, false); // 20 SOL is fine
        
        let score = scorer.score(&token);
        assert_eq!(score, 0.0);
    }

    #[test]
    fn test_low_liquidity_instant_fail() {
        let scorer = TokenScorer::new();
        // 10 SOL * $150 = $1500 USD < $2500 threshold
        let token = create_test_token(Some(10.0), false, false); 
        
        let score = scorer.score(&token);
        assert_eq!(score, 0.0);
    }

    #[test]
    fn test_pump_metrics() {
        let scorer = TokenScorer::new();
        let mut token = create_test_token(Some(50.0), false, false);
        // Set 5m metrics
        token.price_change_5m_pct = Some(20.0); // +20% -> Max Momentum (50)
        token.unique_wallets_5m = Some(40); // >30 -> Max Activity (30)
        token.buy_volume_5m_usd = Some(10000.0);
        token.sell_volume_5m_usd = Some(2000.0); // Ratio 5 -> Max BP (30)
        
        let score = scorer.score(&token);
        // 50 + 30 + 10(vol) + 30(BP) + 15(dist) = 135
        assert!(score >= 100.0, "Score was {}", score);
    }
}
