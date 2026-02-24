use byteorder::{LittleEndian, ReadBytesExt};
use chrono::{DateTime, Utc};
use std::io;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum ReadStringError {
    #[error("failed to read size field")]
    SizeIo(io::Error),

    #[error("failed to read character data")]
    BytesIo(io::Error),

    #[error("size does not fit in `usize`")]
    SizeConversion(std::num::TryFromIntError),

    #[error("invalid utf-16")]
    Utf16Decode(#[from] std::string::FromUtf16Error),

    #[error("invalid utf-8")]
    Utf8Decode(#[from] std::string::FromUtf8Error),
}

#[derive(Error, Debug)]
pub enum ReadTimestampError {
    #[error("failed to read")]
    Io(#[from] io::Error),

    #[error("out-of-range timestamp {0}")]
    OutOfRange(i64),
}

#[derive(Error, Debug)]
pub enum ReadBitfieldError {
    #[error("failed to read size field")]
    SizeIo(io::Error),

    #[error("invalid bitfield size {0} (must be <= 4)")]
    SizeOutOfRange(u8),

    #[error("failed to read bitfield bytes")]
    BitsIo(io::Error),
}

/// Extends `ReadBytesExt` with methods for parsing sdoc binary files (which are little-endian).
pub trait ByteStreamLe: ReadBytesExt {
    /// Reads exactly `n` bytes into a `Vec`, and returns it.
    fn read_u8_buf(&mut self, n: usize) -> io::Result<Vec<u8>> {
        let mut bytes = vec![0_u8; n];
        self.read_exact(&mut bytes)?;

        Ok(bytes)
    }

    /// Reads `n_chars` bytes and returns the UTF-8 decoded result.
    fn read_u8_string(&mut self, n_chars: usize) -> Result<String, ReadStringError> {
        Ok(String::from_utf8(
            self.read_u8_buf(n_chars)
                .map_err(ReadStringError::BytesIo)?,
        )?)
    }

    /// Reads `2 * n_chars` bytes and returns the UTF-16 decoded result.
    fn read_u16_string(&mut self, n_chars: usize) -> Result<String, ReadStringError> {
        let mut buf = vec![0_u16; n_chars];
        self.read_u16_into::<LittleEndian>(&mut buf)
            .map_err(ReadStringError::BytesIo)?;

        Ok(String::from_utf16(&buf)?)
    }

    /// Reads `n_chars: u16`, then `2 * n_chars` bytes, and returns the UTF-16 decoded result.
    fn read_short_u16_string(&mut self) -> Result<String, ReadStringError> {
        let n_chars: usize = self.read_u16_le().map_err(ReadStringError::SizeIo)?.into();
        self.read_u16_string(n_chars)
    }

    /// Reads `n_chars: u32`, then `2 * n_chars` bytes, and returns the UTF-16 decoded result.
    fn read_long_u16_string(&mut self) -> Result<String, ReadStringError> {
        let n_chars: usize = self
            .read_u32_le()
            .map_err(ReadStringError::SizeIo)?
            .try_into()
            .map_err(ReadStringError::SizeConversion)?;

        self.read_u16_string(n_chars)
    }

    /// Reads `n_chars: u16`, then `n_chars` bytes, and returns the UTF-8 decoded result.
    fn read_short_u8_string(&mut self) -> Result<String, ReadStringError> {
        let n_chars: usize = self.read_u16_le().map_err(ReadStringError::SizeIo)?.into();
        self.read_u8_string(n_chars)
    }

    /// Reads an `i64` microsecond timestamp and converts it to a `DateTime`.
    fn read_timestamp(&mut self) -> Result<DateTime<Utc>, ReadTimestampError> {
        let value = self.read_i64_le()?;

        DateTime::from_timestamp_micros(value).ok_or(ReadTimestampError::OutOfRange(value))
    }

    /// Reads `n_bytes: u8` and then a bitfield of that size.
    fn read_variable_length_bitfield(&mut self) -> Result<u32, ReadBitfieldError> {
        let n_bytes = self.read_u8().map_err(ReadBitfieldError::SizeIo)?;

        match n_bytes {
            0 => return Ok(0),

            1 => self.read_u8().map(From::from),
            2 => self.read_u16_le().map(From::from),
            3 => self.read_u24_le(),
            4 => self.read_u32_le(),

            too_big => return Err(ReadBitfieldError::SizeOutOfRange(too_big)),
        }
        .map_err(ReadBitfieldError::BitsIo)
    }

    fn read_u16_le(&mut self) -> io::Result<u16> {
        self.read_u16::<LittleEndian>()
    }

    fn read_u24_le(&mut self) -> io::Result<u32> {
        self.read_u24::<LittleEndian>()
    }

    fn read_u32_le(&mut self) -> io::Result<u32> {
        self.read_u32::<LittleEndian>()
    }

    fn read_u48_le(&mut self) -> io::Result<u64> {
        self.read_u48::<LittleEndian>()
    }

    fn read_u64_le(&mut self) -> io::Result<u64> {
        self.read_u64::<LittleEndian>()
    }

    fn read_i16_le(&mut self) -> io::Result<i16> {
        self.read_i16::<LittleEndian>()
    }

    fn read_i24_le(&mut self) -> io::Result<i32> {
        self.read_i24::<LittleEndian>()
    }

    fn read_i32_le(&mut self) -> io::Result<i32> {
        self.read_i32::<LittleEndian>()
    }

    fn read_i48_le(&mut self) -> io::Result<i64> {
        self.read_i48::<LittleEndian>()
    }

    fn read_i64_le(&mut self) -> io::Result<i64> {
        self.read_i64::<LittleEndian>()
    }

    fn read_f32_le(&mut self) -> io::Result<f32> {
        self.read_f32::<LittleEndian>()
    }

    fn read_f64_le(&mut self) -> io::Result<f64> {
        self.read_f64::<LittleEndian>()
    }
}

impl<T: ReadBytesExt> ByteStreamLe for T {}

/// An error type for use when parsing should finish at a particular offset in the stream, but
/// ends somewhere else. This may indicate a parsing bug.
#[derive(Error, Debug)]
#[error("end offset {actual_end} differs from the expected {expected_end}")]
pub struct WrongEndOffsetError {
    pub actual_end: u64,
    pub expected_end: u64,
}
