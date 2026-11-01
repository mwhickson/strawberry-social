use rusqlite::{params, Connection, Result};
use std::path::Path;
use serde::{Serialize, Deserialize};

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ChatSession {
    pub id: i64,
    pub character_name: String,
    pub character_card_json: Option<String>,
    pub title: String,
    pub created_at: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ChatMessage {
    pub id: Option<i64>,
    pub session_id: i64,
    pub role: String, // "user", "assistant", "system"
    pub content: String,
    pub timestamp: String,
}

pub struct Database {
    conn: Connection,
}

impl Database {
    pub fn init<P: AsRef<Path>>(path: P) -> Result<Self> {
        let conn = Connection::open(path)?;

        conn.execute(
            "CREATE TABLE IF NOT EXISTS sessions (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                character_name TEXT NOT NULL,
                character_card_json TEXT,
                title TEXT NOT NULL,
                created_at DATETIME DEFAULT CURRENT_TIMESTAMP
            )",
            [],
        )?;

        // Ensure legacy DBs get the column added if initialized prior to this change
        let _ = conn.execute(
            "ALTER TABLE sessions ADD COLUMN character_card_json TEXT",
            [],
        );

        conn.execute(
            "CREATE TABLE IF NOT EXISTS messages (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                session_id INTEGER NOT NULL,
                role TEXT NOT NULL,
                content TEXT NOT NULL,
                timestamp DATETIME DEFAULT CURRENT_TIMESTAMP,
                FOREIGN KEY(session_id) REFERENCES sessions(id) ON DELETE CASCADE
            )",
            [],
        )?;

        Ok(Self { conn })
    }

    pub fn create_session(
        &mut self,
        character_name: &str,
        character_card_json: Option<&str>,
        title: &str,
    ) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO sessions (character_name, character_card_json, title) VALUES (?1, ?2, ?3)",
            params![character_name, character_card_json, title],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn get_sessions(&self) -> Result<Vec<ChatSession>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, character_name, character_card_json, title, created_at FROM sessions ORDER BY id DESC"
        )?;
        let session_iter = stmt.query_map([], |row| {
            Ok(ChatSession {
                id: row.get(0)?,
                character_name: row.get(1)?,
                character_card_json: row.get(2)?,
                title: row.get(3)?,
                created_at: row.get(4)?,
            })
        })?;

        let mut sessions = Vec::new();
        for session in session_iter {
            sessions.push(session?);
        }
        Ok(sessions)
    }

    pub fn delete_session(&mut self, session_id: i64) -> Result<()> {
        self.conn.execute("DELETE FROM messages WHERE session_id = ?1", params![session_id])?;
        self.conn.execute("DELETE FROM sessions WHERE id = ?1", params![session_id])?;
        Ok(())
    }

    pub fn add_message(&mut self, session_id: i64, role: &str, content: &str) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO messages (session_id, role, content) VALUES (?1, ?2, ?3)",
            params![session_id, role, content],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn get_messages(&self, session_id: i64) -> Result<Vec<ChatMessage>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, session_id, role, content, timestamp FROM messages WHERE session_id = ?1 ORDER BY id ASC"
        )?;
        let msg_iter = stmt.query_map(params![session_id], |row| {
            Ok(ChatMessage {
                id: Some(row.get(0)?),
                session_id: row.get(1)?,
                role: row.get(2)?,
                content: row.get(3)?,
                timestamp: row.get(4)?,
            })
        })?;

        let mut messages = Vec::new();
        for msg in msg_iter {
            messages.push(msg?);
        }
        Ok(messages)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_database_sessions_and_messages() {
        let mut db = Database::init(":memory:").expect("Failed to initialize in-memory DB");

        let session_id = db
            .create_session("Test Character", Some(r#"{"name":"Test Character"}"#), "First Chat Session")
            .expect("Failed to create session");
        assert!(session_id > 0);

        let sessions = db.get_sessions().expect("Failed to retrieve sessions");
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].character_name, "Test Character");
        assert_eq!(sessions[0].character_card_json, Some(r#"{"name":"Test Character"}"#.to_string()));
        assert_eq!(sessions[0].title, "First Chat Session");

        let msg_id1 = db.add_message(session_id, "user", "Hello there!")
            .expect("Failed to add user message");
        let msg_id2 = db.add_message(session_id, "assistant", "Greetings!")
            .expect("Failed to add assistant message");

        assert!(msg_id1 > 0);
        assert!(msg_id2 > 0);

        let messages = db.get_messages(session_id).expect("Failed to retrieve messages");
        assert_eq!(messages.len(), 2);
    }
}