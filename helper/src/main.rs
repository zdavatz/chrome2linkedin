//! chrome2linkedin-helper — localhost bridge between the Chrome extension and
//! the LinkedIn /rest/posts API.
//!
//! Two subcommands:
//!   `auth`   — run the OAuth dance and write ~/linkedin_token.json
//!   `serve`  — run the localhost HTTP server (default if no arg given)

use axum::{
    extract::State,
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use base64::Engine;
use serde::{Deserialize, Serialize};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::Mutex;
use tower_http::cors::CorsLayer;

const LINKEDIN_VERSION: &str = "202603";
const DEFAULT_PORT: u16 = 8093;
const AUTH_REDIRECT_URI: &str = "http://localhost:8092/callback";
const AUTH_CALLBACK_PORT: u16 = 8092;

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
        return Err("No refresh_token available. Run `chrome2linkedin-helper auth`.".into());
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
            "Token has no person_id. Re-run `chrome2linkedin-helper auth`.".into(),
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

async fn run_server() {
    let port: u16 = env::var("CHROME2LINKEDIN_PORT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(DEFAULT_PORT);

    let credentials_path = home_path(".linkedin_credentials.json");
    let token_path = home_path(".linkedin_token.json");

    let token = load_token(&token_path).unwrap_or_else(|e| {
        eprintln!("ERROR: {}", e);
        eprintln!("Run `chrome2linkedin-helper auth` first.");
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

async fn run_auth() {
    let credentials_path = home_path(".linkedin_credentials.json");
    let token_path = home_path(".linkedin_token.json");

    let creds = load_credentials(&credentials_path).unwrap_or_else(|e| {
        eprintln!("ERROR: {}", e);
        eprintln!("Create {} with:", credentials_path.display());
        eprintln!(r#"  {{"client_id": "...", "client_secret": "..."}}"#);
        std::process::exit(1);
    });

    let auth_url = format!(
        "https://www.linkedin.com/oauth/v2/authorization?response_type=code&client_id={}&redirect_uri={}&scope=openid%20profile%20w_member_social",
        urlencoding::encode(&creds.client_id),
        urlencoding::encode(AUTH_REDIRECT_URI)
    );

    eprintln!("Opening browser for LinkedIn authorization...");
    eprintln!("URL: {}\n", auth_url);
    if let Err(e) = open::that(&auth_url) {
        eprintln!("WARN: Could not open browser automatically: {}", e);
        eprintln!("Open the URL above manually.");
    }

    // Best-effort cleanup of stale listeners on the callback port
    if let Ok(output) = std::process::Command::new("lsof")
        .args(["-ti", &format!(":{}", AUTH_CALLBACK_PORT)])
        .output()
    {
        if !output.stdout.is_empty() {
            for pid in String::from_utf8_lossy(&output.stdout).trim().lines() {
                let _ = std::process::Command::new("kill").args(["-9", pid.trim()]).output();
            }
            tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        }
    }

    eprintln!("Waiting for callback on {}...", AUTH_REDIRECT_URI);
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", AUTH_CALLBACK_PORT))
        .await
        .unwrap_or_else(|e| {
            eprintln!("ERROR: bind 127.0.0.1:{}: {}", AUTH_CALLBACK_PORT, e);
            std::process::exit(1);
        });

    let (mut socket, _) = listener.accept().await.expect("accept connection");
    let mut buf = vec![0u8; 4096];
    let n = tokio::io::AsyncReadExt::read(&mut socket, &mut buf)
        .await
        .expect("read request");
    let request = String::from_utf8_lossy(&buf[..n]);

    let query = request.split('?').nth(1).unwrap_or("");
    let query = query.split(' ').next().unwrap_or(query);
    let code_raw: String = query
        .split('&')
        .find(|p| p.starts_with("code="))
        .and_then(|p| p.strip_prefix("code="))
        .unwrap_or("")
        .to_string();
    let code = urlencoding::decode(&code_raw)
        .unwrap_or(std::borrow::Cow::Borrowed(&code_raw))
        .to_string();

    let response = "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\n\r\n<h1>LinkedIn authorized. You can close this window.</h1>";
    let _ = tokio::io::AsyncWriteExt::write_all(&mut socket, response.as_bytes()).await;

    if code.is_empty() {
        eprintln!("ERROR: No authorization code received.");
        eprintln!("Raw request: {}", request);
        std::process::exit(1);
    }

    eprintln!("Authorization code received. Exchanging for token...");
    let client = reqwest::Client::new();
    let token_resp = client
        .post("https://www.linkedin.com/oauth/v2/accessToken")
        .header("Content-Type", "application/x-www-form-urlencoded")
        .body(format!(
            "grant_type=authorization_code&code={}&redirect_uri={}&client_id={}&client_secret={}",
            urlencoding::encode(&code),
            urlencoding::encode(AUTH_REDIRECT_URI),
            urlencoding::encode(&creds.client_id),
            urlencoding::encode(&creds.client_secret)
        ))
        .send()
        .await
        .expect("token exchange request failed");

    let token_status = token_resp.status();
    let token_text = token_resp.text().await.expect("read token response");

    if !token_status.is_success() {
        eprintln!("ERROR: Token exchange failed ({}): {}", token_status, token_text);
        std::process::exit(1);
    }

    let token_data: serde_json::Value = serde_json::from_str(&token_text).unwrap_or_else(|e| {
        eprintln!("ERROR: Failed to parse token response: {}\n{}", e, token_text);
        std::process::exit(1);
    });

    if let Some(err) = token_data.get("error") {
        eprintln!("ERROR: Token exchange failed: {}", err);
        eprintln!(
            "Description: {}",
            token_data.get("error_description").unwrap_or(&serde_json::Value::Null)
        );
        std::process::exit(1);
    }

    let access_token = token_data["access_token"].as_str().unwrap_or("").to_string();
    let refresh_token_str = token_data["refresh_token"].as_str().unwrap_or("").to_string();
    let expires_in = token_data["expires_in"].as_u64().unwrap_or(0);

    if access_token.is_empty() {
        eprintln!("ERROR: No access_token in response: {}", token_text);
        std::process::exit(1);
    }

    // Prefer the `sub` claim from the id_token JWT (returned with openid scope).
    let mut person_id = String::new();
    if let Some(id_token) = token_data["id_token"].as_str() {
        let parts: Vec<&str> = id_token.split('.').collect();
        if parts.len() >= 2 {
            let payload = parts[1];
            let padded = match payload.len() % 4 {
                2 => format!("{}==", payload),
                3 => format!("{}=", payload),
                _ => payload.to_string(),
            };
            if let Ok(decoded) = base64::engine::general_purpose::URL_SAFE_NO_PAD
                .decode(padded.trim_end_matches('='))
            {
                if let Ok(claims) = serde_json::from_slice::<serde_json::Value>(&decoded) {
                    if let Some(sub) = claims["sub"].as_str() {
                        person_id = sub.to_string();
                    }
                }
            }
        }
    }

    // Fallback to /v2/userinfo
    if person_id.is_empty() {
        if let Ok(resp) = client
            .get("https://api.linkedin.com/v2/userinfo")
            .header("Authorization", format!("Bearer {}", access_token))
            .send()
            .await
        {
            if resp.status().is_success() {
                if let Ok(text) = resp.text().await {
                    if let Ok(data) = serde_json::from_str::<serde_json::Value>(&text) {
                        if let Some(sub) = data["sub"].as_str() {
                            person_id = sub.to_string();
                        }
                    }
                }
            }
        }
    }

    if person_id.is_empty() {
        eprintln!("ERROR: Could not determine person_id from id_token or /v2/userinfo.");
        std::process::exit(1);
    }

    let token = Token {
        access_token,
        refresh_token: refresh_token_str,
        person_id: person_id.clone(),
        expires_in,
    };

    save_token(&token_path, &token).unwrap_or_else(|e| {
        eprintln!("ERROR: save token: {}", e);
        std::process::exit(1);
    });

    eprintln!("Token saved to {}", token_path.display());
    eprintln!("Person ID: {}", person_id);
    eprintln!("Expires in: {} seconds", expires_in);
    eprintln!("Ready. Start the server with: chrome2linkedin-helper");
}

fn print_usage() {
    eprintln!("chrome2linkedin-helper — LinkedIn posting bridge for the Chrome extension");
    eprintln!();
    eprintln!("Usage:");
    eprintln!("  chrome2linkedin-helper           Run the localhost HTTP server (default)");
    eprintln!("  chrome2linkedin-helper serve     Same as above");
    eprintln!("  chrome2linkedin-helper auth      Run the OAuth flow and write ~/linkedin_token.json");
    eprintln!();
    eprintln!("Environment:");
    eprintln!("  CHROME2LINKEDIN_PORT             Override the server port (default {})", DEFAULT_PORT);
}

#[tokio::main]
async fn main() {
    let args: Vec<String> = env::args().collect();
    match args.get(1).map(|s| s.as_str()) {
        Some("auth") => run_auth().await,
        None | Some("serve") => run_server().await,
        Some("-h") | Some("--help") | Some("help") => print_usage(),
        Some(other) => {
            eprintln!("Unknown subcommand: {}", other);
            print_usage();
            std::process::exit(1);
        }
    }
}
