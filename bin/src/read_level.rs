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

macro_rules! invalid {
    ($($f: tt)+) => {
        io::Error::new(ErrorKind::InvalidData, format!($($f)+))
    };
}

impl WorldData {
    /// Load the world data from a stream.
    pub fn load(mut stream: impl Read + Seek) -> io::Result<()> {
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
        
        // Skip to the coordinates
        let coords: &[u8; 12] = buf.as_slice().get(284 .. 296)
            .map(|slice|
                <&[u8; 12] as TryFrom<&[u8]>>::try_from(slice)
                    .expect("the range will always be 12 long") // This expect gets optimized out https://godbolt.org/z/jjfbfhb6W
            ).ok_or(invalid!("coordinate slice is out of bounds"))?;
        
        // The rest of this should probably use bytemuck
        // See the bottom of https://wiki.vg/Classic_DAT_Format
        
        todo!("implement rest of decoding")
    }
}
