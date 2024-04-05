//! Module handling the networking side of the server.
#![allow(clippy::cast_sign_loss, clippy::cast_possible_wrap, clippy::cast_possible_truncation, clippy::wildcard_imports, async_fn_in_trait)]

use async_std::io::{Read, Write, ReadExt, WriteExt};

use std::io::{self, ErrorKind};

use crate::packets::*;

// I'll be real, I could've used serde for this. I just didn't want to.

mod sealed {
    use super::*;
    pub trait Sealed {}
    impl Sealed for u8 {}
    impl Sealed for i8 {}
    impl Sealed for x8 {}
    impl Sealed for i16 {}
    impl Sealed for x16 {}
    impl Sealed for Vec<u8> {}
    impl<T: Sealed> Sealed for Position<T> {}
    impl Sealed for String {}
    impl Sealed for Incoming {}
    impl Sealed for Outgoing {}
}

/// Dictates that this type can be loaded from a packet.
pub trait IncomingPacketType : sealed::Sealed {
    #[allow(clippy::missing_errors_doc)]
    /// Dictates how to load this type from a packet.
    async fn load(source: impl Read + Copy + Unpin) -> io::Result<Self> where Self: Sized;
}

/// Dictates that this type can be sent in a packet.
pub trait OutgoingPacketType : sealed::Sealed {
    #[allow(clippy::missing_errors_doc)]
    /// Dictates how to store this type in a packet.
    async fn store(&self, destination: impl Write + Copy + Unpin) -> io::Result<()>;
}

impl IncomingPacketType for u8 {
    async fn load(mut source: impl Read + Unpin) -> io::Result<Self> {
        let mut buf = [0];
        source.read_exact(&mut buf).await?;
        Ok(buf[0])
    }
}

impl OutgoingPacketType for u8 {
    async fn store(&self, mut destination: impl Write + Unpin) -> io::Result<()> {
        destination.write_all(&[*self]).await
    }
}

impl IncomingPacketType for i8 {
    async fn load(mut source: impl Read + Unpin) -> io::Result<Self> {
        let mut buf = [0];
        source.read_exact(&mut buf).await?;
        Ok(buf[0] as i8)
    }
}

impl OutgoingPacketType for i8 {
    async fn store(&self, mut destination: impl Write + Unpin) -> io::Result<()> {
        destination.write_all(&[*self as u8]).await
    }
}

impl IncomingPacketType for i16 {
    async fn load(mut source: impl Read + Unpin) -> io::Result<Self> {
        let mut buf = [0, 0];
        source.read_exact(&mut buf).await?;
        Ok(i16::from_be_bytes(buf))
    }
}

impl OutgoingPacketType for i16 {
    async fn store(&self, mut destination: impl Write + Unpin) -> io::Result<()> {
        destination.write_all(&self.to_be_bytes()).await
    }
}

impl IncomingPacketType for x8 {
    async fn load(mut source: impl Read + Unpin) -> io::Result<Self> {
        let mut buf = [0];
        source.read_exact(&mut buf).await?;
        Ok(x8::from_bits(buf[0] as i8))
    }
}

impl OutgoingPacketType for x8 {
    async fn store(&self, mut destination: impl Write + Unpin) -> io::Result<()> {
        destination.write_all(&[self.to_bits() as u8]).await
    }
}

impl IncomingPacketType for x16 {
    async fn load(mut source: impl Read + Unpin) -> io::Result<Self> {
        let mut buf = [0, 0];
        source.read_exact(&mut buf).await?;
        Ok(x16::from_be_bytes(buf))
    }
}

impl OutgoingPacketType for x16 {
    async fn store(&self, mut destination: impl Write + Unpin) -> io::Result<()> {
        destination.write_all(&self.to_be_bytes()).await
    }
}

impl<T: IncomingPacketType> IncomingPacketType for Position<T> {
    async fn load(source: impl Read + Copy + Unpin) -> io::Result<Self> {
        Ok(Position{
            x: T::load(source).await?,
            y: T::load(source).await?,
            z: T::load(source).await?,
        })
    }
}

impl<T: OutgoingPacketType> OutgoingPacketType for Position<T> {
    async fn store(&self, destination: impl Write + Copy + Unpin) -> io::Result<()> {
        self.x.store(destination).await?;
        self.y.store(destination).await?;
        self.z.store(destination).await
    }
}

impl IncomingPacketType for String {
    async fn load(mut source: impl Read + Unpin) -> io::Result<Self> {
        let mut buf = [0; 64];
        source.read_exact(&mut buf).await?;
        buf.is_ascii()
            // SAFETY: We checked that this is valid ASCII above.
            // By using unchecked, we avoid the unwrap, which would
            // not get optimized out otherwise.
            .then(|| unsafe { String::from_utf8_unchecked(Vec::from(buf)) })
            .ok_or(io::Error::from(ErrorKind::InvalidData))
    }
}

