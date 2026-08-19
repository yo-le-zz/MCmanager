use anyhow::Result;
use serde_json::Value;
use sysinfo::{Pid, System};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::time::{timeout, Duration};

pub fn process_stats(pid: u32) -> (f32, f32) {
    let mut sys = System::new();
    let pid = Pid::from_u32(pid);
    sys.refresh_process(pid);
    if let Some(proc_) = sys.process(pid) {
        let cpu = proc_.cpu_usage();
        let mem_mb = proc_.memory() as f32 / 1024.0 / 1024.0;
        (cpu, mem_mb)
    } else {
        (0.0, 0.0)
    }
}

/// Minimal Minecraft "Server List Ping" (modern handshake+status protocol),
/// used to fetch player counts and MOTD without needing RCON. Retries once
/// with a longer timeout: a server that just started can accept the TCP
/// connection before its status handler is fully ready, which used to read
/// as "offline" for a few seconds after start with a single fixed 800ms try.
pub async fn ping_server(port: u16) -> Result<(Option<u32>, Option<u32>, Option<String>)> {
    for (attempt, timeout_ms) in [400u64, 1200].into_iter().enumerate() {
        match try_ping(port, timeout_ms).await {
            Ok(v) => return Ok(v),
            Err(_) if attempt == 0 => continue,
            Err(_) => return Ok((None, None, None)),
        }
    }
    Ok((None, None, None))
}

async fn try_ping(port: u16, timeout_ms: u64) -> Result<(Option<u32>, Option<u32>, Option<String>)> {
    let addr = format!("127.0.0.1:{port}");
    let fut = async {
        let mut stream = TcpStream::connect(&addr).await?;

        let mut handshake = Vec::new();
        write_varint(&mut handshake, 0x00);
        write_varint(&mut handshake, 763); // protocol version, server ignores mismatch for status
        write_string(&mut handshake, "127.0.0.1");
        handshake.extend_from_slice(&port.to_be_bytes());
        write_varint(&mut handshake, 1); // next state: status

        let mut packet = Vec::new();
        write_varint(&mut packet, handshake.len() as i32);
        packet.extend_from_slice(&handshake);
        stream.write_all(&packet).await?;

        // status request packet: length=1, id=0x00
        stream.write_all(&[0x01, 0x00]).await?;

        let _len = read_varint(&mut stream).await?;
        let _packet_id = read_varint(&mut stream).await?;
        let str_len = read_varint(&mut stream).await? as usize;
        let mut buf = vec![0u8; str_len];
        stream.read_exact(&mut buf).await?;
        let json_str = String::from_utf8_lossy(&buf).to_string();
        let v: Value = serde_json::from_str(&json_str)?;
        let online = v["players"]["online"].as_u64().map(|n| n as u32);
        let max = v["players"]["max"].as_u64().map(|n| n as u32);
        let motd = v["description"]["text"].as_str().map(String::from)
            .or_else(|| v["description"].as_str().map(String::from));
        anyhow::Ok((online, max, motd))
    };

    match timeout(Duration::from_millis(timeout_ms), fut).await {
        Ok(Ok(v)) => Ok(v),
        Ok(Err(e)) => Err(e),
        Err(_) => anyhow::bail!("timeout"),
    }
}

fn write_varint(buf: &mut Vec<u8>, mut value: i32) {
    loop {
        let mut byte = (value & 0x7F) as u8;
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

fn write_string(buf: &mut Vec<u8>, s: &str) {
    write_varint(buf, s.len() as i32);
    buf.extend_from_slice(s.as_bytes());
}

async fn read_varint(stream: &mut TcpStream) -> Result<i32> {
    let mut num_read = 0;
    let mut result: i32 = 0;
    loop {
        let mut byte = [0u8; 1];
        stream.read_exact(&mut byte).await?;
        let value = (byte[0] & 0x7F) as i32;
        result |= value << (7 * num_read);
        num_read += 1;
        if byte[0] & 0x80 == 0 {
            break;
        }
        if num_read > 5 {
            anyhow::bail!("varint trop long");
        }
    }
    Ok(result)
}
