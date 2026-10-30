use base64::{engine::general_purpose, Engine as _};
use serde::{Deserialize, Serialize};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
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
            let chunk_type = String::from_utf8_lossy(&type_buf);

            let mut chunk_data = vec![0u8; length];
            file.read_exact(&mut chunk_data)?;

            let mut crc_buf = [0u8; 4];
            file.read_exact(&mut crc_buf)?;

            if chunk_type == "tEXt" || chunk_type == "iTXt" {
                let parts: Vec<&[u8]> = chunk_data.splitn(2, |&b| b == 0).collect();
                if parts.len() == 2 {
                    let keyword = String::from_utf8_lossy(parts[0]).trim().to_lowercase();
                    // For iTXt, clean up extra headers if needed; simplified text extraction:
                    let text_bytes = if chunk_type == "iTXt" {
                        let inner_parts: Vec<&[u8]> = parts[1].splitn(4, |&b| b == 0).collect();
                        if inner_parts.len() == 4 { inner_parts[3] } else { parts[1] }
                    } else {
                        parts[1]
                    };
                    let text = String::from_utf8_lossy(text_bytes).to_string();

                    if keyword == "ccv3" {
                        ccv3_data = Some(text);
                    } else if keyword == "chara" {
                        chara_data = Some(text);
                    }
                }
            }

            if chunk_type == "IEND" {
                break;
            }
        }

        let raw_b64 = ccv3_data.or(chara_data)
            .ok_or("No character metadata ('ccv3' or 'chara') found in PNG image.")?;

        let decoded_bytes = general_purpose::STANDARD.decode(raw_b64.trim())?;
        let json_str = String::from_utf8(decoded_bytes)?;
        Self::from_json_str(&json_str)
    }
}