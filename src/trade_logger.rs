//! Comprehensive trade logging for Invictus Sniper Bot
//! 
//! All important events are logged to invictus.log with timestamps.
//! This file is the single source of truth for trade history and performance.

use anyhow::Result;
use std::fs::OpenOptions;
use std::io::Write;
use std::sync::Mutex;
use chrono::Local;
use lazy_static::lazy_static;

lazy_static! {
    static ref LOGGER: Mutex<TradeLogger> = Mutex::new(TradeLogger::new("invictus.log"));
}

pub struct TradeLogger {
    file_path: String,
}

impl TradeLogger {
    pub fn new(file_path: &str) -> Self {
        Self {
            file_path: file_path.to_string(),
        }
    }

    pub fn log(message: &str) {
        if let Ok(mut logger) = LOGGER.lock() {
            let _ = logger.write(message);
        }
    }

    fn write(&mut self, message: &str) -> Result<()> {
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.file_path)?;

        let timestamp = Local::now().format("%Y-%m-%d %H:%M:%S%.3f");
        writeln!(file, "[{}] {}", timestamp, message)?;
        Ok(())
    }

    /// Log detailed enrichment data
    pub fn log_enrichment_data(
        mint: &str, 
        liquidity_usd: f64, 
        top_10_pct: f64, 
        unique_holders: Option<u64>, 
        socials_count: usize
    ) {
        let holders_str = unique_holders.map(|h| h.to_string()).unwrap_or_else(|| "?".to_string());
        TradeLogger::log(&format!(
            "🔍 ENRICHMENT: {} | Liq: ${:.0} | Top10: {:.1}% | Holders: {} | Socials: {}", 
            &mint[..12.min(mint.len())], liquidity_usd, top_10_pct, holders_str, socials_count
        ));
    }

    /// Log scoring breakdown
    pub fn log_scoring_breakdown(
        mint: &str, 
        liquidity_score: f64, 
        holder_score: f64, 
        social_score: f64, 
        final_score: f64
    ) {
        TradeLogger::log(&format!(
            "🧮 SCORING: {} | Liq: {:.1} | Holders: {:.1} | Socials: {:.1} -> Final: {:.1}", 
            &mint[..12.min(mint.len())], liquidity_score, holder_score, social_score, final_score
        ));
    }

    /// Log the full serialized EnrichedToken struct for deep debugging
    pub fn log_enriched_token(token: &crate::enrichment::EnrichedToken) {
        if let Ok(json) = serde_json::to_string_pretty(token) {
            TradeLogger::log(&format!("💎 FULL_ENRICHED_DATA [{}]:\n{}", token.mint, json));
        }
    }
}

/// Log a specific step in the trade pipeline with timing
pub fn log_pipeline_step(mint: &str, step: &str, duration_ms: u128, success: bool) {
    let emoji = if success { "✅" } else { "❌" };
    TradeLogger::log(&format!(
        "⏱️ STEP: {} | {} | {}ms | {}", 
        &mint[..12.min(mint.len())], step, duration_ms, emoji
    ));
}

/// Log a detailed error for a specific pipeline step
pub fn log_error_detailed(mint: &str, step: &str, error: &str) {
    TradeLogger::log(&format!(
        "❌ ERROR: {} | Step: {} | Reason: {}", 
        &mint[..12.min(mint.len())], step, error
    ));
}

/// Log detailed enrichment debug info (pool address, retry attempts)
pub fn log_enrichment_debug(mint: &str, pool_address: &str, liquidity_usd: f64, attempt: u8) {
    TradeLogger::log(&format!(
        "🔎 DEBUG: {} | Pool: {} | Liq: ${:.0} | Attempt: {}", 
        &mint[..12.min(mint.len())], pool_address, liquidity_usd, attempt
    ));
}

/// Log priority fees used
pub fn log_priority_fees(mint: &str, lamports: u64) {
    TradeLogger::log(&format!(
        "💸 FEES: {} | Priority Fee: {:.6} SOL", 
        &mint[..12.min(mint.len())], lamports as f64 / 1_000_000_000.0
    ));
}

