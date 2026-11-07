use super::*;
use crate::ai::{
    Answer, INSTRUCTIONS, Message,
    credentials::{NativeStore, Store, StoreError},
};
use base64::{
    Engine,
    engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD},
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    io::{BufRead, BufReader, Read, Seek, SeekFrom, Write},
    net::TcpListener,
    path::PathBuf,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

const ISSUER: &str = "https://auth.openai.com";
const AUTHORIZE: &str = "https://auth.openai.com/api/accounts/authorize";
const TOKEN: &str = "https://auth.openai.com/api/accounts/oauth/token";
const RESOURCE: &str = "https://api.openai.com/v1";
const SCOPES: &str =
    "openid profile email offline_access resource.invoke chatgpt.tokens.use.direct";
const DYNAMIC: &str = "dynamic_agent_client";
fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
fn random() -> Result<String, String> {
    let mut b = [0; 32];
    getrandom::fill(&mut b).map_err(|_| "Secure random generation failed.")?;
    Ok(URL_SAFE_NO_PAD.encode(b))
}
fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(45)))
        .max_redirects(0)
        .http_status_as_error(false)
        .build()
        .into()
}
fn get(url: &str) -> Result<Value, String> {
    let mut r = agent()
        .get(url)
        .call()
        .map_err(|_| "Could not reach OpenAI's sign-in service.")?;
    if !r.status().is_success() {
        return Err(format!(
            "OpenAI sign-in service returned HTTP {}.",
            r.status().as_u16()
        ));
    }
    r.body_mut()
        .with_config()
        .limit(256 * 1024)
        .read_json()
        .map_err(|_| "Invalid sign-in service response.".into())
}
enum AuthFailure {
    Permanent(String),
    Temporary(String),
}
impl From<AuthFailure> for String {
    fn from(error: AuthFailure) -> Self {
        match error {
            AuthFailure::Permanent(s) | AuthFailure::Temporary(s) => s,
        }
    }
}
fn form(url: &str, fields: &[(&str, &str)]) -> Result<Value, AuthFailure> {
    let mut r = agent()
        .post(url)
        .send_form(fields.iter().copied())
        .map_err(|_| {
            AuthFailure::Temporary(
                "Could not reach OpenAI's authorization service. Try again.".into(),
            )
        })?;
    if !r.status().is_success() {
        let v: Value = r
            .body_mut()
            .with_config()
            .limit(65536)
            .read_json()
            .unwrap_or(Value::Null);
        let code = v.get("error").and_then(Value::as_str).unwrap_or("");
        return Err(match code {
            "invalid_grant" | "invalid_client" => AuthFailure::Permanent("The ChatGPT authorization expired or was revoked. Continue with ChatGPT again.".into()),
            "access_denied" => AuthFailure::Permanent("ChatGPT plan access was not authorized. Continue with ChatGPT and approve plan usage.".into()),
            _ => AuthFailure::Temporary("OpenAI could not authorize this session. Try again or check your account's app permissions.".into()),
        });
    }
    r.body_mut()
        .with_config()
        .limit(256 * 1024)
        .read_json()
        .map_err(|_| AuthFailure::Temporary("Invalid token response.".into()))
}
fn metadata() -> Result<Value, String> {
    let v = get("https://auth.openai.com/.well-known/openid-configuration")?;
    if v["issuer"] != ISSUER {
        return Err("Unexpected OpenAI identity issuer.".into());
    }
    Ok(v)
}
fn trusted_auth_url(value: &str) -> Result<&str, String> {
    let u = url::Url::parse(value).map_err(|_| "Invalid identity endpoint.")?;
    if u.scheme() != "https"
        || u.host_str() != Some("auth.openai.com")
        || u.port_or_known_default() != Some(443)
        || !u.username().is_empty()
        || u.password().is_some()
        || u.query().is_some()
        || u.fragment().is_some()
    {
        return Err("Unexpected OpenAI identity endpoint.".into());
    }
    Ok(value)
}
fn file_options() -> std::fs::OpenOptions {
    let mut o = std::fs::OpenOptions::new();
    o.read(true).write(true).create(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        o.mode(0o600);
    }
    o
}
fn dir() -> Result<PathBuf, String> {
    let p =
        crate::settings::private_app_dir().ok_or("No private settings directory is available.")?;
    std::fs::create_dir_all(&p).map_err(|_| "Could not create the app settings directory.")?;
    Ok(p)
}
fn host_id() -> Result<String, String> {
    host_id_at(&dir()?.join("chatgpt-host-id"))
}
fn host_id_at(path: &std::path::Path) -> Result<String, String> {
    let file = file_options()
        .open(path)
        .map_err(|_| "Could not persist the ChatGPT host identity.")?;
    fs2::FileExt::lock_exclusive(&file).map_err(|_| "Could not lock the ChatGPT host identity.")?;
    let mut file = &file;
    let mut old = String::new();
    file.take(256)
        .read_to_string(&mut old)
        .map_err(|_| "Could not read the ChatGPT host identity.")?;
    if valid_host_id(&old) {
        return Ok(old);
    }
    // Migrate the unsupported identifier written by the first implementation before retrying OAuth.
    if !(old.is_empty() || old.starts_with("honeydew-") && old.len() < 128) {
        return Err("Invalid ChatGPT host identity file.".into());
    }
    let id = new_host_id()?;
    file.seek(SeekFrom::Start(0))
        .and_then(|_| file.write_all(id.as_bytes()))
        .and_then(|_| file.set_len(id.len() as u64))
        .and_then(|_| file.sync_all())
        .map_err(|_| "Could not save the ChatGPT host identity.")?;
    Ok(id)
}
// RFC 9562 UUIDv4, encoded as the UUID URN required by OpenAI's host identifier contract.
fn new_host_id() -> Result<String, String> {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes)
        .map_err(|_| "Could not generate a secure ChatGPT host identity.")?;
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    Ok(format!(
        "urn:uuid:{}-{}-{}-{}-{}",
        &hex[..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..]
    ))
}
fn valid_host_id(id: &str) -> bool {
    let Some(uuid) = id.strip_prefix("urn:uuid:") else {
        return false;
    };
    uuid.len() == 36
        && uuid.bytes().enumerate().all(|(i, b)| {
            if [8, 13, 18, 23].contains(&i) {
                b == b'-'
            } else {
                b.is_ascii_hexdigit()
            }
        })
        && uuid.as_bytes()[14] == b'4'
        && matches!(uuid.as_bytes()[19], b'8' | b'9' | b'a' | b'b')
}
fn session_lock(account: &Account) -> Result<std::fs::File, String> {
    let p = dir()?.join(format!("chatgpt-{}.lock", vault_id(account)));
    let f = file_options()
        .open(p)
        .map_err(|_| "Could not open the ChatGPT session lock.")?;
    fs2::FileExt::lock_exclusive(&f).map_err(|_| "Could not lock the ChatGPT session.")?;
    Ok(f)
}
fn vault_id(a: &Account) -> String {
    format!(
        "{:x}",
        Sha256::digest(format!("{}\n{}", a.client_id, a.subject))
    )
}
fn vault_key(a: &Account) -> String {
    format!("chatgpt-session-v1-{}", vault_id(a))
}
#[derive(Serialize, Deserialize)]
struct Manifest {
    generation: String,
    count: usize,
    digest: String,
}
fn manifest(store: &impl Store, key: &str) -> Result<Manifest, String> {
    let raw=store.get(key).map_err(|_|"No remembered ChatGPT session is available. Sign in again, or unlock your system credential store.")?;
    let m: Manifest = serde_json::from_str(&raw).map_err(|_| "Invalid saved ChatGPT session.")?;
    if m.count == 0
        || m.count > 64
        || m.generation.len() > 64
        || !m
            .generation
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_".contains(&b))
    {
        return Err("Invalid saved ChatGPT session manifest.".into());
    }
    Ok(m)
}
fn erase_chunks(store: &impl Store, key: &str, m: &Manifest) {
    for i in 0..m.count {
        let _ = store.delete(&format!("{key}.{}.{i}", m.generation));
    }
}
fn save_tokens(store: &impl Store, a: &Account, t: &Tokens) -> Result<(), String> {
    let data = serde_json::to_vec(t).map_err(|_| "Could not encode ChatGPT credentials.")?;
    if data.len() > 64 * 1536 {
        return Err("ChatGPT credential record is too large for secure storage.".into());
    }
    let key = vault_key(a);
    let old = manifest(store, &key).ok();
    let m = Manifest {
        generation: random()?,
        count: data.len().div_ceil(1536),
        digest: format!("{:x}", Sha256::digest(&data)),
    };
    for (i, chunk) in data.chunks(1536).enumerate() {
        if store
            .set(
                &format!("{key}.{}.{i}", m.generation),
                &STANDARD.encode(chunk),
            )
            .is_err()
        {
            erase_chunks(store, &key, &m);
            return Err("Could not save the ChatGPT session in your system credential store. This session remains in memory only.".into());
        }
    }
    if store
        .set(
            &key,
            &serde_json::to_string(&m).map_err(|_| "Could not encode secure session metadata.")?,
        )
        .is_err()
    {
        erase_chunks(store, &key, &m);
        return Err(
            "Could not commit the remembered ChatGPT session. It remains in memory only.".into(),
        );
    }
    if let Some(old) = old {
        erase_chunks(store, &key, &old);
    }
    Ok(())
}
fn read_tokens(store: &impl Store, a: &Account) -> Result<Tokens, String> {
    let key = vault_key(a);
    let m = manifest(store, &key)?;
    let mut bytes = Vec::new();
    for i in 0..m.count {
        let s = store
            .get(&format!("{key}.{}.{i}", m.generation))
            .map_err(|_| "Could not read the secure ChatGPT session. Sign in again.")?;
        let chunk = STANDARD
            .decode(s)
            .map_err(|_| "Invalid secure session encoding.")?;
        if chunk.len() > 1536 {
            return Err("Invalid secure session size.".into());
        }
        bytes.extend(chunk);
    }
    if format!("{:x}", Sha256::digest(&bytes)) != m.digest {
        return Err("Saved ChatGPT session is incomplete. Sign in again.".into());
    }
    let t: Tokens = serde_json::from_slice(&bytes).map_err(|_| "Invalid saved ChatGPT session.")?;
    if t.client_id != a.client_id || t.subject != a.subject {
        return Err("Saved session identity does not match the selected ChatGPT account.".into());
    }
    Ok(t)
}
fn delete_tokens(store: &impl Store, a: &Account) -> Result<(), String> {
    let key = vault_key(a);
    match store.get(&key) {
        Err(StoreError::Missing) => return Ok(()),
        Err(StoreError::Unavailable) => {
            return Err(
                "Could not remove the remembered session. Unlock the credential store and retry."
                    .into(),
            );
        }
        Ok(_) => {}
    }
    // Remove the pointer even if its contents are damaged. Never mistake a locked vault for an absent session.
    let saved_manifest = manifest(store, &key);
    match store.delete(&key) {
        Ok(()) | Err(StoreError::Missing) => {}
        Err(StoreError::Unavailable) => {
            return Err(
                "Could not remove the remembered session. Unlock the credential store and retry."
                    .into(),
            );
        }
    }
    if let Ok(m) = saved_manifest {
        for i in 0..m.count {
            match store.delete(&format!("{key}.{}.{i}", m.generation)) {
                Ok(()) | Err(StoreError::Missing) => {},
                Err(StoreError::Unavailable) => return Err("Automatic loading is disabled, but some saved credentials could not be removed. Unlock your credential store and remove Honeydew's saved ChatGPT session.".into()),
            }
        }
    } else {
        return Err("Removed the saved session pointer, but damaged metadata prevented complete credential cleanup in the system store.".into());
    }
    Ok(())
}

