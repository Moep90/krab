//! Reference functions (`?{type:path||func:arg|func2}`), kapitan's
//! `refs/functions.py`: they generate the secret a missing ref is created
//! with. Each function reads and updates the context's `data`; `base64` is
//! a marker that base64-encodes the result before it is stored.

use rand::rngs::OsRng;
use rand::seq::SliceRandom;
use sha2::{Digest, Sha256};

use super::{RefController, RefError, b64_decode, b64_encode};
use crate::inputs::Reads;

/// State carried across the functions of one tag.
#[derive(Debug)]
pub struct FunctionContext<'a> {
    pub data: Option<String>,
    pub encode_base64: bool,
    /// Encoding of the ref `reveal` read, so `publickey` can decode it.
    pub ref_encoding: String,
    pub token: &'a str,
}

impl<'a> FunctionContext<'a> {
    pub fn new(token: &'a str) -> Self {
        FunctionContext {
            data: None,
            encode_base64: false,
            ref_encoding: "original".into(),
            token,
        }
    }
}

pub const NAMES: [&str; 10] = [
    "randomstr",
    "random",
    "sha256",
    "ed25519",
    "rsa",
    "rsapublic",
    "publickey",
    "reveal",
    "loweralphanum",
    "basicauth",
];

const ASCII_LETTERS: &str = "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ";
const ASCII_LOWERCASE: &str = "abcdefghijklmnopqrstuvwxyz";
const ASCII_UPPERCASE: &str = "ABCDEFGHIJKLMNOPQRSTUVWXYZ";
const DIGITS: &str = "0123456789";
/// Python's `string.punctuation`.
pub const PUNCTUATION: &str = "!\"#$%&'()*+,-./:;<=>?@[\\]^_`{|}~";

/// Evaluate `||func:a:b|func2` against `ctx`.
pub fn eval_chain(
    ctx: &mut FunctionContext,
    func_str: &str,
    controller: &RefController,
    reads: &mut Reads,
) -> Result<(), RefError> {
    let body = func_str
        .strip_prefix("||")
        .ok_or_else(|| RefError(format!("{func_str}: functions must start with ||")))?;
    for func in body.split('|') {
        let mut parts = func.trim().split(':');
        let name = parts.next().unwrap_or("");
        let params: Vec<&str> = parts.collect();
        if name == "base64" {
            // Not a real function: encode the result before storing it.
            ctx.encode_base64 = true;
            continue;
        }
        eval_func(name, &params, ctx, controller, reads)?;
    }
    Ok(())
}

fn too_many(name: &str, params: &[&str]) -> RefError {
    RefError(format!(
        "{params:?}: too many arguments for function {name}"
    ))
}

fn eval_func(
    name: &str,
    params: &[&str],
    ctx: &mut FunctionContext,
    controller: &RefController,
    reads: &mut Reads,
) -> Result<(), RefError> {
    let arg = |i: usize, default: &str| -> String {
        params
            .get(i)
            .map(|s| s.to_string())
            .unwrap_or_else(|| default.to_string())
    };
    match name {
        "randomstr" => {
            if params.len() > 1 {
                return Err(too_many(name, params));
            }
            random(ctx, "str", &arg(0, ""), PUNCTUATION)
        }
        "random" => {
            if params.len() > 3 {
                return Err(too_many(name, params));
            }
            let special = arg(2, PUNCTUATION);
            random(ctx, &arg(0, "str"), &arg(1, ""), &special)
        }
        "loweralphanum" => {
            if params.len() > 1 {
                return Err(too_many(name, params));
            }
            random(ctx, "loweralphanum", &arg(0, "8"), PUNCTUATION)
        }
        "sha256" => {
            if params.len() > 1 {
                return Err(too_many(name, params));
            }
            sha256(ctx, &arg(0, ""))
        }
        "ed25519" => {
            if !params.is_empty() {
                return Err(too_many(name, params));
            }
            ed25519_private_key(ctx)
        }
        "rsa" => {
            if params.len() > 1 {
                return Err(too_many(name, params));
            }
            rsa_private_key(ctx, &arg(0, "4096"))
        }
        "rsapublic" => {
            if !params.is_empty() {
                return Err(too_many(name, params));
            }
            if ctx.data.as_deref().unwrap_or("").is_empty() {
                return Err(RefError(
                    "Ref error: eval_func: RSA public key cannot be derived; try something like '|reveal:path/to/encrypted_private_key|rsapublic'".into(),
                ));
            }
            public_key(ctx)
        }
        "publickey" => {
            if !params.is_empty() {
                return Err(too_many(name, params));
            }
            if ctx.data.as_deref().unwrap_or("").is_empty() {
                return Err(RefError(
                    "Ref error: eval_func: public key cannot be derived; try something like '|reveal:path/to/encrypted_private_key|publickey'".into(),
                ));
            }
            public_key(ctx)
        }
        "reveal" => {
            if params.len() != 1 {
                return Err(RefError(format!(
                    "{params:?}: reveal takes exactly one argument, the path of the reference to reveal"
                )));
            }
            reveal(ctx, params[0], controller, reads)
        }
        "basicauth" => {
            if params.len() > 2 {
                return Err(too_many(name, params));
            }
            basicauth(ctx, &arg(0, ""), &arg(1, ""))
        }
        other => Err(RefError(format!(
            "{other}: unknown ref function used. Choose one of: {NAMES:?}"
        ))),
    }
}

