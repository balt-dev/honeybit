//! Handles the reading and writing of a level.

use std::io::{self, Cursor, ErrorKind, Read, Seek, SeekFrom};
use flate2::read::GzDecoder;
use jaded::Parser;
use mint::Vector3;
use oxine::packets::x16;

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
    pub spawn_point: Vector3<x16>
}

macro_rules! invalid {
    ($($f: tt)+) => {
        io::Error::new(ErrorKind::InvalidData, format!($($f)+))
    };
}

impl WorldData {
    /// Load the world data from a .mine or server_level.dat file.
    pub fn import(mut stream: impl Read + Seek) -> io::Result<()> {
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
        
        /*
        (object.width, object.height, object.depth, object.blocks.len(), object.name, object.xSpawn, object.ySpawn, object.zSpawn, object.rotSpawn) = (
            256,
            256,
            64,
            4194304,
            "A Nice World",
            150,
            34,
            158,
            0.0,
        )
 */
    }
}
