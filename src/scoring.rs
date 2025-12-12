use crate::enrichment::EnrichedToken;
use log::{info, warn};
use crate::trade_logger::TradeLogger;

/// Scorer for enriched tokens
/// GRADUATED TOKENS ONLY (Pump.fun/Bonk.fun)
const LIQUIDITY_HIGH_THRESHOLD: f64 = 60.0;
const LIQUIDITY_MED_THRESHOLD: f64 = 30.0;
const SCORE_LIQUIDITY_HIGH: f64 = 70.0;
const SCORE_LIQUIDITY_MED: f64 = 40.0;

const HOLDER_TOP_1_PENALTY_THRESHOLD: f64 = 30.0;
const HOLDER_TOP_10_PENALTY_THRESHOLD: f64 = 70.0;
const HOLDER_TOP_1_BONUS_THRESHOLD: f64 = 10.0;
const SCORE_PENALTY_TOP_1: f64 = 50.0;
const SCORE_PENALTY_TOP_10: f64 = 20.0;
const SCORE_BONUS_TOP_1: f64 = 10.0;

const UNIQUE_HOLDERS_HIGH: u64 = 100;
const UNIQUE_HOLDERS_LOW: u64 = 20;
const SCORE_BONUS_UNIQUE_HIGH: f64 = 10.0;
const SCORE_PENALTY_UNIQUE_LOW: f64 = 20.0;

const MIN_MARKET_CAP: f64 = 50_000.0;
const SCORE_PENALTY_LOW_MC: f64 = 10.0;

#[derive(Debug, Clone)]
pub struct TokenScorer;

impl TokenScorer {
    pub fn new() -> Self {
        Self
    }

