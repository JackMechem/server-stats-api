use axum::body::Body;
use axum::extract::State;
use axum::{
    http::HeaderMap,
    http::Request,
    http::StatusCode,
    middleware::Next,
    response::IntoResponse,
    response::Json,
    response::Response,
};
use base64::{Engine, engine::general_purpose};
use jsonwebtoken::{DecodingKey, EncodingKey, Header, Validation, decode, encode};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use url::Url;
use uuid::Uuid;
use webauthn_rs::prelude::*;
use yescrypt::{PasswordHash, PasswordVerifier, Yescrypt};

static JWT_SECRET: OnceLock<String> = OnceLock::new();

const ROTATION_DAYS: u64 = 7;
const CREDENTIAL_DIR: &str = "/var/lib/server-dash-api/webauthn-credentials";
const CHALLENGE_TTL: Duration = Duration::from_secs(300);
const RP_ID: &str = "jackmechem.dev";
const RP_ORIGIN: &str = "https://dashboard.jackmechem.dev";

#[derive(Serialize, Deserialize)]
struct StoredCredentials {
    user_id: Uuid,
    credentials: Vec<Passkey>,
}

pub struct AppState {
    pub webauthn: Webauthn,
    pending_auth: Mutex<HashMap<String, (PasskeyAuthentication, Instant, String)>>,
    pending_reg: Mutex<HashMap<String, (PasskeyRegistration, Instant, String, Uuid)>>,
}

impl AppState {
    pub fn new() -> Self {
        let rp_origin = Url::parse(RP_ORIGIN).expect("Invalid RP origin");
        let webauthn = WebauthnBuilder::new(RP_ID, &rp_origin)
            .expect("Invalid WebAuthn config")
            .rp_name("Server Dashboard")
            .build()
            .expect("Failed to build WebAuthn");
        Self {
            webauthn,
            pending_auth: Mutex::new(HashMap::new()),
            pending_reg: Mutex::new(HashMap::new()),
        }
    }
}

fn secret_path() -> PathBuf {
    PathBuf::from("/var/lib/server-dash-api/jwt_secret")
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
    let (user, password) = s.split_once(':')?;
    Some((user.to_string(), password.to_string()))
}

fn verify_password(username: &str, password: &str) -> bool {
    let shadow_content = match std::fs::read_to_string("/etc/shadow") {
        Ok(c) => c,
        Err(e) => {
            println!("Failed to read /etc/shadow: {}", e);
            return false;
        }
    };
    for line in shadow_content.lines() {
        let fields: Vec<&str> = line.split(':').collect();
        if fields.len() < 2 {
            continue;
        }
        if fields[0] != username {
            continue;
        }
        return verify_shadow_hash(password, fields[1]);
    }
    println!("User not found in shadow");
    false
}

fn verify_shadow_hash(password: &str, hash: &str) -> bool {
    let parsed_hash = match PasswordHash::new(hash) {
        Ok(h) => h,
        Err(e) => {
            println!("Failed to parse hash: {:?}", e);
            return false;
        }
    };
    Yescrypt::default()
        .verify_password(password.as_bytes(), &parsed_hash)
        .is_ok()
}

fn load_credentials(username: &str) -> Option<StoredCredentials> {
    let path = PathBuf::from(CREDENTIAL_DIR).join(format!("{}.json", username));
    let data = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&data).ok()
}

fn save_credentials(username: &str, creds: &StoredCredentials) -> Result<(), String> {
    let dir = PathBuf::from(CREDENTIAL_DIR);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let path = dir.join(format!("{}.json", username));
    let data = serde_json::to_string(creds).map_err(|e| e.to_string())?;
    std::fs::write(&path, &data).map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).ok();
    }
    Ok(())
}

fn generate_session_id() -> String {
    format!(
        "{:016x}{:016x}",
        rand::random::<u64>(),
        rand::random::<u64>()
    )
}

pub async fn require_auth(headers: HeaderMap, request: Request<Body>, next: Next) -> Response {
    if verify_token(&headers) {
        next.run(request).await
    } else {
        (StatusCode::UNAUTHORIZED, "Unauthorized").into_response()
    }
}

// POST /auth/login — verifies password, returns a WebAuthn challenge for the YubiKey
pub async fn post_login(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> impl IntoResponse {
    let (username, password) = match decode_basic_auth(&headers) {
        Some(c) => c,
        None => {
            return (StatusCode::UNAUTHORIZED, "Missing or invalid Authorization header")
                .into_response()
        }
    };

    if !verify_password(&username, &password) {
        return (StatusCode::UNAUTHORIZED, "Invalid credentials").into_response();
    }

    let stored = match load_credentials(&username) {
        Some(s) => s,
        None => {
            return (StatusCode::UNAUTHORIZED, "No YubiKey registered for this user")
                .into_response()
        }
    };

    let (rcr, auth_state) = match state
        .webauthn
        .start_passkey_authentication(&stored.credentials)
    {
        Ok(r) => r,
        Err(e) => {
            println!("WebAuthn start auth error: {:?}", e);
            return (StatusCode::INTERNAL_SERVER_ERROR, "WebAuthn error").into_response();
        }
    };

    let session_id = generate_session_id();
    {
        let mut pending = state.pending_auth.lock().unwrap();
        pending.retain(|_, (_, created, _)| created.elapsed() < CHALLENGE_TTL);
        pending.insert(session_id.clone(), (auth_state, Instant::now(), username));
    }

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "session_id": session_id,
            "challenge": rcr,
        })),
    )
        .into_response()
}

