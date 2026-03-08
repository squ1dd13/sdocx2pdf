use byteorder::{LittleEndian, ReadBytesExt};
use chrono::{DateTime, Utc};
use io::Read;
use std::io::{self, Seek, SeekFrom, Take};
use thiserror::Error;

pub trait TryParse<R: ?Sized>: Sized {
    type ParseError;

    fn try_parse(reader: &mut R) -> Result<Self, Self::ParseError>;
}

#[derive(Error, Debug)]
pub enum ReadStringError {
    #[error("failed to read size field")]
    SizeIo(#[source] io::Error),

    #[error("failed to read character data")]
    BytesIo(#[source] io::Error),

    #[error("size does not fit in `usize`")]
    SizeConversion(#[source] std::num::TryFromIntError),

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
    SizeIo(#[source] io::Error),

    #[error("invalid bitfield size {0} (must be <= 4)")]
    SizeOutOfRange(u8),

    #[error("failed to read bitfield bytes")]
    BitsIo(#[source] io::Error),
}

#[derive(Error, Debug)]
pub enum TakeInclusiveLengthPrefixedError {
    #[error("failed to read size field")]
    Io(#[from] io::Error),

    #[error("size {0} cannot be inclusive as the size field itself is 4 bytes")]
    SizeTooSmall(u32),
}

/// Reader adapter that provides access to a limited range of the underlying reader.
///
/// A `Window<T>` is used exactly like the underlying `T`, except that it returns an error if an
/// operation attempts to go beyond its bounds. In particular, if `T: Seek`, `Window<T>` uses the
/// same positioning as `T`, such that `window.seek(x)` is equivalent to `inner.seek(x)` (provided
/// `x` is within the window).
///
/// A `Window<T>` may be turned into a `BlindWindow<T>`, which uses zero-based positioning relative
/// to the start of the window.
pub struct Window<T> {
    inner: T,

    /// Offset of the next byte from the start of the window.
    local_pos: u64,

    /// Size of the window.
    length: u64,
}

/// Like a `Window<T>`, except that `SeekFrom::Start(0)` and `SeekFrom::End(0)` in a
/// `BlindWindow<T>` refer respectively to the start and end of the window, as opposed to the
/// start and end of the underlying `T`.
pub struct BlindWindow<T>(Window<T>);

impl<T> Window<T> {
    /// Returns a window into `inner` providing access to at most `length` bytes from the current
    /// position.
    #[expect(dead_code)]
    pub const fn new(inner: T, length: u64) -> Window<T> {
        Window {
            inner,
            local_pos: 0,
            length,
        }
    }

    /// Returns a window into `inner` providing access to `length - local_pos` bytes after
    /// and `local_pos` bytes before the current position; that is, a window of `length` bytes
    /// such that the next byte in `inner` is considered the `(local_pos)`th byte in the window.
    ///
    /// If `local_pos == 0`, this is equivalent to `new`.
    ///
    /// Returns an error if `local_pos > length`.
    pub fn new_at(inner: T, local_pos: u64, length: u64) -> io::Result<Window<T>> {
        if local_pos > length {
            return Err(io::Error::from(io::ErrorKind::InvalidInput));
        }

        Ok(Window {
            inner,
            local_pos,
            length,
        })
    }
}

impl<T> From<Window<T>> for BlindWindow<T> {
    fn from(value: Window<T>) -> BlindWindow<T> {
        BlindWindow(value)
    }
}

impl<T> ExactSizedStream for Window<T> {
    fn n_remaining(&self) -> u64 {
        self.length.strict_sub(self.local_pos)
    }
}

impl<T> ExactSizedStream for BlindWindow<T> {
    fn n_remaining(&self) -> u64 {
        self.0.n_remaining()
    }
}

impl<T: Seek> Window<T> {
    fn inner_length(&mut self) -> io::Result<u64> {
        // todo: Use `stream_len`, once stable
        let original = self.inner.stream_position()?;
        let length = self.inner.seek(SeekFrom::End(0))?;
        self.inner.seek(SeekFrom::Start(original))?;

        Ok(length)
    }

    fn start(&mut self) -> io::Result<u64> {
        // If stream position `x` corresponds to local position `y`, then the start of the
        // window must be at `x - y`.
        Ok(self.inner.stream_position()? - self.local_pos)
    }

    fn end(&mut self) -> io::Result<u64> {
        Ok(self.start()? + self.length)
    }
}

impl<T: Read> Read for Window<T> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = (&mut self.inner)
            .take(self.length - self.local_pos)
            .read(buf)?;

        self.local_pos += n as u64;

        Ok(n)
    }
}

impl<T: Read> Read for BlindWindow<T> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.0.read(buf)
    }
}

impl<T: Seek> Seek for Window<T> {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        match pos {
            SeekFrom::Start(new_real) => {
                if let Some(new_local) = new_real.checked_sub(self.start()?)
                    && new_local <= self.length
                {
                    self.local_pos = new_local;
                    return self.inner.seek(pos);
                }
            }

            SeekFrom::End(end_offset) => {
                if let Some(new_real) = self.inner_length()?.checked_add_signed(end_offset) {
                    return self.seek(SeekFrom::Start(new_real));
                }

                // Failure here is actually for attempting to seek before the start of `inner`,
                // but that is still technically outside the window.
            }

            SeekFrom::Current(offset) => {
                if let Some(new_local) = self.local_pos.checked_add_signed(offset)
                    && new_local <= self.length
                {
                    self.local_pos = new_local;
                    return self.inner.seek(pos);
                }
            }
        }

        Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "attempted to seek outside window",
        ))
    }
}

