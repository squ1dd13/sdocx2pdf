use std::{
    cmp::Ordering,
    io::{Read, Seek},
};

use thiserror::Error;

use crate::{
    bits::{CheckedBitfield, UnhandledBitsError},
    byte_stream::{BlindWindow, ByteStreamLe, ReadBitfieldError, TakeInclusiveLengthPrefixedError},
};

#[derive(Error, Debug)]
pub enum FlagBlockError {
    #[error(transparent)]
    Io(#[from] std::io::Error),

    #[error("failed to parse property flags")]
    PropertyFlags(#[source] ReadBitfieldError),

    #[error("failed to parse field flags")]
    FieldFlags(#[source] ReadBitfieldError),

    #[error("one or more property flags were not handled")]
    UnhandledProperty(#[source] UnhandledBitsError),

    #[error("one or more field flags were not handled")]
    UnhandledField(#[source] UnhandledBitsError),
}

pub struct FlagBlock {
    flex_offset: u32,
    property_flags: CheckedBitfield,
    field_flags: CheckedBitfield,
}

impl FlagBlock {
    pub fn try_parse<R: Read>(mut stream: R) -> Result<FlagBlock, FlagBlockError> {
        let flex_offset = stream.read_u32_le()?;

        let property_flags =
            CheckedBitfield::try_parse(&mut stream).map_err(FlagBlockError::PropertyFlags)?;

        let field_flags =
            CheckedBitfield::try_parse(&mut stream).map_err(FlagBlockError::FieldFlags)?;

        Ok(FlagBlock {
            flex_offset,
            property_flags,
            field_flags,
        })
    }

    pub const fn property_flags_mut(&mut self) -> &mut CheckedBitfield {
        &mut self.property_flags
    }

    /// Seeks to the flex offset and returns a mutable reference to the field flags so the fields
    /// can be read immediately using the flags.
    ///
    /// `reader` must have position 0 at the point the flex offset is relative to.
    pub fn init_flex<'me, R: Read + Seek>(
        &'me mut self,
        reader: &mut BlindWindow<R>,
    ) -> std::io::Result<&'me mut CheckedBitfield> {
        if self.flex_offset != 0 {
            let flex_offset: u64 = self.flex_offset.into();
            let here: u64 = reader.stream_position()?;

            match flex_offset.cmp(&here) {
                // todo: Error
                // This is a pretty big issue.
                Ordering::Less => eprintln!(
                    "Warning: Flex offset ({}) is **behind** here ({}) by {} byte(s)!",
                    flex_offset,
                    here,
                    here - flex_offset
                ),

                Ordering::Equal => (),

                Ordering::Greater => eprintln!(
                    "Warning: Flex offset ({}) is ahead of here ({}) by {} byte(s)",
                    flex_offset,
                    here,
                    flex_offset - here
                ),
            }

            reader.seek(std::io::SeekFrom::Start(flex_offset))?;
        } else {
            if self.field_flags.any_set() {
                eprintln!(
                    "Warning: Ignoring field flags {:?} because flex offset is zero",
                    self.field_flags
                );
            }

            self.field_flags.clear();
        }

        Ok(&mut self.field_flags)
    }

    pub fn ensure_flags_used(self) -> Result<(), FlagBlockError> {
        self.property_flags
            .ensure_none_set_unchecked()
            .map_err(FlagBlockError::UnhandledProperty)?;

        self.field_flags
            .ensure_none_set_unchecked()
            .map_err(FlagBlockError::UnhandledField)?;

        Ok(())
    }
}

#[derive(Error, Debug)]
#[error(transparent)]
pub enum ObjectHeaderError {
    Io(#[from] std::io::Error),
    BadSize(#[from] TakeInclusiveLengthPrefixedError),
    FlagBlock(#[from] FlagBlockError),

    #[error("expected data type {is}, not {not}")]
    WrongDataType {
        is: u16,
        not: u16,
    },
}

pub fn try_parse_object_header<R: Read>(
    stream: R,
    expected_data_type: u16,
) -> Result<(FlagBlock, BlindWindow<R>), ObjectHeaderError> {
    let mut stream: BlindWindow<_> = stream.take_inclusive_length_prefixed()?.into();

    let data_type = stream.read_u16_le()?;

    if data_type != expected_data_type {
        return Err(ObjectHeaderError::WrongDataType {
            is: data_type,
            not: expected_data_type,
        });
    }

    Ok((FlagBlock::try_parse(&mut stream)?, stream))
}

// Wrapper for `FlagBlock`, which used to be `ObjectHeader`.
// todo: Remove uses of `ObjectHeader` and just use `FlagBlock`.
pub struct ObjectHeader(FlagBlock);

impl ObjectHeader {
    pub fn try_parse<R: Read>(
        stream: R,
        expected_data_type: u16,
    ) -> Result<(ObjectHeader, BlindWindow<R>), ObjectHeaderError> {
        try_parse_object_header(stream, expected_data_type).map(|(fb, bw)| (ObjectHeader(fb), bw))
    }

    pub const fn property_flags_mut(&mut self) -> &mut CheckedBitfield {
        self.0.property_flags_mut()
    }

    pub fn init_flex<'me, R: Read + Seek>(
        &'me mut self,
        reader: &mut BlindWindow<R>,
    ) -> std::io::Result<&'me mut CheckedBitfield> {
        self.0.init_flex(reader)
    }

    pub fn ensure_flags_used(self) -> Result<(), ObjectHeaderError> {
        self.0.ensure_flags_used().map_err(Into::into)
    }
}