/// Log a significant pipeline event
pub fn log_pipeline_event(mint: &str, event: &str, details: &str) {
    TradeLogger::log(&format!(
        "⚡ PIPELINE: {} | {} | {}", 
        &mint[..12.min(mint.len())], event, details
    ));
}

// ============================================================================
// STARTUP / SHUTDOWN
// ============================================================================

/// Log bot startup
pub fn log_startup(wallet_pubkey: &str, max_trade_sol: f64, min_liquidity_usd: f64) {
    TradeLogger::log("═══════════════════════════════════════════════════════════════");
    TradeLogger::log("🚀 INVICTUS SNIPER BOT STARTED");
    TradeLogger::log(&format!("   Wallet: {}", wallet_pubkey));
    TradeLogger::log(&format!("   Max Trade: {:.4} SOL | Min Liquidity: ${:.0}", max_trade_sol, min_liquidity_usd));
    TradeLogger::log("═══════════════════════════════════════════════════════════════");
}

/// Log bot shutdown
pub fn log_shutdown(positions_closed: usize, total_pnl: Option<f64>) {
    TradeLogger::log("═══════════════════════════════════════════════════════════════");
    TradeLogger::log("🛑 INVICTUS SNIPER BOT SHUTDOWN");
    TradeLogger::log(&format!("   Positions Closed: {}", positions_closed));
    if let Some(pnl) = total_pnl {
        let emoji = if pnl >= 0.0 { "✅" } else { "❌" };
        TradeLogger::log(&format!("   Session P/L: {} {:.4} SOL", emoji, pnl));
    }
    TradeLogger::log("═══════════════════════════════════════════════════════════════");
}

// ============================================================================
// TOKEN DISCOVERY & SCORING
// ============================================================================

/// Log a high score token discovery
pub fn log_discovery(mint: &str, score: f64, liquidity_usd: f64) {
    TradeLogger::log(&format!(
        "✨ DISCOVERY: {} | Score: {:.1}/150 | Liq: ${:.0}", 
        &mint[..12.min(mint.len())], score, liquidity_usd
    ));
}

/// Log a token added to watchlist (dip strategy)
pub fn log_watchlist_add(mint: &str, score: f64, price: f64) {
    TradeLogger::log(&format!(
        "👀 WATCHLIST: {} | Score: {:.1} | Entry Price: {:.10} SOL/token", 
        &mint[..12.min(mint.len())], score, price
    ));
}

/// Log a watchlist dip buy trigger
pub fn log_dip_trigger(mint: &str, dip_pct: f64, volume_5m: f64) {
    TradeLogger::log(&format!(
        "📉 DIP TRIGGER: {} | Dip: -{:.1}% | Vol 5m: ${:.0}", 
        &mint[..12.min(mint.len())], dip_pct, volume_5m
    ));
}

/// Log token rejected (low score)
pub fn log_token_rejected(mint: &str, score: f64, reason: &str) {
    TradeLogger::log(&format!(
        "⏭️  SKIPPED: {} | Score: {:.1} | Reason: {}", 
        &mint[..12.min(mint.len())], score, reason
    ));
}



// ============================================================================
// TRADE EXECUTION
// ============================================================================

/// Log a buy execution attempt
pub fn log_buy_attempt(mint: &str, amount_sol: f64, is_watchlist: bool) {
    let source = if is_watchlist { "WATCHLIST" } else { "DIRECT" };
    TradeLogger::log(&format!(
        "🎯 BUY ATTEMPT [{}]: {} | Amount: {:.4} SOL", 
        source, &mint[..12.min(mint.len())], amount_sol
    ));
}

/// Log a successful buy execution
pub fn log_buy(mint: &str, amount_sol: f64, identifier: &str) {
    TradeLogger::log(&format!(
        "🚀 BUY EXECUTED: {} | Amount: {:.4} SOL | ID: {}", 
        &mint[..12.min(mint.len())], amount_sol, &identifier[..20.min(identifier.len())]
    ));
}

