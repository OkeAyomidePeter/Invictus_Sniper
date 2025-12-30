use crate::enrichment::EnrichedToken;
use anyhow::Result;
use log::{info, warn};
use sqlx::{sqlite::SqlitePoolOptions, Pool, Sqlite, Row};
use std::path::Path;
use std::fs::File;

#[derive(Clone)]
pub struct Database {
    pub(crate) pool: Pool<Sqlite>,
}

impl Database {
    pub async fn new(database_url: &str) -> Result<Self> {
        // Ensure database file exists
        let db_path = database_url.trim_start_matches("sqlite://");
        if !Path::new(db_path).exists() {
            info!("Creating database file: {}", db_path);
            File::create(db_path)?;
        }

        let pool = SqlitePoolOptions::new()
            .max_connections(5)
            .connect(database_url)
            .await?;

        let db = Self { pool };
        db.init_schema().await?;

        Ok(db)
    }

    async fn init_schema(&self) -> Result<()> {
        // Table: Tokens (Discovered & Scored)
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS tokens (
                mint TEXT PRIMARY KEY,
                decimals INTEGER,
                supply TEXT,
                initial_liquidity_sol REAL,
                liquidity_usd REAL,
                score REAL,
                platform TEXT,
                discovered_at INTEGER
            );
            "#,
        )
        .execute(&self.pool)
        .await?;