/// Holds an already-bound loopback listener and per-attempt secrets. Never format it or its authorization URL.
pub struct Login {
    pub url: String,
    listener: TcpListener,
    redirect: String,
    state: String,
    nonce: String,
    verifier: String,
    returning: Option<Account>,
    pub cancel: Arc<AtomicBool>,
    remember: bool,
}
pub fn begin(returning: Option<Account>, remember: bool) -> Result<Login, String> {
    let host = host_id()?;
    let listener = TcpListener::bind(("127.0.0.1", 0))
        .map_err(|_| "Could not start the local sign-in callback.")?;
    listener
        .set_nonblocking(true)
        .map_err(|_| "Could not configure sign-in callback.")?;
    let redirect = format!(
        "http://127.0.0.1:{}/auth/callback",
        listener
            .local_addr()
            .map_err(|_| "No callback port.")?
            .port()
    );
    let state = random()?;
    let nonce = random()?;
    let verifier = random()?;
    let mut url = url::Url::parse(AUTHORIZE).map_err(|_| "Invalid authorization URL.")?;
    {
        let mut q = url.query_pairs_mut();
        q.append_pair(
            "client_id",
            returning.as_ref().map_or(DYNAMIC, |a| a.client_id.as_str()),
        )
        .append_pair("ext_agent_host_id", &host)
        .append_pair("response_type", "code")
        .append_pair("redirect_uri", &redirect)
        .append_pair("scope", SCOPES)
        .append_pair("resource", RESOURCE)
        .append_pair("state", &state)
        .append_pair("nonce", &nonce)
        .append_pair("code_challenge_method", "S256")
        .append_pair(
            "code_challenge",
            &URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes())),
        );
        if returning.is_none() {
            q.append_pair("agent_name_hint", "Honeydew Dewey");
        } else if let Some(a) = &returning {
            if !a.email.is_empty() {
                q.append_pair("login_hint", &a.email);
            }
            if let Ok(t) = a.session.0.tokens.try_lock()
                && let Some(t) = &*t
                && !t.id_token.is_empty()
            {
                q.append_pair("id_token_hint", &t.id_token);
            }
        }
    }
    Ok(Login {
        url: url.into(),
        listener,
        redirect,
        state,
        nonce,
        verifier,
        returning,
        cancel: Arc::new(AtomicBool::new(false)),
        remember,
    })
}
fn callback_target(login: &Login, target: &str) -> Result<(String, String), String> {
    let url = url::Url::parse(&format!("http://127.0.0.1{target}"))
        .map_err(|_| "Invalid callback URL.")?;
    if url.path() != "/auth/callback" {
        return Err("Not an authorization callback.".into());
    }
    let mut values = std::collections::HashMap::new();
    for (k, v) in url.query_pairs() {
        if values.insert(k.into_owned(), v.into_owned()).is_some() {
            return Err("Duplicate callback parameters.".into());
        }
    }
    if values.get("state") != Some(&login.state) {
        return Err("Sign-in state did not match this attempt.".into());
    }
    if values.contains_key("error") {
        return Err("ChatGPT sign-in was cancelled or plan access was not approved.".into());
    }
    let code = values
        .get("code")
        .filter(|s| !s.is_empty())
        .ok_or("The authorization callback did not contain a code.")?
        .clone();
    let client = if let Some(a) = &login.returning {
        if values.get("client_id").is_some_and(|id| id != &a.client_id) {
            return Err("The callback changed the registered client identity.".into());
        }
        a.client_id.clone()
    } else {
        values
            .get("client_id")
            .filter(|id| id.as_str() != DYNAMIC && !id.is_empty() && id.len() < 256)
            .ok_or("OpenAI did not issue a client ID. Start sign-in again.")?
            .clone()
    };
    Ok((code, client))
}
fn callback(login: &Login) -> Result<(String, String), String> {
    let deadline = Instant::now() + Duration::from_secs(300);
    while Instant::now() < deadline && !login.cancel.load(Ordering::Acquire) {
        match login.listener.accept() {
            Ok((mut stream, peer)) => {
                if !peer.ip().is_loopback() {
                    continue;
                }
                let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
                let _ = stream.set_write_timeout(Some(Duration::from_secs(2)));
                let mut data = Vec::new();
                let mut buf = [0; 1024];
                while data.len() < 16384 {
                    let n = stream
                        .read(&mut buf)
                        .map_err(|_| "Could not read the local sign-in callback.")?;
                    if n == 0 {
                        break;
                    }
                    data.extend_from_slice(&buf[..n]);
                    if data.windows(4).any(|w| w == b"\r\n\r\n") {
                        break;
                    }
                }
                let line = std::str::from_utf8(&data)
                    .ok()
                    .and_then(|s| s.lines().next())
                    .unwrap_or("");
                let mut pieces = line.split_whitespace();
                let method = pieces.next();
                let target = pieces.next().unwrap_or("");
                let valid_path = target.starts_with("/auth/callback?");
                let result = if method == Some("GET") && valid_path {
                    callback_target(login, target)
                } else {
                    Err("Not a sign-in callback.".into())
                };
                let message = if result.is_ok() {
                    "Authorization received. Return to Honeydew to finish signing in."
                } else {
                    "This sign-in callback was not accepted. Return to Honeydew."
                };
                let reply = format!(
                    "HTTP/1.1 {}\r\nContent-Type: text/plain; charset=utf-8\r\nCache-Control: no-store\r\nReferrer-Policy: no-referrer\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{}",
                    if result.is_ok() {
                        "200 OK"
                    } else {
                        "400 Bad Request"
                    },
                    message.len(),
                    message
                );
                let _ = stream.write_all(reply.as_bytes());
                if result.is_ok() {
                    return result;
                }
                // Ignore unsolicited callbacks, but finish a genuine denial with the matching state.
                if valid_path
                    && target.contains(&format!("state={}", login.state))
                    && target.contains("error=")
                {
                    return result;
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(50))
            }
            Err(_) => return Err("The local sign-in listener stopped.".into()),
        }
    }
    Err(if login.cancel.load(Ordering::Acquire) {
        "ChatGPT sign-in cancelled."
    } else {
        "ChatGPT sign-in timed out. Please try again."
    }
    .into())
}
fn validated_id(
    token: &str,
    client: &str,
    nonce: Option<&str>,
    expected_sub: Option<&str>,
) -> Result<Value, String> {
    let header = jsonwebtoken::decode_header(token).map_err(|_| "Invalid identity token.")?;
    if header.alg != jsonwebtoken::Algorithm::RS256 {
        return Err("Unsupported identity-token signature.".into());
    }
    let meta = metadata()?;
    let uri = trusted_auth_url(
        meta["jwks_uri"]
            .as_str()
            .ok_or("Missing identity signing-key endpoint.")?,
    )?;
    verify_id(token, client, nonce, expected_sub, &get(uri)?)
}
fn verify_id(
    token: &str,
    client: &str,
    nonce: Option<&str>,
    expected_sub: Option<&str>,
    jwks: &Value,
) -> Result<Value, String> {
    use jsonwebtoken::{Algorithm, DecodingKey, Validation, decode, decode_header};
    let header = decode_header(token).map_err(|_| "Invalid ChatGPT identity token.")?;
    if header.alg != Algorithm::RS256 {
        return Err("Unsupported identity-token signature.".into());
    }
    let kid = header.kid.ok_or("Identity token has no signing-key ID.")?;
    let jwk = jwks["keys"]
        .as_array()
        .and_then(|keys| {
            keys.iter().find(|k| {
                k["kid"] == kid && k["kty"] == "RSA" && k.get("alg").is_none_or(|a| a == "RS256")
            })
        })
        .ok_or("Identity signing key was not found.")?;
    let key = DecodingKey::from_rsa_components(
        jwk["n"].as_str().ok_or("Invalid RSA key.")?,
        jwk["e"].as_str().ok_or("Invalid RSA key.")?,
    )
    .map_err(|_| "Invalid identity signing key.")?;
    let mut validation = Validation::new(Algorithm::RS256);
    validation.validate_nbf = true;
    validation.set_audience(&[client]);
    validation.set_issuer(&[ISSUER]);
    validation.set_required_spec_claims(&["exp", "iss", "aud", "sub"]);
    let claims = decode::<Value>(token, &key, &validation)
        .map_err(
            |_| "ChatGPT identity signature, issuer, audience or expiry could not be verified.",
        )?
        .claims;
    if let Some(nonce) = nonce
        && claims["nonce"].as_str() != Some(nonce)
    {
        return Err("ChatGPT identity nonce did not match this sign-in attempt.".into());
    }
    if claims
        .get("azp")
        .and_then(Value::as_str)
        .is_some_and(|azp| azp != client)
        || (claims["aud"].as_array().is_some_and(|a| a.len() > 1)
            && claims["azp"].as_str() != Some(client))
    {
        return Err("ChatGPT identity authorized-party validation failed.".into());
    }
    let sub = claims["sub"]
        .as_str()
        .filter(|s| !s.is_empty())
        .ok_or("Identity token has no account subject.")?;
    if expected_sub.is_some_and(|expected| expected != sub) {
        return Err("The signed-in identity does not match this saved account. Add it as a different account instead.".into());
    }
    Ok(claims)
}
fn parse_tokens(
    v: Value,
    client: &str,
    claims: &Value,
    old: Option<&Tokens>,
) -> Result<Tokens, String> {
    let scope = v["scope"]
        .as_str()
        .or_else(|| old.map(|t| t.scope.as_str()))
        .unwrap_or("")
        .to_owned();
    for required in ["resource.invoke", "chatgpt.tokens.use.direct"] {
        if !scope.split_whitespace().any(|s| s == required) {
            return Err(
                "ChatGPT plan usage was not authorized. Sign in again and approve plan access."
                    .into(),
            );
        }
    }
    if !v["token_type"]
        .as_str()
        .is_some_and(|s| s.eq_ignore_ascii_case("Bearer"))
    {
        return Err("Unsupported ChatGPT token type.".into());
    }
    let expires = v["expires_in"]
        .as_u64()
        .filter(|n| *n > 0 && *n < 86400 * 30)
        .ok_or("Invalid token expiry.")?;
    Ok(Tokens {
        client_id: client.into(),
        subject: claims["sub"]
            .as_str()
            .ok_or("Missing verified identity.")?
            .into(),
        email: claims["email"].as_str().unwrap_or("").into(),
        access_token: v["access_token"]
            .as_str()
            .filter(|s| !s.is_empty())
            .ok_or("No access token was returned.")?
            .into(),
        refresh_token: v["refresh_token"]
            .as_str()
            .or_else(|| old.map(|t| t.refresh_token.as_str()))
            .unwrap_or("")
            .into(),
        id_token: v["id_token"]
            .as_str()
            .or_else(|| old.map(|t| t.id_token.as_str()))
            .unwrap_or("")
            .into(),
        scope,
        expires_at: now().saturating_add(expires),
    })
}
fn install(a: &Account, t: Tokens) {
    *a.session.0.tokens.lock().unwrap_or_else(|e| e.into_inner()) = Some(t);
    a.session.0.ready.store(true, Ordering::Release);
    a.mark_loading();
}
pub fn finish(login: Login) -> Result<Account, String> {
    let (code, client) = callback(&login)?;
    if login.cancel.load(Ordering::Acquire) {
        return Err("ChatGPT sign-in cancelled.".into());
    }
    let v = form(
        TOKEN,
        &[
            ("grant_type", "authorization_code"),
            ("client_id", &client),
            ("code", &code),
            ("code_verifier", &login.verifier),
            ("redirect_uri", &login.redirect),
            ("resource", RESOURCE),
        ],
    )?;
    let id = v["id_token"]
        .as_str()
        .ok_or("OpenAI did not return an identity token.")?;
    let claims = validated_id(
        id,
        &client,
        Some(&login.nonce),
        login.returning.as_ref().map(|a| a.subject.as_str()),
    )?;
    let t = parse_tokens(v, &client, &claims, None)?;
    let mut a = Account {
        client_id: client,
        subject: t.subject.clone(),
        email: t.email.clone(),
        remember: login.remember,
        session: Session::default(),
    };
    // Cancellation takes effect until this commit point. Once committing starts, return the account
    // to the UI so a late cancel cannot hide a successfully persisted session.
    if login.cancel.load(Ordering::Acquire) {
        let revoked = revoke(&a, &t);
        return Err(if revoked { "ChatGPT sign-in cancelled." } else {
            "ChatGPT sign-in cancelled. Remote revocation was not confirmed; disconnect Honeydew Dewey in ChatGPT Settings → Usage."
        }.into());
    }
    if a.remember {
        let saved = session_lock(&a).and_then(|_lock| save_tokens(&NativeStore, &a, &t));
        if let Err(e) = saved {
            a.remember = false;
            *a.session.0.notice.lock().unwrap_or_else(|p| p.into_inner()) = e;
        }
    }
    install(&a, t);
    Ok(a)
}
pub fn restore(a: &Account) -> Result<(), String> {
    let _lock = session_lock(a)?;
    let t = read_tokens(&NativeStore, a)?;
    install(a, t);
    Ok(())
}
pub fn remember(a: &Account, enabled: bool) -> Result<(), String> {
    let _lock = session_lock(a)?;
    if enabled {
        let guard = a
            .session
            .0
            .tokens
            .lock()
            .map_err(|_| "ChatGPT session is unavailable.")?;
        let t = guard
            .as_ref()
            .ok_or("Sign in before remembering this session.")?;
        save_tokens(&NativeStore, a, t)
    } else {
        delete_tokens(&NativeStore, a)
    }
}
fn access(a: &Account) -> Result<String, String> {
    let _lock = session_lock(a)?;
    let mut guard = a
        .session
        .0
        .tokens
        .lock()
        .map_err(|_| "ChatGPT session is unavailable.")?;
    if guard.is_none() {
        return Err("Continue with ChatGPT before sending a request.".into());
    }
    if a.remember
        && !a.session.0.persist_failed.load(Ordering::Acquire)
        && let Ok(latest) = read_tokens(&NativeStore, a)
    {
        *guard = Some(latest);
    }
    let old = guard.as_ref().ok_or("No ChatGPT session.")?;
    if old.expires_at > now() + 90 {
        return Ok(old.access_token.clone());
    }
    if old.refresh_token.is_empty() {
        return Err("This ChatGPT session expired. Continue with ChatGPT again.".into());
    }
    let value = match form(
        TOKEN,
        &[
            ("grant_type", "refresh_token"),
            ("client_id", &a.client_id),
            ("refresh_token", &old.refresh_token),
            ("resource", RESOURCE),
        ],
    ) {
        Ok(v) => v,
        Err(e) => {
            if matches!(e, AuthFailure::Permanent(_)) {
                *guard = None;
                a.session.0.ready.store(false, Ordering::Release);
            }
            return Err(e.into());
        }
    };
    let claims = if let Some(id) = value["id_token"].as_str() {
        validated_id(id, &a.client_id, None, Some(&a.subject))?
    } else {
        json!({"sub":a.subject,"email":a.email})
    };
    let fresh = parse_tokens(value, &a.client_id, &claims, Some(old))?;
    let result = fresh.access_token.clone();
    if a.remember
        && let Err(e) = save_tokens(&NativeStore, a, &fresh)
    {
        a.session.0.persist_failed.store(true, Ordering::Release);
        *a.session.0.notice.lock().unwrap_or_else(|p| p.into_inner()) = e;
    }
    *guard = Some(fresh);
    Ok(result)
}
fn revoke(a: &Account, t: &Tokens) -> bool {
    let mut revoked = false;
    if !t.refresh_token.is_empty()
        && let Ok(m) = metadata()
        && let Some(url) = m["revocation_endpoint"].as_str()
        && let Ok(url) = trusted_auth_url(url)
    {
        for delay in [0, 1] {
            if delay > 0 {
                std::thread::sleep(Duration::from_secs(delay));
            }
            if let Ok(r) = agent().post(url).send_form([
                ("token", t.refresh_token.as_str()),
                ("token_type_hint", "refresh_token"),
                ("client_id", a.client_id.as_str()),
            ]) && r.status().is_success()
            {
                revoked = true;
                break;
            }
        }
    }
    revoked
}

