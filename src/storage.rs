use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RequestLog {
    pub id: Option<i64>,
    pub session_id: String,
    pub timestamp: DateTime<Utc>,
    pub provider: String,
    pub model: String,
    pub endpoint: String,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cached_tokens: i64,
    pub latency_ms: i64,
    pub cost_usd: f64,
    pub status_code: u16,
    pub is_stream: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub session_id: String,
    pub label: Option<String>,
    pub started_at: DateTime<Utc>,
    pub ended_at: Option<DateTime<Utc>>,
    pub pid: u32,
}

pub struct Database {
    conn: Connection,
}

fn db_path() -> Result<PathBuf> {
    let data_dir = dirs::data_dir()
        .context("Could not determine data directory")?
        .join("tokmon");
    std::fs::create_dir_all(&data_dir)?;
    Ok(data_dir.join("tokmon.db"))
}

impl Database {
    pub fn open() -> Result<Self> {
        let path = db_path()?;
        let conn = Connection::open(&path)
            .with_context(|| format!("Failed to open database at {}", path.display()))?;

        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")?;

        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS sessions (
                session_id TEXT PRIMARY KEY,
                label TEXT,
                started_at TEXT NOT NULL,
                ended_at TEXT,
                pid INTEGER NOT NULL
            );

            CREATE TABLE IF NOT EXISTS requests (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                session_id TEXT NOT NULL REFERENCES sessions(session_id),
                timestamp TEXT NOT NULL,
                provider TEXT NOT NULL,
                model TEXT NOT NULL,
                endpoint TEXT NOT NULL,
                input_tokens INTEGER NOT NULL DEFAULT 0,
                output_tokens INTEGER NOT NULL DEFAULT 0,
                cached_tokens INTEGER NOT NULL DEFAULT 0,
                latency_ms INTEGER NOT NULL DEFAULT 0,
                cost_usd REAL NOT NULL DEFAULT 0.0,
                status_code INTEGER NOT NULL DEFAULT 0,
                is_stream INTEGER NOT NULL DEFAULT 0
            );

            CREATE TABLE IF NOT EXISTS config (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL
            );

            CREATE INDEX IF NOT EXISTS idx_requests_session ON requests(session_id);
            CREATE INDEX IF NOT EXISTS idx_requests_timestamp ON requests(timestamp);",
        )?;