        // Table: Trades (Buy/Sell execution)
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS trades (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                mint TEXT,
                action TEXT,
                amount_token TEXT,
                amount_sol INTEGER,
                price_sol REAL,
                signature TEXT,
                jito_bundle_id TEXT,
                timestamp INTEGER,
                entry_price REAL DEFAULT 0.0,
                decimals INTEGER DEFAULT 6,
                exit_price REAL,
                pnl_sol REAL,
                sell_trigger TEXT,
                exit_signature TEXT,
                highest_price_reached REAL,
                timeout_extensions INTEGER DEFAULT 0,
                partial_exit_executed INTEGER DEFAULT 0,
                remaining_amount_pct REAL DEFAULT 100.0,
                FOREIGN KEY(mint) REFERENCES tokens(mint)
            );
            "#,
        )
        .execute(&self.pool)
        .await?;

        // Run migrations for existing databases (add new columns if they don't exist)
        // SQLite doesn't support IF NOT EXISTS for columns, so we ignore errors
        let _ = sqlx::query("ALTER TABLE trades ADD COLUMN entry_price REAL DEFAULT 0.0")
            .execute(&self.pool).await;
        let _ = sqlx::query("ALTER TABLE trades ADD COLUMN partial_exit_executed INTEGER DEFAULT 0")
            .execute(&self.pool).await;
        let _ = sqlx::query("ALTER TABLE trades ADD COLUMN remaining_amount_pct REAL DEFAULT 100.0")
            .execute(&self.pool).await;
        // Migration for decimals
        let _ = sqlx::query("ALTER TABLE trades ADD COLUMN decimals INTEGER DEFAULT 6")
            .execute(&self.pool).await;
        // Migration for liquidity_usd
        let _ = sqlx::query("ALTER TABLE tokens ADD COLUMN liquidity_usd REAL DEFAULT 0.0")
            .execute(&self.pool).await;
        // Migration for entry_1m_move
        let _ = sqlx::query("ALTER TABLE trades ADD COLUMN entry_1m_move REAL DEFAULT 0.0")
            .execute(&self.pool).await;

        info!("✅ Database schema initialized");
        Ok(())
    }

    pub async fn store_token(&self, token: &EnrichedToken, score: f64) -> Result<()> {
        sqlx::query(
            r#"
            INSERT OR REPLACE INTO tokens (mint, decimals, supply, initial_liquidity_sol, liquidity_usd, score, platform, discovered_at)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(&token.mint)
        .bind(token.decimals as i64)
        .bind(token.supply.map(|s| s.to_string()))
        .bind(token.initial_liquidity_sol)
        .bind(token.liquidity_usd)
        .bind(score)
        .bind(format!("{:?}", token.platform))
        .bind(token.enrichment_timestamp)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    pub async fn record_trade(
        &self,
        mint: &str,
        action: &str, // "BUY" or "SELL"
        amount_token: u64,
        amount_sol: u64,
        signature: &str,
        jito_bundle_id: Option<&str>,
        entry_price: Option<f64>,
        decimals: u8,
        entry_1m_move: f64,
    ) -> Result<()> {
        let price_sol = if amount_token > 0 {
            amount_sol as f64 / amount_token as f64
        } else {
            0.0
        };

        sqlx::query(
            r#"
            INSERT INTO trades (mint, action, amount_token, amount_sol, price_sol, signature, jito_bundle_id, timestamp, entry_price, decimals, entry_1m_move)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(mint)
        .bind(action)
        .bind(amount_token.to_string())
        .bind(amount_sol as i64)
        .bind(price_sol)
        .bind(signature)
        .bind(jito_bundle_id)
        .bind(chrono::Utc::now().timestamp())
        .bind(entry_price.unwrap_or(0.0))
        .bind(decimals as i32)
        .bind(entry_1m_move)
        .execute(&self.pool)
        .await?;

        info!("💾 Trade recorded: {} {} (Sig: {})", action, mint, signature);
        Ok(())
    }

    /// Update position snapshot (highest price, extensions)
    pub async fn update_position_snapshot(
        &self,
        mint: &str,
        highest_price: f64,
        timeout_extensions: u32,
    ) -> Result<()> {
        let trade_id: Option<(i64,)> = sqlx::query_as(
            r#"
            SELECT id FROM trades 
            WHERE mint = ? AND action = 'BUY' AND exit_price IS NULL 
            ORDER BY timestamp DESC LIMIT 1
            "#
        )
        .bind(mint)
        .fetch_optional(&self.pool)
        .await?;

        if let Some((id,)) = trade_id {
            sqlx::query(
                r#"
                UPDATE trades 
                SET highest_price_reached = ?, timeout_extensions = ?
                WHERE id = ?
                "#,
            )
            .bind(highest_price)
            .bind(timeout_extensions)
            .bind(id)
            .execute(&self.pool)
            .await?;
        }

        Ok(())
    }

    /// Update a trade with exit information (for SELL orders)
    pub async fn update_trade_exit(
        &self,
        mint: &str,
        exit_price: f64,
        pnl_sol: f64,
        sell_trigger: &str, // "PROFIT_TARGET", "STOP_LOSS", "TIMEOUT", "MANUAL"
        exit_signature: &str,
    ) -> Result<()> {
        // Find the most recent BUY trade for this mint that hasn't been closed
        let trade_id: Option<(i64,)> = sqlx::query_as(
            r#"
            SELECT id FROM trades 
            WHERE mint = ? AND action = 'BUY' AND exit_price IS NULL 
            ORDER BY timestamp DESC LIMIT 1
            "#
        )
        .bind(mint)
        .fetch_optional(&self.pool)
        .await?;

        if let Some((id,)) = trade_id {
            sqlx::query(
                r#"
                UPDATE trades 
                SET exit_price = ?, pnl_sol = ?, sell_trigger = ?, exit_signature = ?
                WHERE id = ?
                "#,
            )
            .bind(exit_price)
            .bind(pnl_sol)
            .bind(sell_trigger)
            .bind(exit_signature)
            .bind(id)
            .execute(&self.pool)
            .await?;

            info!("💾 Trade exit updated for {}: P/L = {:.4} SOL ({})", mint, pnl_sol, sell_trigger);
        } else {
            warn!("No open BUY trade found for {} to update", mint);
        }

        Ok(())
    }

    /// Get the most recent BUY trade for a mint (for entry price lookup)
    pub async fn get_trade_by_mint(&self, mint: &str) -> Result<Option<(f64, u64)>> {
        // Returns (entry_price, amount_token)
        let row: Option<(f64, String)> = sqlx::query_as(
            r#"
            SELECT entry_price, amount_token FROM trades 
            WHERE mint = ? AND action = 'BUY' 
            ORDER BY timestamp DESC LIMIT 1
            "#
        )
        .bind(mint)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.map(|(price, amt_str)| {
            let amt = amt_str.parse::<u64>().unwrap_or(0);
            (price, amt)
        }))
    }

    // ========== STATS QUERIES ==========

    pub async fn get_token_count(&self) -> Result<i64> {
        let count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM tokens")
            .fetch_one(&self.pool)
            .await?;
        Ok(count.0)
    }

    pub async fn get_trade_count(&self) -> Result<i64> {
        let count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM trades")
            .fetch_one(&self.pool)
            .await?;
        Ok(count.0)
    }

    pub async fn get_top_tokens(&self, limit: i64) -> Result<Vec<(String, f64, f64)>> {
        // Returns (mint, score, liquidity)
        let rows: Vec<(String, f64, f64)> = sqlx::query_as(
            "SELECT mint, score, initial_liquidity_sol FROM tokens ORDER BY score DESC LIMIT ?"
        )
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    pub async fn get_recent_trades(&self, limit: i64) -> Result<Vec<(String, String, f64)>> {
        // Returns (action, mint, price_sol)
        let rows: Vec<(String, String, f64)> = sqlx::query_as(
            "SELECT action, mint, price_sol FROM trades ORDER BY id DESC LIMIT ?"
        )
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    /// Get all active positions (BUY trades without exit_price)
    pub async fn get_active_positions(&self) -> Result<Vec<(String, f64, u64, i64)>> {
        // Returns (mint, entry_price, amount_token, timestamp)
        let rows: Vec<(String, f64, String, i64)> = sqlx::query_as(
            r#"
            SELECT mint, entry_price, amount_token, timestamp 
            FROM trades 
            WHERE action = 'BUY' AND exit_price IS NULL
            ORDER BY timestamp DESC
            "#
        )
        .fetch_all(&self.pool)
        .await?;
        
        // Convert string amount back to u64
        let result = rows.into_iter().map(|(mint, price, amt_str, ts)| {
            let amt = amt_str.parse::<u64>().unwrap_or(0);
            (mint, price, amt, ts)
        }).collect();
        
        Ok(result)
    }

    /// Get full active positions with state for restoration
    /// Returns (mint, entry_price, amount_token, timestamp, highest_price, extensions, amount_sol, partial_exit_executed, remaining_amount_pct, decimals, entry_1m_move)
    pub async fn get_open_positions_state(&self) -> Result<Vec<(String, f64, u64, i64, f64, u32, u64, bool, f64, u8, f64)>> {
        let rows: Vec<sqlx::sqlite::SqliteRow> = sqlx::query(
            r#"
            SELECT mint, entry_price, amount_token, timestamp, highest_price_reached, timeout_extensions, amount_sol, partial_exit_executed, remaining_amount_pct, decimals, entry_1m_move
            FROM trades 
            WHERE action = 'BUY' AND exit_price IS NULL
            ORDER BY timestamp DESC
            "#
        )
        .fetch_all(&self.pool)
        .await?;
        
        let result = rows.into_iter().map(|row| {
            let mint: String = row.get("mint");
            let entry_price: f64 = row.get("entry_price");
            let amount_token_str: String = row.get("amount_token");
            let timestamp: i64 = row.get("timestamp");
            let highest_price_reached: Option<f64> = row.get("highest_price_reached");
            let timeout_extensions: Option<i32> = row.get("timeout_extensions");
            let amount_sol: i64 = row.get("amount_sol");
            let partial_exit_executed: Option<i32> = row.get("partial_exit_executed");
            let remaining_amount_pct: Option<f64> = row.get("remaining_amount_pct");
            let decimals: i32 = row.get("decimals");
            let entry_1m_move: Option<f64> = row.get("entry_1m_move");

            let amt = amount_token_str.parse::<u64>().unwrap_or(0);
            (
                mint, 
                entry_price, 
                amt, 
                timestamp, 
                highest_price_reached.unwrap_or(entry_price), 
                timeout_extensions.unwrap_or(0) as u32,
                amount_sol as u64,
                partial_exit_executed.unwrap_or(0) != 0,  // Convert to bool
                remaining_amount_pct.unwrap_or(100.0),
                decimals as u8,
                entry_1m_move.unwrap_or(0.0)
            )
        }).collect();
        
        Ok(result)
    }

    /// Update position partial exit state
    pub async fn update_position_partial_exit(
        &self,
        mint: &str,
        partial_exit_executed: bool,
        remaining_amount_pct: f64,
    ) -> Result<()> {
        let trade_id: Option<(i64,)> = sqlx::query_as(
            r#"
            SELECT id FROM trades 
            WHERE mint = ? AND action = 'BUY' AND exit_price IS NULL 
            ORDER BY timestamp DESC LIMIT 1
            "#
        )
        .bind(mint)
        .fetch_optional(&self.pool)
        .await?;

        if let Some((id,)) = trade_id {
            sqlx::query(
                r#"
                UPDATE trades 
                SET partial_exit_executed = ?, remaining_amount_pct = ?
                WHERE id = ?
                "#,
            )
            .bind(if partial_exit_executed { 1 } else { 0 })
            .bind(remaining_amount_pct)
            .bind(id)
            .execute(&self.pool)
            .await?;

            info!("💾 Position partial exit state updated for {}: executed={}, remaining={:.1}%", 
                mint, partial_exit_executed, remaining_amount_pct);
        } else {
            warn!("No open BUY trade found for {} to update partial exit state", mint);
        }

        Ok(())
    }

    /// Get detailed trade statistics including P/L
    pub async fn get_trade_statistics(&self) -> Result<(i64, i64, f64, f64, i64)> {
        // Returns (total_trades, closed_trades, total_pnl, win_rate, active_positions)
        let total_trades: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM trades")
            .fetch_one(&self.pool)
            .await?;

        let closed_trades: (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM trades WHERE exit_price IS NOT NULL"
        )
        .fetch_one(&self.pool)
        .await?;

        let total_pnl: (Option<f64>,) = sqlx::query_as(
            "SELECT SUM(pnl_sol) FROM trades WHERE pnl_sol IS NOT NULL"
        )
        .fetch_one(&self.pool)
        .await?;

        let winning_trades: (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM trades WHERE pnl_sol > 0"
        )
        .fetch_one(&self.pool)
        .await?;

        let active_positions: (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM trades WHERE action = 'BUY' AND exit_price IS NULL"
        )
        .fetch_one(&self.pool)
        .await?;

        let win_rate = if closed_trades.0 > 0 {
            (winning_trades.0 as f64 / closed_trades.0 as f64) * 100.0
        } else {
            0.0
        };

        Ok((
            total_trades.0,
            closed_trades.0,
            total_pnl.0.unwrap_or(0.0),
            win_rate,
            active_positions.0,
        ))
    }
}