/// Log a buy failure
pub fn log_buy_failed(mint: &str, amount_sol: f64, error: &str) {
    TradeLogger::log(&format!(
        "❌ BUY FAILED: {} | Amount: {:.4} SOL | Error: {}", 
        &mint[..12.min(mint.len())], amount_sol, error
    ));
}

/// Log a sell execution
pub fn log_sell(mint: &str, pnl_sol: f64, pnl_pct: f64, trigger: &str, identifier: &str) {
    let emoji = if pnl_sol >= 0.0 { "💰" } else { "🔻" };
    let sign = if pnl_sol >= 0.0 { "+" } else { "" };
    TradeLogger::log(&format!(
        "{} SELL EXECUTED: {} | P/L: {}{:.4} SOL ({}{:.2}%) | Trigger: {} | ID: {}", 
        emoji, &mint[..12.min(mint.len())], sign, pnl_sol, sign, pnl_pct, trigger,
        &identifier[..20.min(identifier.len())]
    ));
}

/// Log a sell failure
pub fn log_sell_failed(mint: &str, trigger: &str, error: &str) {
    TradeLogger::log(&format!(
        "❌ SELL FAILED: {} | Trigger: {} | Error: {}", 
        &mint[..12.min(mint.len())], trigger, error
    ));
}

// ============================================================================
// BUNDLE & TRANSACTION STATUS
// ============================================================================

/// Log bundle sent to Jito
pub fn log_bundle_sent(bundle_id: &str, tx_count: usize) {
    TradeLogger::log(&format!(
        "📦 BUNDLE SENT: {} | Txs: {}", 
        &bundle_id[..20.min(bundle_id.len())], tx_count
    ));
}

/// Log bundle confirmation result
pub fn log_bundle_confirmed(bundle_id: &str, status: &str) {
    let emoji = if status == "Landed" { "✅" } else { "❌" };
    TradeLogger::log(&format!(
        "{} BUNDLE {}: {}", 
        emoji, status.to_uppercase(), &bundle_id[..20.min(bundle_id.len())]
    ));
}

/// Log detailed Jito debug info
pub fn log_jito_debug(action: &str, details: &str) {
    TradeLogger::log(&format!("🌩️ JITO DEBUG: {} | {}", action, details));
}

// ============================================================================
// POSITION MONITORING
// ============================================================================

/// Log optimistic monitoring start (before balance verification)
pub fn log_optimistic_start(mint: &str, expected_tokens: u64) {
    TradeLogger::log(&format!(
        "🛡️  OPTIMISTIC: Starting monitoring immediately for {} with expected {} tokens", 
        &mint[..12.min(mint.len())], expected_tokens
    ));
}

/// Log position monitoring started
pub fn log_position_started(mint: &str, entry_price: f64, target_pct: f64, stop_pct: f64) {
    TradeLogger::log(&format!(
        "📊 MONITORING: {} | Entry: {:.10} SOL/token | Target: +{:.0}% | Stop: -{:.0}%", 
        &mint[..12.min(mint.len())], entry_price, target_pct, stop_pct
    ));
}

/// Log position price update (periodic)
pub fn log_position_update(mint: &str, current_price: f64, pnl_pct: f64, highest_price: f64) {
    let emoji = if pnl_pct >= 0.0 { "📈" } else { "📉" };
    let sign = if pnl_pct >= 0.0 { "+" } else { "" };
    TradeLogger::log(&format!(
        "{} POSITION: {} | P/L: {}{:.2}% | Price: {:.10} | Peak: {:.10}", 
        emoji, &mint[..12.min(mint.len())], sign, pnl_pct, current_price, highest_price
    ));
}

/// Log partial exit execution
pub fn log_partial_exit(mint: &str, exit_pct: f64, pnl_pct: f64, remaining_pct: f64) {
    TradeLogger::log(&format!(
        "💵 PARTIAL EXIT: {} | Sold: {:.0}% | P/L: +{:.2}% | Remaining: {:.0}%", 
        &mint[..12.min(mint.len())], exit_pct, pnl_pct, remaining_pct
    ));
}