#[derive(Deserialize)]
pub struct VerifyRequest {
    session_id: String,
    credential: PublicKeyCredential,
}

// POST /auth/verify — verifies the YubiKey assertion and returns a JWT
pub async fn post_verify(
    State(state): State<Arc<AppState>>,
    Json(body): Json<VerifyRequest>,
) -> impl IntoResponse {
    let (auth_state, username) = {
        let mut pending = state.pending_auth.lock().unwrap();
        match pending.remove(&body.session_id) {
            Some((s, created, u)) if created.elapsed() < CHALLENGE_TTL => (s, u),
            Some(_) => return (StatusCode::UNAUTHORIZED, "Challenge expired").into_response(),
            None => return (StatusCode::UNAUTHORIZED, "Invalid session").into_response(),
        }
    };

    let auth_result = match state
        .webauthn
        .finish_passkey_authentication(&body.credential, &auth_state)
    {
        Ok(r) => r,
        Err(e) => {
            println!("WebAuthn finish auth error: {:?}", e);
            return (StatusCode::UNAUTHORIZED, "WebAuthn verification failed").into_response();
        }
    };

    // Persist updated credential counter
    if let Some(mut stored) = load_credentials(&username) {
        for cred in &mut stored.credentials {
            cred.update_credential(&auth_result);
        }
        save_credentials(&username, &stored).ok();
    }

    let token = create_token(&username);
    (StatusCode::OK, Json(serde_json::json!({ "token": token }))).into_response()
}

// POST /auth/register/start — verifies password, returns a WebAuthn registration challenge
pub async fn post_register_start(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> impl IntoResponse {
    let (username, password) = match decode_basic_auth(&headers) {
        Some(c) => c,
        None => {
            return (StatusCode::UNAUTHORIZED, "Missing or invalid Authorization header")
                .into_response()
        }
    };

    if !verify_password(&username, &password) {
        return (StatusCode::UNAUTHORIZED, "Invalid credentials").into_response();
    }

    let stored = load_credentials(&username);
    let user_id = stored.as_ref().map(|s| s.user_id).unwrap_or_else(Uuid::new_v4);

    let exclude: Option<Vec<CredentialID>> = stored.as_ref().map(|s| {
        s.credentials
            .iter()
            .map(|c| c.cred_id().clone())
            .collect()
    });

    let (ccr, reg_state) = match state
        .webauthn
        .start_passkey_registration(user_id, &username, &username, exclude)
    {
        Ok(r) => r,
        Err(e) => {
            println!("WebAuthn start reg error: {:?}", e);
            return (StatusCode::INTERNAL_SERVER_ERROR, "WebAuthn error").into_response();
        }
    };

    let session_id = generate_session_id();
    {
        let mut pending = state.pending_reg.lock().unwrap();
        pending.retain(|_, (_, created, _, _)| created.elapsed() < CHALLENGE_TTL);
        pending.insert(
            session_id.clone(),
            (reg_state, Instant::now(), username, user_id),
        );
    }

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "session_id": session_id,
            "challenge": ccr,
        })),
    )
        .into_response()
}

#[derive(Deserialize)]
pub struct RegisterFinishRequest {
    session_id: String,
    credential: RegisterPublicKeyCredential,
}

// POST /auth/register/finish — completes YubiKey enrollment and saves the credential
pub async fn post_register_finish(
    State(state): State<Arc<AppState>>,
    Json(body): Json<RegisterFinishRequest>,
) -> impl IntoResponse {
    let (reg_state, username, user_id) = {
        let mut pending = state.pending_reg.lock().unwrap();
        match pending.remove(&body.session_id) {
            Some((s, created, u, id)) if created.elapsed() < CHALLENGE_TTL => (s, u, id),
            Some(_) => return (StatusCode::UNAUTHORIZED, "Challenge expired").into_response(),
            None => return (StatusCode::UNAUTHORIZED, "Invalid session").into_response(),
        }
    };

    let passkey = match state
        .webauthn
        .finish_passkey_registration(&body.credential, &reg_state)
    {
        Ok(p) => p,
        Err(e) => {
            println!("WebAuthn finish reg error: {:?}", e);
            return (StatusCode::BAD_REQUEST, "WebAuthn registration failed").into_response();
        }
    };

    let mut stored = load_credentials(&username).unwrap_or(StoredCredentials {
        user_id,
        credentials: vec![],
    });
    stored.credentials.push(passkey);

    if let Err(e) = save_credentials(&username, &stored) {
        println!("Failed to save credentials: {}", e);
        return (StatusCode::INTERNAL_SERVER_ERROR, "Failed to save credential").into_response();
    }

    (
        StatusCode::OK,
        Json(serde_json::json!({ "message": "YubiKey registered successfully" })),
    )
        .into_response()
}