fn choose(pool: &[char], n: usize) -> String {
    let mut rng = OsRng;
    (0..n).map(|_| *pool.choose(&mut rng).unwrap()).collect()
}

/// `random(type="str", nchars="", special_chars=string.punctuation)`.
fn random(
    ctx: &mut FunctionContext,
    kind: &str,
    nchars: &str,
    special_chars: &str,
) -> Result<(), RefError> {
    let pool: String = match kind {
        "str" => format!("{ASCII_LETTERS}{DIGITS}-_"),
        "int" => DIGITS.to_string(),
        "loweralpha" => ASCII_LOWERCASE.to_string(),
        "upperalpha" => ASCII_UPPERCASE.to_string(),
        "loweralphanum" => format!("{ASCII_LOWERCASE}{DIGITS}"),
        "upperalphanum" => format!("{ASCII_UPPERCASE}{DIGITS}"),
        "special" => format!("{ASCII_LETTERS}{DIGITS}{special_chars}"),
        other => {
            return Err(RefError(format!(
                "{other}: unknown random type used. Choose one of ['str', 'int', 'loweralpha', 'upperalpha', 'loweralphanum', 'upperalphanum', 'special']"
            )));
        }
    };
    let n = if nchars.is_empty() {
        match kind {
            "str" => 43,
            "int" => 16,
            _ => 8,
        }
    } else {
        nchars.parse::<usize>().map_err(|_| {
            RefError(format!(
                "Ref error: eval_func: {nchars} cannot be converted into integer."
            ))
        })?
    };
    if kind != "special" && special_chars != PUNCTUATION {
        return Err(RefError(format!(
            "Ref error: eval_func: {kind} has no option to use special characters. Use type special instead, i.e. ||random:special:{special_chars}"
        )));
    }
    // Only letters, digits and punctuation, each once.
    let mut chars: Vec<char> = pool
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || PUNCTUATION.contains(*c))
        .collect();
    chars.sort_unstable();
    chars.dedup();
    ctx.data = Some(choose(&chars, n));
    Ok(())
}

fn sha256(ctx: &mut FunctionContext, salt: &str) -> Result<(), RefError> {
    match ctx.data.as_deref().filter(|d| !d.is_empty()) {
        Some(data) => {
            let salted = format!("{salt}:{data}");
            ctx.data = Some(hex::encode(Sha256::digest(salted.as_bytes())));
            Ok(())
        }
        None => Err(RefError(
            "Ref error: eval_func: nothing to sha256 hash; try something like '|random:str|sha256'"
                .into(),
        )),
    }
}

fn basicauth(ctx: &mut FunctionContext, username: &str, password: &str) -> Result<(), RefError> {
    let lower: Vec<char> = ASCII_LOWERCASE.chars().collect();
    let alnum: Vec<char> = format!("{ASCII_LETTERS}{DIGITS}").chars().collect();
    let username = if username.is_empty() {
        choose(&lower, 8)
    } else {
        username.to_string()
    };
    let password = if password.is_empty() {
        choose(&alnum, 8)
    } else {
        password.to_string()
    };
    ctx.data = Some(b64_encode(format!("{username}:{password}").as_bytes()));
    Ok(())
}

