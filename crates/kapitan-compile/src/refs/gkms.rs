//! Google Cloud KMS over its REST API, authenticated the way
//! `google.auth.default()` is: an explicit `GOOGLE_APPLICATION_CREDENTIALS`
//! file or the gcloud application-default credentials (`authorized_user`
//! refresh token or `service_account` key), the GCE metadata server, and
//! finally `gcloud auth application-default print-access-token`.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use serde_json::{Value as Json, json};

use super::{RefError, b64_decode, b64_encode};

const KMS: &str = "https://cloudkms.googleapis.com/v1";
const TOKEN_URI: &str = "https://oauth2.googleapis.com/token";
const METADATA_TOKEN: &str =
    "http://metadata.google.internal/computeMetadata/v1/instance/service-accounts/default/token";
const SCOPE: &str = "https://www.googleapis.com/auth/cloud-platform";

pub struct Client {
    agent: ureq::Agent,
    token: Mutex<Option<(String, Instant)>>,
}

impl Default for Client {
    fn default() -> Self {
        Self::new()
    }
}

impl Client {
    pub fn new() -> Client {
        let agent = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(60)))
            .http_status_as_error(false)
            .user_agent(format!("kapitan/{}", env!("CARGO_PKG_VERSION")))
            .build()
            .new_agent();
        Client {
            agent,
            token: Mutex::new(None),
        }
    }

    /// The ciphertext of `plaintext` under `key`
    /// (`projects/P/locations/L/keyRings/R/cryptoKeys/K`).
    pub fn encrypt(&self, key: &str, plaintext: &[u8]) -> Result<Vec<u8>, RefError> {
        if key == "mock" {
            // kapitan's own test double.
            return Ok(b64_encode(b"mock").into_bytes());
        }
        let resp = self.call(
            &format!("{KMS}/{key}:encrypt"),
            json!({ "plaintext": b64_encode(plaintext) }),
        )?;
        let ciphertext = resp
            .get("ciphertext")
            .and_then(Json::as_str)
            .ok_or_else(|| RefError("gkms: encrypt response has no ciphertext".into()))?;
        b64_decode(ciphertext)
    }

    pub fn decrypt(&self, key: &str, ciphertext: &[u8]) -> Result<Vec<u8>, RefError> {
        if key == "mock" {
            return Ok(b"mock".to_vec());
        }
        if key.is_empty() {
            return Err(RefError("gkms: reference has no key".into()));
        }
        let resp = self.call(
            &format!("{KMS}/{key}:decrypt"),
            json!({ "ciphertext": b64_encode(ciphertext) }),
        )?;
        let plaintext = resp
            .get("plaintext")
            .and_then(Json::as_str)
            .ok_or_else(|| RefError("gkms: decrypt response has no plaintext".into()))?;
        b64_decode(plaintext)
    }

    fn call(&self, url: &str, body: Json) -> Result<Json, RefError> {
        let token = self.token()?;
        let mut resp = self
            .agent
            .post(url)
            .header("Authorization", &format!("Bearer {token}"))
            .send_json(&body)
            .map_err(|e| RefError(format!("gkms: {url}: {e}")))?;
        let status = resp.status();
        let json: Json = resp
            .body_mut()
            .read_json()
            .map_err(|e| RefError(format!("gkms: {url}: invalid response: {e}")))?;
        if !status.is_success() {
            let message = json
                .pointer("/error/message")
                .and_then(Json::as_str)
                .unwrap_or("");
            return Err(RefError(format!("gkms: {url}: HTTP {status} {message}")));
        }
        Ok(json)
    }

    fn token(&self) -> Result<String, RefError> {
        let mut guard = self.token.lock();
        if let Some((t, expires)) = guard.as_ref()
            && *expires > Instant::now() + Duration::from_secs(60)
        {
            return Ok(t.clone());
        }
        let (token, ttl) = acquire_token(&self.agent)?;
        *guard = Some((token.clone(), Instant::now() + Duration::from_secs(ttl)));
        Ok(token)
    }
}

/// Where the application-default credentials file lives.
fn adc_file() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("GOOGLE_APPLICATION_CREDENTIALS") {
        return Some(PathBuf::from(p));
    }
    let config = std::env::var("CLOUDSDK_CONFIG")
        .map(PathBuf::from)
        .ok()
        .or_else(|| {
            std::env::var("HOME")
                .ok()
                .map(|h| PathBuf::from(h).join(".config/gcloud"))
        })?;
    let p = config.join("application_default_credentials.json");
    p.is_file().then_some(p)
}

