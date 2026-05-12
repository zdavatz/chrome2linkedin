//! chrome2linkedin-helper — localhost bridge between the Chrome extension and
//! the LinkedIn /rest/posts API. Reuses the token file written by
//! `li_push --auth` (from the li_push_rs project) so the client_secret never
//! has to live inside the extension bundle.

use axum::{
    extract::State,
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::Mutex;
use tower_http::cors::CorsLayer;

const LINKEDIN_VERSION: &str = "202603";
const DEFAULT_PORT: u16 = 8093;

#[derive(Serialize, Deserialize, Clone)]
struct Credentials {
    client_id: String,
    client_secret: String,
}

#[derive(Serialize, Deserialize, Clone)]
struct Token {
    access_token: String,
    #[serde(default)]
    refresh_token: String,
    #[serde(default)]
    person_id: String,
    #[serde(default)]
    expires_in: u64,
}

#[derive(Clone)]
struct AppState {
    credentials_path: PathBuf,
    token_path: PathBuf,
    token: Arc<Mutex<Token>>,
    client: reqwest::Client,
}

#[derive(Deserialize)]
struct PostRequest {
    commentary: String,
    #[serde(default = "default_visibility")]
    visibility: String,
}

fn default_visibility() -> String {
    "PUBLIC".to_string()
}

#[derive(Serialize)]
struct PostResponse {
    post_id: String,
    post_url: String,
}

fn home_path(name: &str) -> PathBuf {
    let home = env::var("HOME").unwrap_or_else(|_| "/root".to_string());
    PathBuf::from(home).join(name)
}

fn load_credentials(path: &Path) -> Result<Credentials, String> {
    let data = fs::read_to_string(path)
        .map_err(|e| format!("Cannot read {}: {}", path.display(), e))?;
    serde_json::from_str(&data)
        .map_err(|e| format!("Invalid JSON in {}: {}", path.display(), e))
}

fn load_token(path: &Path) -> Result<Token, String> {
    let data = fs::read_to_string(path)
        .map_err(|e| format!("Cannot read {}: {}", path.display(), e))?;
    serde_json::from_str(&data)
        .map_err(|e| format!("Invalid JSON in {}: {}", path.display(), e))
}

fn save_token(path: &Path, token: &Token) -> Result<(), String> {
    let json = serde_json::to_string_pretty(token).map_err(|e| e.to_string())?;
    fs::write(path, json).map_err(|e| e.to_string())
}

async fn refresh_token(state: &AppState) -> Result<Token, String> {
    let current = state.token.lock().await.clone();
    if current.refresh_token.is_empty() {
        return Err("No refresh_token available. Re-run `li_push --auth`.".into());
    }
    let creds = load_credentials(&state.credentials_path)?;
    let resp = state
        .client
        .post("https://www.linkedin.com/oauth/v2/accessToken")
        .header("Content-Type", "application/x-www-form-urlencoded")
        .body(format!(
            "grant_type=refresh_token&refresh_token={}&client_id={}&client_secret={}",
            current.refresh_token, creds.client_id, creds.client_secret
        ))
        .send()
        .await
        .map_err(|e| format!("refresh request failed: {}", e))?;

    let status = resp.status();
    let text = resp.text().await.map_err(|e| e.to_string())?;
    if !status.is_success() {
        return Err(format!("refresh failed ({}): {}", status, text));
    }
    let data: serde_json::Value =
        serde_json::from_str(&text).map_err(|e| format!("parse refresh response: {}", e))?;
    let access_token = data["access_token"].as_str().unwrap_or("").to_string();
    if access_token.is_empty() {
        return Err(format!("no access_token in refresh response: {}", text));
    }
    let new_token = Token {
        access_token,
        refresh_token: data["refresh_token"]
            .as_str()
            .unwrap_or(&current.refresh_token)
            .to_string(),
        person_id: current.person_id.clone(),
        expires_in: data["expires_in"].as_u64().unwrap_or(0),
    };
    save_token(&state.token_path, &new_token)?;
    *state.token.lock().await = new_token.clone();
    Ok(new_token)
}

async fn create_post(
    state: &AppState,
    body: &PostRequest,
) -> Result<PostResponse, (StatusCode, String)> {
    let token = state.token.lock().await.clone();
    if token.person_id.is_empty() {
        return Err((
            StatusCode::PRECONDITION_FAILED,
            "Token has no person_id. Re-run `li_push --auth`.".into(),
        ));
    }
    let owner = format!("urn:li:person:{}", token.person_id);
    let visibility = match body.visibility.to_uppercase().as_str() {
        "CONNECTIONS" | "FRIENDS" => "CONNECTIONS",
        _ => "PUBLIC",
    };

    let post_body = serde_json::json!({
        "author": owner,
        "commentary": body.commentary,
        "visibility": visibility,
        "distribution": {
            "feedDistribution": "MAIN_FEED",
            "targetEntities": [],
            "thirdPartyDistributionChannels": []
        },
        "lifecycleState": "PUBLISHED",
        "isReshareDisabledByAuthor": false
    });

    let send = |access_token: String| {
        let body = post_body.clone();
        let client = state.client.clone();
        async move {
            client
                .post("https://api.linkedin.com/rest/posts")
                .header("Authorization", format!("Bearer {}", access_token))
                .header("Content-Type", "application/json")
                .header("LinkedIn-Version", LINKEDIN_VERSION)
                .header("X-Restli-Protocol-Version", "2.0.0")
                .json(&body)
                .send()
                .await
        }
    };

    let resp = send(token.access_token.clone())
        .await
        .map_err(|e| (StatusCode::BAD_GATEWAY, format!("linkedin request: {}", e)))?;
    let status = resp.status();
    let headers = resp.headers().clone();
    let text = resp.text().await.unwrap_or_default();

    let (status, headers, text) = if status == StatusCode::UNAUTHORIZED {
        let new_token = refresh_token(state)
            .await
            .map_err(|e| (StatusCode::UNAUTHORIZED, format!("token refresh: {}", e)))?;
        let resp = send(new_token.access_token)
            .await
            .map_err(|e| (StatusCode::BAD_GATEWAY, format!("linkedin request: {}", e)))?;
        let status = resp.status();
        let headers = resp.headers().clone();
        let text = resp.text().await.unwrap_or_default();
        (status, headers, text)
    } else {
        (status, headers, text)
    };

    if !status.is_success() {
        return Err((StatusCode::BAD_GATEWAY, format!("linkedin {}: {}", status, text)));
    }

    let post_id = headers
        .get("x-restli-id")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("(unknown)")
        .to_string();
    let post_url = format!("https://www.linkedin.com/feed/update/{}/", post_id);
    Ok(PostResponse { post_id, post_url })
}

async fn handle_post(
    State(state): State<AppState>,
    Json(body): Json<PostRequest>,
) -> impl IntoResponse {
    if body.commentary.trim().is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "commentary cannot be empty"})),
        )
            .into_response();
    }
    match create_post(&state, &body).await {
        Ok(r) => (
            StatusCode::OK,
            Json(serde_json::to_value(r).unwrap_or(serde_json::Value::Null)),
        )
            .into_response(),
        Err((code, msg)) => (code, Json(serde_json::json!({ "error": msg }))).into_response(),
    }
}

