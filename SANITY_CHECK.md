# 🔍 INVICTUS SNIPER BOT - PRODUCTION SANITY CHECK

> **Purpose**: Comprehensive verification checklist to detect placeholders, hallucinated code, false implementations, and ensure production-grade quality across the entire codebase.

---

## 📋 HOW TO USE THIS CHECKLIST

1. **Go through each section sequentially**
2. **For each file, verify the specific items listed**
3. **Mark items as**: ✅ (verified), ❌ (issue found), ⚠️ (needs attention)
4. **Document any issues found in the "Issues Found" section at the bottom**
5. **Fix issues before deploying to production**

---

## 🔐 SECTION 1: API KEYS & CONFIGURATION

### File: `.env` and `.env.example`

**Real-world API verification:**

- [ ] **Helius API Key** (`HELIUS_API_KEY`)
  - Verify key format: Should be a valid API key string
  - Test connection: `curl "https://mainnet.helius-rpc.com/?api-key=YOUR_KEY" -X POST -H "Content-Type: application/json" -d '{"jsonrpc":"2.0","id":1,"method":"getHealth"}'`
  - Check rate limits: Verify your Helius plan supports WebSocket connections
  - **Hallucination check**: Ensure no placeholder like `"your-api-key-here"` or `"sk_test_xxx"`

- [ ] **Telegram Bot Token** (`TELEGRAM_BOT_TOKEN`)
  - Format: Should be `<bot_id>:<auth_token>` (e.g., `123456789:ABCdefGHIjklMNOpqrsTUVwxyz`)
  - Verify with: `curl "https://api.telegram.org/bot<YOUR_TOKEN>/getMe"`
  - **Hallucination check**: Ensure it's a real token from @BotFather, not a fake example

- [ ] **Telegram Chat ID** (`TELEGRAM_CHAT_ID`)
  - Format: Should be a numeric ID (e.g., `123456789` or `-100123456789` for groups)
  - Verify by sending test message: Check if bot can send to this chat
  - **Hallucination check**: Not a placeholder like `"your-chat-id"`

- [ ] **Solana Wallet Path** (`WALLET_PATH`)
  - Verify file exists at specified path
  - Check file format: Should be valid Solana keypair JSON `[1,2,3,...]` with 64 bytes
  - **Security check**: Ensure wallet file has restricted permissions (chmod 600)
  - **Hallucination check**: Not pointing to non-existent `/path/to/wallet.json`

- [ ] **Database Path** (`DATABASE_URL`)
  - Format: `sqlite:./invictus.db` or absolute path
  - Verify database file is created and accessible
  - **Hallucination check**: Not using placeholder database path

- [ ] **RPC URLs** (`SOLANA_RPC_URL`, `SOLANA_WS_URL`)
  - Default should be Helius: `https://mainnet.helius-rpc.com/?api-key=<KEY>`
  - WebSocket: `wss://mainnet.helius-rpc.com/?api-key=<KEY>`
  - Test HTTP RPC: `curl "<RPC_URL>" -X POST -H "Content-Type: application/json" -d '{"jsonrpc":"2.0","id":1,"method":"getHealth"}'`
  - **Hallucination check**: Not using public endpoints like `https://api.mainnet-beta.solana.com` (rate-limited)

---

## 🎯 SECTION 2: HELIUS LISTENER (WebSocket Integration)

### File: `src/helius_listener.rs`

**Real-world Helius API verification:**

- [ ] **WebSocket URL Format**
  - Line ~260: `wss://mainnet.helius-rpc.com/?api-key={}`
  - **Verify**: This is the correct Helius WebSocket endpoint
  - **Check docs**: https://docs.helius.dev/solana-rpc-nodes/websocket-subscriptions

- [ ] **Subscription Methods**
  - Line ~281: `logsSubscribe` method
  - **Verify**: Helius supports `logsSubscribe` with `mentions` filter
  - **Check docs**: https://docs.helius.dev/solana-rpc-nodes/websocket-subscriptions/logs-subscribe
  - **Hallucination check**: Ensure we're not using non-existent methods like `poolSubscribe` or `tokenSubscribe`

- [ ] **Program IDs - Pump.fun**
  - Line ~131: `PUMP_FUN_PROGRAM_ID = "6EF8rrecthR5Dkzon8Nwi3bTW1w4Q5PgdHzCfyqXYUVh"`
  - **Verify on Solscan**: https://solscan.io/account/6EF8rrecthR5Dkzon8Nwi3bTW1w4Q5PgdHzCfyqXYUVh
  - **Check**: Should show as Pump.fun program
  - **Hallucination check**: Not a made-up address

