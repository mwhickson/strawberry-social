use axum::{
    extract::{Path, State},
    routing::get,
    Json, Router,
};
use rand::seq::SliceRandom;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use crate::{
    card::CharacterCard,
    db::{ChatMessage, ChatSession},
    llm::{ChatCompletionRequest, ChatMessagePayload, LlmClient},
    AppState,
};

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/sessions", get(list_sessions).post(create_session))
        .route("/sessions/:id", axum::routing::delete(delete_session))
        .route("/sessions/:id/messages", get(list_messages).post(send_message))
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
}

async fn create_session(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<CreateSessionPayload>,
) -> Result<Json<ChatSession>, StatusCodeResponse> {
    let db_path = state.data_dir.join("sessions/chats.db");
    let mut db = crate::db::Database::init(&db_path).map_err(internal_error)?;
    let session_id = db
        .create_session(&payload.character_name, &payload.title)
        .map_err(internal_error)?;

    // Try to find the character card file to extract its first message / greeting
    let card = find_character_card(&state, &payload.character_name);

    if let Some(c) = card {
        let data = c.data;

        let mut possible_greetings = vec![data.first_mes.clone()];
        for alt in &data.alternate_greetings {
            if !alt.is_empty() {
                possible_greetings.push(alt.clone());
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
        title: payload.title,
        created_at: "".to_string(),
    }))
}

// Helper to locate a character card by name in the characters directory
fn find_character_card(state: &AppState, name: &str) -> Option<CharacterCard> {
    let root_dir = state.data_dir.join("characters");
    let mut found_card = None;

    fn search_dir(dir: &std::path::Path, target_name: &str, result: &mut Option<CharacterCard>) {
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    search_dir(&path, target_name, result);
                } else if path.is_file() {
                    if let Ok(card) = CharacterCard::from_file(&path) {
                        if card.data.name.eq_ignore_ascii_case(target_name) {
                            *result = Some(card);
                            return;
                        }
                    }
                }
            }
        }
    }

    search_dir(&root_dir, name, &mut found_card);
    found_card
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
    let session = sessions.iter().find(|s| s.id == session_id)
        .ok_or_else(|| StatusCodeResponse(axum::http::StatusCode::NOT_FOUND, "Session not found".into()))?;

    let character_name = session.character_name.clone();

    db.add_message(session_id, "user", &payload.content)
        .map_err(internal_error)?;

    let history = db.get_messages(session_id).map_err(internal_error)?;

    let card = find_character_card(&state, &character_name);
    let mut llm_messages = Vec::new();

    if let Some(c) = card {
        let d = c.data;
        let system_instruction = format!(
            "You are roleplaying as {}. \
            Description: {} \
            Personality: {} \
            Scenario: {} \
            Instructions: Stay strictly in character. Never break character or refer to yourself as an AI.",
            d.name, d.description, d.personality, d.scenario
        );
        llm_messages.push(ChatMessagePayload {
            role: "system".to_string(),
            content: system_instruction,
        });
    }

    for m in history {
        llm_messages.push(ChatMessagePayload {
            role: m.role,
            content: m.content,
        });
    }

    let llm_client = LlmClient::new(
        state.config.llm.api_base_url.clone(),
        state.config.llm.api_key.clone(),
    );

    let llm_req = ChatCompletionRequest {
        model: state.config.llm.default_model.clone(),
        messages: llm_messages,
        temperature: state.config.llm.temperature,
        top_p: state.config.llm.top_p,
        max_tokens: std::cmp::min(512, state.config.llm.context_window),
        stream: false, //state.config.ui.stream_responses,
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
    StatusCodeResponse(axum::http::StatusCode::INTERNAL_SERVER_ERROR, err.to_string())
}