async fn handle_status(State(state): State<AppState>) -> impl IntoResponse {
    let token = state.token.lock().await;
    Json(serde_json::json!({
        "has_token": !token.access_token.is_empty(),
        "person_id_present": !token.person_id.is_empty(),
        "refresh_token_present": !token.refresh_token.is_empty(),
    }))
}

#[tokio::main]
async fn main() {
    let port: u16 = env::var("CHROME2LINKEDIN_PORT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(DEFAULT_PORT);

    let credentials_path = home_path("linkedin_credentials.json");
    let token_path = home_path("linkedin_token.json");

    let token = load_token(&token_path).unwrap_or_else(|e| {
        eprintln!("ERROR: {}", e);
        eprintln!(
            "Run `li_push --auth` (from li_push_rs) to create {}.",
            token_path.display()
        );
        std::process::exit(1);
    });

    if !credentials_path.exists() {
        eprintln!(
            "WARN: {} not found. Token refresh will fail when the access token expires.",
            credentials_path.display()
        );
    }

    let state = AppState {
        credentials_path,
        token_path,
        token: Arc::new(Mutex::new(token)),
        client: reqwest::Client::new(),
    };

    let app = Router::new()
        .route("/status", get(handle_status))
        .route("/post", post(handle_post))
        .layer(CorsLayer::permissive())
        .with_state(state);

    let addr = format!("127.0.0.1:{}", port);
    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .unwrap_or_else(|e| {
            eprintln!("ERROR: bind {}: {}", addr, e);
            std::process::exit(1);
        });
    eprintln!("chrome2linkedin-helper listening on http://{}", addr);
    eprintln!("Endpoints: GET /status, POST /post");
    axum::serve(listener, app).await.expect("server");
}