/// Log trailing stop update
pub fn log_trailing_stop_update(mint: &str, new_stop_price: f64, distance_from_peak: f64) {
    TradeLogger::log(&format!(
        "🔒 TRAILING STOP: {} | New Stop: {:.10} | Distance: {:.2}% from peak", 
        &mint[..12.min(mint.len())], new_stop_price, distance_from_peak
    ));
}

/// Log position timeout
pub fn log_position_timeout(mint: &str, extensions: u32, final_pnl_pct: f64) {
    TradeLogger::log(&format!(
        "⏰ TIMEOUT: {} | Extensions: {} | Final P/L: {:.2}%", 
        &mint[..12.min(mint.len())], extensions, final_pnl_pct
    ));
}

// ============================================================================
// RISK MANAGEMENT
// ============================================================================

/// Log risk check rejection
pub fn log_risk_rejected(mint: &str, reason: &str) {
    TradeLogger::log(&format!(
        "🛡️  RISK BLOCK: {} | Reason: {}", 
        &mint[..12.min(mint.len())], reason
    ));
}

/// Log wallet balance alert
pub fn log_wallet_alert(balance_sol: f64, threshold_sol: f64) {
    TradeLogger::log(&format!(
        "⚠️  LOW BALANCE: {:.4} SOL (threshold: {:.4} SOL)", 
        balance_sol, threshold_sol
    ));
}

/// Log daily stats summary
pub fn log_daily_summary(trades: i64, wins: i64, total_pnl: f64) {
    let win_rate = if trades > 0 { (wins as f64 / trades as f64) * 100.0 } else { 0.0 };
    let emoji = if total_pnl >= 0.0 { "✅" } else { "❌" };
    let sign = if total_pnl >= 0.0 { "+" } else { "" };
    TradeLogger::log("───────────────────────────────────────────────────────────────");
    TradeLogger::log(&format!(
        "📊 SUMMARY: {} trades | Win Rate: {:.1}% | P/L: {} {}{:.4} SOL", 
        trades, win_rate, emoji, sign, total_pnl
    ));
    TradeLogger::log("───────────────────────────────────────────────────────────────");
}

// ============================================================================
// ERRORS & WARNINGS
// ============================================================================

/// Log a critical error
pub fn log_error(context: &str, error: &str) {
    TradeLogger::log(&format!("❌ ERROR [{}]: {}", context, error));
}

/// Log a warning
pub fn log_warning(context: &str, message: &str) {
    TradeLogger::log(&format!("⚠️  WARNING [{}]: {}", context, message));
}

/// Log system health status
pub fn log_health_status(ws_age: i64, enrich_age: i64, is_healthy: bool) {
    let status = if is_healthy { "HEALTHY ✅" } else { "DEGRADED ⚠️" };
    TradeLogger::log(&format!(
        "🏥 HEALTH: {} | WS: {}s ago | Enrich: {}s ago", 
        status, ws_age, enrich_age
    ));
}

/// Log rejection due to price instability
pub fn log_price_stability_failed(mint: &str, drop_pct: f64) {
    TradeLogger::log(&format!("🥀 STABILITY FAILED: {} | Drop: {:.2}% | Trade Aborted", 
        &mint[..12.min(mint.len())], drop_pct));
}

/// Log rejection due to high volatility
pub fn log_volatility_rejected(mint: &str, pct: f64) {
    TradeLogger::log(&format!("💀 VOLATILITY REJECTED: {} | 5m Swing: {:.1}% | Risk too high", 
        &mint[..12.min(mint.len())], pct));
}

/// Log rejection due to whale concentration
pub fn log_whale_rejected(mint: &str, top10: f64) {
    TradeLogger::log(&format!("💀 WHALE REJECTED: {} | Top10: {:.1}% | Concentration too high", 
        &mint[..12.min(mint.len())], top10));
}
