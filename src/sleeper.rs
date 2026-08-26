//! "Serveur dynamique" (sleep/wake): once a server has auto-stopped from
//! inactivity (`stop_when_empty_minutes`), this takes over its port with a
//! tiny listener instead of leaving it dead. It answers server-list pings
//! (so the server still shows up, with a "sleeping" message, instead of
//! looking offline) and, the moment someone actually tries to join, starts
//! the real server and tells them to retry in a few seconds.
//!
//! Deliberately does NOT try to proxy the actual game connection through
//! to the real server once it's up (buffering/replaying already-read login
//! bytes across a raw TCP relay is exactly the kind of "clever" fragile
//! trick that breaks in some Minecraft client versions / behind some
//! proxies). Kicking the player with a friendly message and letting their
//! client's own reconnect (or a manual retry) pick up the now-started
//! server is simpler and far more robust - the trade-off, spelled out to
//! the user in the UI, is that the *first* join after a sleep takes as
//! long as the server's normal boot time instead of being instant.
//!
//! Java Edition support is a real (if minimal) protocol implementation,
//! covered by unit tests below. Bedrock/Geyser support is best-effort: it
//! answers RakNet's unconnected ping so the server still appears in the
//! Bedrock server list, and treats an Open Connection Request 1 (the start
//! of a real join, distinct from a list ping) as the wake trigger - this
//! part hasn't been exercised against a real Bedrock client, so treat it
//! as experimental.

use std::sync::Arc;

use anyhow::{Context, Result};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream, UdpSocket};
use uuid::Uuid;

use crate::state::AppState;

/// Takes over `port` for server `id` until a real join attempt wakes it up
/// (or the process is otherwise torn down). Runs both the Java (TCP) and,
/// if `bedrock_port` is given, Bedrock (UDP) listeners concurrently; either
/// one waking the server cancels the other.
pub async fn run(state: AppState, id: Uuid, motd: String, java_port: u16, bedrock_port: Option<u16>) -> Result<()> {
    let woken = Arc::new(tokio::sync::Notify::new());

    let java_task = {
        let state = state.clone();
        let motd = motd.clone();
        let woken = woken.clone();
        tokio::spawn(async move {
            if let Err(e) = run_java_listener(state, id, motd, java_port, woken).await {
                tracing::warn!("[serveur dynamique] listener Java arrete: {e}");
            }
        })
    };

    let bedrock_task = bedrock_port.map(|port| {
        let state = state.clone();
        let woken = woken.clone();
        tokio::spawn(async move {
            if let Err(e) = run_bedrock_listener(state, id, motd, port, woken).await {
                tracing::warn!("[serveur dynamique] listener Bedrock arrete: {e}");
            }
        })
    });

    woken.notified().await;
    java_task.abort();
    if let Some(t) = bedrock_task {
        t.abort();
    }
    Ok(())
}

async fn wake(state: &AppState, id: Uuid, woken: &tokio::sync::Notify) {
    // Only the first caller actually triggers a start - concurrent join
    // attempts during the same wake-up are harmless no-ops since
    // start_server() itself is a no-op if already running/starting.
    woken.notify_one();
    let _ = crate::process::start_server(state, id).await;
}

// ───────────────────────── Java Edition ─────────────────────────

async fn run_java_listener(state: AppState, id: Uuid, motd: String, port: u16, woken: Arc<tokio::sync::Notify>) -> Result<()> {
    let listener = TcpListener::bind(("0.0.0.0", port)).await
        .with_context(|| format!("impossible d'ecouter sur le port {port} (deja utilise ?)"))?;
    tracing::info!("[serveur dynamique] en veille sur le port {port} (Java) en attendant un joueur");
    loop {
        let (socket, _) = listener.accept().await?;
        let state = state.clone();
        let motd = motd.clone();
        let woken = woken.clone();
        tokio::spawn(async move {
            match handle_java_connection(socket, &motd).await {
                Ok(true) => wake(&state, id, &woken).await,
                Ok(false) => {}
                Err(_) => {} // malformed/unexpected traffic on the port - ignore and keep sleeping
            }
        });
    }
}

