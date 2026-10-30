use axum::{
    extract::{Path, State},
    routing::{get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use crate::{
    db::{ChatMessage, ChatSession},
    llm::{ChatCompletionRequest, ChatMessagePayload, LlmClient},
    AppState,
};

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/sessions", get(list_sessions).post(create_session))
        .route("/sessions/:id/messages", get(list_messages).post(send_message))
}

async fn list_sessions(
    State(state): State<Arc<AppState>>,
) -> Result<Json<Vec<ChatSession>>, StatusCodeResponse> {
    let mut db = crate::db::Database::init("sessions/chats.db").map_err(internal_error)?;
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
    let mut db = crate::db::Database::init("sessions/chats.db").map_err(internal_error)?;
    let id = db
        .create_session(&payload.character_name, &payload.title)
        .map_err(internal_error)?;
    
    Ok(Json(ChatSession {
        id,
        character_name: payload.character_name,
        title: payload.title,
        created_at: "".to_string(),
    }))
}

async fn list_messages(
    State(_state): State<Arc<AppState>>,
    Path(session_id): Path<i64>,
) -> Result<Json<Vec<ChatMessage>>, StatusCodeResponse> {
    let db = crate::db::Database::init("sessions/chats.db").map_err(internal_error)?;
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
    let mut db = crate::db::Database::init("sessions/chats.db").map_err(internal_error)?;

    // 1. Save user message
    db.add_message(session_id, "user", &payload.content)
        .map_err(internal_error)?;

    // 2. Fetch history for context
    let history = db.get_messages(session_id).map_err(internal_error)?;
    let llm_messages: Vec<ChatMessagePayload> = history
        .into_iter()
        .map(|m| ChatMessagePayload {
            role: m.role,
            content: m.content,
        })
        .collect();

    // 3. Call local LLM endpoint
    let llm_client = LlmClient::new(
        state.config.llm.api_base_url.clone(),
        state.config.llm.api_key.clone(),
    );

    let llm_req = ChatCompletionRequest {
        model: state.config.llm.default_model.clone(),
        messages: llm_messages,
        temperature: state.config.llm.temperature,
        top_p: state.config.llm.top_p,
        max_tokens: 512,
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

    // 4. Save assistant response
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

// Helper for error conversion
struct StatusCodeResponse(axum::http::StatusCode, String);
impl axum::response::IntoResponse for StatusCodeResponse {
    fn into_response(self) -> axum::response::Response {
        (self.0, self.1).into_response()
    }
}

fn internal_error<E: std::fmt::Display>(err: E) -> StatusCodeResponse {
    StatusCodeResponse(axum::http::StatusCode::INTERNAL_SERVER_ERROR, err.to_string())
}