    /// Calculate score for a token (0-70)
    /// GRADUATED TOKENS ONLY - No tax checks needed
    /// Higher is better. 0 means DO NOT BUY.
    pub fn score(&self, token: &EnrichedToken) -> f64 {
        let mut score = 0.0;

        // =====================================================================
        // 1. SAFETY CHECKS (CRITICAL) - FAIL FAST
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

        // =====================================================================
        // 2. LIQUIDITY SCORING (Max 70 points)
        // =====================================================================
        // Pump.fun tokens usually graduate with ~70-80 SOL.
        // We want to ensure it's a "healthy" graduation.
        if let Some(liquidity_sol) = token.initial_liquidity_sol {
            if liquidity_sol >= LIQUIDITY_HIGH_THRESHOLD {
                score += SCORE_LIQUIDITY_HIGH; // Standard/Good Graduation (~$10k+)
            } else if liquidity_sol >= LIQUIDITY_MED_THRESHOLD {
                score += SCORE_LIQUIDITY_MED; // Low but acceptable
            } else {
                // < 30 SOL is suspicious for a graduated token
                // It might mean liquidity was pulled or it's a weird migration
                score += 0.0; 
                warn!("    - LOW LIQUIDITY: {:.1} SOL (Expected > {})", liquidity_sol, LIQUIDITY_MED_THRESHOLD);
            }
        }

        // GRADUATED TOKENS ONLY - No tax checks
        // Graduated tokens are pre-verified safe by bonding curve

        // Cap at 70 (liquidity only)
        if score > 70.0 {
            score = 70.0;
        }
        let liquidity_score = score;

        // =====================================================================
        // 3. METADATA SCORING (Max 20 points)
        // =====================================================================
        let mut social_score = 0.0;
        if let Some(metadata) = &token.metadata {
            if let Some(socials) = &metadata.socials {
                if socials.twitter.is_some() { social_score += 10.0; }
                if socials.telegram.is_some() { social_score += 5.0; }
                if socials.website.is_some() { social_score += 5.0; }
                
                score += social_score;
                info!("    + Socials: {:.1} points", social_score);
            }
        }

        // =====================================================================
        // 4. HOLDER SCORING (Max 10 points + Penalties)
        // =====================================================================
        let mut holder_score = 0.0;
        if let Some(holders) = &token.holders {
            // Penalty: Top 1 holder > 30% (excluding pool)
            if holders.top_1_pct > HOLDER_TOP_1_PENALTY_THRESHOLD {
                holder_score -= SCORE_PENALTY_TOP_1; // Huge penalty for whale dominance
                warn!("    - PENALTY: Top 1 holder owns {:.1}%", holders.top_1_pct);
            }
            
            // Penalty: Top 10 holders > 70%
            if holders.top_10_pct > HOLDER_TOP_10_PENALTY_THRESHOLD {
                holder_score -= SCORE_PENALTY_TOP_10;
                warn!("    - PENALTY: Top 10 holders own {:.1}%", holders.top_10_pct);
            }

            // Bonus: Good distribution (Top 1 < 10%)
            if holders.top_1_pct < HOLDER_TOP_1_BONUS_THRESHOLD {
                holder_score += SCORE_BONUS_TOP_1;
            }

            // NEW: Unique Holder Count
            if let Some(unique) = holders.unique_holders {
                if unique > UNIQUE_HOLDERS_HIGH {
                    holder_score += SCORE_BONUS_UNIQUE_HIGH; // Healthy community
                    info!("    + Community: {} unique holders", unique);
                } else if unique < UNIQUE_HOLDERS_LOW {
                    holder_score -= SCORE_PENALTY_UNIQUE_LOW; // Ghost town / Dev wallet farm
                    warn!("    - PENALTY: Only {} unique holders", unique);
                }
            }
            score += holder_score;
        }

        // =====================================================================
        // 5. MARKET CAP CHECK (Penalty only)
        // =====================================================================
        // If MC is super low (<$50k), it means price dumped below graduation.
        // If MC is super low (<$50k), it means price dumped below graduation.
        if let Some(mc) = token.market_cap {
            if mc < MIN_MARKET_CAP {
                score -= SCORE_PENALTY_LOW_MC;
                warn!("    - PENALTY: Low Market Cap (${:.0})", mc);
            }
        }

        // Final Cap at 100
        if score > 100.0 {
            score = 100.0;
        }
        
        // Minimum score 0
        if score < 0.0 {
            score = 0.0;
        }

        info!("📊 SCORING: {} -> {:.1}/100 (Liq: {:.1} SOL, Platform: Graduated)", 
            token.mint, 
            score, 
            token.initial_liquidity_sol.unwrap_or(0.0)
        );

        TradeLogger::log_scoring_breakdown(
            &token.mint,
            liquidity_score,
            holder_score,
            social_score,
            score
        );

        score
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::helius_listener::TokenPlatform;

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
            holders: None,
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
    fn test_mint_authority_returns_zero() {
        let scorer = TokenScorer::new();
        let token = create_test_token(Some(70.0), false, true);
        
        let score = scorer.score(&token);
        assert_eq!(score, 0.0);
    }

    #[test]
    fn test_excellent_liquidity() {
        let scorer = TokenScorer::new();
        let token = create_test_token(Some(65.0), false, false); // > 60
        
        let score = scorer.score(&token);
        assert_eq!(score, 70.0);
    }

    #[test]
    fn test_good_liquidity() {
        let scorer = TokenScorer::new();
        let token = create_test_token(Some(40.0), false, false); // > 30
        
        let score = scorer.score(&token);
        assert_eq!(score, 40.0);
    }

    #[test]
    fn test_low_liquidity_penalty() {
        let scorer = TokenScorer::new();
        let token = create_test_token(Some(20.0), false, false); // < 30
        
        let score = scorer.score(&token);
        assert_eq!(score, 0.0); // Should be 0
    }

    #[test]
    fn test_unique_holder_bonus() {
        let scorer = TokenScorer::new();
        let mut token = create_test_token(Some(70.0), false, false);
        token.holders = Some(crate::enrichment::HolderAnalysis {
            top_1_pct: 5.0,
            top_10_pct: 20.0,
            unique_holders: Some(150), // > 100 bonus
        });
        
        let score = scorer.score(&token);
        // 70 (liq) + 10 (holders < 10%) + 10 (unique > 100) = 90
        assert_eq!(score, 90.0);
    }

    #[test]
    fn test_unique_holder_penalty() {
        let scorer = TokenScorer::new();
        let mut token = create_test_token(Some(70.0), false, false);
        token.holders = Some(crate::enrichment::HolderAnalysis {
            top_1_pct: 5.0,
            top_10_pct: 20.0,
            unique_holders: Some(10), // < 20 penalty
        });
        
        let score = scorer.score(&token);
        // 70 (liq) + 10 (holders < 10%) - 20 (unique < 20) = 60
        assert_eq!(score, 60.0);
    }

    #[test]
    fn test_market_cap_penalty() {
        let scorer = TokenScorer::new();
        let mut token = create_test_token(Some(70.0), false, false);
        token.market_cap = Some(40_000.0); // < 50k penalty
        
        let score = scorer.score(&token);
        // 70 (liq) - 10 (low MC) = 60
        assert_eq!(score, 60.0);
    }
}
