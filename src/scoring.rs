use crate::enrichment::EnrichedToken;
use log::{info, warn};
use crate::trade_logger::TradeLogger;

/// Scorer for enriched tokens - GROWTH TREND FOCUSED
/// GRADUATED TOKENS ONLY (Pump.fun/Bonk.fun)
/// 
/// NEW SCORING PHILOSOPHY:
/// - Liquidity is a MINIMUM THRESHOLD, not the main score
/// - Focus on MOMENTUM: 30m price change, volume, unique wallets
/// - Identify "pumping" tokens with strong buy pressure
/// 
/// MAX SCORE: 150 points
/// - Momentum: 50 points (price change 30m)
/// - Activity: 40 points (unique wallets 30m, volume)
/// - Buy Pressure: 30 points (buy/sell ratio)
/// - Distribution: 20 points (holder concentration)
/// - Socials: 10 points (community presence)
/// 
/// PENALTIES:
/// - Authorities (freeze/mint): Instant fail
/// - Low liquidity (<20 SOL): Instant fail
/// - Extreme whale concentration: -50 points

// Liquidity thresholds (minimum requirements)
// Liquidity thresholds (minimum requirements in USD)
const LIQUIDITY_MIN_THRESHOLD_USD: f64 = 2500.0; // ~$20 SOL at $125
const LIQUIDITY_HEALTHY_THRESHOLD_USD: f64 = 5000.0; // ~$40 SOL

// Momentum scoring (30m window)
const PRICE_CHANGE_30M_EXCELLENT: f64 = 50.0; // +50% in 30m
const PRICE_CHANGE_30M_GOOD: f64 = 20.0; // +20% in 30m
const PRICE_CHANGE_30M_NEGATIVE_PENALTY: f64 = -10.0; // Negative momentum
const SCORE_MOMENTUM_MAX: f64 = 50.0;

// Activity scoring (30m window)
const UNIQUE_WALLETS_30M_HIGH: u64 = 100; // Very active
const UNIQUE_WALLETS_30M_MED: u64 = 30; // Moderate activity
const UNIQUE_WALLETS_30M_LOW: u64 = 10; // Low activity
const SCORE_ACTIVITY_MAX: f64 = 40.0;

// Buy pressure scoring
const BUY_SELL_RATIO_EXCELLENT: f64 = 2.0; // 2x more buying than selling
const BUY_SELL_RATIO_GOOD: f64 = 1.2; // 20% more buying
const BUY_SELL_RATIO_POOR: f64 = 0.8; // More selling than buying
const SCORE_BUY_PRESSURE_MAX: f64 = 30.0;

// Holder distribution
const HOLDER_TOP_1_PENALTY_THRESHOLD: f64 = 30.0;
const HOLDER_TOP_10_PENALTY_THRESHOLD: f64 = 60.0;
const HOLDER_TOP_10_BONUS_THRESHOLD: f64 = 30.0;
const SCORE_PENALTY_WHALE: f64 = 50.0;
const SCORE_PENALTY_TOP_10: f64 = 20.0;
const SCORE_BONUS_DISTRIBUTION: f64 = 20.0;

// Socials
const SCORE_SOCIAL_MAX: f64 = 10.0;

// Total holder count
const UNIQUE_HOLDERS_CRITICAL_LOW: u64 = 10;
const SCORE_PENALTY_GHOST_TOWN: f64 = 150.0; // Instant fail

#[derive(Debug, Clone)]
pub struct TokenScorer;

impl TokenScorer {
    pub fn new() -> Self {
        Self
    }

