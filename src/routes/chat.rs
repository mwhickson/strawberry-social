use crate::{
    card::CharacterCard,
    db::{ChatMessage, ChatSession},
    llm::{ChatCompletionRequest, ChatMessagePayload, LlmClient},
    AppState,
};
use axum::{
    extract::{Path, State},
    routing::get,
    Json, Router,
};
use base64::{engine::general_purpose, Engine as _};
use rand::seq::SliceRandom;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/sessions", get(list_sessions).post(create_session))
        .route("/sessions/:id", axum::routing::delete(delete_session))
        .route(
            "/sessions/:id/messages",
            get(list_messages).post(send_message),
        )
}

async fn list_sessions(
    State(state): State<Arc<AppState>>,
) -> Result<Json<Vec<ChatSession>>, StatusCodeResponse> {
    let db_path = state.data_dir.join("sessions/chats.db");
    let db = crate::db::Database::init(&db_path).map_err(internal_error)?;
    let sessions = db.get_sessions().map_err(internal_error)?;
    Ok(Json(sessions))
}

#[derive(Deserialize)]
struct CreateSessionPayload {
    character_name: String,
    title: String,
    character_hash: Option<String>,
    user_profile: Option<UserChatProfile>,
}

async fn create_session(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<CreateSessionPayload>,
) -> Result<Json<ChatSession>, StatusCodeResponse> {
    let db_path = state.data_dir.join("sessions/chats.db");
    let mut db = crate::db::Database::init(&db_path).map_err(internal_error)?;

    // Locate the character file and extract both card data and avatar snapshot if PNG
    let (card, card_json, found_hash) = find_character_card_and_snapshot(
        &state,
        Some(&payload.character_name),
        payload.character_hash.as_deref(),
    );
    let character_hash = payload.character_hash.clone().or(found_hash);

    let session_id = db
        .create_session(
            &payload.character_name,
            character_hash.as_deref(),
            card_json.as_deref(),
            &payload.title,
        )
        .map_err(internal_error)?;

    if let Some(ref c) = card {
        let data = &c.data;

        let mut possible_greetings = vec![expand_template(
            &data.first_mes,
            Some(data),
            &data.name,
            payload.user_profile.as_ref(),
        )];
        for alt in &data.alternate_greetings {
            if !alt.is_empty() {
                possible_greetings.push(expand_template(
                    alt,
                    Some(data),
                    &data.name,
                    payload.user_profile.as_ref(),
                ));
            }
        }

        let chosen_greeting = possible_greetings
            .choose(&mut rand::thread_rng())
            .cloned()
            .unwrap_or_else(|| "Hello!".to_string());

        db.add_message(session_id, "assistant", &chosen_greeting)
            .map_err(internal_error)?;
    }

    Ok(Json(ChatSession {
        id: session_id,
        character_name: payload.character_name,
        character_hash,
        character_card_json: card_json,
        title: payload.title,
        created_at: "".to_string(),
    }))
}