// fixme: This implementation is very backwards... Window should be built on top of BlindWindow
impl<T: Seek> Seek for BlindWindow<T> {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        let pos = match pos {
            SeekFrom::Start(from_window_start) => {
                SeekFrom::Start(self.0.start()? + from_window_start)
            }

            SeekFrom::End(from_window_end) => SeekFrom::Start(
                self.0
                    .end()?
                    .checked_add_signed(from_window_end)
                    .ok_or_else(|| io::Error::from(io::ErrorKind::InvalidInput))?,
            ),

            current @ SeekFrom::Current(_) => current,
        };

        let seeked_pos_inner = self.0.seek(pos)?;

        debug_assert_eq!(
            seeked_pos_inner.strict_sub(self.0.start().unwrap()),
            self.0.local_pos
        );

        Ok(self.0.local_pos)
    }
}

macro_rules! vec_read {
    ($fn_name:ident, $read_type:ty, $read_into_fn:ident) => {
        #[allow(dead_code)]
        fn $fn_name(&mut self, n: usize) -> io::Result<Vec<$read_type>> {
            let mut v = vec![Default::default(); n];
            self.$read_into_fn::<LittleEndian>(&mut v)?;
            Ok(v)
        }
    };
}

macro_rules! le_wrapper {
    ($le_name:ident, $t:ty, $bo_name:ident) => {
        #[allow(dead_code)]
        fn $le_name(&mut self) -> io::Result<$t> {
            self.$bo_name::<LittleEndian>()
        }
    };
}

/// Extends `ReadBytesExt` with methods for parsing sdoc binary files (which are little-endian).
pub trait ByteStreamLe: Read {
    /// Reads `size: u32` from `self`, and then returns a `Window` that can read at most `size - 4`
    /// further bytes from `self`. The 4 bytes before the current position in the returned reader
    /// are the bytes of `size`.
    ///
    /// This method is intended for obtaining a stream for reading data that declares its own size
    /// while including the size of the `u32` used to encode that size. As such, it returns an
    /// error if `size < 4`.
    fn take_inclusive_length_prefixed(
        mut self,
    ) -> Result<Window<Self>, TakeInclusiveLengthPrefixedError>
    where
        Self: Sized,
    {
        let frame_size = self.read_u32_le()?;

        Window::new_at(self, 4, frame_size.into())
            .map_err(|_| TakeInclusiveLengthPrefixedError::SizeTooSmall(frame_size))
    }

    /// Reads `size: u32` from `self` and returns a wrapper that can read at most `size` further
    /// bytes from `self`.
    fn take_exclusive_length_prefixed(mut self) -> io::Result<Take<Self>>
    where
        Self: Sized,
    {
        let size: u64 = self.read_u32_le()?.into();
        Ok(self.take(size))
    }

    /// Reads exactly `n` bytes into a `Vec`, and returns it.
    fn read_u8s(&mut self, n: usize) -> io::Result<Vec<u8>> {
        let mut bytes = vec![0_u8; n];
        self.read_exact(&mut bytes)?;

        Ok(bytes)
    }

    /// Reads `n_chars` bytes and returns the UTF-8 decoded result.
    fn read_u8_string(&mut self, n_chars: usize) -> Result<String, ReadStringError> {
        Ok(String::from_utf8(
            self.read_u8s(n_chars).map_err(ReadStringError::BytesIo)?,
        )?)
    }

