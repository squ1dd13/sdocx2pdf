use byteorder::{LittleEndian, ReadBytesExt};
use chrono::{DateTime, Utc};
use color_eyre::{Result, eyre::eyre};

/// Extends `ReadBytesExt` with methods for parsing sdoc binary files (which are little-endian).
pub trait ByteStreamLe: ReadBytesExt {
    /// Reads exactly `n` bytes into a `Vec`, and returns it.
    fn read_u8_buf(&mut self, n: usize) -> Result<Vec<u8>> {
        let mut bytes = vec![0_u8; n];
        self.read_exact(&mut bytes)?;

        Ok(bytes)
    }

    /// Reads `n_chars` bytes and returns the UTF-8 decoded result.
    fn read_u8_string(&mut self, n_chars: usize) -> Result<String> {
        String::from_utf8(self.read_u8_buf(n_chars)?).map_err(From::from)
    }

    /// Reads `2 * n_chars` bytes and returns the UTF-16 decoded result.
    fn read_u16_string(&mut self, n_chars: usize) -> Result<String> {
        let mut buf = vec![0_u16; n_chars];
        self.read_u16_into::<LittleEndian>(&mut buf)?;

        char::decode_utf16(buf)
            .collect::<Result<String, _>>()
            .map_err(From::from)
    }

    /// Reads `n_chars: u16`, then `2 * n_chars` bytes, and returns the UTF-16 decoded result.
    fn read_short_u16_string(&mut self) -> Result<String> {
        let n_chars: usize = self.read_u16_le()?.into();
        self.read_u16_string(n_chars)
    }

    /// Reads `n_chars: u32`, then `2 * n_chars` bytes, and returns the UTF-16 decoded result.
    fn read_long_u16_string(&mut self) -> Result<String> {
        let n_chars: usize = self.read_u32_le()?.try_into()?;
        self.read_u16_string(n_chars)
    }

    /// Reads `n_chars: u16`, then `n_chars` bytes, and returns the UTF-8 decoded result.
    fn read_short_u8_string(&mut self) -> Result<String> {
        let n_chars: usize = self.read_u16_le()?.into();
        self.read_u8_string(n_chars)
    }

    /// Reads an `i64` microsecond timestamp and converts it to a `DateTime`.
    fn read_timestamp(&mut self) -> Result<DateTime<Utc>> {
        let value = self.read_i64_le()?;

        DateTime::from_timestamp_micros(value)
            .ok_or_else(|| eyre!("Invalid timestamp value {value}"))
    }

    /// Reads `n_bytes: u8` and then a bitfield of that size.
    fn read_variable_length_bitfield(&mut self) -> Result<u32> {
        let n_bytes = self.read_u8()?;

        Ok(match n_bytes {
            0 => 0,
            1 => self.read_u8()?.into(),
            2 => self.read_u16_le()?.into(),
            3 => self.read_u24_le()?,
            4 => self.read_u32_le()?,
            5.. => {
                return Err(eyre!(
                    "Variable length bitfield cannot be more than 4 bytes (found {n_bytes})"
                ));
            }
        })
    }

    fn read_u16_le(&mut self) -> std::io::Result<u16> {
        self.read_u16::<LittleEndian>()
    }

    fn read_u24_le(&mut self) -> std::io::Result<u32> {
        self.read_u24::<LittleEndian>()
    }

    fn read_u32_le(&mut self) -> std::io::Result<u32> {
        self.read_u32::<LittleEndian>()
    }

    fn read_u48_le(&mut self) -> std::io::Result<u64> {
        self.read_u48::<LittleEndian>()
    }

    fn read_u64_le(&mut self) -> std::io::Result<u64> {
        self.read_u64::<LittleEndian>()
    }

    fn read_i16_le(&mut self) -> std::io::Result<i16> {
        self.read_i16::<LittleEndian>()
    }

    fn read_i24_le(&mut self) -> std::io::Result<i32> {
        self.read_i24::<LittleEndian>()
    }

    fn read_i32_le(&mut self) -> std::io::Result<i32> {
        self.read_i32::<LittleEndian>()
    }

    fn read_i48_le(&mut self) -> std::io::Result<i64> {
        self.read_i48::<LittleEndian>()
    }

    fn read_i64_le(&mut self) -> std::io::Result<i64> {
        self.read_i64::<LittleEndian>()
    }

    fn read_f32_le(&mut self) -> std::io::Result<f32> {
        self.read_f32::<LittleEndian>()
    }

    fn read_f64_le(&mut self) -> std::io::Result<f64> {
        self.read_f64::<LittleEndian>()
    }
}

impl<T: ReadBytesExt> ByteStreamLe for T {}