fn ed25519_private_key(ctx: &mut FunctionContext) -> Result<(), RefError> {
    use ed25519_dalek::pkcs8::{EncodePrivateKey, KeypairBytes, spki::der::pem::LineEnding};
    let key = ed25519_dalek::SigningKey::generate(&mut OsRng);
    // PKCS#8 v1 (no public key), like `cryptography` writes it.
    let bytes = KeypairBytes {
        secret_key: key.to_bytes(),
        public_key: None,
    };
    let pem = bytes
        .to_pkcs8_pem(LineEnding::LF)
        .map_err(|e| RefError(format!("ed25519: cannot encode key: {e}")))?;
    ctx.data = Some(pem.to_string());
    Ok(())
}

fn rsa_private_key(ctx: &mut FunctionContext, key_size: &str) -> Result<(), RefError> {
    use rsa::pkcs8::{EncodePrivateKey, LineEnding};
    let bits: usize = key_size.parse().map_err(|_| {
        RefError(format!(
            "Ref error: eval_func: {key_size} cannot be converted into integer."
        ))
    })?;
    let key = rsa::RsaPrivateKey::new(&mut OsRng, bits)
        .map_err(|e| RefError(format!("rsa: cannot generate a {bits}-bit key: {e}")))?;
    let pem = key
        .to_pkcs8_pem(LineEnding::LF)
        .map_err(|e| RefError(format!("rsa: cannot encode key: {e}")))?;
    ctx.data = Some(pem.to_string());
    Ok(())
}

/// The SubjectPublicKeyInfo PEM of the private key in `ctx.data` (RSA in
/// PKCS#8 or PKCS#1 form, or Ed25519 in PKCS#8).
fn public_key(ctx: &mut FunctionContext) -> Result<(), RefError> {
    let data = ctx.data.clone().unwrap_or_default();
    let pem = if ctx.ref_encoding == "base64" {
        String::from_utf8(b64_decode(&data)?)
            .map_err(|e| RefError(format!("publickey: private key is not UTF-8: {e}")))?
    } else {
        data
    };
    ctx.data = Some(public_key_pem(&pem)?);
    Ok(())
}

pub fn public_key_pem(private_pem: &str) -> Result<String, RefError> {
    use rsa::pkcs1::DecodeRsaPrivateKey;
    use rsa::pkcs8::{DecodePrivateKey, EncodePublicKey, LineEnding};
    if let Ok(key) = rsa::RsaPrivateKey::from_pkcs8_pem(private_pem)
        .or_else(|_| rsa::RsaPrivateKey::from_pkcs1_pem(private_pem))
    {
        return rsa::RsaPublicKey::from(&key)
            .to_public_key_pem(LineEnding::LF)
            .map_err(|e| RefError(format!("publickey: cannot encode RSA public key: {e}")));
    }
    if let Ok(key) = ed25519_dalek::SigningKey::from_pkcs8_pem(private_pem) {
        use ed25519_dalek::pkcs8::spki::EncodePublicKey as _;
        return key
            .verifying_key()
            .to_public_key_pem(LineEnding::LF)
            .map_err(|e| RefError(format!("publickey: cannot encode Ed25519 public key: {e}")));
    }
    Err(RefError(
        "publickey: data is not a PEM RSA or Ed25519 private key".into(),
    ))
}

