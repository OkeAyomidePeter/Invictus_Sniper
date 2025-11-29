use crate::enrichment::EnrichedToken;
use log::{info, warn};

/// Scorer for enriched tokens
/// GRADUATED TOKENS ONLY (Pump.fun/Bonk.fun)
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
            return 0.0;
        }

        // MINT AUTHORITY: Instant Fail
        if token.has_mint_authority {
            warn!("💀 SCORING: {} has mint authority -> SCORE 0", token.mint);
            return 0.0;
        }

        // =====================================================================
        // 2. LIQUIDITY SCORING (Max 70 points)
        // =====================================================================
        if let Some(liquidity_sol) = token.initial_liquidity_sol {
            if liquidity_sol >= 50.0 {
                score += 70.0; // Excellent liquidity
            } else if liquidity_sol >= 20.0 {
                score += 50.0; // Good liquidity
            } else if liquidity_sol >= 10.0 {
                score += 30.0; // Decent liquidity
            } else if liquidity_sol >= 5.0 {
                score += 15.0; // Minimal liquidity
            } else if liquidity_sol >= 1.0 {
                score += 5.0;  // Very low liquidity
            }
            // < 1 SOL gets 0 points for liquidity
        }

        // GRADUATED TOKENS ONLY - No tax checks
        // Graduated tokens are pre-verified safe by bonding curve

        // Cap at 70 (liquidity only)
        if score > 70.0 {
            score = 70.0;
        }

        // =====================================================================
        // 3. METADATA SCORING (Max 20 points)
        // =====================================================================
        if let Some(metadata) = &token.metadata {
            if let Some(socials) = &metadata.socials {
                let mut social_score = 0.0;
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
        if let Some(holders) = &token.holders {
            // Penalty: Top 1 holder > 30% (excluding pool)
            if holders.top_1_pct > 30.0 {
                score -= 50.0; // Huge penalty for whale dominance
                warn!("    - PENALTY: Top 1 holder owns {:.1}%", holders.top_1_pct);
            }
            
            // Penalty: Top 10 holders > 70%
            if holders.top_10_pct > 70.0 {
                score -= 20.0;
                warn!("    - PENALTY: Top 10 holders own {:.1}%", holders.top_10_pct);
            }

            // Bonus: Good distribution (Top 1 < 10%)
            if holders.top_1_pct < 10.0 {
                score += 10.0;
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
        let token = create_test_token(Some(50.0), true, false);
        
        let score = scorer.score(&token);
        assert_eq!(score, 0.0);
    }

    #[test]
    fn test_mint_authority_returns_zero() {
        let scorer = TokenScorer::new();
        let token = create_test_token(Some(50.0), false, true);
        
        let score = scorer.score(&token);
        assert_eq!(score, 0.0);
    }

    #[test]
    fn test_excellent_liquidity() {
        let scorer = TokenScorer::new();
        let token = create_test_token(Some(50.0), false, false);
        
        let score = scorer.score(&token);
        assert_eq!(score, 70.0);
    }

    #[test]
    fn test_good_liquidity() {
        let scorer = TokenScorer::new();
        let token = create_test_token(Some(25.0), false, false);
        
        let score = scorer.score(&token);
        assert_eq!(score, 50.0);
    }

    #[test]
    fn test_decent_liquidity() {
        let scorer = TokenScorer::new();
        let token = create_test_token(Some(15.0), false, false);
        
        let score = scorer.score(&token);
        assert_eq!(score, 30.0);
    }

    #[test]
    fn test_minimal_liquidity() {
        let scorer = TokenScorer::new();
        let token = create_test_token(Some(7.0), false, false);
        
        let score = scorer.score(&token);
        assert_eq!(score, 15.0);
    }

    #[test]
    fn test_very_low_liquidity() {
        let scorer = TokenScorer::new();
        let token = create_test_token(Some(2.0), false, false);
        
        let score = scorer.score(&token);
        assert_eq!(score, 5.0);
    }

    #[test]
    fn test_no_liquidity() {
        let scorer = TokenScorer::new();
        let token = create_test_token(Some(0.5), false, false);
        
        let score = scorer.score(&token);
        assert_eq!(score, 0.0);
    }

    #[test]
    fn test_none_liquidity() {
        let scorer = TokenScorer::new();
        let token = create_test_token(None, false, false);
        
        let score = scorer.score(&token);
        assert_eq!(score, 0.0);
    }
}
