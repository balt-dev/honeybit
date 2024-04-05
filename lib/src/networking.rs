//! Module handling the networking side of the server.
#![allow(clippy::cast_sign_loss, clippy::cast_possible_wrap, clippy::cast_possible_truncation, clippy::wildcard_imports)]

use std::io::{Read, Write, self, ErrorKind};

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
    fn load(source: impl Read + Copy) -> io::Result<Self> where Self: Sized;
}

/// Dictates that this type can be sent in a packet.
pub trait OutgoingPacketType : sealed::Sealed {
    #[allow(clippy::missing_errors_doc)]
    /// Dictates how to store this type in a packet.
    fn store(&self, destination: impl Write + Copy) -> io::Result<()>;
}

impl IncomingPacketType for u8 {
    fn load(mut source: impl Read) -> io::Result<Self> {
        let mut buf = [0];
        source.read_exact(&mut buf)?;
        Ok(buf[0])
    }
}

impl OutgoingPacketType for u8 {
    fn store(&self, mut destination: impl Write) -> io::Result<()> {
        destination.write_all(&[*self])
    }
}

impl IncomingPacketType for i8 {
    fn load(mut source: impl Read) -> io::Result<Self> {
        let mut buf = [0];
        source.read_exact(&mut buf)?;
        Ok(buf[0] as i8)
    }
}

impl OutgoingPacketType for i8 {
    fn store(&self, mut destination: impl Write) -> io::Result<()> {
        destination.write_all(&[*self as u8])
    }
}

impl IncomingPacketType for i16 {
    fn load(mut source: impl Read) -> io::Result<Self> {
        let mut buf = [0, 0];
        source.read_exact(&mut buf)?;
        Ok(i16::from_be_bytes(buf))
    }
}

impl OutgoingPacketType for i16 {
    fn store(&self, mut destination: impl Write) -> io::Result<()> {
        destination.write_all(&self.to_be_bytes())
    }
}

impl IncomingPacketType for x8 {
    fn load(mut source: impl Read) -> io::Result<Self> {
        let mut buf = [0];
        source.read_exact(&mut buf)?;
        Ok(x8::from_bits(buf[0] as i8))
    }
}

impl OutgoingPacketType for x8 {
    fn store(&self, mut destination: impl Write) -> io::Result<()> {
        destination.write_all(&[self.to_bits() as u8])
    }
}

impl IncomingPacketType for x16 {
    fn load(mut source: impl Read) -> io::Result<Self> {
        let mut buf = [0, 0];
        source.read_exact(&mut buf)?;
        Ok(x16::from_be_bytes(buf))
    }
}

impl OutgoingPacketType for x16 {
    fn store(&self, mut destination: impl Write) -> io::Result<()> {
        destination.write_all(&self.to_be_bytes())
    }
}

impl<T: IncomingPacketType> IncomingPacketType for Position<T> {
    fn load(source: impl Read + Copy) -> io::Result<Self> {
        Ok(Position{
            x: T::load(source)?,
            y: T::load(source)?,
            z: T::load(source)?,
        })
    }
}

impl<T: OutgoingPacketType> OutgoingPacketType for Position<T> {
    fn store(&self, destination: impl Write + Copy) -> io::Result<()> {
        self.x.store(destination)?;
        self.y.store(destination)?;
        self.z.store(destination)
    }
}

impl IncomingPacketType for String {
    fn load(mut source: impl Read) -> io::Result<Self> {
        let mut buf = [0; 64];
        source.read_exact(&mut buf)?;
        buf.is_ascii()
            // SAFETY: We checked that this is valid ASCII above.
            // By using unchecked, we avoid the unwrap, which would
            // not get optimized out otherwise.
            .then(|| unsafe { String::from_utf8_unchecked(Vec::from(buf)) })
            .ok_or(io::Error::from(ErrorKind::InvalidData))
    }
}

impl OutgoingPacketType for String {
    fn store(&self, mut destination: impl Write) -> io::Result<()> {
        if !self.is_ascii() {
            return Err(io::Error::from(ErrorKind::InvalidData));
        }
        destination.write_all(self.as_bytes())
    }
}


impl IncomingPacketType for Vec<u8> {
    fn load(mut source: impl Read + Copy) -> io::Result<Self> {
        let length = i16::load(source)?;
        let mut buf = [0; 1024];
        source.read_exact(&mut buf)?;
        Ok(Vec::from(&buf[..length as usize]))
    }
}