/// `reveal:path`: the plaintext of another ref of the same type.
fn reveal(
    ctx: &mut FunctionContext,
    secret_path: &str,
    controller: &RefController,
    reads: &mut Reads,
) -> Result<(), RefError> {
    let type_name = RefController::token_type(ctx.token);
    let tag = format!("?{{{type_name}:{secret_path}}}");
    let r = controller.get(&tag, reads).map_err(|_| {
        RefError(format!(
            "|reveal function error: {secret_path} file in {}|reveal:{secret_path} does not exist",
            ctx.token
        ))
    })?;
    ctx.ref_encoding = r.encoding.clone();
    ctx.data = Some(controller.reveal_ref(&r)?);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(funcs: &str) -> Result<FunctionContext<'static>, RefError> {
        let rc = RefController::new(std::env::temp_dir(), false);
        let mut reads = Reads::default();
        let mut ctx = FunctionContext::new("base64:x");
        eval_chain(&mut ctx, funcs, &rc, &mut reads)?;
        Ok(ctx)
    }

    #[test]
    fn random_pools_and_lengths() {
        let s = run("||random").unwrap().data.unwrap();
        assert_eq!(s.len(), 43);
        assert!(
            s.chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        );
        let s = run("||random:int").unwrap().data.unwrap();
        assert_eq!(s.len(), 16);
        assert!(s.chars().all(|c| c.is_ascii_digit()));
        let s = run("||random:loweralpha:5").unwrap().data.unwrap();
        assert_eq!(s.len(), 5);
        assert!(s.chars().all(|c| c.is_ascii_lowercase()));
        let s = run("||random:upperalphanum").unwrap().data.unwrap();
        assert_eq!(s.len(), 8);
        let s = run("||random:special:100:#").unwrap().data.unwrap();
        assert_eq!(s.len(), 100);
        assert!(s.chars().all(|c| c.is_ascii_alphanumeric() || c == '#'));
        assert!(
            run("||random:str:x")
                .unwrap_err()
                .0
                .contains("cannot be converted")
        );
        assert!(
            run("||random:str:8:#")
                .unwrap_err()
                .0
                .contains("no option to use special")
        );
        assert!(
            run("||random:bogus")
                .unwrap_err()
                .0
                .contains("unknown random type")
        );
        assert!(run("||randomstr:7").unwrap().data.unwrap().len() == 7);
        assert!(run("||loweralphanum").unwrap().data.unwrap().len() == 8);
        assert!(
            run("||nope")
                .unwrap_err()
                .0
                .contains("unknown ref function")
        );
        assert!(
            run("||random:a:b:c:d")
                .unwrap_err()
                .0
                .contains("too many arguments")
        );
    }

    #[test]
    fn sha256_and_basicauth_and_base64_marker() {
        let ctx = run("||random:str|sha256:salt|base64").unwrap();
        assert!(ctx.encode_base64);
        assert_eq!(ctx.data.unwrap().len(), 64);
        assert!(run("||sha256").unwrap_err().0.contains("nothing to sha256"));
        let token = run("||basicauth:admin:secret").unwrap().data.unwrap();
        assert_eq!(b64_decode(&token).unwrap(), b"admin:secret");
        let token = run("||basicauth").unwrap().data.unwrap();
        let decoded = String::from_utf8(b64_decode(&token).unwrap()).unwrap();
        let (u, p) = decoded.split_once(':').unwrap();
        assert_eq!((u.len(), p.len()), (8, 8));
    }

    #[test]
    fn keys_and_public_keys() {
        let rsa_pem = run("||rsa:1024").unwrap().data.unwrap();
        assert!(rsa_pem.starts_with("-----BEGIN PRIVATE KEY-----\n"));
        assert!(rsa_pem.ends_with("-----END PRIVATE KEY-----\n"));
        let public = public_key_pem(&rsa_pem).unwrap();
        assert!(public.starts_with("-----BEGIN PUBLIC KEY-----\n"));
        let ed_pem = run("||ed25519").unwrap().data.unwrap();
        assert!(ed_pem.starts_with("-----BEGIN PRIVATE KEY-----\n"));
        // v1 PKCS#8 of an Ed25519 key is 48 bytes DER = 64 base64 chars, one line.
        assert_eq!(ed_pem.lines().count(), 3, "{ed_pem}");
        let public = public_key_pem(&ed_pem).unwrap();
        assert!(public.starts_with("-----BEGIN PUBLIC KEY-----\n"));
        assert!(
            run("||publickey")
                .unwrap_err()
                .0
                .contains("cannot be derived")
        );
        assert!(
            run("||rsapublic")
                .unwrap_err()
                .0
                .contains("RSA public key cannot be derived")
        );
    }
}
