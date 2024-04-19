//! Handles the reading and writing of a level.

use std::io::{self, ErrorKind, Read, Seek, SeekFrom};
use flate2::read::GzDecoder;

/// A holding class for a serialized level .DAT file.
struct WorldData {
    /// The raw level data.
    pub level_data: Vec<u8>,
    /// The level's dimensions.
    pub dimensions: (u16, u16, u16),
    /// The player spawn point, in fixed point, with 5 bits after the radix point.
    pub spawn_point: (u16, u16, u16)
}

impl WorldData {
    /// Load the world data from a stream.
    pub fn load(mut stream: impl Read + Seek) -> Result<(), io::Error> {
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

        // Seek to the start of the serialized data
        let mut found = buf.windows(2)
            .find(|b| *b == [0xAC, 0xED])
            .ok_or(io::Error::from(ErrorKind::InvalidData))?;

        // Skip the headers
        found = &found[6..];


        todo!("https://gist.github.com/ddevault/324122945a569a513bae")
    }
}