pub fn sign_out(a: &Account) -> Result<String, String> {
    let _lock = session_lock(a)?;
    let tokens = a
        .session
        .0
        .tokens
        .lock()
        .map_err(|_| "ChatGPT session is unavailable.")?
        .take()
        .or_else(|| read_tokens(&NativeStore, a).ok());
    a.session.0.ready.store(false, Ordering::Release);
    let revoked = tokens.as_ref().is_some_and(|t| revoke(a, t));
    let deletion = delete_tokens(&NativeStore, a);
    let mut message = if revoked {
        "Signed out of ChatGPT and revoked the renewable session.".to_owned()
    } else {
        "Signed out locally. Remote revocation was not confirmed; disconnect Honeydew Dewey in ChatGPT Settings → Usage.".to_owned()
    };
    if let Err(e) = deletion {
        message.push(' ');
        message.push_str(&e);
    }
    Ok(message)
}

fn api(
    a: &Account,
    path: &str,
    body: Option<Value>,
) -> Result<ureq::http::Response<ureq::Body>, String> {
    let token = access(a)?;
    let url = format!("{RESOURCE}/{path}");
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(120)))
        .max_redirects(0)
        .http_status_as_error(false)
        .build()
        .into();
    let mut r = if let Some(body) = body {
        agent
            .post(&url)
            .header("Authorization", format!("Bearer {token}"))
            .send_json(body)
    } else {
        agent
            .get(&url)
            .header("Authorization", format!("Bearer {token}"))
            .call()
    }
    .map_err(|_| "Could not reach ChatGPT plan inference. Check the network or try again.")?;
    if !r.status().is_success() {
        let status = r.status().as_u16();
        let error = r
            .body_mut()
            .with_config()
            .limit(65536)
            .read_json::<Value>()
            .unwrap_or(Value::Null);
        if status == 401 {
            a.clear_local();
            return Err(
                "Your ChatGPT session is no longer authorized. Continue with ChatGPT again.".into(),
            );
        }
        if status == 429 {
            return Err(plan_error(&error));
        }
        return Err(format!(
            "ChatGPT plan request returned HTTP {status}. Check account eligibility, app permissions and the selected model. API-key billing was not used."
        ));
    }
    Ok(r)
}
fn plan_error(v: &Value) -> String {
    let code = v
        .pointer("/response/error/code")
        .or_else(|| v.pointer("/error/code"))
        .and_then(Value::as_str)
        .unwrap_or("");
    match code{
        "subscription_sharing_usage_limit_exceeded"=>"Your ChatGPT plan's shared usage limit was reached. Check ChatGPT Settings → Usage for Honeydew Dewey and wait for its reset or adjust the app's permitted usage.".into(),
        "subscription_sharing_usage_unavailable"=>"ChatGPT plan usage is unavailable for this account or app. Check plan eligibility and permissions in ChatGPT Settings → Usage; API billing was not used.".into(),
        _=>"ChatGPT could not complete this plan request. Check the account's usage limits and model availability, then try again. No partial pattern was applied.".into(),
    }
}
#[derive(Clone)]
pub struct Model {
    pub slug: String,
    pub name: String,
}
pub fn models(plan: &Plan) -> Result<Vec<Model>, String> {
    let a = plan.active().ok_or("Choose a ChatGPT account first.")?;
    let value = api(a, "models", None)?
        .body_mut()
        .with_config()
        .limit(2 * 1024 * 1024)
        .read_json::<Value>()
        .map_err(|_| "Invalid ChatGPT model catalog.")?;
    let rows = value["models"]
        .as_array()
        .ok_or("ChatGPT did not return an account model catalog.")?;
    let result = rows
        .iter()
        .filter(|m| m["visibility"] == "list")
        .filter_map(|m| {
            let slug = m["slug"]
                .as_str()
                .filter(|s| !s.is_empty() && s.len() < 256)?;
            Some(Model {
                slug: slug.into(),
                name: m["display_name"].as_str().unwrap_or(slug).into(),
            })
        })
        .collect::<Vec<_>>();
    if result.is_empty() {
        return Err("No models are available to this ChatGPT account.".into());
    }
    Ok(result)
}
pub fn response_body(model: &str, messages: &[Message], context: &Value) -> Value {
    json!({"model":model,"instructions":format!("{INSTRUCTIONS}\nCurrent song:\n{context}"),
        "input":messages.iter().map(|m|json!({"role":if m.assistant{"assistant"}else{"user"},"content":m.text})).collect::<Vec<_>>(),
        "stream":true,"store":false})
}
pub fn chat(
    plan: &Plan,
    model: &str,
    messages: &[Message],
    context: &Value,
) -> Result<Answer, String> {
    if model.trim().is_empty() {
        return Err("Refresh models and choose a ChatGPT plan model first.".into());
    }
    let a = plan
        .active()
        .ok_or("Continue with ChatGPT before sending.")?;
    let body = response_body(model, messages, context);
    if body.to_string().len() > 2 * 1024 * 1024 {
        return Err(
            "The ChatGPT conversation is too large. Clear chat and try a smaller request.".into(),
        );
    }
    let mut response = api(a, "responses", Some(body))?;
    let text = read_sse(BufReader::new(
        response.body_mut().as_reader().take(8 * 1024 * 1024),
    ))?;
    Answer::parse(&text)
}
fn output_text(v: &Value) -> String {
    v.pointer("/response/output")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|i| i["content"].as_array())
                .flatten()
                .filter(|c| c["type"] == "output_text")
                .filter_map(|c| c["text"].as_str())
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_default()
}
fn read_sse(mut reader: impl BufRead) -> Result<String, String> {
    let mut data = String::new();
    let mut line = String::new();
    let mut text = String::new();
    loop {
        line.clear();
        let count = reader.read_line(&mut line).map_err(
            |_| "ChatGPT's response stream was interrupted. No partial pattern was applied.",
        )?;
        if count == 0 {
            return Err("ChatGPT's stream ended before confirmed completion. No partial pattern was applied.".into());
        }
        if line.trim_end().is_empty() {
            if data.is_empty() {
                continue;
            }
            let event: Value =
                serde_json::from_str(&data).map_err(|_| "Invalid ChatGPT response event.")?;
            data.clear();
            match event["type"].as_str().unwrap_or("") {
                "response.output_text.delta" => {
                    if let Some(delta) = event["delta"].as_str() {
                        text.push_str(delta);
                    }
                    if text.len() > 2 * 1024 * 1024 {
                        return Err("ChatGPT's reply exceeded the supported size.".into());
                    }
                }
                "response.completed" => {
                    if event
                        .pointer("/response/status")
                        .is_some_and(|s| s != "completed")
                    {
                        return Err(plan_error(&event));
                    }
                    let final_text = output_text(&event);
                    if !final_text.is_empty() {
                        text = final_text;
                    }
                    if text.trim().is_empty() {
                        return Err(
                            "ChatGPT completed without a text reply. Try another request or model."
                                .into(),
                        );
                    }
                    return Ok(text);
                }
                "response.failed" | "response.incomplete" | "error" => {
                    return Err(plan_error(&event));
                }
                _ => {}
            }
        } else if let Some(part) = line.strip_prefix("data:") {
            data.push_str(part.trim_start_matches(' ').trim_end_matches(['\r', '\n']));
            data.push('\n');
            if data.len() > 2 * 1024 * 1024 {
                return Err("ChatGPT response event exceeded the supported size.".into());
            }
        }
    }
}