/// An access token and how many seconds it is good for.
fn acquire_token(agent: &ureq::Agent) -> Result<(String, u64), RefError> {
    if let Ok(t) = std::env::var("GOOGLE_OAUTH_ACCESS_TOKEN")
        && !t.is_empty()
    {
        return Ok((t, 3600));
    }
    let mut tried = Vec::new();
    if let Some(path) = adc_file() {
        let text = std::fs::read_to_string(&path)
            .map_err(|e| RefError(format!("gkms: cannot read {}: {e}", path.display())))?;
        let creds: Json = serde_json::from_str(&text)
            .map_err(|e| RefError(format!("gkms: {} is not JSON: {e}", path.display())))?;
        match creds.get("type").and_then(Json::as_str) {
            Some("authorized_user") => return refresh_user_token(agent, &creds),
            Some("service_account") => return service_account_token(agent, &creds),
            other => tried.push(format!(
                "{}: credential type {} needs gcloud",
                path.display(),
                other.unwrap_or("?")
            )),
        }
    } else {
        tried.push("no application-default credentials file".to_string());
        match metadata_token(agent) {
            Ok(t) => return Ok(t),
            Err(e) => tried.push(e),
        }
    }
    match gcloud_token() {
        Ok(t) => Ok((t, 1800)),
        Err(e) => {
            tried.push(e);
            Err(RefError(format!(
                "gkms: no Google credentials found ({}); run `gcloud auth application-default login` or set GOOGLE_APPLICATION_CREDENTIALS",
                tried.join("; ")
            )))
        }
    }
}

fn token_response(
    mut resp: ureq::http::Response<ureq::Body>,
    what: &str,
) -> Result<(String, u64), RefError> {
    let status = resp.status();
    let json: Json = resp
        .body_mut()
        .read_json()
        .map_err(|e| RefError(format!("gkms: {what}: invalid token response: {e}")))?;
    if !status.is_success() {
        return Err(RefError(format!(
            "gkms: {what}: HTTP {status} {}",
            json.get("error_description")
                .or(json.get("error"))
                .map(|v| v.to_string())
                .unwrap_or_default()
        )));
    }
    let token = json
        .get("access_token")
        .and_then(Json::as_str)
        .ok_or_else(|| RefError(format!("gkms: {what}: no access_token in response")))?;
    let ttl = json
        .get("expires_in")
        .and_then(Json::as_u64)
        .unwrap_or(3600);
    Ok((token.to_string(), ttl))
}

fn refresh_user_token(agent: &ureq::Agent, creds: &Json) -> Result<(String, u64), RefError> {
    let field = |k: &str| {
        creds
            .get(k)
            .and_then(Json::as_str)
            .ok_or_else(|| RefError(format!("gkms: credentials file lacks `{k}`")))
    };
    let resp = agent
        .post(TOKEN_URI)
        .send_form([
            ("grant_type", "refresh_token"),
            ("client_id", field("client_id")?),
            ("client_secret", field("client_secret")?),
            ("refresh_token", field("refresh_token")?),
        ])
        .map_err(|e| RefError(format!("gkms: token refresh: {e}")))?;
    token_response(resp, "token refresh")
}

fn service_account_token(agent: &ureq::Agent, creds: &Json) -> Result<(String, u64), RefError> {
    use rsa::pkcs8::DecodePrivateKey;
    use rsa::signature::{SignatureEncoding, Signer};
    let field = |k: &str| {
        creds
            .get(k)
            .and_then(Json::as_str)
            .ok_or_else(|| RefError(format!("gkms: service account file lacks `{k}`")))
    };
    let token_uri = creds
        .get("token_uri")
        .and_then(Json::as_str)
        .unwrap_or(TOKEN_URI);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let url = base64::engine::general_purpose::URL_SAFE_NO_PAD;
    use base64::Engine as _;
    let header = url.encode(json!({"alg": "RS256", "typ": "JWT"}).to_string());
    let claims = url.encode(
        json!({
            "iss": field("client_email")?,
            "scope": SCOPE,
            "aud": token_uri,
            "iat": now,
            "exp": now + 3600,
        })
        .to_string(),
    );
    let signing_input = format!("{header}.{claims}");
    let key = rsa::RsaPrivateKey::from_pkcs8_pem(field("private_key")?)
        .map_err(|e| RefError(format!("gkms: service account private key: {e}")))?;
    let signer = rsa::pkcs1v15::SigningKey::<sha2::Sha256>::new(key);
    let signature = url.encode(signer.sign(signing_input.as_bytes()).to_bytes());
    let assertion = format!("{signing_input}.{signature}");
    let resp = agent
        .post(token_uri)
        .send_form([
            ("grant_type", "urn:ietf:params:oauth:grant-type:jwt-bearer"),
            ("assertion", assertion.as_str()),
        ])
        .map_err(|e| RefError(format!("gkms: service account token: {e}")))?;
    token_response(resp, "service account token")
}

fn metadata_token(_agent: &ureq::Agent) -> Result<(String, u64), String> {
    let agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(2)))
        .http_status_as_error(false)
        .build()
        .new_agent();
    let resp = agent
        .get(METADATA_TOKEN)
        .header("Metadata-Flavor", "Google")
        .call()
        .map_err(|e| format!("metadata server: {e}"))?;
    token_response(resp, "metadata server").map_err(|e| e.0)
}

fn gcloud_token() -> Result<String, String> {
    for args in [
        ["auth", "application-default", "print-access-token"].as_slice(),
        ["auth", "print-access-token"].as_slice(),
    ] {
        let out = std::process::Command::new("gcloud")
            .args(args)
            .stdin(std::process::Stdio::null())
            .output()
            .map_err(|e| format!("gcloud: {e}"))?;
        if out.status.success() {
            let token = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !token.is_empty() {
                return Ok(token);
            }
        }
    }
    Err("gcloud has no active credentials".into())
}