// Helper to locate a character card by name and build a JSON snapshot including base64 avatar if PNG
fn find_character_card_and_snapshot(
    state: &AppState,
    name: Option<&str>,
    target_hash: Option<&str>,
) -> (Option<CharacterCard>, Option<String>, Option<String>) {
    let root_dir = state.data_dir.join("characters");
    let mut found_card = None;
    let mut card_json_snapshot = None;
    let mut found_hash = None;

    fn search_dir(
        dir: &std::path::Path,
        target_name: Option<&str>,
        target_hash: Option<&str>,
        result_card: &mut Option<CharacterCard>,
        result_json: &mut Option<String>,
        result_hash: &mut Option<String>,
    ) {
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    search_dir(
                        &path,
                        target_name,
                        target_hash,
                        result_card,
                        result_json,
                        result_hash,
                    );
                } else if path.is_file() {
                    let ext = path
                        .extension()
                        .and_then(|e| e.to_str())
                        .unwrap_or("")
                        .to_lowercase();

                    if ext == "json" || ext == "png" {
                        let source_hash = std::fs::read(&path)
                            .ok()
                            .map(|bytes| crate::card::content_hash(&bytes));
                        if let Ok(card) = CharacterCard::from_file(&path) {
                            let matches = target_hash
                                .map(|hash| source_hash.as_deref() == Some(hash))
                                .unwrap_or_else(|| {
                                    target_name.is_some_and(|name| {
                                        card.data.name.eq_ignore_ascii_case(name)
                                    })
                                });
                            if matches {
                                // Determine avatar URL/data depending on whether it's a PNG chunk or raw JSON
                                let avatar_url = if ext == "png" {
                                    match std::fs::read(&path) {
                                        Ok(bytes) => Some(format!(
                                            "data:image/png;base64,{}",
                                            general_purpose::STANDARD.encode(&bytes)
                                        )),
                                        Err(_) => None,
                                    }
                                } else {
                                    let png_path = path.with_extension("png");
                                    if png_path.exists() {
                                        match std::fs::read(&png_path) {
                                            Ok(bytes) => Some(format!(
                                                "data:image/png;base64,{}",
                                                general_purpose::STANDARD.encode(&bytes)
                                            )),
                                            Err(_) => None,
                                        }
                                    } else {
                                        None
                                    }
                                };

                                // Construct a serializable view dynamically using serde_json::json!
                                let mut val = match serde_json::to_value(&card.data) {
                                    Ok(v) => v,
                                    Err(_) => continue,
                                };

                                if let Some(obj) = val.as_object_mut() {
                                    obj.insert(
                                        "avatar_url".to_string(),
                                        serde_json::json!(avatar_url),
                                    );
                                }

                                *result_json = serde_json::to_string(&val).ok();
                                *result_card = Some(card);
                                *result_hash = source_hash;
                                return;
                            }
                        }
                    }
                }
            }
        }
    }

    search_dir(
        &root_dir,
        name,
        target_hash,
        &mut found_card,
        &mut card_json_snapshot,
        &mut found_hash,
    );
    (found_card, card_json_snapshot, found_hash)
}

// Keep a standard find_character_card helper for message sending context lookups
fn find_character_card(state: &AppState, name: &str) -> Option<CharacterCard> {
    let (card, _, _) = find_character_card_and_snapshot(state, Some(name), None);
    card
}

fn find_character_card_by_hash(state: &AppState, hash: &str) -> Option<CharacterCard> {
    let (card, _, _) = find_character_card_and_snapshot(state, None, Some(hash));
    card
}

fn parse_card_snapshot(snapshot: &str) -> Option<crate::card::CardData> {
    serde_json::from_str(snapshot).ok().or_else(|| {
        serde_json::from_str::<CharacterCard>(snapshot)
            .ok()
            .map(|card| card.data)
    })
}

fn expand_template(
    text: &str,
    card: Option<&crate::card::CardData>,
    character_name: &str,
    profile: Option<&UserChatProfile>,
) -> String {
    let mut expanded = text.to_string();
    // Resolve nested field values such as {{charFirstMessage}} containing {{user}},
    // while bounding expansion so circular card references cannot loop forever.
    for _ in 0..4 {
        let next = expand_template_once(&expanded, card, character_name, profile);
        if next == expanded {
            break;
        }
        expanded = next;
    }
    expanded
}

