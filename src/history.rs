use anyhow::{Context, Result};
use rusqlite::Connection;
use std::path::Path;
use tracing::info;

pub struct HistoryStore {
    conn: Connection,
    max_entries: u32,
}

pub struct HistoryEntry {
    pub id: i64,
    pub timestamp: String,
    pub duration_ms: u64,
    pub text: String,
    pub model: String,
    pub language: String,
}

impl HistoryStore {
    pub fn open(db_path: &Path, max_entries: u32) -> Result<Self> {
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("Failed to create history dir: {:?}", parent))?;
        }

        let conn = Connection::open(db_path)
            .with_context(|| format!("Failed to open history database: {:?}", db_path))?;

        conn.execute_batch("PRAGMA journal_mode=WAL;")?;

        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS history (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                timestamp TEXT NOT NULL DEFAULT (datetime('now')),
                duration_ms INTEGER NOT NULL DEFAULT 0,
                text TEXT NOT NULL,
                model TEXT NOT NULL DEFAULT '',
                language TEXT NOT NULL DEFAULT ''
            );",
        )?;

        info!("History database opened at {:?}", db_path);
        Ok(Self { conn, max_entries })
    }

    pub fn save(&self, entry: &HistoryEntry) -> Result<()> {
        self.conn.execute(
            "INSERT INTO history (duration_ms, text, model, language) VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![entry.duration_ms, entry.text, entry.model, entry.language],
        )?;

        self.enforce_retention()?;
        Ok(())
    }

    pub fn list(&self, limit: u32) -> Result<Vec<HistoryEntry>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, timestamp, duration_ms, text, model, language FROM history ORDER BY id DESC LIMIT ?1",
        )?;

        let entries = stmt
            .query_map(rusqlite::params![limit], |row| {
                Ok(HistoryEntry {
                    id: row.get(0)?,
                    timestamp: row.get(1)?,
                    duration_ms: row.get(2)?,
                    text: row.get(3)?,
                    model: row.get(4)?,
                    language: row.get(5)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;

        Ok(entries)
    }

    pub fn clear(&self) -> Result<()> {
        self.conn.execute("DELETE FROM history", [])?;
        info!("History cleared");
        Ok(())
    }

    fn enforce_retention(&self) -> Result<()> {
        self.conn.execute(
            "DELETE FROM history WHERE id NOT IN (SELECT id FROM history ORDER BY id DESC LIMIT ?1)",
            rusqlite::params![self.max_entries],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_history_store_save_and_list() {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test_history.db");

        let store = HistoryStore::open(&db_path, 100).unwrap();

        let entry = HistoryEntry {
            id: 0,
            timestamp: String::new(),
            duration_ms: 5000,
            text: "Test transcription".to_string(),
            model: "base".to_string(),
            language: "en".to_string(),
        };

        store.save(&entry).unwrap();

        let entries = store.list(10).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].text, "Test transcription");
        assert_eq!(entries[0].model, "base");
    }

    #[test]
    fn test_history_store_retention() {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test_history.db");

        let store = HistoryStore::open(&db_path, 3).unwrap();

        // Save 5 entries
        for i in 0..5 {
            let entry = HistoryEntry {
                id: 0,
                timestamp: String::new(),
                duration_ms: 1000,
                text: format!("Entry {}", i),
                model: "base".to_string(),
                language: "en".to_string(),
            };
            store.save(&entry).unwrap();
        }

        // Should only have 3 entries (the most recent)
        let entries = store.list(10).unwrap();
        assert_eq!(entries.len(), 3);
        // Entries should be in reverse order (newest first)
        assert_eq!(entries[0].text, "Entry 4");
        assert_eq!(entries[1].text, "Entry 3");
        assert_eq!(entries[2].text, "Entry 2");
    }

    #[test]
    fn test_history_store_clear() {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test_history.db");

        let store = HistoryStore::open(&db_path, 100).unwrap();

        let entry = HistoryEntry {
            id: 0,
            timestamp: String::new(),
            duration_ms: 5000,
            text: "Test".to_string(),
            model: "base".to_string(),
            language: "en".to_string(),
        };

        store.save(&entry).unwrap();
        assert_eq!(store.list(10).unwrap().len(), 1);

        store.clear().unwrap();
        assert_eq!(store.list(10).unwrap().len(), 0);
    }

    #[test]
    fn test_history_store_multiple_saves() {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test_history.db");

        let store = HistoryStore::open(&db_path, 100).unwrap();

        for i in 0..10 {
            let entry = HistoryEntry {
                id: 0,
                timestamp: String::new(),
                duration_ms: i * 1000,
                text: format!("Entry number {}", i),
                model: "base".to_string(),
                language: "en".to_string(),
            };
            store.save(&entry).unwrap();
        }

        let entries = store.list(5).unwrap();
        assert_eq!(entries.len(), 5);
        // Should get the 5 most recent
        assert_eq!(entries[0].text, "Entry number 9");
        assert_eq!(entries[4].text, "Entry number 5");
    }
}