    /// Calculate score for a token (0-150)
    /// GRADUATED TOKENS ONLY - Growth trend focused
    /// Higher is better. 0 means DO NOT BUY.
    pub fn score(&self, token: &EnrichedToken) -> f64 {
        let mut score = 0.0;
        let mut breakdown = ScoringBreakdown::default();

        // =====================================================================
        // 1. SAFETY CHECKS (CRITICAL) - INSTANT FAIL
        // =====================================================================
        
        // FREEZE AUTHORITY: Instant Fail
        if token.has_freeze_authority {
            warn!("💀 SCORING: {} has freeze authority -> SCORE 0", token.mint);
            TradeLogger::log(&format!("💀 SCORING REJECTED: {} has freeze authority", token.mint));
            return 0.0;
        }

        // MINT AUTHORITY: Instant Fail
        if token.has_mint_authority {
            warn!("💀 SCORING: {} has mint authority -> SCORE 0", token.mint);
            TradeLogger::log(&format!("💀 SCORING REJECTED: {} has mint authority", token.mint));
            return 0.0;
        }

        // LIQUIDITY MINIMUM: Instant Fail if too low
        let liquidity_usd = token.liquidity_usd.unwrap_or(0.0);
        if liquidity_usd < LIQUIDITY_MIN_THRESHOLD_USD {
            warn!("💀 SCORING: {} has insufficient liquidity (${:.0} < ${})", 
                token.mint, liquidity_usd, LIQUIDITY_MIN_THRESHOLD_USD);
            TradeLogger::log(&format!("💀 SCORING REJECTED: {} insufficient liquidity", token.mint));
            return 0.0;
        }

        // GHOST TOKEN: Instant Fail if too few holders
        if let Some(holders) = &token.holders {
            if let Some(unique) = holders.unique_holders {
                if unique < UNIQUE_HOLDERS_CRITICAL_LOW {
                    warn!("💀 SCORING: {} has only {} holders (ghost token)", token.mint, unique);
                    TradeLogger::log(&format!("💀 SCORING REJECTED: {} ghost token", token.mint));
                    return 0.0;
                }
            }
        }

        // =====================================================================
        // 2. MOMENTUM SCORING (Max 50 points) - PRIMARY DRIVER
        // =====================================================================
        let momentum_score = if let Some(price_change_30m) = token.price_change_30m_pct {
            let mut points = 0.0;
            
            if price_change_30m >= PRICE_CHANGE_30M_EXCELLENT {
                // Explosive growth
                points = SCORE_MOMENTUM_MAX;
                info!("    🚀 EXPLOSIVE MOMENTUM: +{:.1}% in 30m -> {:.1} points", price_change_30m, points);
            } else if price_change_30m >= PRICE_CHANGE_30M_GOOD {
                // Strong growth - scale linearly
                points = (price_change_30m / PRICE_CHANGE_30M_EXCELLENT) * SCORE_MOMENTUM_MAX;
                info!("    📈 STRONG MOMENTUM: +{:.1}% in 30m -> {:.1} points", price_change_30m, points);
            } else if price_change_30m > 0.0 {
                // Positive but weak
                points = (price_change_30m / PRICE_CHANGE_30M_GOOD) * 10.0;
                info!("    ➡️ WEAK MOMENTUM: +{:.1}% in 30m -> {:.1} points", price_change_30m, points);
            } else {
                // Negative momentum - penalty
                points = PRICE_CHANGE_30M_NEGATIVE_PENALTY;
                warn!("    📉 NEGATIVE MOMENTUM: {:.1}% in 30m -> {:.1} points", price_change_30m, points);
            }
            
            points
        } else {
            // No momentum data - neutral
            warn!("    ⚠️ NO MOMENTUM DATA (Birdeye unavailable)");
            0.0
        };
        
        score += momentum_score;
        breakdown.momentum = momentum_score;

        // =====================================================================
        // 3. ACTIVITY SCORING (Max 40 points) - REAL USER ENGAGEMENT
        // =====================================================================
        let mut activity_score = 0.0;
        
        // Unique wallets in 30m (primary activity metric)
        if let Some(unique_wallets_30m) = token.unique_wallets_30m {
            let wallet_points = if unique_wallets_30m >= UNIQUE_WALLETS_30M_HIGH {
                30.0 // Very active
            } else if unique_wallets_30m >= UNIQUE_WALLETS_30M_MED {
                // Scale between 15-30
                15.0 + ((unique_wallets_30m - UNIQUE_WALLETS_30M_MED) as f64 / 
                        (UNIQUE_WALLETS_30M_HIGH - UNIQUE_WALLETS_30M_MED) as f64) * 15.0
            } else if unique_wallets_30m >= UNIQUE_WALLETS_30M_LOW {
                // Scale between 5-15
                5.0 + ((unique_wallets_30m - UNIQUE_WALLETS_30M_LOW) as f64 / 
                       (UNIQUE_WALLETS_30M_MED - UNIQUE_WALLETS_30M_LOW) as f64) * 10.0
            } else {
                0.0 // Very low activity
            };
            
            activity_score += wallet_points;
            info!("    👥 ACTIVITY: {} unique wallets (30m) -> {:.1} points", unique_wallets_30m, wallet_points);
        }
        
        // Volume bonus (if available)
        if let Some(buy_vol) = token.buy_volume_30m_usd {
            if let Some(sell_vol) = token.sell_volume_30m_usd {
                let total_vol = buy_vol + sell_vol;
                if total_vol > 10_000.0 {
                    activity_score += 10.0;
                    info!("    💰 HIGH VOLUME: ${:.0} (30m) -> +10 points", total_vol);
                } else if total_vol > 5_000.0 {
                    activity_score += 5.0;
                    info!("    💵 MODERATE VOLUME: ${:.0} (30m) -> +5 points", total_vol);
                }
            }
        }
        
        score += activity_score;
        breakdown.activity = activity_score;

        // =====================================================================
        // 4. BUY PRESSURE SCORING (Max 30 points) - DEMAND SIGNAL
        // =====================================================================
        let buy_pressure_score = if let (Some(buy_vol), Some(sell_vol)) = 
            (token.buy_volume_30m_usd, token.sell_volume_30m_usd) {
            
            if sell_vol > 0.0 {
                let buy_sell_ratio = buy_vol / sell_vol;
                let points = if buy_sell_ratio >= BUY_SELL_RATIO_EXCELLENT {
                    SCORE_BUY_PRESSURE_MAX // Strong buying pressure
                } else if buy_sell_ratio >= BUY_SELL_RATIO_GOOD {
                    // Scale between 15-30
                    15.0 + ((buy_sell_ratio - BUY_SELL_RATIO_GOOD) / 
                            (BUY_SELL_RATIO_EXCELLENT - BUY_SELL_RATIO_GOOD)) * 15.0
                } else if buy_sell_ratio >= 1.0 {
                    // Slightly more buying than selling
                    10.0
                } else if buy_sell_ratio >= BUY_SELL_RATIO_POOR {
                    // Neutral to slightly negative
                    5.0
                } else {
                    // Heavy selling pressure
                    -10.0
                };
                
                info!("    📊 BUY/SELL RATIO: {:.2} (${:.0}/${:.0}) -> {:.1} points", 
                    buy_sell_ratio, buy_vol, sell_vol, points);
                points
            } else {
                // Only buying, no selling - very bullish
                info!("    🔥 PURE BUYING PRESSURE (no sells) -> {:.1} points", SCORE_BUY_PRESSURE_MAX);
                SCORE_BUY_PRESSURE_MAX
            }
        } else {
            warn!("    ⚠️ NO VOLUME DATA (Birdeye unavailable)");
            0.0
        };
        
        score += buy_pressure_score;
        breakdown.buy_pressure = buy_pressure_score;

        // =====================================================================
        // 5. DISTRIBUTION SCORING (Max 20 points, Penalties)
        // =====================================================================
        let mut distribution_score = 0.0;
        if let Some(holders) = &token.holders {
            // Penalty: Whale dominance (Top 1 > 30%)
            if holders.top_1_pct > HOLDER_TOP_1_PENALTY_THRESHOLD {
                distribution_score -= SCORE_PENALTY_WHALE;
                warn!("    🐋 WHALE ALERT: Top 1 holder owns {:.1}% -> -{:.1} points", 
                    holders.top_1_pct, SCORE_PENALTY_WHALE);
            }
            
            // Penalty: Top 10 concentration (> 60%)
            if holders.top_10_pct > HOLDER_TOP_10_PENALTY_THRESHOLD {
                distribution_score -= SCORE_PENALTY_TOP_10;
                warn!("    ⚠️ CONCENTRATED: Top 10 own {:.1}% -> -{:.1} points", 
                    holders.top_10_pct, SCORE_PENALTY_TOP_10);
            }
            
            // Bonus: Good distribution (Top 10 < 30%)
            if holders.top_10_pct < HOLDER_TOP_10_BONUS_THRESHOLD {
                distribution_score += SCORE_BONUS_DISTRIBUTION;
                info!("    ✅ WELL DISTRIBUTED: Top 10 own only {:.1}% -> +{:.1} points", 
                    holders.top_10_pct, SCORE_BONUS_DISTRIBUTION);
            }
        }
        
        score += distribution_score;
        breakdown.distribution = distribution_score;

        // =====================================================================
        // 6. SOCIALS SCORING (Max 10 points) - COMMUNITY PRESENCE
        // =====================================================================
        let mut social_score = 0.0;
        if let Some(metadata) = &token.metadata {
            if let Some(socials) = &metadata.socials {
                if socials.twitter.is_some() { social_score += 5.0; }
                if socials.telegram.is_some() { social_score += 3.0; }
                if socials.website.is_some() { social_score += 2.0; }
                
                if social_score > 0.0 {
                    info!("    🌐 SOCIALS: {:.1} points", social_score);
                }
            }
        }
        
        score += social_score;
        breakdown.socials = social_score;

        // =====================================================================
        // 7. LIQUIDITY BONUS (if exceptionally high)
        // =====================================================================
        if liquidity_usd >= LIQUIDITY_HEALTHY_THRESHOLD_USD {
            info!("    💧 HEALTHY LIQUIDITY: ${:.0} (threshold met)", liquidity_usd);
        }

        // Final bounds
        if score < 0.0 {
            score = 0.0;
        }
        if score > 150.0 {
            score = 150.0;
        }

        info!("📊 SCORING: {} -> {:.1}/150 (Liq: ${:.0}, Momentum: {:.1}%, Wallets: {})", 
            token.mint, 
            score, 
            liquidity_usd,
            token.price_change_30m_pct.unwrap_or(0.0),
            token.unique_wallets_30m.unwrap_or(0)
        );

        TradeLogger::log_scoring_breakdown(
            &token.mint,
            breakdown.momentum,
            breakdown.activity + breakdown.buy_pressure,
            breakdown.distribution + breakdown.socials,
            score
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
        }
    }