        Ok(Database { conn })
    }

    pub fn create_session(&self, session: &Session) -> Result<()> {
        self.conn.execute(
            "INSERT INTO sessions (session_id, label, started_at, pid)
             VALUES (?1, ?2, ?3, ?4)",
            params![
                session.session_id,
                session.label,
                session.started_at.to_rfc3339(),
                session.pid,
            ],
        )?;
        Ok(())
    }

    pub fn end_session(&self, session_id: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE sessions SET ended_at = ?1 WHERE session_id = ?2",
            params![Utc::now().to_rfc3339(), session_id],
        )?;
        Ok(())
    }

    pub fn log_request(&self, req: &RequestLog) -> Result<()> {
        self.conn.execute(
            "INSERT INTO requests (session_id, timestamp, provider, model, endpoint,
             input_tokens, output_tokens, cached_tokens, latency_ms, cost_usd, status_code, is_stream)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            params![
                req.session_id,
                req.timestamp.to_rfc3339(),
                req.provider,
                req.model,
                req.endpoint,
                req.input_tokens,
                req.output_tokens,
                req.cached_tokens,
                req.latency_ms,
                req.cost_usd,
                req.status_code,
                req.is_stream,
            ],
        )?;
        Ok(())
    }

    pub fn set_budget(&self, amount: f64) -> Result<()> {
        self.conn.execute(
            "INSERT OR REPLACE INTO config (key, value) VALUES ('daily_budget', ?1)",
            params![amount.to_string()],
        )?;
        Ok(())
    }

    pub fn get_budget(&self) -> Result<Option<f64>> {
        let mut stmt = self
            .conn
            .prepare("SELECT value FROM config WHERE key = 'daily_budget'")?;
        let result = stmt
            .query_row([], |row| row.get::<_, String>(0))
            .ok()
            .and_then(|v| v.parse::<f64>().ok());
        Ok(result)
    }

    pub fn get_requests_since(&self, since: DateTime<Utc>) -> Result<Vec<RequestLog>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, session_id, timestamp, provider, model, endpoint,
                    input_tokens, output_tokens, cached_tokens, latency_ms, cost_usd, status_code, is_stream
             FROM requests WHERE timestamp >= ?1 ORDER BY timestamp DESC",
        )?;

        let rows = stmt.query_map(params![since.to_rfc3339()], |row| {
            Ok(RequestLog {
                id: Some(row.get(0)?),
                session_id: row.get(1)?,
                timestamp: row
                    .get::<_, String>(2)?
                    .parse::<DateTime<Utc>>()
                    .unwrap_or_default(),
                provider: row.get(3)?,
                model: row.get(4)?,
                endpoint: row.get(5)?,
                input_tokens: row.get(6)?,
                output_tokens: row.get(7)?,
                cached_tokens: row.get(8)?,
                latency_ms: row.get(9)?,
                cost_usd: row.get(10)?,
                status_code: row.get(11)?,
                is_stream: row.get::<_, bool>(12)?,
            })
        })?;

        let mut requests = Vec::new();
        for row in rows {
            requests.push(row?);
        }
        Ok(requests)
    }

    pub fn get_recent_requests(&self, count: usize) -> Result<Vec<RequestLog>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, session_id, timestamp, provider, model, endpoint,
                    input_tokens, output_tokens, cached_tokens, latency_ms, cost_usd, status_code, is_stream
             FROM requests ORDER BY timestamp DESC LIMIT ?1",
        )?;

        let rows = stmt.query_map(params![count as i64], |row| {
            Ok(RequestLog {
                id: Some(row.get(0)?),
                session_id: row.get(1)?,
                timestamp: row
                    .get::<_, String>(2)?
                    .parse::<DateTime<Utc>>()
                    .unwrap_or_default(),
                provider: row.get(3)?,
                model: row.get(4)?,
                endpoint: row.get(5)?,
                input_tokens: row.get(6)?,
                output_tokens: row.get(7)?,
                cached_tokens: row.get(8)?,
                latency_ms: row.get(9)?,
                cost_usd: row.get(10)?,
                status_code: row.get(11)?,
                is_stream: row.get::<_, bool>(12)?,
            })
        })?;

        let mut requests = Vec::new();
        for row in rows {
            requests.push(row?);
        }
        Ok(requests)
    }

    pub fn get_sessions_since(&self, since: DateTime<Utc>) -> Result<Vec<Session>> {
        let mut stmt = self.conn.prepare(
            "SELECT session_id, label, started_at, ended_at, pid
             FROM sessions WHERE started_at >= ?1 ORDER BY started_at DESC",
        )?;

        let rows = stmt.query_map(params![since.to_rfc3339()], |row| {
            Ok(Session {
                session_id: row.get(0)?,
                label: row.get(1)?,
                started_at: row
                    .get::<_, String>(2)?
                    .parse::<DateTime<Utc>>()
                    .unwrap_or_default(),
                ended_at: row
                    .get::<_, Option<String>>(3)?
                    .and_then(|s| s.parse::<DateTime<Utc>>().ok()),
                pid: row.get(4)?,
            })
        })?;

        let mut sessions = Vec::new();
        for row in rows {
            sessions.push(row?);
        }
        Ok(sessions)
    }

    pub fn get_today_cost(&self) -> Result<f64> {
        let today = Utc::now()
            .date_naive()
            .and_hms_opt(0, 0, 0)
            .unwrap();
        let today_utc: DateTime<Utc> = DateTime::from_naive_utc_and_offset(today, Utc);

        let cost: f64 = self
            .conn
            .query_row(
                "SELECT COALESCE(SUM(cost_usd), 0.0) FROM requests WHERE timestamp >= ?1",
                params![today_utc.to_rfc3339()],
                |row| row.get(0),
            )?;
        Ok(cost)
    }

    pub fn get_session_summary(&self, session_id: &str) -> Result<SessionSummary> {
        let row = self.conn.query_row(
            "SELECT COUNT(*), COALESCE(SUM(input_tokens), 0), COALESCE(SUM(output_tokens), 0),
                    COALESCE(SUM(cached_tokens), 0), COALESCE(SUM(cost_usd), 0.0),
                    COALESCE(AVG(latency_ms), 0)
             FROM requests WHERE session_id = ?1",
            params![session_id],
            |row| {
                Ok(SessionSummary {
                    request_count: row.get(0)?,
                    total_input_tokens: row.get(1)?,
                    total_output_tokens: row.get(2)?,
                    total_cached_tokens: row.get(3)?,
                    total_cost: row.get(4)?,
                    avg_latency_ms: row.get(5)?,
                })
            },
        )?;
        Ok(row)
    }
}

#[derive(Debug, Clone)]
pub struct SessionSummary {
    pub request_count: i64,
    pub total_input_tokens: i64,
    pub total_output_tokens: i64,
    pub total_cached_tokens: i64,
    pub total_cost: f64,
    pub avg_latency_ms: f64,
}