impl OutgoingPacketType for Vec<u8> {
    fn store(&self, mut destination: impl Write + Copy) -> io::Result<()> {
        let length = self.len().min(1024) as i16;
        length.store(destination)?;
        let length = length as usize;
        let mut buf = [0; 1024];
        buf[..length].copy_from_slice(&self.as_slice()[..length]);
        destination.write_all(&buf)
    }
}

impl IncomingPacketType for Incoming {
    fn load(source: impl Read + Copy) -> io::Result<Self> {
        let discriminant = u8::load(source)?;
        Ok(match discriminant {
            0x00 => {
                let ret = Incoming::PlayerIdentification {
                    version: u8::load(source)?,
                    username: String::load(source)?,
                    key: String::load(source)?
                };
                let _ = u8::load(source)?;
                ret
            },
            0x05 => {
                let position = Position::<i16>::load(source)?;
                let mode = u8::load(source)? != 0;
                Incoming::SetBlock {
                    position,
                    state: mode.then_some(u8::load(source)?)
                }
            },
            0x08 => {
                let _ = u8::load(source)?;
                Incoming::SetPosition {
                    position: Position::<x16>::load(source)?,
                    yaw: u8::load(source)?,
                    pitch: u8::load(source)?
                }
            },
            0x0d => {
                let _ = u8::load(source)?;
                Incoming::Message {
                    message: String::load(source)?
                }
            }
            _ => return Err(
                io::Error::from(ErrorKind::InvalidData)
            )
        })
    }
}

impl OutgoingPacketType for Outgoing {
    fn store(&self, destination: impl Write + Copy) -> io::Result<()> {
        match self {
            Outgoing::ServerIdentification { version, name, motd, operator } => {
                0x0u8.store(destination)?;
                version.store(destination)?;
                name.store(destination)?;
                motd.store(destination)?;
                (if *operator { 0x64u8 } else { 0x00u8 }).store(destination)
            },
            Outgoing::Ping => 0x1u8.store(destination),
            Outgoing::LevelInit => 0x2u8.store(destination),
            Outgoing::LevelDataChunk { data_chunk, percent_complete } => {
                0x3u8.store(destination)?;
                data_chunk.store(destination)?;
                percent_complete.store(destination)
            },
            Outgoing::LevelFinalize { size } => {
                0x4u8.store(destination)?;
                size.store(destination)
            },
            Outgoing::SetBlock { position, block } => {
                0x6u8.store(destination)?;
                position.store(destination)?;
                block.store(destination)
            },
            Outgoing::SpawnPlayer { id, name, spawn, yaw, pitch } => {
                0x7u8.store(destination)?;
                id.store(destination)?;
                name.store(destination)?;
                spawn.store(destination)?;
                yaw.store(destination)?;
                pitch.store(destination)
            },
            Outgoing::TeleportPlayer { id, position, yaw, pitch } => {
                0x8u8.store(destination)?;
                id.store(destination)?;
                position.store(destination)?;
                yaw.store(destination)?;
                pitch.store(destination)
            },
            Outgoing::UpdatePlayerLocation { id, position_change, yaw, pitch } => {
                0x9u8.store(destination)?;
                id.store(destination)?;
                position_change.store(destination)?;
                yaw.store(destination)?;
                pitch.store(destination)
            },
            Outgoing::UpdatePlayerPosition { id, position_change } => {
                0xau8.store(destination)?;
                id.store(destination)?;
                position_change.store(destination)
            },
            Outgoing::UpdatePlayerRotation { id, yaw, pitch } => {
                0xbu8.store(destination)?;
                id.store(destination)?;
                yaw.store(destination)?;
                pitch.store(destination)
            },
            Outgoing::DespawnPlayer { id } => {
                0xcu8.store(destination)?;
                id.store(destination)
            },
            Outgoing::Message { id, message } => {
                0xdu8.store(destination)?;
                id.store(destination)?;
                message.store(destination)
            },
            Outgoing::Disconnect { reason } => {
                0xeu8.store(destination)?;
                reason.store(destination)
            },
            Outgoing::UpdateUser { operator } => {
                0xfu8.store(destination)?;
                (if *operator {0x64} else {0u8}).store(destination)
            }
        }
    }
}