fn expand_template_once(
    text: &str,
    card: Option<&crate::card::CardData>,
    character_name: &str,
    profile: Option<&UserChatProfile>,
) -> String {
    let mut output = String::with_capacity(text.len());
    let mut remaining = text;
    while let Some(start) = remaining.find("{{") {
        output.push_str(&remaining[..start]);
        let after_open = &remaining[start + 2..];
        let Some(end) = after_open.find("}}") else {
            output.push_str(&remaining[start..]);
            return output;
        };
        let original = &remaining[start..start + 2 + end + 2];
        let token = after_open[..end].trim().to_ascii_lowercase();
        let replacement = match token.as_str() {
            "user" => Some(profile.map_or("User", |value| value.name.as_str())),
            "char" | "character" => Some(card.map_or(character_name, |value| value.name.as_str())),
            "persona" => Some(profile.map_or("", |value| value.description.as_str())),
            "description" => card.map(|value| value.description.as_str()),
            "personality" => card.map(|value| value.personality.as_str()),
            "scenario" => card.map(|value| value.scenario.as_str()),
            "system" | "charprompt" => card.map(|value| value.system_prompt.as_str()),
            "charinstruction" => card.map(|value| value.post_history_instructions.as_str()),
            "charcreatornotes" => card.map(|value| value.creator_notes.as_str()),
            "charversion" => card.map(|value| value.character_version.as_str()),
            "charfirstmessage" => card.map(|value| value.first_mes.as_str()),
            "mesexamples" | "mesexamplesraw" => card.map(|value| value.mes_example.as_str()),
            _ => None,
        };
        if let Some(value) = replacement {
            output.push_str(value);
        } else {
            output.push_str(original);
        }
        remaining = &after_open[end + 2..];
    }
    output.push_str(remaining);
    output
}

async fn list_messages(
    State(state): State<Arc<AppState>>,
    Path(session_id): Path<i64>,
) -> Result<Json<Vec<ChatMessage>>, StatusCodeResponse> {
    let db_path = state.data_dir.join("sessions/chats.db");
    let db = crate::db::Database::init(&db_path).map_err(internal_error)?;
    let messages = db.get_messages(session_id).map_err(internal_error)?;
    Ok(Json(messages))
}

#[derive(Deserialize)]
struct SendMessagePayload {
    content: String,
    user_profile: Option<UserChatProfile>,
}

#[derive(Deserialize)]
struct UserChatProfile {
    name: String,
    description: String,
    instructions: String,
}

#[derive(Serialize)]
struct SendMessageResponse {
    user_message: ChatMessage,
    assistant_message: ChatMessage,
}