- [ ] **Program IDs - Pump.fun Migration Account**
  - Line ~134: `PUMPFUN_MIGRATION_ACCOUNT = "39azUYFWPz3VHgKCf3VChUwbpURdCHRxjWVowf5jUJjg"`
  - **CRITICAL**: Verify this is the actual migration account
  - **Verify on Solscan**: https://solscan.io/account/39azUYFWPz3VHgKCf3VChUwbpURdCHRxjWVowf5jUJjg
  - **Hallucination check**: Confirm this account is involved in Pump.fun graduations

- [ ] **Program IDs - Bonk.fun**
  - Line ~137: `BONK_FUN_PROGRAM_ID = "FfYek5vEz23cMkWsdJwG2oa6EphsvXSHrGpdALN4g6W1"`
  - **Verify on Solscan**: https://solscan.io/account/FfYek5vEz23cMkWsdJwG2oa6EphsvXSHrGpdALN4g6W1
  - **Hallucination check**: Confirm this is the real Bonk.fun program

- [ ] **Program IDs - Raydium**
  - Line ~122: `RAYDIUM_AMM_V4_PROGRAM_ID = "675kPX9MHTjS2zt1qfr1NYHuzeLXfQM9H24wFSUt1Mp8"`
  - Line ~125: `RAYDIUM_CPMM_PROGRAM_ID = "CPMMoo8L3F4NbTegBCKVNunggL7H1ZpdTHKxQB5qKP1C"`
  - Line ~128: `RAYDIUM_LIQUIDITY_POOL_V4 = "RVKd61ztZW9GUwhRbbLoYVRE5Xf1B2tVscKqwZqXgEr"`
  - **Verify each on Solscan**: Confirm these are official Raydium program IDs
  - **Hallucination check**: Not made-up addresses

- [ ] **Well-Known Token Mints**
  - Lines ~157-190: WSOL, USDC, USDT, BONK, etc.
  - **Verify each mint address on Solscan**:
    - WSOL: `So11111111111111111111111111111111111111112`
    - USDC: `EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v`
    - USDT: `Es9vMFrzaCERmJfrF4H2FYD4KCoNkY11McCe8BenwNYB`
  - **Hallucination check**: These should match official token mints

- [ ] **Age Filtering Logic**
  - Line ~196: `MAX_TOKEN_AGE_SECONDS = 60`
  - Lines ~534-555: Age calculation using `SystemTime::now()`
  - **Verify**: Logic correctly calculates token age from blockchain timestamp
  - **Test**: Ensure old tokens are properly rejected
  - **Hallucination check**: Not using fake timestamp sources

- [ ] **Transaction Fetching**
  - Lines ~876-915: `fetch_transaction()` using `getTransaction` RPC method
  - **Verify**: Using correct Helius RPC endpoint
  - **Check params**: `encoding: "jsonParsed"`, `maxSupportedTransactionVersion: 0`
  - **Verify docs**: https://docs.helius.dev/solana-rpc-nodes/alpha-rpc-methods/gettransaction
  - **Hallucination check**: Not using non-existent RPC methods

---

## 💰 SECTION 3: TOKEN ENRICHMENT (Helius DAS API)

### File: `src/enrichment.rs`

**Real-world Helius DAS API verification:**

- [ ] **DAS API Endpoint**
  - Line ~50: `https://mainnet.helius-rpc.com/v0/token-metadata`
  - **Verify**: This is the correct Helius DAS endpoint
  - **Check docs**: https://docs.helius.dev/solana-apis/digital-asset-standard-das-api
  - **Hallucination check**: Not using a made-up endpoint

- [ ] **Request Format**
  - Lines ~52-60: Request body with `mintAccounts` array
  - **Verify**: Matches Helius DAS API specification
  - **Test**: Send sample request with known token mint
  - **Hallucination check**: Not using incorrect field names

- [ ] **Response Parsing**
  - Lines ~70-120: Parsing `onChainMetadata`, `offChainMetadata`, etc.
  - **Verify**: Field names match actual Helius DAS response
  - **Check docs**: https://docs.helius.dev/solana-apis/digital-asset-standard-das-api/get-asset
  - **Test**: Parse real response from Helius
  - **Hallucination check**: Not parsing non-existent fields

