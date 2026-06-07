use axum::{
    extract::State,
    http::StatusCode,
    response::Json,
    routing::post,
    Router,
};
use cyrus_playground::*;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::Mutex;
use tower_http::cors::CorsLayer;
use tower_http::limit::RequestBodyLimitLayer;

#[derive(Deserialize)]
struct ExecuteRequest {
    code: String,
}

#[derive(Serialize)]
struct ExecuteResponse {
    success: bool,
    stdout: String,
    stderr: String,
    execution_time: f64,
}

#[derive(Serialize)]
struct ErrorResponse {
    error: String,
}

async fn execute_handler(
    State(executor): State<Arc<Mutex<CyrusExecutor>>>,
    Json(payload): Json<ExecuteRequest>,
) -> Result<Json<ExecuteResponse>, (StatusCode, Json<ErrorResponse>)> {
    if payload.code.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: "Code cannot be empty".to_string(),
            }),
        ));
    }

    if payload.code.len() > 50000 {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: "Code is too long (max 50KB)".to_string(),
            }),
        ));
    }

    match execute_cyrus_code(executor, &payload.code).await {
        Ok(result) => Ok(Json(ExecuteResponse {
            success: result.success,
            stdout: result.stdout,
            stderr: result.stderr,
            execution_time: result.execution_time,
        })),
        Err(e) => Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse { error: e }),
        )),
    }
}

async fn health_check() -> &'static str {
    "OK"
}

#[tokio::main]
async fn main() {
    env_logger::init();
    log::info!("Starting Cyrus Playground API");

    let executor = Arc::new(Mutex::new(CyrusExecutor::new()));

    let executor_clone = Arc::clone(&executor);
    tokio::spawn(async move {
        auto_update_cyrus(executor_clone).await;
    });

    let app = Router::new()
        .route("/api/execute", post(execute_handler))
        .route("/api/health", axum::routing::get(health_check))
        .layer(CorsLayer::permissive())
        .layer(RequestBodyLimitLayer::new(100 * 1024))
        .with_state(executor);

    let port = std::env::var("PORT").unwrap_or_else(|_| "3000".to_string());
    let addr = format!("0.0.0.0:{}", port);

    log::info!("API listening on {}", addr);

    let listener = tokio::net::TcpListener::bind(&addr).await.unwrap();
    axum::serve(listener, app).await.unwrap();
}