    #[test]
    fn test_freeze_authority_returns_zero() {
        let scorer = TokenScorer::new();
        let token = create_test_token(Some(70.0), true, false);
        
        let score = scorer.score(&token);
        assert_eq!(score, 0.0);
    }

    #[test]
    fn test_low_liquidity_instant_fail() {
        let scorer = TokenScorer::new();
        let token = create_test_token(Some(15.0), false, false); // < 20 SOL
        
        let score = scorer.score(&token);
        assert_eq!(score, 0.0);
    }

    #[test]
    fn test_explosive_momentum() {
        let scorer = TokenScorer::new();
        let mut token = create_test_token(Some(50.0), false, false);
        token.price_change_30m_pct = Some(60.0); // +60% in 30m
        token.unique_wallets_30m = Some(120); // High activity
        token.buy_volume_30m_usd = Some(15_000.0);
        token.sell_volume_30m_usd = Some(5_000.0); // 3:1 buy/sell ratio
        
        let score = scorer.score(&token);
        // Should be high: 50 (momentum) + 30 (activity) + 10 (volume) + 30 (buy pressure) + 20 (distribution) = 140
        assert!(score >= 130.0, "Score was {}", score);
    }

    #[test]
    fn test_negative_momentum_penalty() {
        let scorer = TokenScorer::new();
        let mut token = create_test_token(Some(50.0), false, false);
        token.price_change_30m_pct = Some(-15.0); // Dumping
        
        let score = scorer.score(&token);
        // Should have negative momentum penalty
        assert!(score < 30.0, "Score was {}", score);
    }
}