    /// Reads `2 * n_chars` bytes and returns the UTF-16 decoded result.
    fn read_u16_string(&mut self, n_chars: usize) -> Result<String, ReadStringError> {
        Ok(String::from_utf16(
            &self.read_u16s(n_chars).map_err(ReadStringError::BytesIo)?,
        )?)
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

    /// Reads `n_chars: u32`, then `n_chars` bytes, and returns the UTF-8 decoded result.
    fn read_long_u8_string(&mut self) -> Result<String, ReadStringError> {
        let n_chars: usize = self
            .read_u32_le()
            .map_err(ReadStringError::SizeIo)?
            .try_into()
            .map_err(ReadStringError::SizeConversion)?;

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

    fn read_u8(&mut self) -> io::Result<u8> {
        ReadBytesExt::read_u8(self)
    }

    fn read_4_bytes(&mut self) -> io::Result<[u8; 4]> {
        self.read_u32_le().map(u32::to_le_bytes)
    }

    le_wrapper!(read_u16_le, u16, read_u16);
    le_wrapper!(read_u24_le, u32, read_u24);
    le_wrapper!(read_u32_le, u32, read_u32);
    le_wrapper!(read_u48_le, u64, read_u48);
    le_wrapper!(read_u64_le, u64, read_u64);

    le_wrapper!(read_i16_le, i16, read_i16);
    le_wrapper!(read_i24_le, i32, read_i24);
    le_wrapper!(read_i32_le, i32, read_i32);
    le_wrapper!(read_i48_le, i64, read_i48);
    le_wrapper!(read_i64_le, i64, read_i64);

    le_wrapper!(read_f32_le, f32, read_f32);
    le_wrapper!(read_f64_le, f64, read_f64);

    vec_read!(read_u16s, u16, read_u16_into);
    vec_read!(read_u32s, u32, read_u32_into);
    vec_read!(read_u64s, u64, read_u64_into);

    vec_read!(read_f32s, f32, read_f32_into);
    vec_read!(read_f64s, f64, read_f64_into);
}

impl<T: ReadBytesExt> ByteStreamLe for T {}

pub trait SeekableByteStreamLe: ByteStreamLe + Seek {}

impl<T: ByteStreamLe + Seek> SeekableByteStreamLe for T {}

#[derive(Error, Debug)]
#[error("{remaining} bytes remain after parsing")]
pub struct UnfinishedParsingError {
    remaining: u64,
}

pub trait ExactSizedStream {
    fn n_remaining(&self) -> u64;

    fn ensure_eof(&self) -> Result<(), UnfinishedParsingError> {
        match self.n_remaining() {
            0 => Ok(()),
            remaining => Err(UnfinishedParsingError { remaining }),
        }
    }
}

impl<T> ExactSizedStream for Take<T> {
    fn n_remaining(&self) -> u64 {
        self.limit()
    }
}

#[macro_export]
macro_rules! _read_size_and_vec_inner {
    ($sz_read_as_usize:expr, $idx:ident => $elem:expr) => {{
        let count: usize = $sz_read_as_usize;
        let mut v = Vec::with_capacity(count);

        for $idx in 0..count {
            v.push($elem);
        }

        v
    }};

    ($sz_read_as_usize:expr, $elem:expr) => {{
        let count: usize = $sz_read_as_usize;
        let mut v = Vec::with_capacity(count);

        for _ in 0..count {
            v.push($elem);
        }

        v
    }};
}

#[macro_export]
macro_rules! read_size_and_vec {
    ($stream:expr, u8, $($t:tt)+) => {
        $crate::_read_size_and_vec_inner!($stream.read_u8()?.into(), $($t)+)
    };

    ($stream:expr, u16, $($t:tt)+) => {
        $crate::_read_size_and_vec_inner!($stream.read_u16_le()?.into(), $($t)+)
    };

    ($stream:expr, u32, $usize_err:expr, $($t:tt)+) => {
        $crate::_read_size_and_vec_inner!({
            let count = $stream.read_u32_le()?;
            count.try_into().map_err(|_| $usize_err(count))?
        }, $($t)+)
    };
}

#[macro_export]
macro_rules! read_u16_sized_vec {
    ($stream:expr, $($t:tt)+) => {
        $crate::read_size_and_vec!($stream, u16, $($t)+)
    };
}

#[macro_export]
macro_rules! read_u32_sized_vec {
    ($stream:expr, $($t:tt)+) => {
        $crate::read_size_and_vec!($stream, u32, $($t)+)
    };
}

#[macro_export]
macro_rules! _read_size_and_map_inner {
    ($sz_read_as_usize:expr, $kv:expr) => {{
        let count: usize = $sz_read_as_usize;
        let mut m = std::collections::HashMap::with_capacity(count);

        for _ in 0..count {
            let (k, v) = $kv;
            m.insert(k, v);
        }

        m
    }};
}

// fixme: `r_s_a_m` doesn't complain if it finds duplicate keys.
// We shouldn't really need to check for them, but it would be more correct, and yet another
// guard against parser bugs...

#[macro_export]
macro_rules! read_size_and_map {
    ($stream:expr, u8, $kv:expr) => {
        $crate::_read_size_and_map_inner!($stream.read_u8()?.into(), $kv)
    };

    ($stream:expr, u16, $kv:expr) => {
        $crate::_read_size_and_map_inner!($stream.read_u16_le()?.into(), $kv)
    };

    ($stream:expr, u32, $usize_err:expr, $kv:expr) => {
        $crate::_read_size_and_map_inner!(
            {
                let count = $stream.read_u32_le()?;
                count.try_into().map_err(|_| $usize_err(count))?
            },
            $kv
        )
    };
}
