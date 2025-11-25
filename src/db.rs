use crate::enrichment::EnrichedToken;
use anyhow::Result;
use log::{info, warn};
use sqlx::{sqlite::SqlitePoolOptions, Pool, Sqlite};
use std::path::Path;
use std::fs::File;

#[derive(Clone)]
pub struct Database {
    pool: Pool<Sqlite>,
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
                supply INTEGER,
                initial_liquidity_sol REAL,
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
                amount_token INTEGER,
                amount_sol INTEGER,
                price_sol REAL,
                signature TEXT,
                jito_bundle_id TEXT,
                timestamp INTEGER,
                entry_price REAL DEFAULT 0.0,
                exit_price REAL,
                pnl_sol REAL,
                sell_trigger TEXT,
                FOREIGN KEY(mint) REFERENCES tokens(mint)
            );
            "#,
        )
        .execute(&self.pool)
        .await?;

        info!("✅ Database schema initialized");
        Ok(())
    }

    pub async fn store_token(&self, token: &EnrichedToken, score: f64) -> Result<()> {
        sqlx::query(
            r#"
            INSERT OR REPLACE INTO tokens (mint, decimals, supply, initial_liquidity_sol, score, platform, discovered_at)
            VALUES (?, ?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(&token.mint)
        .bind(token.decimals as i64)
        .bind(token.supply.map(|s| s as i64))
        .bind(token.initial_liquidity_sol)
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
    ) -> Result<()> {
        let price_sol = if amount_token > 0 {
            amount_sol as f64 / amount_token as f64
        } else {
            0.0
        };

        sqlx::query(
            r#"
            INSERT INTO trades (mint, action, amount_token, amount_sol, price_sol, signature, jito_bundle_id, timestamp, entry_price)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(mint)
        .bind(action)
        .bind(amount_token as i64)
        .bind(amount_sol as i64)
        .bind(price_sol)
        .bind(signature)
        .bind(jito_bundle_id)
        .bind(chrono::Utc::now().timestamp())
        .bind(entry_price)
        .execute(&self.pool)
        .await?;

        info!("💾 Trade recorded: {} {} (Sig: {})", action, mint, signature);
        Ok(())
    }

    /// Update a trade with exit information (for SELL orders)
    pub async fn update_trade_exit(
        &self,
        mint: &str,
        exit_price: f64,
        pnl_sol: f64,
        sell_trigger: &str, // "PROFIT_TARGET", "STOP_LOSS", "TIMEOUT", "MANUAL"
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
                SET exit_price = ?, pnl_sol = ?, sell_trigger = ?
                WHERE id = ?
                "#,
            )
            .bind(exit_price)
            .bind(pnl_sol)
            .bind(sell_trigger)
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
        let row: Option<(f64, i64)> = sqlx::query_as(
            r#"
            SELECT entry_price, amount_token FROM trades 
            WHERE mint = ? AND action = 'BUY' 
            ORDER BY timestamp DESC LIMIT 1
            "#
        )
        .bind(mint)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.map(|(price, amt)| (price, amt as u64)))
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
}
