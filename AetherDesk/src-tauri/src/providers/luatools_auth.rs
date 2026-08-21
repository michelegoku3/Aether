use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use once_cell::sync::Lazy;
use rand::RngCore;
use serde::{Deserialize, Serialize};
use tokio::sync::oneshot;
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

use crate::core::paths::LocalAppPaths;
use crate::providers::http;

const SUPABASE_URL: &str = "https://db.lua.tools";
const SUPABASE_ANON_KEY: &str = "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJpYXQiOjE3NzYwMzkzNzYsImV4cCI6MTg5MzQ1NjAwMCwicm9sZSI6ImFub24iLCJpc3MiOiJzdXBhYmFzZSJ9.f_-K38u3odjltP-g_67FVmG32Vg-_-k-lNBvIaVUVBM";
const CALLBACK_PORT: u16 = 53789;
const CALLBACK_URL: &str = "http://localhost:53789/callback";
const AUTH_TIMEOUT_SECONDS: u64 = 5 * 60;
const AUTH_FILE_NAME: &str = "luatools_auth.dat";

/// At most one interactive OAuth listener exists per process. The sender lets
/// the Settings UI cancel a browser flow when the user closes or denies it,
/// instead of leaving the Login button busy until the timeout expires.
static OAUTH_CANCEL: Lazy<Mutex<Option<oneshot::Sender<()>>>> =
    Lazy::new(|| Mutex::new(None));