/// Returns `Ok(true)` if this connection was a real join attempt (wake the
/// server), `Ok(false)` if it was just a status ping (answered inline).
async fn handle_java_connection(mut socket: TcpStream, motd: &str) -> Result<bool> {
    let handshake = read_packet(&mut socket).await?;
    let mut r = &handshake[..];
    let packet_id = read_varint(&mut r)?;
    if packet_id != 0x00 {
        anyhow::bail!("paquet de handshake inattendu");
    }
    let _protocol_version = read_varint(&mut r)?;
    let _server_address = read_string(&mut r)?;
    let _server_port = read_u16(&mut r)?;
    let next_state = read_varint(&mut r)?;

    match next_state {
        1 => {
            // Status: answer the request, echo the ping, then close.
            let _status_request = read_packet(&mut socket).await.ok();
            let json = serde_json::json!({
                "version": { "name": "MCManager", "protocol": 0 },
                "players": { "max": 0, "online": 0, "sample": [] },
                "description": { "text": motd },
            });
            write_packet(&mut socket, &{
                let mut buf = Vec::new();
                write_varint(&mut buf, 0x00);
                write_string(&mut buf, &json.to_string());
                buf
            }).await?;
            if let Ok(ping) = read_packet(&mut socket).await {
                // Ping packet (0x01) carries an 8-byte payload to echo back verbatim.
                write_packet(&mut socket, &ping).await.ok();
            }
            Ok(false)
        }
        2 => {
            // Login: this is a real join attempt. Politely bounce them
            // while the real server boots, rather than trying to hold the
            // connection open for however long that takes.
            let _login_start = read_packet(&mut socket).await.ok();
            let reason = serde_json::json!({ "text": motd_wake_message() });
            write_packet(&mut socket, &{
                let mut buf = Vec::new();
                write_varint(&mut buf, 0x00);
                write_string(&mut buf, &reason.to_string());
                buf
            }).await.ok();
            Ok(true)
        }
        _ => anyhow::bail!("etat suivant inattendu: {next_state}"),
    }
}

fn motd_wake_message() -> String {
    "§eLe serveur demarre suite a votre connexion (serveur dynamique) - reessayez dans quelques secondes...".to_string()
}

// ───────────────────────── Bedrock / RakNet (best-effort) ─────────────────────────

const RAKNET_MAGIC: [u8; 16] = [0x00, 0xff, 0xff, 0x00, 0xfe, 0xfe, 0xfe, 0xfe, 0xfd, 0xfd, 0xfd, 0xfd, 0x12, 0x34, 0x56, 0x78];
const UNCONNECTED_PING: u8 = 0x01;
const UNCONNECTED_PONG: u8 = 0x1c;
const OPEN_CONNECTION_REQUEST_1: u8 = 0x05;

async fn run_bedrock_listener(state: AppState, id: Uuid, motd: String, port: u16, woken: Arc<tokio::sync::Notify>) -> Result<()> {
    let socket = UdpSocket::bind(("0.0.0.0", port)).await
        .with_context(|| format!("impossible d'ecouter sur le port UDP {port} (deja utilise ?)"))?;
    tracing::info!("[serveur dynamique] en veille sur le port {port} (Bedrock, experimental) en attendant un joueur");
    let mut buf = [0u8; 1500];
    loop {
        let (n, from) = socket.recv_from(&mut buf).await?;
        if n == 0 {
            continue;
        }
        match buf[0] {
            UNCONNECTED_PING => {
                if let Some(pong) = build_unconnected_pong(&buf[..n], &motd) {
                    let _ = socket.send_to(&pong, from).await;
                }
            }
            OPEN_CONNECTION_REQUEST_1 => {
                // A real connection attempt (not just a server-list ping).
                // We deliberately don't answer with OPEN_CONNECTION_REPLY_1
                // here - the client will simply retry once the real
                // Bedrock/Geyser listener is up, same "kick and let them
                // reconnect" approach as the Java side.
                wake(&state, id, &woken).await;
                return Ok(());
            }
            _ => {}
        }
    }
}

fn build_unconnected_pong(ping_packet: &[u8], motd: &str) -> Option<Vec<u8>> {
    // Unconnected Ping: [1 id][8 time][16 magic][8 client guid]
    if ping_packet.len() < 33 || ping_packet[9..25] != RAKNET_MAGIC {
        return None;
    }
    let time = &ping_packet[1..9];
    let server_guid: u64 = 0x4d434d67_4d43676du64; // arbitrary fixed "MCMgMCgm"-ish server GUID, fine for a ping responder
    let motd_line = format!("MCPE;{motd};0;0.0.0;0;10;{server_guid};MCManager;Survival;1;19132;19133;");

    let mut out = Vec::with_capacity(35 + motd_line.len());
    out.push(UNCONNECTED_PONG);
    out.extend_from_slice(time);
    out.extend_from_slice(&server_guid.to_be_bytes());
    out.extend_from_slice(&RAKNET_MAGIC);
    out.extend_from_slice(&(motd_line.len() as u16).to_be_bytes());
    out.extend_from_slice(motd_line.as_bytes());
    Some(out)
}