- [ ] **Metadata Fields**
  - Verify these fields exist in Helius responses:
    - `onChainMetadata.metadata.name`
    - `onChainMetadata.metadata.symbol`
    - `content.metadata.description`
    - `content.links.image`
  - **Hallucination check**: Not accessing fields that don't exist in API response

- [ ] **Social Links Extraction**
  - Lines ~200-250: Extracting Twitter, Telegram, Website
  - **Verify**: Field paths match Helius response structure
  - **Test**: With real token metadata
  - **Hallucination check**: Not using made-up field paths

- [ ] **Holder Count Endpoint**
  - Check if holder count fetching uses real Helius endpoint
  - **Verify docs**: Confirm Helius provides holder count data
  - **Hallucination check**: Not using non-existent holder count API

---

## 🔄 SECTION 4: TRANSACTION EXECUTION

### File: `src/tx.rs`

**Real-world Solana transaction verification:**

- [ ] **Jito Bundle Endpoint**
  - Check if using Jito for MEV protection
  - **Verify endpoint**: Should be `https://mainnet.block-engine.jito.wtf/api/v1/bundles` or similar
  - **Check docs**: https://jito-labs.gitbook.io/mev/searcher-services/json-rpc-api-reference
  - **Hallucination check**: Not using fake Jito endpoints

- [ ] **Transaction Building**
  - Verify `solana-sdk` usage is correct
  - **Check**: `Transaction::new_signed_with_payer()` is real method
  - **Verify**: Instruction building uses actual Solana program interfaces
  - **Hallucination check**: Not using non-existent SDK methods

- [ ] **Compute Budget Instructions**
  - Check if adding compute budget instructions
  - **Verify**: Using real `ComputeBudgetInstruction` from `solana-sdk`
  - **Check limits**: Compute units and priority fees are reasonable
  - **Hallucination check**: Not using made-up instruction types

- [ ] **Token Swap Instructions**
  - If implementing swaps, verify instruction format
  - **Check**: Using real Raydium/Jupiter instruction builders
  - **Verify**: Account ordering matches program requirements
  - **Hallucination check**: Not using fake swap instruction formats

- [ ] **Transaction Sending**
  - Verify `send_transaction` RPC method usage
  - **Check params**: `skipPreflight`, `preflightCommitment`, etc.
  - **Verify docs**: https://docs.solana.com/api/http#sendtransaction
  - **Hallucination check**: Not using non-existent RPC methods

---

## 🗄️ SECTION 5: DATABASE OPERATIONS

### File: `src/db.rs`

**SQLite schema verification:**

- [ ] **Database Initialization**
  - Lines ~30-100: Table creation SQL
  - **Verify**: SQL syntax is valid SQLite
  - **Test**: Run `sqlite3 invictus.db ".schema"` to verify tables
  - **Hallucination check**: Not using PostgreSQL-specific syntax in SQLite

- [ ] **Table Schemas**
  - Verify each table has proper columns and types
  - **Check**: Primary keys, foreign keys, indexes are correctly defined
  - **Test**: Insert sample data to verify schema works
  - **Hallucination check**: Not using non-existent SQLite data types

- [ ] **SQLx Queries**
  - Verify `sqlx::query!()` macros use correct syntax
  - **Check**: Placeholders (`$1`, `$2`) match parameter count
  - **Test**: Compile-time verification with `cargo sqlx prepare`
  - **Hallucination check**: Not using incorrect query syntax

- [ ] **Connection Pooling**
  - Verify `SqlitePool` usage is correct
  - **Check**: Pool configuration is reasonable
  - **Test**: Verify connections don't leak
  - **Hallucination check**: Not using non-existent pool methods

---

## 📊 SECTION 6: POSITION TRACKING

### File: `src/position_tracker.rs`

**Position management verification:**

- [ ] **Position State Machine**
  - Verify position states are logical: `Open`, `Closed`, `PartialExit`, etc.
  - **Check**: State transitions are valid
  - **Test**: Simulate position lifecycle
  - **Hallucination check**: Not using non-existent position states

- [ ] **PnL Calculation**
  - Verify profit/loss calculation is mathematically correct
  - **Formula check**: `(exit_price - entry_price) / entry_price * 100`
  - **Test**: With known values to verify accuracy
  - **Hallucination check**: Not using incorrect formulas

- [ ] **Token Balance Tracking**
  - Verify balance fetching uses real Solana RPC methods
  - **Check**: `getTokenAccountBalance` is correct method
  - **Verify docs**: https://docs.solana.com/api/http#gettokenaccountbalance
  - **Hallucination check**: Not using fake balance methods