static OAUTH_CANCEL_REQUESTED: AtomicBool = AtomicBool::new(false);

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LuaToolsAuthStatus {
    pub signed_in: bool,
    pub display_name: Option<String>,
    pub email: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoredSession {
    access_token: String,
    refresh_token: String,
    expires_at: u64,
    display_name: Option<String>,
    email: Option<String>,
}

#[derive(Debug, Deserialize)]
struct CodeRedeemResponse {
    token: String,
}

#[derive(Debug, Deserialize)]
struct SupabaseSession {
    access_token: String,
    refresh_token: String,
    expires_in: u64,
    user: Option<SupabaseUser>,
}

#[derive(Debug, Deserialize)]
struct SupabaseUser {
    email: Option<String>,
    user_metadata: Option<UserMetadata>,
}

#[derive(Debug, Deserialize)]
struct UserMetadata {
    full_name: Option<String>,
    name: Option<String>,
    custom_claims: Option<CustomClaims>,
}

#[derive(Debug, Deserialize)]
struct CustomClaims {
    global_name: Option<String>,
}

/// Owns the LuaTools Supabase session lifecycle. Tokens are persisted outside
/// settings.json and protected with Windows DPAPI for the current OS user.
pub struct LuaToolsAuth {
    path: PathBuf,
    client: reqwest::Client,
}

impl LuaToolsAuth {
    pub fn new() -> Self {
        Self {
            path: LocalAppPaths::data_root()
                .join("config")
                .join(AUTH_FILE_NAME),
            client: http::build_client(30),
        }
    }

    pub fn status(&self) -> LuaToolsAuthStatus {
        match self.load_session() {
            Ok(Some(session)) => LuaToolsAuthStatus {
                signed_in: true,
                display_name: session.display_name,
                email: session.email,
            },
            _ => LuaToolsAuthStatus {
                signed_in: false,
                display_name: None,
                email: None,
            },
        }
    }

    pub async fn sign_in(&self) -> Result<LuaToolsAuthStatus, String> {
        let listener = TcpListener::bind(("127.0.0.1", CALLBACK_PORT))
            .await
            .map_err(|e| format!("Could not start the LuaTools login callback: {e}"))?;

        let mut verifier_bytes = [0u8; 48];
        rand::thread_rng().fill_bytes(&mut verifier_bytes);
        let verifier = URL_SAFE_NO_PAD.encode(verifier_bytes);
        let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
        let authorize_url = format!(
            "{SUPABASE_URL}/auth/v1/authorize?provider=discord&redirect_to={}&code_challenge={challenge}&code_challenge_method=s256",
            url::form_urlencoded::byte_serialize(CALLBACK_URL.as_bytes()).collect::<String>()
        );

        crate::util::browser::open_external_url(&authorize_url)?;

        let (cancel_tx, cancel_rx) = oneshot::channel();
        if let Ok(mut slot) = OAUTH_CANCEL.lock() {
            if let Some(previous) = slot.replace(cancel_tx) {
                let _ = previous.send(());
            }
            // Handles the small race where the user presses Cancel before the
            // async command has finished registering its sender.
            if OAUTH_CANCEL_REQUESTED.swap(false, Ordering::SeqCst) {
                if let Some(cancel) = slot.take() {
                    let _ = cancel.send(());
                }
            }
        }
        let callback = tokio::time::timeout(
            Duration::from_secs(AUTH_TIMEOUT_SECONDS),
            wait_for_callback(listener),
        );
        let code_result = tokio::select! {
            result = callback => result
                .map_err(|_| "LuaTools sign-in timed out after 5 minutes".to_string())?,
            _ = cancel_rx => Err("LuaTools sign-in cancelled".to_string()),
        };
        if let Ok(mut slot) = OAUTH_CANCEL.lock() {
            slot.take();
        }
        OAUTH_CANCEL_REQUESTED.store(false, Ordering::SeqCst);
        let code = code_result?;

        let response = self
            .client
            .post(format!("{SUPABASE_URL}/auth/v1/token?grant_type=pkce"))
            .header("apikey", SUPABASE_ANON_KEY)
            .json(&serde_json::json!({
                "auth_code": code,
                "code_verifier": verifier,
            }))
            .send()
            .await
            .map_err(|e| format!("LuaTools token exchange failed: {e}"))?;
        let status = response.status();
        let body = response
            .text()
            .await
            .map_err(|e| format!("Could not read the LuaTools login response: {e}"))?;
        if !status.is_success() {
            return Err(format!("LuaTools token exchange returned HTTP {status}: {body}"));
        }
        let session: SupabaseSession = serde_json::from_str(&body)
            .map_err(|e| format!("Could not parse the LuaTools login response: {e}"))?;
        self.persist_session(session)
    }

    /// Privacy-oriented login offered by the LuaTools bot. The short-lived
    /// six-character code is redeemed for a Supabase magic-link token, then
    /// exchanged for the same refreshable session used by Discord OAuth.
    pub async fn sign_in_with_code(&self, code: &str) -> Result<LuaToolsAuthStatus, String> {
        let code = code.trim().to_ascii_uppercase();
        if code.len() != 6 || !code.chars().all(|character| character.is_ascii_alphanumeric()) {
            return Err("Enter the 6-character code generated by @Luie /login".to_string());
        }

        let redeem = self
            .client
            .post("https://lua.tools/api/auth/code/redeem")
            .json(&serde_json::json!({ "code": code }))
            .send()
            .await
            .map_err(|e| format!("Could not redeem the LuaTools login code: {e}"))?;
        let redeem_status = redeem.status();
        let redeem_body = redeem
            .text()
            .await
            .map_err(|e| format!("Could not read the LuaTools code response: {e}"))?;
        if !redeem_status.is_success() {
            return Err(match redeem_status.as_u16() {
                400 | 404 => "The @Luie login code is invalid or has already been used".to_string(),
                410 => "The @Luie login code expired. Generate a new one with /login".to_string(),
                _ => format!("LuaTools code redemption returned HTTP {redeem_status}"),
            });
        }
        let redeemed: CodeRedeemResponse = serde_json::from_str(&redeem_body)
            .map_err(|e| format!("Could not parse the LuaTools code response: {e}"))?;
        if redeemed.token.trim().is_empty() {
            return Err("LuaTools returned an empty login token".to_string());
        }

        let verify = self
            .client
            .post(format!("{SUPABASE_URL}/auth/v1/verify"))
            .header("apikey", SUPABASE_ANON_KEY)
            .json(&serde_json::json!({
                "type": "magiclink",
                "token_hash": redeemed.token,
            }))
            .send()
            .await
            .map_err(|e| format!("Could not verify the LuaTools login code: {e}"))?;
        let verify_status = verify.status();
        let verify_body = verify
            .text()
            .await
            .map_err(|e| format!("Could not read the verified LuaTools session: {e}"))?;
        if !verify_status.is_success() {
            return Err(format!(
                "LuaTools code verification returned HTTP {verify_status}"
            ));
        }
        let session: SupabaseSession = serde_json::from_str(&verify_body)
            .map_err(|e| format!("Could not parse the verified LuaTools session: {e}"))?;
        self.persist_session(session)
    }

    pub fn cancel_sign_in() {
        OAUTH_CANCEL_REQUESTED.store(true, Ordering::SeqCst);
        if let Ok(mut slot) = OAUTH_CANCEL.lock() {
            if let Some(cancel) = slot.take() {
                let _ = cancel.send(());
            }
        }
    }

    pub fn sign_out(&self) -> Result<(), String> {
        if self.path.exists() {
            fs::remove_file(&self.path)
                .map_err(|e| format!("Could not remove the LuaTools session: {e}"))?;
        }
        Ok(())
    }

    pub async fn valid_access_token(&self) -> Result<String, String> {
        let mut session = self
            .load_session()?
            .ok_or_else(|| "Sign in to LuaTools from Settings first".to_string())?;
        if session.expires_at > now_unix().saturating_add(120) {
            return Ok(session.access_token);
        }

        let response = self
            .client
            .post(format!(
                "{SUPABASE_URL}/auth/v1/token?grant_type=refresh_token"
            ))
            .header("apikey", SUPABASE_ANON_KEY)
            .json(&serde_json::json!({ "refresh_token": session.refresh_token }))
            .send()
            .await
            .map_err(|e| format!("LuaTools session refresh failed: {e}"))?;
        if !response.status().is_success() {
            let _ = self.sign_out();
            return Err("The LuaTools session expired. Sign in again from Settings.".to_string());
        }
        let refreshed: SupabaseSession = response
            .json()
            .await
            .map_err(|e| format!("Could not parse the refreshed LuaTools session: {e}"))?;
        let previous_display_name = session.display_name;
        let previous_email = session.email;
        session = stored_from_session(refreshed);
        session.display_name = session.display_name.or(previous_display_name);
        session.email = session.email.or(previous_email);
        self.save_session(&session)?;
        Ok(session.access_token)
    }

    fn persist_session(&self, session: SupabaseSession) -> Result<LuaToolsAuthStatus, String> {
        let stored = stored_from_session(session);
        self.save_session(&stored)?;
        Ok(LuaToolsAuthStatus {
            signed_in: true,
            display_name: stored.display_name,
            email: stored.email,
        })
    }

    fn load_session(&self) -> Result<Option<StoredSession>, String> {
        if !self.path.exists() {
            return Ok(None);
        }
        let encrypted = fs::read(&self.path)
            .map_err(|e| format!("Could not read the LuaTools session: {e}"))?;
        let plain = crate::core::secure_storage::unprotect(&encrypted)?;
        serde_json::from_slice(&plain)
            .map(Some)
            .map_err(|e| format!("Could not parse the stored LuaTools session: {e}"))
    }

    fn save_session(&self, session: &StoredSession) -> Result<(), String> {
        let plain = serde_json::to_vec(session)
            .map_err(|e| format!("Could not serialize the LuaTools session: {e}"))?;
        let encrypted = crate::core::secure_storage::protect(&plain)?;
        let parent = self
            .path
            .parent()
            .ok_or_else(|| "Invalid LuaTools session path".to_string())?;
        fs::create_dir_all(parent)
            .map_err(|e| format!("Could not create the LuaTools session folder: {e}"))?;
        let temp = self.path.with_extension("dat.tmp");
        fs::write(&temp, encrypted)
            .map_err(|e| format!("Could not write the LuaTools session: {e}"))?;
        fs::rename(&temp, &self.path)
            .map_err(|e| format!("Could not commit the LuaTools session: {e}"))
    }
}

fn stored_from_session(session: SupabaseSession) -> StoredSession {
    let (display_name, email) = session.user.map_or((None, None), |user| {
        let display_name = user.user_metadata.and_then(|metadata| {
            metadata
                .custom_claims
                .and_then(|claims| claims.global_name)
                .or(metadata.full_name)
                .or(metadata.name)
        });
        (display_name, user.email)
    });
    StoredSession {
        access_token: session.access_token,
        refresh_token: session.refresh_token,
        expires_at: now_unix().saturating_add(session.expires_in),
        display_name,
        email,
    }
}

async fn wait_for_callback(listener: TcpListener) -> Result<String, String> {
    loop {
        let (mut stream, _) = listener
            .accept()
            .await
            .map_err(|e| format!("LuaTools callback failed: {e}"))?;
        let mut request = Vec::with_capacity(1024);
        loop {
            let mut chunk = [0u8; 1024];
            let read = stream
                .read(&mut chunk)
                .await
                .map_err(|e| format!("Could not read the LuaTools callback: {e}"))?;
            if read == 0 {
                break;
            }
            request.extend_from_slice(&chunk[..read]);
            if request.windows(4).any(|window| window == b"\r\n\r\n") {
                break;
            }
            if request.len() >= 8192 {
                return Err("LuaTools callback request was unexpectedly large".to_string());
            }
        }
        let first_line = String::from_utf8_lossy(&request)
            .lines()
            .next()
            .unwrap_or("")
            .to_string();
        let target = first_line.split_whitespace().nth(1).unwrap_or("");
        let parsed = url::Url::parse(&format!("http://localhost{target}"));
        let (code, error) = match parsed {
            Ok(url) => {
                let mut code = None;
                let mut error = None;
                for (key, value) in url.query_pairs() {
                    match key.as_ref() {
                        "code" => code = Some(value.into_owned()),
                        "error_description" => error = Some(value.into_owned()),
                        _ => {}
                    }
                }
                (code, error)
            }
            Err(_) => (None, None),
        };

        if code.is_none() && error.is_none() {
            let _ = write_callback_response(&mut stream, false, "Not found").await;
            continue;
        }
        let ok = code.is_some();
        let message = error.as_deref().unwrap_or(if ok {
            "Signed in. You can close this tab and return to AetherDesk."
        } else {
            "LuaTools sign-in was denied."
        });
        let _ = write_callback_response(&mut stream, ok, message).await;
        if let Some(error) = error {
            return Err(format!("LuaTools sign-in failed: {error}"));
        }
        return code.ok_or_else(|| "LuaTools callback did not contain an authorization code".to_string());
    }
}

async fn write_callback_response(
    stream: &mut tokio::net::TcpStream,
    ok: bool,
    message: &str,
) -> Result<(), String> {
    let color = if ok { "#a78bfa" } else { "#f87171" };
    let escaped = message
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;");
    let body = format!(
        "<!doctype html><meta charset=\"utf-8\"><title>AetherDesk</title><style>body{{background:#0b0b12;color:#e5e7eb;font-family:Segoe UI,sans-serif;display:grid;place-items:center;height:100vh;margin:0}}div{{padding:40px;background:#14141c;border:1px solid #ffffff14;border-radius:14px}}h1{{color:{color}}}</style><div><h1>LuaTools</h1><p>{escaped}</p></div>"
    );
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        body.len(),
        body
    );
    stream
        .write_all(response.as_bytes())
        .await
        .map_err(|e| format!("Could not answer the LuaTools callback: {e}"))
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}