// ───────────────────────── protocole Java : VarInt / framing ─────────────────────────

async fn read_packet(socket: &mut TcpStream) -> Result<Vec<u8>> {
    let len = read_varint_async(socket).await? as usize;
    if len > 1_048_576 {
        anyhow::bail!("paquet anormalement grand ({len} octets)");
    }
    let mut buf = vec![0u8; len];
    socket.read_exact(&mut buf).await?;
    Ok(buf)
}

async fn write_packet(socket: &mut TcpStream, payload: &[u8]) -> Result<()> {
    let mut framed = Vec::new();
    write_varint(&mut framed, payload.len() as i32);
    framed.extend_from_slice(payload);
    socket.write_all(&framed).await?;
    Ok(())
}

async fn read_varint_async(socket: &mut TcpStream) -> Result<i32> {
    let mut result: i32 = 0;
    for i in 0..5 {
        let mut byte = [0u8; 1];
        socket.read_exact(&mut byte).await?;
        result |= ((byte[0] & 0x7f) as i32) << (7 * i);
        if byte[0] & 0x80 == 0 {
            return Ok(result);
        }
    }
    anyhow::bail!("VarInt trop long")
}

fn read_varint(buf: &mut &[u8]) -> Result<i32> {
    let mut result: i32 = 0;
    for i in 0..5 {
        if buf.is_empty() {
            anyhow::bail!("VarInt tronque");
        }
        let byte = buf[0];
        *buf = &buf[1..];
        result |= ((byte & 0x7f) as i32) << (7 * i);
        if byte & 0x80 == 0 {
            return Ok(result);
        }
    }
    anyhow::bail!("VarInt trop long")
}

fn write_varint(buf: &mut Vec<u8>, mut value: i32) {
    loop {
        let mut byte = (value & 0x7f) as u8;
        value = ((value as u32) >> 7) as i32;
        if value != 0 {
            byte |= 0x80;
        }
        buf.push(byte);
        if value == 0 {
            break;
        }
    }
}

fn read_string(buf: &mut &[u8]) -> Result<String> {
    let len = read_varint(buf)? as usize;
    if buf.len() < len {
        anyhow::bail!("chaine tronquee");
    }
    let s = String::from_utf8_lossy(&buf[..len]).to_string();
    *buf = &buf[len..];
    Ok(s)
}

fn write_string(buf: &mut Vec<u8>, s: &str) {
    write_varint(buf, s.len() as i32);
    buf.extend_from_slice(s.as_bytes());
}