impl OutgoingPacketType for String {
    async fn store(&self, mut destination: impl Write + Unpin) -> io::Result<()> {
        if !self.is_ascii() {
            return Err(io::Error::from(ErrorKind::InvalidData));
        }
        destination.write_all(self.as_bytes()).await
    }
}


impl IncomingPacketType for Vec<u8> {
    async fn load(mut source: impl Read + Copy + Unpin) -> io::Result<Self> {
        let length = i16::load(source).await?;
        let mut buf = [0; 1024];
        source.read_exact(&mut buf).await?;
        Ok(Vec::from(&buf[..length as usize]))
    }
}

impl OutgoingPacketType for Vec<u8> {
    async fn store(&self, mut destination: impl Write + Copy + Unpin) -> io::Result<()> {
        let length = self.len().min(1024) as i16;
        length.store(destination).await?;
        let length = length as usize;
        let mut buf = [0; 1024];
        buf[..length].copy_from_slice(&self.as_slice()[..length]);
        destination.write_all(&buf).await
    }
}

impl IncomingPacketType for Incoming {
    async fn load(source: impl Read + Copy + Unpin) -> io::Result<Self> {
        let discriminant = u8::load(source).await?;
        Ok(match discriminant {
            0x00 => {
                let ret = Incoming::PlayerIdentification {
                    version: u8::load(source).await?,
                    username: String::load(source).await?,
                    key: String::load(source).await?
                };
                let _ = u8::load(source).await?;
                ret
            },
            0x05 => {
                let position = Position::<i16>::load(source).await?;
                let mode = u8::load(source).await? != 0;
                Incoming::SetBlock {
                    position,
                    state: mode.then_some(u8::load(source).await?)
                }
            },
            0x08 => {
                let _ = u8::load(source).await?;
                Incoming::SetPosition {
                    position: Position::<x16>::load(source).await?,
                    yaw: u8::load(source).await?,
                    pitch: u8::load(source).await?
                }
            },
            0x0d => {
                let _ = u8::load(source).await?;
                Incoming::Message {
                    message: String::load(source).await?
                }
            }
            _ => return Err(
                io::Error::from(ErrorKind::InvalidData)
            )
        })
    }
}

impl OutgoingPacketType for Outgoing {
    async fn store(&self, destination: impl Write + Copy + Unpin) -> io::Result<()> {
        match self {
            Outgoing::ServerIdentification { version, name, motd, operator } => {
                0x0u8.store(destination).await?;
                version.store(destination).await?;
                name.store(destination).await?;
                motd.store(destination).await?;
                (if *operator { 0x64u8 } else { 0x00u8 }).store(destination).await
            },
            Outgoing::Ping => 0x1u8.store(destination).await,
            Outgoing::LevelInit => 0x2u8.store(destination).await,
            Outgoing::LevelDataChunk { data_chunk, percent_complete } => {
                0x3u8.store(destination).await?;
                data_chunk.store(destination).await?;
                percent_complete.store(destination).await
            },
            Outgoing::LevelFinalize { size } => {
                0x4u8.store(destination).await?;
                size.store(destination).await
            },
            Outgoing::SetBlock { position, block } => {
                0x6u8.store(destination).await?;
                position.store(destination).await?;
                block.store(destination).await
            },
            Outgoing::SpawnPlayer { id, name, spawn, yaw, pitch } => {
                0x7u8.store(destination).await?;
                id.store(destination).await?;
                name.store(destination).await?;
                spawn.store(destination).await?;
                yaw.store(destination).await?;
                pitch.store(destination).await
            },
            Outgoing::TeleportPlayer { id, position, yaw, pitch } => {
                0x8u8.store(destination).await?;
                id.store(destination).await?;
                position.store(destination).await?;
                yaw.store(destination).await?;
                pitch.store(destination).await
            },
            Outgoing::UpdatePlayerLocation { id, position_change, yaw, pitch } => {
                0x9u8.store(destination).await?;
                id.store(destination).await?;
                position_change.store(destination).await?;
                yaw.store(destination).await?;
                pitch.store(destination).await
            },
            Outgoing::UpdatePlayerPosition { id, position_change } => {
                0xau8.store(destination).await?;
                id.store(destination).await?;
                position_change.store(destination).await
            },
            Outgoing::UpdatePlayerRotation { id, yaw, pitch } => {
                0xbu8.store(destination).await?;
                id.store(destination).await?;
                yaw.store(destination).await?;
                pitch.store(destination).await
            },
            Outgoing::DespawnPlayer { id } => {
                0xcu8.store(destination).await?;
                id.store(destination).await
            },
            Outgoing::Message { id, message } => {
                0xdu8.store(destination).await?;
                id.store(destination).await?;
                message.store(destination).await
            },
            Outgoing::Disconnect { reason } => {
                0xeu8.store(destination).await?;
                reason.store(destination).await
            },
            Outgoing::UpdateUser { operator } => {
                0xfu8.store(destination).await?;
                (if *operator {0x64} else {0u8}).store(destination).await
            }
        }
    }
}