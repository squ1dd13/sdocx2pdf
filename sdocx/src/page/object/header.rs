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

/// Flag block structure.
///
/// A common pattern in the binary format is to prefix serialised data with three fields: a "flex
/// offset", a set of property flags, and a set of field flags. The property flags are stored as a
/// variable-length bitfield and interpreted as on/off settings. The field flags are encoded in the
/// same way, but specific bits are used to indicate the presence of specific "flex fields". Flex
/// fields, where present, are in the "flex area". The flex area is located at some logical
/// reference point plus the flex offset, and usually begins after any fixed fields (that is,
/// fields which are always present); these fixed fields typically come directly after the field
/// flags.
///
/// To fully deserialise an object encoded using a flag block, it is necessary to read the flex
/// offset, the property flags, the field flags, and the fixed data, before using the flex offset
/// to seek to the beginning of the flex area, from which point the field flags can be used to
/// parse the flex fields.
///
/// This structure keeps ownership of the flags and offset and provides utilities for working with
/// them.
pub struct FlagBlock {
    flex_offset: u32,
    property_flags: CheckedBitfield,
    field_flags: CheckedBitfield,
}

impl FlagBlock {
    /// Reads a flag block from `stream`.
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

    /// Provides access to the property flags. A mutable reference is returned so that the bitfield
    /// can keep track of which bits have been checked.
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

    /// Returns an error iff any property or field flags that are set have not been checked.
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

/// Reads an object header (binary size, data type, flag block) from `stream`. Returns the flag
/// block and a stream that can read up to the end of the object data, but no further.
///
/// Returns an error if the data type read does not match `expected_data_type`.
pub fn try_parse_object_header<R: Read>(
    stream: R,
    expected_data_type: u16,
) -> Result<(FlagBlock, BlindWindow<R>), ObjectHeaderError> {
    let mut stream = stream.inclusive_blind_window()?;

    let data_type = stream.read_u16_le()?;

    if data_type != expected_data_type {
        return Err(ObjectHeaderError::WrongDataType {
            is: data_type,
            not: expected_data_type,
        });
    }

    Ok((FlagBlock::try_parse(&mut stream)?, stream))
}
