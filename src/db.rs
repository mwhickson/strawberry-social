use rusqlite::{params, Connection, OptionalExtension, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ChatSession {
    pub id: i64,
    pub character_name: String,
    pub character_hash: Option<String>,
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

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct UserProfile {
    pub id: String,
    pub name: String,
    pub description: String,
    pub instructions: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct CharacterSpeechSettings {
    pub voice_uri: String,
    pub rate: f64,
    pub pitch: f64,
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

        conn.execute(
            "CREATE TABLE IF NOT EXISTS user_profiles (
                id TEXT PRIMARY KEY NOT NULL,
                name TEXT NOT NULL,
                description TEXT NOT NULL DEFAULT '',
                instructions TEXT NOT NULL DEFAULT ''
            )",
            [],
        )?;
        conn.execute(
            "CREATE TABLE IF NOT EXISTS app_preferences (
                key TEXT PRIMARY KEY NOT NULL,
                value TEXT NOT NULL
            )",
            [],
        )?;

        conn.execute(
            "CREATE TABLE IF NOT EXISTS character_speech_settings (
                character_hash TEXT PRIMARY KEY NOT NULL,
                voice_uri TEXT NOT NULL DEFAULT '',
                rate REAL NOT NULL DEFAULT 1.0,
                pitch REAL NOT NULL DEFAULT 1.0
            )",
            [],
        )?;

        // Ensure legacy DBs get the column added if initialized prior to this change
        let _ = conn.execute(
            "ALTER TABLE sessions ADD COLUMN character_card_json TEXT",
            [],
        );
        let _ = conn.execute("ALTER TABLE sessions ADD COLUMN character_hash TEXT", []);

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
        character_hash: Option<&str>,
        character_card_json: Option<&str>,
        title: &str,
    ) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO sessions (character_name, character_hash, character_card_json, title) VALUES (?1, ?2, ?3, ?4)",
            params![character_name, character_hash, character_card_json, title],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn get_sessions(&self) -> Result<Vec<ChatSession>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, character_name, character_hash, character_card_json, title, created_at FROM sessions ORDER BY id DESC"
        )?;
        let session_iter = stmt.query_map([], |row| {
            Ok(ChatSession {
                id: row.get(0)?,
                character_name: row.get(1)?,
                character_hash: row.get(2)?,
                character_card_json: row.get(3)?,
                title: row.get(4)?,
                created_at: row.get(5)?,
            })
        })?;

        let mut sessions = Vec::new();
        for session in session_iter {
            sessions.push(session?);
        }
        Ok(sessions)
    }

    pub fn delete_session(&mut self, session_id: i64) -> Result<()> {
        self.conn.execute(
            "DELETE FROM messages WHERE session_id = ?1",
            params![session_id],
        )?;
        self.conn
            .execute("DELETE FROM sessions WHERE id = ?1", params![session_id])?;
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

    pub fn get_profiles(&self) -> Result<Vec<UserProfile>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, name, description, instructions FROM user_profiles ORDER BY name COLLATE NOCASE",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(UserProfile {
                id: row.get(0)?,
                name: row.get(1)?,
                description: row.get(2)?,
                instructions: row.get(3)?,
            })
        })?;
        rows.collect()
    }

    pub fn save_profile(&self, profile: &UserProfile) -> Result<()> {
        self.conn.execute(
            "INSERT INTO user_profiles (id, name, description, instructions) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(id) DO UPDATE SET name = excluded.name, description = excluded.description, instructions = excluded.instructions",
            params![profile.id, profile.name, profile.description, profile.instructions],
        )?;
        Ok(())
    }

    pub fn delete_profile(&self, id: &str) -> Result<()> {
        self.conn
            .execute("DELETE FROM user_profiles WHERE id = ?1", params![id])?;
        self.conn.execute(
            "DELETE FROM app_preferences WHERE key = 'active_profile_id' AND value = ?1",
            params![id],
        )?;
        Ok(())
    }

    pub fn get_preference(&self, key: &str) -> Result<Option<String>> {
        self.conn
            .query_row(
                "SELECT value FROM app_preferences WHERE key = ?1",
                params![key],
                |row| row.get(0),
            )
            .optional()
    }

    pub fn set_preference(&self, key: &str, value: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO app_preferences (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![key, value],
        )?;
        Ok(())
    }

    pub fn get_character_speech_settings(
        &self,
        character_hash: &str,
    ) -> Result<Option<CharacterSpeechSettings>> {
        self.conn
            .query_row(
                "SELECT voice_uri, rate, pitch FROM character_speech_settings WHERE character_hash = ?1",
                params![character_hash],
                |row| Ok(CharacterSpeechSettings { voice_uri: row.get(0)?, rate: row.get(1)?, pitch: row.get(2)? }),
            )
            .optional()
    }

    pub fn save_character_speech_settings(
        &self,
        character_hash: &str,
        settings: &CharacterSpeechSettings,
    ) -> Result<()> {
        self.conn.execute(
            "INSERT INTO character_speech_settings (character_hash, voice_uri, rate, pitch) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(character_hash) DO UPDATE SET voice_uri = excluded.voice_uri, rate = excluded.rate, pitch = excluded.pitch",
            params![character_hash, settings.voice_uri, settings.rate, settings.pitch],
        )?;
        Ok(())
    }

    pub fn delete_character_speech_settings(&self, character_hash: &str) -> Result<()> {
        self.conn.execute(
            "DELETE FROM character_speech_settings WHERE character_hash = ?1",
            params![character_hash],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_database_sessions_and_messages() {
        let mut db = Database::init(":memory:").expect("Failed to initialize in-memory DB");

        let session_id = db
            .create_session(
                "Test Character",
                None,
                Some(r#"{"name":"Test Character"}"#),
                "First Chat Session",
            )
            .expect("Failed to create session");
        assert!(session_id > 0);

        let sessions = db.get_sessions().expect("Failed to retrieve sessions");
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].character_name, "Test Character");
        assert_eq!(
            sessions[0].character_card_json,
            Some(r#"{"name":"Test Character"}"#.to_string())
        );
        assert_eq!(sessions[0].title, "First Chat Session");

        let msg_id1 = db
            .add_message(session_id, "user", "Hello there!")
            .expect("Failed to add user message");
        let msg_id2 = db
            .add_message(session_id, "assistant", "Greetings!")
            .expect("Failed to add assistant message");

        assert!(msg_id1 > 0);
        assert!(msg_id2 > 0);

        let messages = db
            .get_messages(session_id)
            .expect("Failed to retrieve messages");
        assert_eq!(messages.len(), 2);
    }
}
