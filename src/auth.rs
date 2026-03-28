use axum::body::Body;
use axum::{
    http::HeaderMap, http::Request, http::StatusCode, middleware::Next, response::IntoResponse,
    response::Json, response::Response,
};
use base64::{Engine, engine::general_purpose};
use jsonwebtoken::{DecodingKey, EncodingKey, Header, Validation, decode, encode};
use pam::Client;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::{SystemTime, UNIX_EPOCH};

static JWT_SECRET: OnceLock<String> = OnceLock::new();

const ROTATION_DAYS: u64 = 7;

fn secret_path() -> PathBuf {
    PathBuf::from("/home/jack/.local/share/sysapi/jwt_secret")
}

fn generate_secret() -> String {
    format!(
        "{:016x}{:016x}",
        rand::random::<u64>(),
        rand::random::<u64>()
    )
}

fn current_timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

pub fn jwt_secret() -> &'static str {
    JWT_SECRET.get_or_init(|| {
        let path = secret_path();
        std::fs::create_dir_all(path.parent().unwrap()).ok();

        // file format: "timestamp:secret"
        if let Ok(contents) = std::fs::read_to_string(&path) {
            if let Some((ts_str, secret)) = contents.trim().split_once(':') {
                if let Ok(ts) = ts_str.parse::<u64>() {
                    if current_timestamp() - ts < ROTATION_DAYS * 86400 {
                        return secret.to_string();
                    }
                    println!("JWT secret expired, rotating...");
                }
            }
        }

        let secret = generate_secret();
        let contents = format!("{}:{}", current_timestamp(), secret);
        std::fs::write(&path, &contents).ok();

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).ok();
        }

        println!("Generated new JWT secret");
        secret
    })
}

#[derive(Serialize, Deserialize)]
struct Claims {
    sub: String,
    exp: usize,
}

pub fn create_token(username: &str) -> String {
    let claims = Claims {
        sub: username.to_owned(),
        exp: (chrono::Utc::now() + chrono::Duration::hours(8)).timestamp() as usize,
    };
    encode(
        &Header::default(),
        &claims,
        &EncodingKey::from_secret(jwt_secret().as_bytes()),
    )
    .unwrap()
}

pub fn verify_token(headers: &HeaderMap) -> bool {
    let Some(val) = headers.get("Authorization") else {
        return false;
    };
    let token = val.to_str().unwrap_or("").replace("Bearer ", "");
    decode::<Claims>(
        &token,
        &DecodingKey::from_secret(jwt_secret().as_bytes()),
        &Validation::default(),
    )
    .is_ok()
}

pub fn decode_basic_auth(headers: &HeaderMap) -> Option<(String, String)> {
    let val = headers.get("Authorization")?.to_str().ok()?;
    let encoded = val.strip_prefix("Basic ")?;
    let decoded = general_purpose::STANDARD.decode(encoded).ok()?;
    let s = String::from_utf8(decoded).ok()?;
    let (user, pass) = s.split_once(':')?;
    Some((user.to_string(), pass.to_string()))
}

pub fn verify_system_credentials(username: &str, password: &str) -> bool {
    let mut client = match Client::with_password("login") {
        Ok(c) => c,
        Err(_) => return false,
    };
    client
        .conversation_mut()
        .set_credentials(username, password);
    client.authenticate().is_ok()
}

pub async fn require_auth(headers: HeaderMap, request: Request<Body>, next: Next) -> Response {
    if verify_token(&headers) {
        next.run(request).await
    } else {
        (StatusCode::UNAUTHORIZED, "Unauthorized").into_response()
    }
}

// POST /auth/login
pub async fn post_login(headers: HeaderMap) -> impl IntoResponse {
    let (username, password) = match decode_basic_auth(&headers) {
        Some(c) => c,
        None => {
            return (
                StatusCode::UNAUTHORIZED,
                "Missing or invalid Authorization header",
            )
                .into_response();
        }
    };
    if !verify_system_credentials(&username, &password) {
        return (StatusCode::UNAUTHORIZED, "Invalid credentials").into_response();
    }
    let token = create_token(&username);
    (StatusCode::OK, Json(serde_json::json!({ "token": token }))).into_response()
}
