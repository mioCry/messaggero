use std::sync::Arc;

use crate::core::jsonrpc::{
    JsonRpcRequest, JsonRpcResponse, INTERNAL_ERROR, INVALID_PARAMS, INVALID_REQUEST,
    JSONRPC_VERSION, METHOD_AGENT_CARD, METHOD_NOT_FOUND, METHOD_TASKS_CANCEL, METHOD_TASKS_GET,
    METHOD_TASKS_SEND, TASK_NOT_FOUND,
};
use crate::core::{Agent, TaskRequest, TransportError};
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use tracing::info;

struct AppState {
    agent: Arc<dyn Agent>,
}

/// Serve an agent over HTTP using the A2A-compatible JSON-RPC protocol.
pub async fn serve(agent: Arc<dyn Agent>, addr: impl Into<String>) -> Result<(), TransportError> {
    let addr = addr.into();
    let state = Arc::new(AppState { agent });

    let app = Router::new()
        .route("/.well-known/agent.json", get(agent_card_handler))
        .route("/", post(jsonrpc_handler))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .map_err(TransportError::Io)?;

    info!(addr = %addr, "A2A HTTP transport listening");

    axum::serve(listener, app)
        .await
        .map_err(|e| TransportError::Connection(e.to_string()))
}

async fn agent_card_handler(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let card = state.agent.card();
    Json(card)
}

async fn jsonrpc_handler(
    State(state): State<Arc<AppState>>,
    Json(request): Json<JsonRpcRequest>,
) -> impl IntoResponse {
    if request.jsonrpc != JSONRPC_VERSION {
        return (
            StatusCode::OK,
            Json(JsonRpcResponse::error(
                request.id,
                INVALID_REQUEST,
                "invalid JSON-RPC version",
            )),
        );
    }

    let response = match request.method.as_str() {
        METHOD_TASKS_SEND => handle_tasks_send(&state, &request).await,
        METHOD_TASKS_GET => handle_tasks_get(&request),
        METHOD_TASKS_CANCEL => handle_tasks_cancel(&state, &request).await,
        METHOD_AGENT_CARD => handle_agent_card(&state, &request),
        _ => JsonRpcResponse::error(
            request.id,
            METHOD_NOT_FOUND,
            format!("method not found: {}", request.method),
        ),
    };

    (StatusCode::OK, Json(response))
}

async fn handle_tasks_send(state: &AppState, request: &JsonRpcRequest) -> JsonRpcResponse {
    let Some(params) = &request.params else {
        return JsonRpcResponse::error(request.id.clone(), INVALID_PARAMS, "missing params");
    };

    let task_request: TaskRequest = match serde_json::from_value(params.clone()) {
        Ok(r) => r,
        Err(e) => {
            return JsonRpcResponse::error(
                request.id.clone(),
                INVALID_PARAMS,
                format!("invalid task request: {e}"),
            );
        }
    };

    match state.agent.handle_task(task_request).await {
        Ok(response) => match serde_json::to_value(&response) {
            Ok(val) => JsonRpcResponse::success(request.id.clone(), val),
            Err(e) => JsonRpcResponse::error(
                request.id.clone(),
                INTERNAL_ERROR,
                format!("serialization error: {e}"),
            ),
        },
        Err(e) => JsonRpcResponse::error(request.id.clone(), INTERNAL_ERROR, e.to_string()),
    }
}

fn handle_tasks_get(request: &JsonRpcRequest) -> JsonRpcResponse {
    // Stateless agents don't track tasks; return not-found by default.
    // Stateful agents can override via a custom implementation.
    JsonRpcResponse::error(
        request.id.clone(),
        TASK_NOT_FOUND,
        "task storage not enabled",
    )
}

async fn handle_tasks_cancel(state: &AppState, request: &JsonRpcRequest) -> JsonRpcResponse {
    let task_id = request
        .params
        .as_ref()
        .and_then(|p| p.get("id"))
        .and_then(|v| v.as_str())
        .unwrap_or("");

    match state.agent.handle_cancel(task_id).await {
        Ok(status) => match serde_json::to_value(&status) {
            Ok(val) => JsonRpcResponse::success(request.id.clone(), val),
            Err(e) => JsonRpcResponse::error(
                request.id.clone(),
                INTERNAL_ERROR,
                format!("serialization error: {e}"),
            ),
        },
        Err(e) => JsonRpcResponse::error(request.id.clone(), INTERNAL_ERROR, e.to_string()),
    }
}

fn handle_agent_card(state: &AppState, request: &JsonRpcRequest) -> JsonRpcResponse {
    let card = state.agent.card();
    match serde_json::to_value(&card) {
        Ok(val) => JsonRpcResponse::success(request.id.clone(), val),
        Err(e) => JsonRpcResponse::error(
            request.id.clone(),
            INTERNAL_ERROR,
            format!("serialization error: {e}"),
        ),
    }
}
