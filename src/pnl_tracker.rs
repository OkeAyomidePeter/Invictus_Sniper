use crate::db::Database;
use anyhow::Result;
use serde::{Deserialize, Serialize};

/// PnL statistics for the bot
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PnLStats {
    /// Total number of trades (buys + sells)
    pub total_trades: i64,
    /// Number of closed positions (with exit data)
    pub closed_trades: i64,
    /// Total realized profit/loss in SOL
    pub total_pnl_sol: f64,
    /// Win rate percentage (0-100)
    pub win_rate_pct: f64,
    /// Number of currently open positions
    pub active_positions: i64,
    /// Number of winning trades
    pub winning_trades: i64,
    /// Number of losing trades
    pub losing_trades: i64,
    /// Average profit per winning trade (SOL)
    pub avg_win_sol: f64,
    /// Average loss per losing trade (SOL)
    pub avg_loss_sol: f64,
}

/// Per-token PnL breakdown
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenPnL {
    pub mint: String,
    pub entry_price: f64,
    pub exit_price: Option<f64>,
    pub pnl_sol: Option<f64>,
    pub sell_trigger: Option<String>,
    pub is_open: bool,
}

pub struct PnLTracker {
    db: Database,
}

impl PnLTracker {
    pub fn new(db: Database) -> Self {
        Self { db }
    }

    /// Get overall PnL statistics
    pub async fn get_stats(&self) -> Result<PnLStats> {
        let (total_trades, closed_trades, total_pnl_sol, win_rate_pct, active_positions) =
            self.db.get_trade_statistics().await?;

        // Get winning and losing trade counts
        let winning_trades: (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM trades WHERE pnl_sol > 0"
        )
        .fetch_one(&self.db.pool)
        .await?;

        let losing_trades: (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM trades WHERE pnl_sol < 0"
        )
        .fetch_one(&self.db.pool)
        .await?;

        // Get average win/loss
        let avg_win: (Option<f64>,) = sqlx::query_as(
            "SELECT AVG(pnl_sol) FROM trades WHERE pnl_sol > 0"
        )
        .fetch_one(&self.db.pool)
        .await?;

        let avg_loss: (Option<f64>,) = sqlx::query_as(
            "SELECT AVG(pnl_sol) FROM trades WHERE pnl_sol < 0"
        )
        .fetch_one(&self.db.pool)
        .await?;

        Ok(PnLStats {
            total_trades,
            closed_trades,
            total_pnl_sol,
            win_rate_pct,
            active_positions,
            winning_trades: winning_trades.0,
            losing_trades: losing_trades.0,
            avg_win_sol: avg_win.0.unwrap_or(0.0),
            avg_loss_sol: avg_loss.0.unwrap_or(0.0),
        })
    }

    /// Get PnL for the last N days
    pub async fn get_stats_for_period(&self, days: i64) -> Result<PnLStats> {
        let cutoff_timestamp = chrono::Utc::now().timestamp() - (days * 24 * 60 * 60);

        let total_trades: (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM trades WHERE timestamp >= ?"
        )
        .bind(cutoff_timestamp)
        .fetch_one(&self.db.pool)
        .await?;

        let closed_trades: (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM trades WHERE exit_price IS NOT NULL AND timestamp >= ?"
        )
        .bind(cutoff_timestamp)
        .fetch_one(&self.db.pool)
        .await?;

        let total_pnl: (Option<f64>,) = sqlx::query_as(
            "SELECT SUM(pnl_sol) FROM trades WHERE pnl_sol IS NOT NULL AND timestamp >= ?"
        )
        .bind(cutoff_timestamp)
        .fetch_one(&self.db.pool)
        .await?;

        let winning_trades: (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM trades WHERE pnl_sol > 0 AND timestamp >= ?"
        )
        .bind(cutoff_timestamp)
        .fetch_one(&self.db.pool)
        .await?;

        let losing_trades: (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM trades WHERE pnl_sol < 0 AND timestamp >= ?"
        )
        .bind(cutoff_timestamp)
        .fetch_one(&self.db.pool)
        .await?;

        let avg_win: (Option<f64>,) = sqlx::query_as(
            "SELECT AVG(pnl_sol) FROM trades WHERE pnl_sol > 0 AND timestamp >= ?"
        )
        .bind(cutoff_timestamp)
        .fetch_one(&self.db.pool)
        .await?;

        let avg_loss: (Option<f64>,) = sqlx::query_as(
            "SELECT AVG(pnl_sol) FROM trades WHERE pnl_sol < 0 AND timestamp >= ?"
        )
        .bind(cutoff_timestamp)
        .fetch_one(&self.db.pool)
        .await?;

        let active_positions: (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM trades WHERE action = 'BUY' AND exit_price IS NULL AND timestamp >= ?"
        )
        .bind(cutoff_timestamp)
        .fetch_one(&self.db.pool)
        .await?;

        let win_rate_pct = if closed_trades.0 > 0 {
            (winning_trades.0 as f64 / closed_trades.0 as f64) * 100.0
        } else {
            0.0
        };

        Ok(PnLStats {
            total_trades: total_trades.0,
            closed_trades: closed_trades.0,
            total_pnl_sol: total_pnl.0.unwrap_or(0.0),
            win_rate_pct,
            active_positions: active_positions.0,
            winning_trades: winning_trades.0,
            losing_trades: losing_trades.0,
            avg_win_sol: avg_win.0.unwrap_or(0.0),
            avg_loss_sol: avg_loss.0.unwrap_or(0.0),
        })
    }

    /// Get top N most profitable trades
    pub async fn get_top_trades(&self, limit: i64) -> Result<Vec<TokenPnL>> {
        let rows: Vec<(String, f64, Option<f64>, Option<f64>, Option<String>)> = sqlx::query_as(
            r#"
            SELECT mint, entry_price, exit_price, pnl_sol, sell_trigger
            FROM trades
            WHERE pnl_sol IS NOT NULL
            ORDER BY pnl_sol DESC
            LIMIT ?
            "#
        )
        .bind(limit)
        .fetch_all(&self.db.pool)
        .await?;

        Ok(rows.into_iter().map(|(mint, entry, exit, pnl, trigger)| {
            TokenPnL {
                mint,
                entry_price: entry,
                exit_price: exit,
                pnl_sol: pnl,
                sell_trigger: trigger,
                is_open: false,
            }
        }).collect())
    }

    /// Get worst N trades (biggest losses)
    pub async fn get_worst_trades(&self, limit: i64) -> Result<Vec<TokenPnL>> {
        let rows: Vec<(String, f64, Option<f64>, Option<f64>, Option<String>)> = sqlx::query_as(
            r#"
            SELECT mint, entry_price, exit_price, pnl_sol, sell_trigger
            FROM trades
            WHERE pnl_sol IS NOT NULL
            ORDER BY pnl_sol ASC
            LIMIT ?
            "#
        )
        .bind(limit)
        .fetch_all(&self.db.pool)
        .await?;

        Ok(rows.into_iter().map(|(mint, entry, exit, pnl, trigger)| {
            TokenPnL {
                mint,
                entry_price: entry,
                exit_price: exit,
                pnl_sol: pnl,
                sell_trigger: trigger,
                is_open: false,
            }
        }).collect())
    }
}