#[cfg(test)]
#[path = "test_key.rs"]
mod test_key;
#[cfg(test)]
mod tests {
    use super::*;
    use std::{cell::RefCell, collections::HashMap, io::Cursor};
    #[derive(Default)]
    struct Memory(RefCell<HashMap<String, String>>);
    impl Store for Memory {
        fn get(&self, k: &str) -> Result<String, crate::ai::credentials::StoreError> {
            self.0
                .borrow()
                .get(k)
                .cloned()
                .ok_or(crate::ai::credentials::StoreError::Missing)
        }
        fn set(&self, k: &str, v: &str) -> Result<(), crate::ai::credentials::StoreError> {
            assert!(v.len() < 2560);
            self.0.borrow_mut().insert(k.into(), v.into());
            Ok(())
        }
        fn delete(&self, k: &str) -> Result<(), crate::ai::credentials::StoreError> {
            self.0
                .borrow_mut()
                .remove(k)
                .map(|_| ())
                .ok_or(crate::ai::credentials::StoreError::Missing)
        }
    }
    fn account() -> Account {
        Account {
            client_id: "oaiapp_test".into(),
            subject: "test-subject".into(),
            email: "test@example.invalid".into(),
            ..Default::default()
        }
    }
    fn tokens() -> Tokens {
        Tokens {
            client_id: "oaiapp_test".into(),
            subject: "test-subject".into(),
            email: "test@example.invalid".into(),
            access_token: "test-access-only".repeat(400),
            refresh_token: "test-refresh-only".into(),
            id_token: "test-identity-only".repeat(200),
            scope: SCOPES.into(),
            expires_at: now() + 3600,
        }
    }
    fn login() -> Login {
        Login {
            url: String::new(),
            listener: TcpListener::bind(("127.0.0.1", 0)).unwrap(),
            redirect: "http://127.0.0.1:1234/auth/callback".into(),
            state: "state-expected".into(),
            nonce: "nonce-expected".into(),
            verifier: "test-verifier".into(),
            returning: None,
            cancel: Arc::new(AtomicBool::new(false)),
            remember: false,
        }
    }
    #[test]
    fn host_identity_is_a_stable_uuid_urn_and_migrates_the_rejected_legacy_value() {
        let path =
            std::env::temp_dir().join(format!("honeydew-host-id-test-{}", random().unwrap()));
        let id = host_id_at(&path).unwrap();
        assert!(valid_host_id(&id));
        assert_eq!(host_id_at(&path).unwrap(), id);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), id);
        assert_ne!(new_host_id().unwrap(), id);
        std::fs::write(&path, format!("honeydew-{}", "x".repeat(43))).unwrap();
        let migrated = host_id_at(&path).unwrap();
        assert!(valid_host_id(&migrated));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), migrated);
        assert_eq!(host_id_at(&path).unwrap(), migrated);
        std::fs::write(&path, "unexpected malformed identity").unwrap();
        assert!(host_id_at(&path).is_err());
        std::fs::remove_file(path).unwrap();
        for bad in [
            "honeydew-example",
            "urn:uuid:123",
            "urn:uuid:00000000-0000-1000-8000-000000000000",
            "urn:uuid:00000000-0000-4000-0000-000000000000",
        ] {
            assert!(!valid_host_id(bad));
        }
    }
    #[test]
    fn dynamic_registration_requires_issued_client_state_and_exact_callback_path() {
        let mut l = login();
        let (code, client) = callback_target(
            &l,
            "/auth/callback?code=test-code&state=state-expected&client_id=oaiapp_issued",
        )
        .unwrap();
        assert_eq!(code, "test-code");
        assert_eq!(client, "oaiapp_issued");
        for target in [
            "/callback?code=a&state=state-expected&client_id=oaiapp_issued",
            "/auth/callback?code=a&state=wrong&client_id=oaiapp_issued",
            "/auth/callback?code=a&state=state-expected",
            "/auth/callback?code=a&state=state-expected&client_id=dynamic_agent_client",
            "/auth/callback?code=a&state=state-expected&state=wrong&client_id=oaiapp_issued",
        ] {
            assert!(callback_target(&l, target).is_err(), "{target}");
        }
        l.returning = Some(account());
        assert!(callback_target(&l, "/auth/callback?code=a&state=state-expected").is_ok());
        assert!(
            callback_target(
                &l,
                "/auth/callback?code=a&state=state-expected&client_id=oaiapp_other"
            )
            .is_err()
        );
    }
    #[test]
    fn loopback_listener_accepts_only_the_bound_attempt() {
        let l = login();
        l.listener.set_nonblocking(true).unwrap();
        let addr = l.listener.local_addr().unwrap();
        let worker = std::thread::spawn(move || {
            use std::net::TcpStream;
            for target in [
                "/favicon.ico",
                "/auth/callback?code=bad&state=wrong&client_id=oaiapp_x",
                "/auth/callback?code=good&state=state-expected&client_id=oaiapp_bound",
            ] {
                let mut stream = TcpStream::connect(addr).unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(3)))
                    .unwrap();
                write!(stream, "GET {target} HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n").unwrap();
                let mut reply = String::new();
                stream.read_to_string(&mut reply).unwrap();
                assert!(!reply.contains("good"));
            }
        });
        let result = callback(&l).unwrap();
        worker.join().unwrap();
        assert_eq!(result, ("good".into(), "oaiapp_bound".into()));
        l.cancel.store(true, Ordering::Release);
        assert!(callback(&l).unwrap_err().contains("cancelled"));
    }
    fn jwks() -> Value {
        json!({"keys":[{"kid":"test-only","kty":"RSA","alg":"RS256","n":test_key::N,"e":test_key::E}]})
    }
    fn claims() -> Value {
        json!({"iss":ISSUER,"aud":"oaiapp_test","sub":"test-subject","nonce":"nonce-expected","exp":now()+3600,"email":"test@example.invalid"})
    }
    fn signed(claims: &Value) -> String {
        let mut h = jsonwebtoken::Header::new(jsonwebtoken::Algorithm::RS256);
        h.kid = Some("test-only".into());
        jsonwebtoken::encode(
            &h,
            claims,
            &jsonwebtoken::EncodingKey::from_rsa_der(&STANDARD.decode(test_key::DER).unwrap()),
        )
        .unwrap()
    }
    #[test]
    fn identity_signatures_issuer_audience_expiry_nonce_and_subject_are_verified() {
        let c = claims();
        let token = signed(&c);
        assert!(
            verify_id(
                &token,
                "oaiapp_test",
                Some("nonce-expected"),
                Some("test-subject"),
                &jwks()
            )
            .is_ok()
        );
        assert!(
            verify_id(
                &token,
                "wrong-client",
                Some("nonce-expected"),
                None,
                &jwks()
            )
            .is_err()
        );
        assert!(verify_id(&token, "oaiapp_test", Some("wrong-nonce"), None, &jwks()).is_err());
        assert!(verify_id(&token, "oaiapp_test", None, Some("wrong-subject"), &jwks()).is_err());
        for (field, value) in [
            ("iss", json!("https://fake.invalid")),
            ("exp", json!(now() - 3600)),
            ("azp", json!("other-client")),
            ("aud", json!(["oaiapp_test", "other-client"])),
        ] {
            let mut bad = c.clone();
            bad[field] = value;
            assert!(
                verify_id(
                    &signed(&bad),
                    "oaiapp_test",
                    Some("nonce-expected"),
                    None,
                    &jwks()
                )
                .is_err(),
                "{field}"
            );
        }
        let mut altered = token.into_bytes();
        let i = altered.len() - 12;
        altered[i] = if altered[i] == b'a' { b'b' } else { b'a' };
        assert!(
            verify_id(
                std::str::from_utf8(&altered).unwrap(),
                "oaiapp_test",
                None,
                None,
                &jwks()
            )
            .is_err()
        );
    }
    #[test]
    fn secure_sessions_fit_windows_blob_limits_and_never_enter_preferences() {
        let store = Memory::default();
        let mut a = account();
        let mut t = tokens();
        save_tokens(&store, &a, &t).unwrap();
        assert_eq!(
            read_tokens(&store, &a).unwrap().access_token,
            t.access_token
        );
        install(&a, t.clone());
        a.remember = true;
        let mut p = Plan::default();
        p.accept(a.clone());
        let saved = serde_json::to_string(&p).unwrap();
        assert!(!saved.contains("test-access-only"));
        assert!(!saved.contains("test-refresh-only"));
        assert!(!saved.contains("test-identity-only"));
        assert!(!format!("{p:?}").contains("test-access-only"));
        t.access_token = "rotated-access".into();
        t.refresh_token = "rotated-refresh".into();
        save_tokens(&store, &a, &t).unwrap();
        assert_eq!(
            read_tokens(&store, &a).unwrap().refresh_token,
            "rotated-refresh"
        );
        let mut other = a.clone();
        other.subject = "different-account".into();
        assert!(read_tokens(&store, &other).is_err());
        delete_tokens(&store, &a).unwrap();
        assert!(read_tokens(&store, &a).is_err());
        assert!(store.0.borrow().is_empty());
        let loaded: Plan = serde_json::from_str(&saved).unwrap();
        assert!(!loaded.active().unwrap().signed_in());
        assert!(loaded.active().unwrap().remember);
    }
    #[test]
    fn sign_out_distinguishes_a_missing_vault_record_from_a_locked_store() {
        struct Locked;
        impl Store for Locked {
            fn get(&self, _: &str) -> Result<String, StoreError> {
                Err(StoreError::Unavailable)
            }
            fn set(&self, _: &str, _: &str) -> Result<(), StoreError> {
                Err(StoreError::Unavailable)
            }
            fn delete(&self, _: &str) -> Result<(), StoreError> {
                Err(StoreError::Unavailable)
            }
        }
        let a = account();
        assert!(delete_tokens(&Memory::default(), &a).is_ok());
        assert!(delete_tokens(&Locked, &a).unwrap_err().contains("Unlock"));
        let damaged = Memory::default();
        damaged.set(&vault_key(&a), "invalid metadata").unwrap();
        assert!(
            delete_tokens(&damaged, &a)
                .unwrap_err()
                .contains("damaged metadata")
        );
        assert!(damaged.0.borrow().is_empty());
    }
    #[test]
    fn plan_scopes_and_rotating_refresh_tokens_are_checked() {
        let c = claims();
        let old = tokens();
        let response = json!({"access_token":"new-access","refresh_token":"new-refresh","token_type":"Bearer","expires_in":3600,"scope":SCOPES});
        let fresh = parse_tokens(response.clone(), "oaiapp_test", &c, Some(&old)).unwrap();
        assert_eq!(fresh.refresh_token, "new-refresh");
        let mut bad = response.clone();
        bad["scope"] = json!("openid profile email");
        assert!(parse_tokens(bad, "oaiapp_test", &c, None).is_err());
        let mut keep = response;
        keep.as_object_mut().unwrap().remove("refresh_token");
        keep.as_object_mut().unwrap().remove("scope");
        assert_eq!(
            parse_tokens(keep, "oaiapp_test", &c, Some(&old))
                .unwrap()
                .refresh_token,
            old.refresh_token
        );
    }
    #[test]
    fn responses_use_the_plan_contract_and_reject_partial_or_failed_streams() {
        let body = response_body(
            "future-model",
            &[Message {
                assistant: false,
                text: "Make a beat".into(),
            }],
            &json!({}),
        );
        assert_eq!(body["stream"], true);
        assert_eq!(body["store"], false);
        assert!(body.get("max_output_tokens").is_none());
        assert!(body.get("previous_response_id").is_none());
        assert!(body.get("instructions").is_some());
        let delta = "data: {\"type\":\"response.output_text.delta\",\"delta\":\"hello\"}\n\n";
        assert!(read_sse(Cursor::new(delta)).is_err());
        let completed = format!(
            "{delta}data: {{\"type\":\"response.completed\",\"response\":{{\"status\":\"completed\"}}}}\n\n"
        );
        assert_eq!(read_sse(Cursor::new(completed)).unwrap(), "hello");
        let failed = format!(
            "{delta}data: {{\"type\":\"response.failed\",\"response\":{{\"error\":{{\"code\":\"subscription_sharing_usage_limit_exceeded\",\"message\":\"private-secret\"}}}}}}\n\n"
        );
        let error = read_sse(Cursor::new(failed)).unwrap_err();
        assert!(error.contains("shared usage limit"));
        assert!(!error.contains("private-secret"));
    }
}
