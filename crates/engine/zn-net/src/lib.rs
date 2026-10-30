//! zn-net: netcode (snapshot/delta, prediction, lag-compensation).

use serde::{Deserialize, Serialize};
use zn_sim::{InputCommand, Inventory, PlayerProfile, Snapshot};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum ClientPacket {
    Hello {
        name: String,
    },
    Login {
        username: String,
        password: String,
    },
    Connect {
        token: String,
    },
    Input(InputCommand),
    Shot {
        seq: u32,
        origin: [f32; 3],
        dir: [f32; 3],
        client_time: u64,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum ServerPacket {
    Welcome {
        id: u64,
    },
    LoginResult {
        ok: bool,
        token: Option<String>,
        error: Option<String>,
    },
    Profile {
        profile: PlayerProfile,
        inventory: Inventory,
    },
    Snapshot(Snapshot),
    ShotResult {
        hit: Option<u64>,
    },
}

pub fn encode<T: Serialize>(packet: &T) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let data = bincode::serialize(packet)?;
    let len = data.len() as u32;
    let mut out = Vec::with_capacity(4 + data.len());
    out.extend_from_slice(&len.to_le_bytes());
    out.extend_from_slice(&data);
    Ok(out)
}

pub fn decode_client(bytes: &[u8]) -> Result<ClientPacket, Box<dyn std::error::Error>> {
    Ok(bincode::deserialize(bytes)?)
}

pub fn decode_server(bytes: &[u8]) -> Result<ServerPacket, Box<dyn std::error::Error>> {
    Ok(bincode::deserialize(bytes)?)
}

pub async fn send_packet<W, T>(stream: &mut W, packet: &T) -> Result<(), Box<dyn std::error::Error>>
where
    W: tokio::io::AsyncWrite + Unpin,
    T: Serialize,
{
    use tokio::io::AsyncWriteExt;
    let data = encode(packet)?;
    stream.write_all(&data).await?;
    Ok(())
}

pub async fn recv_packet<T: for<'de> Deserialize<'de>>(
    stream: &mut tokio::net::TcpStream,
) -> Result<T, Box<dyn std::error::Error>> {
    use tokio::io::AsyncReadExt;
    let mut len_buf = [0u8; 4];
    stream.read_exact(&mut len_buf).await?;
    let len = u32::from_le_bytes(len_buf) as usize;
    let mut buf = vec![0u8; len];
    stream.read_exact(&mut buf).await?;
    Ok(bincode::deserialize(&buf)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_decode_roundtrip() {
        let pkt = ClientPacket::Hello {
            name: "test".into(),
        };
        let data = encode(&pkt).unwrap();
        // strip 4-byte length prefix
        let client: ClientPacket = bincode::deserialize(&data[4..]).unwrap();
        match client {
            ClientPacket::Hello { name } => assert_eq!(name, "test"),
            _ => panic!("wrong variant"),
        }
    }
}