---

## 🎨 SECTION 7: GUI IMPLEMENTATION

### Files: `src/gui/*.rs`, `src/bin/invictus-gui.rs`

**egui framework verification:**

- [ ] **egui Version Compatibility**
  - Cargo.toml line ~52-53: `eframe = "0.24"`, `egui = "0.24"`
  - **Verify**: These versions are compatible
  - **Check docs**: https://docs.rs/egui/0.24.0/egui/
  - **Test**: Compile and run GUI
  - **Hallucination check**: Not using non-existent egui widgets

- [ ] **Widget Usage**
  - Verify all egui widgets are real: `ui.label()`, `ui.button()`, `ui.text_edit()`, etc.
  - **Check docs**: Confirm each widget exists in egui 0.24
  - **Hallucination check**: Not using made-up widget methods

- [ ] **Layout Methods**
  - Verify layout methods: `ui.horizontal()`, `ui.vertical()`, `ui.columns()`, etc.
  - **Check**: These are real egui layout methods
  - **Hallucination check**: Not using non-existent layout APIs

- [ ] **Theme Implementation** (`src/gui/theme.rs`)
  - Verify color definitions use valid egui `Color32` format
  - **Check**: RGB values are in valid range (0-255)
  - **Test**: Apply theme and verify colors render correctly
  - **Hallucination check**: Not using non-existent color methods

- [ ] **State Management** (`src/gui/mod.rs`)
  - Verify `Arc<Mutex<BotState>>` pattern is correct
  - **Check**: No race conditions or deadlocks
  - **Test**: Verify state updates propagate correctly
  - **Hallucination check**: Not using incorrect async patterns in egui

- [ ] **Bot Runtime Integration** (`src/gui/bot_runtime.rs`)
  - Verify bot can be started/stopped from GUI
  - **Check**: Tokio runtime integration is correct
  - **Test**: Start/stop bot multiple times
  - **Hallucination check**: Not using non-existent runtime methods

---

## 📱 SECTION 8: TELEGRAM INTEGRATION

### File: `src/tele.rs`

**Teloxide framework verification:**

- [ ] **Teloxide Version**
  - Cargo.toml line ~31: `teloxide = { version = "0.9", features = ["macros"] }`
  - **Verify**: Version 0.9 is stable
  - **Check docs**: https://docs.rs/teloxide/0.9.0/teloxide/
  - **Hallucination check**: Not using non-existent teloxide features

- [ ] **Bot Commands**
  - Verify command handlers use correct teloxide macros
  - **Check**: `#[command]` macro exists in teloxide 0.9
  - **Test**: Send commands to bot and verify responses
  - **Hallucination check**: Not using made-up command patterns

- [ ] **Message Sending**
  - Verify `bot.send_message()` is correct method
  - **Check**: Parameters match teloxide API
  - **Test**: Send test messages
  - **Hallucination check**: Not using non-existent send methods

- [ ] **Keyboard Markup**
  - If using inline keyboards, verify markup format
  - **Check**: `InlineKeyboardMarkup` is real type
  - **Hallucination check**: Not using fake keyboard types

---

## ⚙️ SECTION 9: CONFIGURATION & RISK ENGINE

### File: `src/config.rs`

**Configuration validation:**

- [ ] **Config Fields**
  - Verify all config fields have sensible defaults
  - **Check**: No placeholder values like `0.0` for critical thresholds
  - **Test**: Load config from `.env` and verify all fields populated
  - **Hallucination check**: Not using non-existent config fields

- [ ] **Validation Logic**
  - Verify config validation checks are comprehensive
  - **Check**: Min/max buy amounts, slippage limits, etc. are validated
  - **Test**: Try loading invalid config and verify it's rejected
  - **Hallucination check**: Not validating non-existent fields

### File: `src/risk_engine.rs`

**Risk calculation verification:**

- [ ] **Risk Scoring**
  - Verify risk score calculation is logical
  - **Check**: Scoring factors are reasonable (liquidity, holder count, etc.)
  - **Test**: Calculate risk for known tokens
  - **Hallucination check**: Not using made-up risk factors

- [ ] **Position Sizing**
  - Verify position size calculation is correct
  - **Formula check**: Based on risk score and max position size
  - **Test**: Verify position sizes are within limits
  - **Hallucination check**: Not using incorrect formulas

---

## 🔄 SECTION 10: RETRY & RATE LIMITING

