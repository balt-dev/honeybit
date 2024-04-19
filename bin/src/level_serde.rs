//! Handles the reading and writing of a level.

use std::io::{self, Cursor, ErrorKind, Read, Seek, SeekFrom, Write};
use flate2::read::GzDecoder;
use jaded::Parser;
use mint::Vector3;
use oxine::packets::{x16, Location};
use byteorder::{BigEndian, ReadBytesExt};

use crate::world::LevelData;

/// An instance of Java world data.
#[derive(jaded::FromJava)]
#[allow(non_snake_case)]
struct JavaWorld {
    width: i32,
    height: i32,
    depth: i32,
    blocks: Vec<u8>,
    name: String,
    xSpawn: i32,
    ySpawn: i32,
    zSpawn: i32,
    rotSpawn: f32
}

/// A holding class for a serialized level .DAT file.
struct WorldData {
    /// The raw level data.
    pub level_data: LevelData,
    /// The player spawn point.
    pub spawn_point: Location,
    /// The world's name.
    pub name: String
}

macro_rules! invalid {
    ($($f: tt)+) => {
        io::Error::new(ErrorKind::InvalidData, format!($($f)+))
    };
}

const MAGIC: &[u8] = b"OXINELV";
const VERSION: u8 = 0;

impl WorldData {
    /// Load the world data from a .mine or server_level.dat file.
    pub fn import(mut stream: impl Read + Seek) -> io::Result<WorldData> {
        // Read the compressed data length
        let mut data_len_buf = [0; 4];
        let compressed_len = stream.seek(SeekFrom::End(-4))?;
        stream.read_exact(&mut data_len_buf)?;
        stream.rewind()?;
        let data_len = u32::from_be_bytes(data_len_buf);

        // Read the gzipped data into a buffer
        let mut reader = GzDecoder::new(stream.take(compressed_len));
        let mut buf = Vec::with_capacity(data_len as usize);
        reader.read_to_end(&mut buf)?;

        // Find the start of the Java object
        let start = buf.windows(2).position(|win| *win == [0xac, 0xed]).ok_or(invalid!("Could not find Java object"))?;
        let cursor = Cursor::new(&buf[start..]);
        
        // Decode the Java object
        let mut parser = Parser::new(cursor)
            .map_err(|err| invalid!("Decoding error: {err}"))?;
        let object: JavaWorld = parser.read_as().map_err(|err| invalid!("Parsing error: {err}"))?;

        let spawn_point = Location {
            position: Vector3 {
                x: x16::from_num(object.xSpawn), y: x16::from_num(object.ySpawn), z: x16::from_num(object.zSpawn)
            },
            pitch: 0,
            yaw: (object.rotSpawn / 360.0 * 256.0) as u8
        };

        let level_data = LevelData {
            raw_data: object.blocks,
            dimensions: Vector3 { x: object.width as u16, y: object.depth as u16, z: object.height as u16 }
        };
        
        Ok(WorldData {
            level_data,
            spawn_point,
            name: object.name,
        })
    }

    /// Load the world data from a .ox file.
    /// 
    /// The level format is as follows:
    /// - Magic: `b"OXINELV"`
    /// - File version: `u8`
    /// - World dimensions: `[u16; 3]`
    /// - Spawn position: `[x16; 3]`
    /// - Spawn rotation: `[u8; 2]`
    /// - Level name length: `u8`
    /// - Level name: `str`
    /// - Unzipped level data size: `u64`
    /// - Gzipped level data: `[u8]`
    /// 
    /// All values are in big endian.
    /// 
    /// # Errors
    /// Errors if the stream fails to be decoded.
    pub fn load(mut stream: impl Read) -> io::Result<WorldData> {
        // Check magic string
        let mut magic_buf = [0; 7];
        stream.read_exact(&mut magic_buf)
            .map_err(|err| invalid!("Failed to read magic string: {err}"))?;
        if magic_buf != MAGIC {
            return Err(invalid!("Incorrect magic string"));
        }
        // Check file version
        let version = stream.read_u8()
            .map_err(|err| invalid!("Failed to read file version: {err}"))?;
        if version != VERSION {
            return Err(invalid!("Incorrect file version {version} (expected {VERSION})"));
        }
        // Get dimensions, player spawn, and level name
        // NOTE: Since packets use AsyncRead and AsyncWrite, we can't use their implementations
        let mut dimensions = [0u16; 3];
        stream.read_u16_into::<BigEndian>(&mut dimensions)
            .map_err(|err| invalid!("Failed to decode level dimensions: {err}"))?;
        let dimensions = Vector3::<u16>::from(dimensions);

        let mut spawn_position = [0i16; 3];
        stream.read_i16_into::<BigEndian>(&mut spawn_position)
            .map_err(|err| invalid!("Failed to decode player spawn position: {err}"))?;
        let position = Vector3::<x16>::from(spawn_position.map(x16::from_num));

        let mut yaw_pitch = [0u8; 2];
        stream.read_exact(&mut yaw_pitch)
            .map_err(|err| invalid!("Failed to decode player spawn rotation: {err}"))?;
        let [yaw, pitch] = yaw_pitch;

        let mut level_name = String::new();
        let name_len = stream.read_u8()?;
        (&mut stream).take(name_len as u64).read_to_string(&mut level_name)
            .map_err(|err| invalid!("Failed to decode level name: {err}"))?;

        // Get unzipped data length
        let raw_length = stream.read_u64::<BigEndian>()
            .map_err(|err| invalid!("Failed to decode level data length: {err}"))?;
        if raw_length > isize::MAX as u64 {
            return Err(invalid!("World data of {raw_length} bytes is too large to be allocated on this architecture"));
        }
        let mut raw_data = Vec::with_capacity(raw_length as usize);

        // Unzip the data
        let mut decoder = GzDecoder::new(stream);
        decoder.read_exact(&mut raw_data)
            .map_err(|err| invalid!("Failed to decode level data: {err}"))?;

        Ok( WorldData {
            level_data: LevelData { raw_data, dimensions },
            spawn_point: Location { position, yaw, pitch },
            name: level_name
        } )
    }

    /// Store the world data into a .ox file.
    /// See [`load`] for the level format
    /// 
    /// # Errors
    /// Errors if the world fails to be encoded.
    pub fn store(mut stream: impl Write) -> io::Result<()> {
        stream.write_all(MAGIC)
            .map_err(|err| invalid!("Failed to encode magic string: {err}"))?;
    }
}
