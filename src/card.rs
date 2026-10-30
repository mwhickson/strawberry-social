use base64::{engine::general_purpose, Engine as _};
use serde::{Deserialize, Serialize};
use std::fs::File;
use std::io::Read;
use std::path::Path;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct CharacterCard {
    pub spec: Option<String>,
    pub spec_version: Option<String>,
    pub data: CardData,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct CardData {
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub personality: String,
    #[serde(default)]
    pub scenario: String,
    #[serde(default)]
    pub first_mes: String,
    #[serde(default)]
    pub mes_example: String,
    #[serde(default)]
    pub creator_notes: String,
    #[serde(default)]
    pub system_prompt: String,
    #[serde(default)]
    pub post_history_instructions: String,
    #[serde(default)]
    pub alternate_greetings: Vec<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub creator: String,
    #[serde(default)]
    pub character_version: String,
    #[serde(default)]
    pub extensions: serde_json::Value,
    pub character_book: Option<CharacterBook>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct CharacterBook {
    pub name: Option<String>,
    pub description: Option<String>,
    #[serde(default)]
    pub entries: Vec<BookEntry>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct BookEntry {
    #[serde(default)]
    pub keys: Vec<String>,
    #[serde(default)]
    pub content: String,
    #[serde(default)]
    pub comment: Option<String>,
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub insertion_order: i32,
    #[serde(default)]
    pub constant: bool,
}

impl CharacterCard {
    pub fn from_json_str(json_str: &str) -> Result<Self, Box<dyn std::error::Error>> {
        let card: CharacterCard = serde_json::from_str(json_str)?;
        Ok(card)
    }

    pub fn from_file<P: AsRef<Path>>(path: P) -> Result<Self, Box<dyn std::error::Error>> {
        let path = path.as_ref();
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase();

        if ext == "json" {
            let content = std::fs::read_to_string(path)?;
            Self::from_json_str(&content)
        } else if ext == "png" {
            Self::from_png(path)
        } else {
            Err("Unsupported file format. Expected .json or .png".into())
        }
    }

    pub fn from_png<P: AsRef<Path>>(path: P) -> Result<Self, Box<dyn std::error::Error>> {
        let mut file = File::open(path)?;
        let mut signature = [0u8; 8];
        file.read_exact(&mut signature)?;
        if signature != [137, 80, 78, 71, 13, 10, 26, 10] {
            return Err("Not a valid PNG file".into());
        }

        let mut ccv3_data: Option<String> = None;
        let mut chara_data: Option<String> = None;

        loop {
            let mut length_buf = [0u8; 4];
            if file.read_exact(&mut length_buf).is_err() {
                break;
            }
            let length = u32::from_be_bytes(length_buf) as usize;

            let mut type_buf = [0u8; 4];
            file.read_exact(&mut type_buf)?;
            let chunk_type = String::from_utf8_lossy(&type_buf).to_string();

            let mut chunk_data = vec![0u8; length];
            file.read_exact(&mut chunk_data)?;

            let mut crc_buf = [0u8; 4];
            file.read_exact(&mut crc_buf)?;

            if chunk_type == "tEXt" || chunk_type == "iTXt" {
                // Log found text chunks to help debug metadata placement
                tracing::info!("🔍 Found PNG text chunk [{}] of length {} bytes", chunk_type, length);

                // tEXt format: keyword \0 text
                // iTXt format: keyword \0 compression_flag compression_method language_tag \0 translated_keyword \0 text
                if chunk_type == "tEXt" {
                    if let Some(pos) = chunk_data.iter().position(|&b| b == 0) {
                        let keyword = String::from_utf8_lossy(&chunk_data[..pos]).trim().to_lowercase();
                        let text = String::from_utf8_lossy(&chunk_data[pos + 1..]).to_string();
                        tracing::info!("   -> tEXt keyword: '{}'", keyword);
                        if keyword == "ccv3" {
                            ccv3_data = Some(text);
                        } else if keyword == "chara" {
                            chara_data = Some(text);
                        }
                    }
                } else if chunk_type == "iTXt" {
                    // Find first null terminator for keyword
                    if let Some(pos1) = chunk_data.iter().position(|&b| b == 0) {
                        let keyword = String::from_utf8_lossy(&chunk_data[..pos1]).trim().to_lowercase();
                        tracing::info!("   -> iTXt keyword: '{}'", keyword);

                        // Skip past null terminator, compression flag (1), compression method (1), and language tag until next nulls
                        let remainder = &chunk_data[pos1 + 1..];
                        if remainder.len() >= 2 {
                            // Find the 3rd null byte which precedes the text in iTXt format
                            let mut null_counts = 0;
                            let mut text_start = 0;
                            for (idx, &b) in remainder.iter().enumerate() {
                                if b == 0 {
                                    null_counts += 1;
                                    if null_counts == 3 {
                                        text_start = idx + 1;
                                        break;
                                    }
                                }
                            }
                            if text_start > 0 && text_start < remainder.len() {
                                let text = String::from_utf8_lossy(&remainder[text_start..]).to_string();
                                if keyword == "ccv3" {
                                    ccv3_data = Some(text);
                                } else if keyword == "chara" {
                                    chara_data = Some(text);
                                }
                            }
                        }
                    }
                }
            }

            if chunk_type == "IEND" {
                break;
            }
        }

        let raw_b64 = ccv3_data.or(chara_data)
            .ok_or("No character metadata ('ccv3' or 'chara') found in PNG image.")?;

        // Clean up any extraneous whitespace or newlines in base64 string
        let cleaned_b64: String = raw_b64.chars().filter(|c| !c.is_whitespace()).collect();

        let decoded_bytes = general_purpose::STANDARD.decode(&cleaned_b64)?;
        let json_str = String::from_utf8(decoded_bytes)?;
        Self::from_json_str(&json_str)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_JSON_CARD: &str = r#"{
        "spec": "chara_card_v2",
        "spec_version": "2.0",
        "data": {
            "name": "Test Character",
            "description": "A brave test hero.",
            "personality": "Bold, curious, helpful.",
            "scenario": "Testing environment.",
            "first_mes": "Hello! Ready for testing?",
            "creator": "Developer"
        }
    }"#;

    #[test]
    fn test_parse_valid_json_card() {
        let card = CharacterCard::from_json_str(SAMPLE_JSON_CARD);
        assert!(card.is_ok(), "Failed to parse valid V2 JSON card");
        let card = card.unwrap();
        assert_eq!(card.data.name, "Test Character");
        assert_eq!(card.data.description, "A brave test hero.");
        assert_eq!(card.spec.as_deref(), Some("chara_card_v2"));
    }

    #[test]
    fn test_parse_invalid_json() {
        let invalid_json = "{ invalid json data }";
        let card = CharacterCard::from_json_str(invalid_json);
        assert!(card.is_err(), "Expected error when parsing invalid JSON");
    }
}