### File: `src/retry.rs`

**Retry logic verification:**

- [ ] **Backoff Strategy**
  - Verify exponential backoff is implemented correctly
  - **Check**: Delay increases exponentially with each retry
  - **Test**: Simulate failures and verify backoff timing
  - **Hallucination check**: Not using incorrect backoff formulas

- [ ] **Error Classification**
  - Verify network errors are correctly identified
  - **Check**: Retryable vs non-retryable errors
  - **Hallucination check**: Not retrying non-retryable errors

### File: `src/rate_limiter.rs`

**Rate limiting verification:**

- [ ] **Token Bucket Algorithm**
  - Verify token bucket implementation is correct
  - **Check**: Tokens refill at correct rate
  - **Test**: Verify rate limiting works as expected
  - **Hallucination check**: Not using incorrect algorithm

---

## 🔍 SECTION 11: PLACEHOLDER & TODO DETECTION

**Search entire codebase for:**

- [ ] **Placeholder Strings**
  ```bash
  grep -r "TODO" src/
  grep -r "FIXME" src/
  grep -r "PLACEHOLDER" src/
  grep -r "XXX" src/
  grep -r "HACK" src/
  grep -r "your-" src/  # Catches "your-api-key", "your-wallet", etc.
  grep -r "example" src/
  grep -r "test" src/ | grep -i "key\|token\|secret"
  ```

- [ ] **Unimplemented Functions**
  ```bash
  grep -r "unimplemented!()" src/
  grep -r "todo!()" src/
  grep -r "panic!(\"not implemented\")" src/
  ```

- [ ] **Debug/Test Code**
  ```bash
  grep -r "println!" src/  # Should use log macros instead
  grep -r "dbg!" src/
  grep -r "#[cfg(test)]" src/  # Ensure test code is properly gated
  ```

- [ ] **Hardcoded Values**
  ```bash
  grep -r "0.001" src/  # Check if SOL amounts are hardcoded
  grep -r "1000000" src/  # Check for hardcoded lamports
  grep -r "http://" src/  # Should use https:// for production
  ```

---

## 🧪 SECTION 12: INTEGRATION TESTING

**Manual testing checklist:**

- [ ] **End-to-End Test: Helius WebSocket**
  - Start bot and verify WebSocket connection establishes
  - Check logs for successful subscription confirmations
  - Wait for real pool creation events
  - Verify events are parsed correctly

- [ ] **End-to-End Test: Token Enrichment**
  - Trigger enrichment for a known token mint
  - Verify metadata is fetched from Helius DAS API
  - Check that all fields are populated correctly
  - Verify social links are extracted

- [ ] **End-to-End Test: Database**
  - Insert test data into all tables
  - Query data and verify retrieval works
  - Test update and delete operations
  - Verify foreign key constraints work

- [ ] **End-to-End Test: GUI**
  - Launch GUI application
  - Test all tabs: Dashboard, Trades, Positions, Config, Logs
  - Verify theme toggle works
  - Test start/stop bot functionality
  - Verify real-time updates display correctly

- [ ] **End-to-End Test: Telegram**
  - Send `/start` command to bot
  - Verify bot responds
  - Test all commands: `/status`, `/positions`, `/config`, etc.
  - Verify alerts are sent correctly

- [ ] **End-to-End Test: Transaction Execution**
  - **WARNING**: Test on devnet first!
  - Verify transaction building works
  - Check transaction signing
  - Verify transaction sending (on devnet)
  - Check transaction confirmation

---

## 🚨 SECTION 13: SECURITY AUDIT

**Critical security checks:**

- [ ] **Private Key Handling**
  - Verify wallet keypair is never logged
  - Check that private keys are not exposed in error messages
  - Ensure wallet file has restricted permissions
  - **Search**: `grep -r "private_key\|secret_key\|keypair" src/`

- [ ] **API Key Exposure**
  - Verify API keys are not logged
  - Check that keys are redacted in logs (line ~265: `ws_url.replace(api_key, "***")`)
  - Ensure `.env` is in `.gitignore`
  - **Search**: `grep -r "api_key\|api-key" src/`

- [ ] **SQL Injection Prevention**
  - Verify all database queries use parameterized queries
  - Check that user input is never concatenated into SQL
  - **Search**: `grep -r "format!\|concat!" src/db.rs`

- [ ] **Input Validation**
  - Verify all user inputs are validated
  - Check for proper error handling
  - Ensure no unsafe unwraps on user input

