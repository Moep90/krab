//! AWS KMS and Azure Key Vault refs through their command line clients
//! (`aws kms`, `az keyvault key`), which carry the credential handling
//! kapitan gets from boto3 and `DefaultAzureCredential`.

use std::process::{Command, Stdio};

use serde_json::Value as Json;

use super::{RefError, b64_decode, b64_encode};

fn run_json(mut cmd: Command, what: &str) -> Result<Json, RefError> {
    let out = cmd
        .stdin(Stdio::null())
        .output()
        .map_err(|e| RefError(format!("{what}: cannot run {:?}: {e}", cmd.get_program())))?;
    if !out.status.success() {
        return Err(RefError(format!(
            "{what} failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        )));
    }
    serde_json::from_slice(&out.stdout)
        .map_err(|e| RefError(format!("{what}: output is not JSON: {e}")))
}

fn field<'a>(json: &'a Json, key: &str, what: &str) -> Result<&'a str, RefError> {
    json.get(key)
        .and_then(Json::as_str)
        .ok_or_else(|| RefError(format!("{what}: response has no `{key}`")))
}

/// A file only the current user can read, removed on drop.
struct TempFile(std::path::PathBuf);

impl TempFile {
    fn new(prefix: &str, data: &[u8]) -> Result<TempFile, RefError> {
        use std::io::Write as _;
        let path = std::env::temp_dir().join(format!(
            "kapitan-{prefix}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let mut opts = std::fs::OpenOptions::new();
        opts.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        let mut f = opts
            .open(&path)
            .map_err(|e| RefError(format!("cannot create {}: {e}", path.display())))?;
        f.write_all(data)
            .map_err(|e| RefError(format!("cannot write {}: {e}", path.display())))?;
        Ok(TempFile(path))
    }
}

impl Drop for TempFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

pub fn aws_encrypt(key: &str, plaintext: &[u8]) -> Result<Vec<u8>, RefError> {
    if key == "mock" {
        return Ok(b64_encode(b"mock").into_bytes());
    }
    let file = TempFile::new("awskms", plaintext)?;
    let mut cmd = Command::new("aws");
    cmd.args(["kms", "encrypt", "--key-id", key, "--plaintext"])
        .arg(format!("fileb://{}", file.0.display()))
        .args(["--output", "json"]);
    let out = run_json(cmd, "awskms: encrypt")?;
    b64_decode(field(&out, "CiphertextBlob", "awskms: encrypt")?)
}

pub fn aws_decrypt(key: &str, ciphertext: &[u8]) -> Result<Vec<u8>, RefError> {
    if key == "mock" {
        return Ok(b"mock".to_vec());
    }
    let file = TempFile::new("awskms", ciphertext)?;
    let mut cmd = Command::new("aws");
    cmd.args(["kms", "decrypt", "--ciphertext-blob"])
        .arg(format!("fileb://{}", file.0.display()))
        .args(["--output", "json"]);
    let out = run_json(cmd, "awskms: decrypt")?;
    b64_decode(field(&out, "Plaintext", "awskms: decrypt")?)
}

/// `https://vault.vault.azure.net/keys/name/version`, with or without the scheme.
fn az_key_id(key: &str) -> String {
    if key.starts_with("https://") {
        key.to_string()
    } else {
        format!("https://{key}")
    }
}

fn az_crypto(op: &str, key: &str, value_b64: &str) -> Result<Vec<u8>, RefError> {
    let mut cmd = Command::new("az");
    cmd.args(["keyvault", "key", op, "--id"])
        .arg(az_key_id(key))
        .args([
            "--algorithm",
            "RSA-OAEP-256",
            "--data-type",
            "base64",
            "--value",
            value_b64,
            "-o",
            "json",
        ]);
    let what = format!("azkms: {op}");
    let out = run_json(cmd, &what)?;
    b64_decode(field(&out, "result", &what)?)
}

pub fn az_encrypt(key: &str, plaintext: &[u8]) -> Result<Vec<u8>, RefError> {
    if key == "mock" {
        return Ok(b64_encode(plaintext).into_bytes());
    }
    az_crypto("encrypt", key, &b64_encode(plaintext))
}

pub fn az_decrypt(key: &str, ciphertext: &[u8]) -> Result<Vec<u8>, RefError> {
    if key == "mock" {
        return Ok(b"mock".to_vec());
    }
    az_crypto("decrypt", key, &b64_encode(ciphertext))
}
