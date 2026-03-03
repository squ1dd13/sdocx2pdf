use std::io::{Read, Seek};

use thiserror::Error;

use crate::{
    bits::{CheckedBitfield, UnhandledBitsError},
    byte_stream::{BlindWindow, ByteStreamLe, ReadBitfieldError, TakeInclusiveLengthPrefixedError},
};

#[derive(Error, Debug)]
pub enum ObjectHeaderError {
    #[error(transparent)]
    Io(#[from] std::io::Error),

    #[error(transparent)]
    BadSize(#[from] TakeInclusiveLengthPrefixedError),

    #[error("expected data type {is}, not {not}")]
    WrongDataType { is: u16, not: u16 },

    #[error("failed to parse property flags")]
    PropertyFlags(#[source] ReadBitfieldError),

    #[error("failed to parse field flags")]
    FieldFlags(#[source] ReadBitfieldError),

    #[error("one or more property flags were not handled")]
    UnhandledProperty(#[source] UnhandledBitsError),

    #[error("one or more field flags were not handled")]
    UnhandledField(#[source] UnhandledBitsError),
}

pub struct ObjectHeader {
    flex_offset: u32,
    property_flags: CheckedBitfield,
    field_flags: CheckedBitfield,
}

impl ObjectHeader {
    pub fn try_parse<R: Read>(
        reader: R,
        expected_data_type: u16,
    ) -> Result<(ObjectHeader, BlindWindow<R>), ObjectHeaderError> {
        let mut reader: BlindWindow<_> = reader.take_inclusive_length_prefixed()?.into();

        let data_type = reader.read_u16_le()?;

        if data_type != expected_data_type {
            return Err(ObjectHeaderError::WrongDataType {
                is: data_type,
                not: expected_data_type,
            });
        }

        let flex_offset = reader.read_u32_le()?;

        let property_flags =
            CheckedBitfield::try_parse(&mut reader).map_err(ObjectHeaderError::PropertyFlags)?;

        let field_flags =
            CheckedBitfield::try_parse(&mut reader).map_err(ObjectHeaderError::FieldFlags)?;

        Ok((
            ObjectHeader {
                flex_offset,
                property_flags,
                field_flags,
            },
            reader,
        ))
    }

    pub const fn property_flags_mut(&mut self) -> &mut CheckedBitfield {
        &mut self.property_flags
    }

    pub fn init_flex<'me, R: Read + Seek>(
        &'me mut self,
        reader: &mut BlindWindow<R>,
    ) -> std::io::Result<&'me mut CheckedBitfield> {
        if self.flex_offset != 0 {
            reader.seek(std::io::SeekFrom::Start(self.flex_offset.into()))?;
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

    pub fn ensure_flags_used(self) -> Result<(), ObjectHeaderError> {
        self.property_flags
            .ensure_none_set_unchecked()
            .map_err(ObjectHeaderError::UnhandledProperty)?;

        self.field_flags
            .ensure_none_set_unchecked()
            .map_err(ObjectHeaderError::UnhandledField)?;

        Ok(())
    }
}