async fn send_message(
    State(state): State<Arc<AppState>>,
    Path(session_id): Path<i64>,
    Json(payload): Json<SendMessagePayload>,
) -> Result<Json<SendMessageResponse>, StatusCodeResponse> {
    let db_path = state.data_dir.join("sessions/chats.db");
    let mut db = crate::db::Database::init(&db_path).map_err(internal_error)?;

    let sessions = db.get_sessions().map_err(internal_error)?;
    let session = sessions
        .iter()
        .find(|s| s.id == session_id)
        .ok_or_else(|| {
            StatusCodeResponse(
                axum::http::StatusCode::NOT_FOUND,
                "Session not found".into(),
            )
        })?;

    let character_name = session.character_name.clone();
    let character_hash = session.character_hash.clone();
    let card_snapshot = session.character_card_json.clone();

    db.add_message(session_id, "user", &payload.content)
        .map_err(internal_error)?;

    let history = db.get_messages(session_id).map_err(internal_error)?;

    let card_data = card_snapshot
        .as_deref()
        .and_then(parse_card_snapshot)
        .or_else(|| {
            let card = if let Some(hash) = character_hash.as_deref() {
                find_character_card_by_hash(&state, hash)
            } else {
                find_character_card(&state, &character_name)
            }?;
            Some(card.data)
        });
    let mut llm_messages = Vec::new();

    let mut system_parts = Vec::new();
    if let Some(d) = card_data.as_ref() {
        system_parts.push(format!(
            "You are roleplaying as {}. \
            Description: {} \
            Personality: {} \
            Scenario: {} \
            Instructions: Stay strictly in character. Never break character or refer to yourself as an AI.",
            expand_template(&d.name, Some(d), &character_name, payload.user_profile.as_ref()),
            expand_template(&d.description, Some(d), &character_name, payload.user_profile.as_ref()),
            expand_template(&d.personality, Some(d), &character_name, payload.user_profile.as_ref()),
            expand_template(&d.scenario, Some(d), &character_name, payload.user_profile.as_ref())
        ));
        if !d.system_prompt.trim().is_empty() {
            system_parts.push(expand_template(
                &d.system_prompt,
                Some(d),
                &character_name,
                payload.user_profile.as_ref(),
            ));
        }
        if !d.post_history_instructions.trim().is_empty() {
            system_parts.push(expand_template(
                &d.post_history_instructions,
                Some(d),
                &character_name,
                payload.user_profile.as_ref(),
            ));
        }
    }
    if let Some(profile) = payload.user_profile.as_ref() {
        let profile_context = format!(
            "The user's chat profile is named '{}'. About the user: {}. User-provided chat instructions: {}. Treat this as context about the user's persona; do not impersonate the user.",
            profile.name,
            expand_template(&profile.description, card_data.as_ref(), &character_name, Some(profile)),
            expand_template(&profile.instructions, card_data.as_ref(), &character_name, Some(profile))
        );
        let profile_context = expand_template(
            &profile_context,
            card_data.as_ref(),
            &character_name,
            Some(profile),
        );
        system_parts.push(profile_context);
    }
    if !system_parts.is_empty() {
        llm_messages.push(ChatMessagePayload {
            role: "system".to_string(),
            content: system_parts.join("\n\n"),
        });
    }

    for m in history {
        llm_messages.push(ChatMessagePayload {
            role: m.role,
            content: m.content,
        });
    }

    let generation = *state.generation.read().await;
    let llm_client = LlmClient::new(
        state.config.llm.api_base_url.clone(),
        state.config.llm.api_key.clone(),
    );

    let llm_req = ChatCompletionRequest {
        model: state.config.llm.default_model.clone(),
        messages: llm_messages,
        temperature: generation.temperature,
        top_p: generation.top_p,
        max_tokens: std::cmp::min(512, generation.context_window),
        stream: false,
    };

    let resp = llm_client
        .send_chat_completion(&llm_req)
        .await
        .map_err(|e| StatusCodeResponse(axum::http::StatusCode::BAD_GATEWAY, e.to_string()))?;

    #[derive(Deserialize)]
    struct OpenAIResponse {
        choices: Vec<OpenAIChoice>,
    }
    #[derive(Deserialize)]
    struct OpenAIChoice {
        message: ChatMessagePayload,
    }

    let completion: OpenAIResponse = resp.json().await.map_err(internal_error)?;
    let assistant_content = completion
        .choices
        .first()
        .map(|c| c.message.content.clone())
        .unwrap_or_else(|| "(No response generated)".to_string());
    let assistant_content = expand_template(
        &assistant_content,
        card_data.as_ref(),
        &character_name,
        payload.user_profile.as_ref(),
    );

    db.add_message(session_id, "assistant", &assistant_content)
        .map_err(internal_error)?;

    let user_msg = ChatMessage {
        id: None,
        session_id,
        role: "user".to_string(),
        content: payload.content,
        timestamp: "".to_string(),
    };
    let assistant_msg = ChatMessage {
        id: None,
        session_id,
        role: "assistant".to_string(),
        content: assistant_content,
        timestamp: "".to_string(),
    };

    Ok(Json(SendMessageResponse {
        user_message: user_msg,
        assistant_message: assistant_msg,
    }))
}

async fn delete_session(
    State(state): State<Arc<AppState>>,
    Path(session_id): Path<i64>,
) -> Result<axum::http::StatusCode, StatusCodeResponse> {
    let db_path = state.data_dir.join("sessions/chats.db");
    let mut db = crate::db::Database::init(&db_path).map_err(internal_error)?;
    db.delete_session(session_id).map_err(internal_error)?;
    Ok(axum::http::StatusCode::OK)
}

struct StatusCodeResponse(axum::http::StatusCode, String);
impl axum::response::IntoResponse for StatusCodeResponse {
    fn into_response(self) -> axum::response::Response {
        (self.0, self.1).into_response()
    }
}

fn internal_error<E: std::fmt::Display>(err: E) -> StatusCodeResponse {
    StatusCodeResponse(
        axum::http::StatusCode::INTERNAL_SERVER_ERROR,
        err.to_string(),
    )
}
