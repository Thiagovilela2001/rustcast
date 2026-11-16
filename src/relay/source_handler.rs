//! Source ingestion on raw TCP: Icecast-style `PUT` (no length, body until close) and legacy `SOURCE` (Mixxx/BUTT/VirtualDJ).
//! ponytail: chunked PUT bodies not decoded; no Icecast client sends them.
use super::mountpoint::{normalize, Mounts};
use base64::Engine;
use bytes::Bytes;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use sqlx::SqlitePool;
use tokio::net::TcpStream;

pub fn parse_basic(header: Option<&str>) -> Option<(String, String)> {
    let enc = header?.strip_prefix("Basic ")?;
    let dec = base64::engine::general_purpose::STANDARD.decode(enc.trim()).ok()?;
    let (u, p) = String::from_utf8_lossy(&dec).split_once(':').map(|(u, p)| (u.to_string(), p.to_string()))?;
    Some((u, p))
}

pub fn check_basic(header: Option<&str>, password: &str) -> bool {
    parse_basic(header).is_some_and(|(_, p)| p == password)
}

pub fn hash_password(pw: &str) -> String {
    use argon2::password_hash::PasswordHasher;
    argon2::Argon2::default().hash_password(pw.as_bytes()).expect("argon2 hash").to_string()
}

fn verify_password(pw: &str, hash: &str) -> bool {
    use argon2::password_hash::{phc::PasswordHash, PasswordVerifier};
    PasswordHash::new(hash).is_ok_and(|h| argon2::Argon2::default().verify_password(pw.as_bytes(), &h).is_ok())
}

/// Master source password (any user), or an active DJ account of the station that owns `mount`.
pub async fn source_auth(db: &SqlitePool, mount: &str, header: Option<&str>, master: &str) -> bool {
    let Some((user, pass)) = parse_basic(header) else { return false };
    if pass == master {
        return true;
    }
    let hash: Option<(String,)> = sqlx::query_as(
        "SELECT d.password_hash FROM dj_accounts d JOIN stations s ON s.id = d.station_id
         WHERE d.username = ? AND d.is_active = 1 AND s.mountpoint = ?",
    )
    .bind(user)
    .bind(mount)
    .fetch_optional(db)
    .await
    .ok()
    .flatten();
    match hash {
        Some((h,)) => tokio::task::spawn_blocking(move || verify_password(&pass, &h)).await.unwrap_or(false),
        None => false,
    }
}

/// Peek without consuming: does this connection start with "SOURCE " or "PUT "?
pub async fn is_source(sock: &TcpStream) -> bool {
    let mut buf = [0u8; 7];
    for _ in 0..200 {
        let Ok(n) = sock.peek(&mut buf).await else { return false };
        let hit = |p: &[u8]| buf[..n.min(p.len())] == p[..n.min(p.len())];
        if n == 0 || !(hit(b"SOURCE ") || hit(b"PUT ")) {
            return false;
        }
        if buf[..n].starts_with(b"SOURCE ") || buf[..n].starts_with(b"PUT ") {
            return true;
        }
        // ponytail: partial-prefix poll loop, fine for a 7-byte sniff
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    false
}

pub async fn handle_source(sock: TcpStream, mounts: Mounts, db: SqlitePool, password: String) -> std::io::Result<()> {
    let mut r = BufReader::new(sock);
    let mut line = String::new();
    r.read_line(&mut line).await?;
    let path = line.split_whitespace().nth(1).unwrap_or("/").to_string();
    let (mut auth, mut ctype, mut ice_name) = (None, None, None);
    loop {
        line.clear();
        if r.read_line(&mut line).await? == 0 || line.trim().is_empty() {
            break;
        }
        if let Some((k, v)) = line.split_once(':') {
            let v = v.trim().to_string();
            match k.trim().to_ascii_lowercase().as_str() {
                "authorization" => auth = Some(v),
                "content-type" => ctype = Some(v),
                "ice-name" => ice_name = Some(v),
                _ => {}
            }
        }
    }
    let name = normalize(&path);
    if !source_auth(&db, &name, auth.as_deref(), &password).await {
        r.get_mut().write_all(b"HTTP/1.0 401 Unauthorized\r\nWWW-Authenticate: Basic realm=\"rustcast\"\r\n\r\n").await?;
        return Ok(());
    }
    let mount = mounts.get_or_create(&name);
    let Some(_guard) = mount.claim() else {
        r.get_mut().write_all(b"HTTP/1.0 403 Mountpoint in use\r\n\r\n").await?;
        return Ok(());
    };
    if let Some(ct) = ctype {
        *mount.content_type.lock().unwrap() = ct;
    }
    mount.set_now(ice_name.unwrap_or_else(|| "Ao vivo".into()), 0);
    r.get_mut().write_all(b"HTTP/1.0 200 OK\r\n\r\n").await?;
    let mut buf = vec![0u8; 8192];
    loop {
        let n = r.read(&mut buf).await?;
        if n == 0 {
            return Ok(());
        }
        mount.publish(Bytes::copy_from_slice(&buf[..n]));
    }
}