fn read_u16(buf: &mut &[u8]) -> Result<u16> {
    if buf.len() < 2 {
        anyhow::bail!("u16 tronque");
    }
    let v = u16::from_be_bytes([buf[0], buf[1]]);
    *buf = &buf[2..];
    Ok(v)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn varint_roundtrip() {
        for value in [0, 1, 127, 128, 255, 300, 2_097_151, i32::MAX] {
            let mut buf = Vec::new();
            write_varint(&mut buf, value);
            let mut slice = &buf[..];
            let decoded = read_varint(&mut slice).unwrap();
            assert_eq!(decoded, value);
            assert!(slice.is_empty(), "leftover bytes after decoding {value}");
        }
    }

    #[test]
    fn string_roundtrip() {
        let mut buf = Vec::new();
        write_string(&mut buf, "Hello, MCManager! 🎮");
        let mut slice = &buf[..];
        let decoded = read_string(&mut slice).unwrap();
        assert_eq!(decoded, "Hello, MCManager! 🎮");
    }

    #[test]
    fn handshake_packet_parses() {
        // Mirrors a real client handshake: id=0, protocol=765, "localhost", port=25565, next_state=1.
        let mut buf = Vec::new();
        write_varint(&mut buf, 0x00);
        write_varint(&mut buf, 765);
        write_string(&mut buf, "localhost");
        buf.extend_from_slice(&25565u16.to_be_bytes());
        write_varint(&mut buf, 1);

        let mut r = &buf[..];
        assert_eq!(read_varint(&mut r).unwrap(), 0x00);
        assert_eq!(read_varint(&mut r).unwrap(), 765);
        assert_eq!(read_string(&mut r).unwrap(), "localhost");
        assert_eq!(read_u16(&mut r).unwrap(), 25565);
        assert_eq!(read_varint(&mut r).unwrap(), 1);
    }

    #[test]
    fn unconnected_pong_built_correctly() {
        let mut ping = vec![UNCONNECTED_PING];
        ping.extend_from_slice(&12345u64.to_be_bytes()); // time
        ping.extend_from_slice(&RAKNET_MAGIC);
        ping.extend_from_slice(&0u64.to_be_bytes()); // client guid
        let pong = build_unconnected_pong(&ping, "Test MOTD").expect("should build a pong");
        assert_eq!(pong[0], UNCONNECTED_PONG);
        assert_eq!(&pong[1..9], &12345u64.to_be_bytes());
        assert!(String::from_utf8_lossy(&pong).contains("Test MOTD"));
    }

    #[test]
    fn unconnected_pong_rejects_bad_magic() {
        let bad_ping = vec![UNCONNECTED_PING; 40]; // wrong magic bytes
        assert!(build_unconnected_pong(&bad_ping, "x").is_none());
    }

    /// End-to-end: binds a real listener, connects a client socket, sends
    /// an actual handshake+status-request+ping the way a Minecraft client
    /// would, and checks the bytes that come back decode into a well-formed
    /// status response with the echoed ping payload - not just that the
    /// encode/decode helpers work in isolation.
    #[tokio::test]
    async fn status_ping_end_to_end() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener); // free it up again for the real async listener to bind

        let server = tokio::spawn(async move {
            let listener = TcpListener::bind(("127.0.0.1", port)).await.unwrap();
            let (socket, _) = listener.accept().await.unwrap();
            let woke = handle_java_connection(socket, "Test MOTD").await.unwrap();
            assert!(!woke, "a status ping should never be treated as a wake trigger");
        });

        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        let mut client = TcpStream::connect(("127.0.0.1", port)).await.unwrap();

        // Handshake (next_state = 1: status)
        let mut hs = Vec::new();
        write_varint(&mut hs, 0x00);
        write_varint(&mut hs, 765);
        write_string(&mut hs, "localhost");
        hs.extend_from_slice(&port.to_be_bytes());
        write_varint(&mut hs, 1);
        write_packet(&mut client, &hs).await.unwrap();

        // Status request (empty body)
        write_packet(&mut client, &[0x00]).await.unwrap();

        let status = read_packet(&mut client).await.unwrap();
        let mut r = &status[..];
        assert_eq!(read_varint(&mut r).unwrap(), 0x00);
        let json_str = read_string(&mut r).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json_str).unwrap();
        assert_eq!(parsed["description"]["text"], "Test MOTD");

        // Ping (0x01 + 8-byte payload) should be echoed back verbatim.
        let mut ping = Vec::new();
        write_varint(&mut ping, 0x01);
        ping.extend_from_slice(&123456789i64.to_be_bytes());
        write_packet(&mut client, &ping).await.unwrap();
        let pong = read_packet(&mut client).await.unwrap();
        assert_eq!(pong, ping);

        server.await.unwrap();
    }

    /// Same but with next_state = 2 (login): the connection should be
    /// treated as a wake trigger and receive a disconnect packet, not a
    /// status response.
    #[tokio::test]
    async fn login_attempt_triggers_wake() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);

        let server = tokio::spawn(async move {
            let listener = TcpListener::bind(("127.0.0.1", port)).await.unwrap();
            let (socket, _) = listener.accept().await.unwrap();
            handle_java_connection(socket, "Test MOTD").await.unwrap()
        });

        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        let mut client = TcpStream::connect(("127.0.0.1", port)).await.unwrap();

        let mut hs = Vec::new();
        write_varint(&mut hs, 0x00);
        write_varint(&mut hs, 765);
        write_string(&mut hs, "localhost");
        hs.extend_from_slice(&port.to_be_bytes());
        write_varint(&mut hs, 2); // login
        write_packet(&mut client, &hs).await.unwrap();
        write_packet(&mut client, b"\x09Steve").await.unwrap(); // rough login-start stand-in, content unparsed

        let disconnect = read_packet(&mut client).await.unwrap();
        let mut r = &disconnect[..];
        assert_eq!(read_varint(&mut r).unwrap(), 0x00);
        let json_str = read_string(&mut r).unwrap();
        assert!(json_str.contains("demarre"));

        let woke = server.await.unwrap();
        assert!(woke, "a login attempt must be reported as a wake trigger");
    }
}