- [ ] **Dependency Audit**
  ```bash
  cargo audit
  cargo outdated
  ```
  - Check for known vulnerabilities in dependencies
  - Verify all dependencies are from crates.io (not git repos)

---

## 📝 SECTION 14: CODE QUALITY

**Code quality checks:**

- [ ] **Error Handling**
  - Verify all `Result` types are properly handled
  - Check that errors are logged with context
  - Ensure no silent failures (`.ok()` without logging)
  - **Search**: `grep -r "\.unwrap()" src/` (should be minimal)

- [ ] **Logging**
  - Verify appropriate log levels: `error!`, `warn!`, `info!`, `debug!`
  - Check that sensitive data is not logged
  - Ensure logs are structured and parseable

- [ ] **Documentation**
  - Verify all public functions have doc comments
  - Check that complex logic has inline comments
  - Ensure README.md is up-to-date

- [ ] **Type Safety**
  - Verify no excessive use of `Any` or `dyn` types
  - Check that type conversions are safe
  - Ensure no unchecked casts

---

## 🎯 SECTION 15: PERFORMANCE CHECKS

**Performance verification:**

- [ ] **Memory Leaks**
  - Verify no circular references with `Arc`
  - Check that channels are properly closed
  - Ensure WebSocket connections are cleaned up

- [ ] **Connection Pooling**
  - Verify database connection pool is configured
  - Check that HTTP client is reused (not created per request)
  - Ensure WebSocket connections are pooled (line ~193: `WS_POOL_SIZE`)

- [ ] **Async Runtime**
  - Verify Tokio runtime is properly configured
  - Check that blocking operations use `spawn_blocking`
  - Ensure no blocking calls in async context

---

## 📊 ISSUES FOUND

**Document all issues discovered during sanity check:**

### Critical Issues (Must Fix Before Production)
```
1. [File: ] [Line: ] [Issue: ]
   - Description:
   - Fix:

2. 

```

### High Priority Issues (Should Fix Soon)
```
1. 

```

### Medium Priority Issues (Nice to Have)
```
1. 

```

### Low Priority Issues (Future Improvements)
```
1. 

```

---

## ✅ FINAL CHECKLIST

Before deploying to production:

- [ ] All critical issues resolved
- [ ] All high priority issues resolved
- [ ] No placeholders or TODOs in critical paths
- [ ] All API endpoints verified against official documentation
- [ ] All program IDs verified on Solscan
- [ ] Security audit completed
- [ ] Integration tests passed
- [ ] Performance tests passed
- [ ] Documentation updated
- [ ] `.env` file properly configured with real values
- [ ] Wallet funded with sufficient SOL for testing
- [ ] Telegram bot tested and working
- [ ] GUI tested and working
- [ ] Database initialized and tested
- [ ] Helius WebSocket connection tested
- [ ] Transaction execution tested on devnet
- [ ] Rate limiting tested
- [ ] Error handling tested
- [ ] Logs reviewed for sensitive data exposure

---

## 🚀 PRODUCTION DEPLOYMENT CHECKLIST

- [ ] Deploy on dedicated server (not local machine)
- [ ] Set up monitoring and alerting
- [ ] Configure automatic restarts (systemd/supervisor)
- [ ] Set up log rotation
- [ ] Configure firewall rules
- [ ] Set up backup for database
- [ ] Test failover scenarios
- [ ] Document runbook for common issues
- [ ] Set up health check endpoint
- [ ] Configure rate limiting for APIs
- [ ] Test with small position sizes first
- [ ] Monitor for 24 hours before increasing position sizes

---

## 📚 REFERENCE DOCUMENTATION

**Official API Documentation:**
- Helius RPC: https://docs.helius.dev/
- Helius DAS API: https://docs.helius.dev/solana-apis/digital-asset-standard-das-api
- Solana RPC: https://docs.solana.com/api/http
- Solana WebSocket: https://docs.solana.com/api/websocket
- Raydium SDK: https://docs.raydium.io/
- Jito MEV: https://jito-labs.gitbook.io/mev/
- Teloxide: https://docs.rs/teloxide/
- egui: https://docs.rs/egui/

**Blockchain Explorers:**
- Solscan: https://solscan.io/
- Solana Explorer: https://explorer.solana.com/
- Solana Beach: https://solanabeach.io/

---

**Last Updated**: [DATE]
**Reviewed By**: [NAME]
**Status**: [ ] In Progress / [ ] Completed / [ ] Issues Found
