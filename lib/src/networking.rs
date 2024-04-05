//! Module handling the networking side of the server.

use std::io::{Read, Write, self, ErrorKind};

use crate::packets::*;

// I'll be real, I could've used serde for this. I just didn't want to.

trait PacketType {
    /// Dictates how to load this type from a packet.
    fn load(source: impl Read + Copy) -> io::Result<Self> where Self: Sized;

    /// Dictates how to store this type in a packet.
    fn store(&self, destination: impl Write + Copy) -> io::Result<()>;
}

impl PacketType for u8 {
    fn load(mut source: impl Read) -> io::Result<Self> {
        let mut buf = [0];
        source.read_exact(&mut buf)?;
        Ok(buf[0])
    }

    fn store(&self, mut destination: impl Write) -> io::Result<()> {
        destination.write_all(&[*self])
    }
}

impl PacketType for i8 {
    fn load(mut source: impl Read) -> io::Result<Self> {
        let mut buf = [0];
        source.read_exact(&mut buf)?;
        Ok(buf[0] as i8)
    }

    fn store(&self, mut destination: impl Write) -> io::Result<()> {
        destination.write_all(&[*self as u8])
    }
}

impl PacketType for u16 {
    fn load(mut source: impl Read) -> io::Result<Self> {
        let mut buf = [0, 0];
        source.read_exact(&mut buf)?;
        Ok(u16::from_be_bytes(buf))
    }

    fn store(&self, mut destination: impl Write) -> io::Result<()> {
        destination.write_all(&self.to_be_bytes())
    }
}

impl PacketType for i16 {
    fn load(mut source: impl Read) -> io::Result<Self> {
        let mut buf = [0, 0];
        source.read_exact(&mut buf)?;
        Ok(i16::from_be_bytes(buf))
    }

    fn store(&self, mut destination: impl Write) -> io::Result<()> {
        destination.write_all(&self.to_be_bytes())
    }
}

impl PacketType for x8 {
    fn load(mut source: impl Read) -> io::Result<Self> {
        let mut buf = [0];
        source.read_exact(&mut buf)?;
        Ok(x8::from_bits(buf[0] as i8))
    }

    fn store(&self, mut destination: impl Write) -> io::Result<()> {
        destination.write_all(&[self.to_bits() as u8])
    }
}

impl PacketType for x16 {
    fn load(mut source: impl Read) -> io::Result<Self> {
        let mut buf = [0, 0];
        source.read_exact(&mut buf)?;
        Ok(x16::from_be_bytes(buf))
    }

    fn store(&self, mut destination: impl Write) -> io::Result<()> {
        destination.write_all(&self.to_be_bytes())
    }
}

impl<T: PacketType> PacketType for Position<T> {
    fn load(source: impl Read + Copy) -> io::Result<Self> {
        Ok(Position{
            x: T::load(source)?,
            y: T::load(source)?,
            z: T::load(source)?,
        })
    }

    fn store(&self, destination: impl Write + Copy) -> io::Result<()> {
        self.x.store(destination)?;
        self.y.store(destination)?;
        self.z.store(destination)
    }
}

impl PacketType for String {
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

    fn store(&self, mut destination: impl Write) -> io::Result<()> {
        if !self.is_ascii() {
            return Err(io::Error::from(ErrorKind::InvalidData));
        }
        destination.write_all(self.as_bytes())
    }
}


impl PacketType for Vec<u8> {
    fn load(mut source: impl Read + Copy) -> io::Result<Self> {
        let length = u16::load(source)?;
        let mut buf = [0; 1024];
        source.read_exact(&mut buf)?;
        Ok(Vec::from(&buf[..length as usize]))
    }

    fn store(&self, mut destination: impl Write + Copy) -> io::Result<()> {
        let length = self.len().min(1024) as u16;
        length.store(destination)?;
        let length = length as usize;
        let mut buf = [0; 1024];
        buf[..length].copy_from_slice(&self.as_slice()[..length]);
        destination.write_all(&buf)
    }
}

impl PacketType for Incoming {
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

    fn store(&self, _: impl Write + Copy) -> io::Result<()> {
        panic!("incoming packets should not be sent")
    }
}

impl PacketType for Outgoing {
    fn load(_: impl Read + Copy) -> io::Result<Self> where Self: Sized {
        panic!("outgoing packets should not be loaded")
    }

    fn store(&self, destination: impl Write + Copy) -> io::Result<()> {
        match self {
            Outgoing::ServerIdentification { version, name, motd, operator } => {
                0u8.store(destination)?;
                version.store(destination)?;
                name.store(destination)?;
                motd.store(destination)?;
                (if *operator { 0x64u8 } else { 0x00u8 }).store(destination)
            },
            Outgoing::Ping => 1u8.store(destination),
            Outgoing::LevelInit => 2u8.store(destination),
            Outgoing::LevelDataChunk { data_chunk, percent_complete } => {
                3u8.store(destination)?;
                data_chunk.store(destination)?;
                percent_complete.store(destination)
            }
        }
    }
}