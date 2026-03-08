//! The file registry found in `media/mediaInfo.dat`.

use crate::{
    byte_stream::{ByteStreamLe, ReadStringError, ReadTimestampError, TryParse},
    read_size_and_map,
};
use std::{
    collections::HashMap,
    io::{Read, Seek, SeekFrom},
    rc::Rc,
};
use thiserror::Error;

#[derive(Debug)]
#[expect(dead_code)]
pub struct BoundFile {
    name: String,
    hash: String,

    /// Only given in newer versions.
    is_attached: Option<bool>,
}

#[expect(dead_code)]
impl BoundFile {
    pub fn name(&self) -> &str {
        &self.name
    }
}

#[derive(Error, Debug)]
#[error("there is no file registered with id {0}")]
pub struct NoSuchRegisteredFileError(u32);

#[derive(Default, Debug)]
pub struct FileRegistry {
    /// Keys are bind IDs.
    files: HashMap<u32, Rc<BoundFile>>,
}

impl FileRegistry {
    pub fn get(&self, key: u32) -> Option<Rc<BoundFile>> {
        self.files.get(&key).map(Rc::clone)
    }

    pub fn try_get(&self, key: u32) -> Result<Rc<BoundFile>, NoSuchRegisteredFileError> {
        self.get(key).ok_or(NoSuchRegisteredFileError(key))
    }
}

#[derive(Error, Debug)]
#[error(transparent)]
pub enum FileRegistryParseError {
    Io(#[from] std::io::Error),
    String(#[from] ReadStringError),
    Timestamp(#[from] ReadTimestampError),

    #[error("end string '{0}' is not 'EOF' or 'EOFX'")]
    BadEofStr(String),

    #[error("did not read all entry bytes")]
    UnfinishedEntry,
}

impl<R: Read + Seek> TryParse<R> for FileRegistry {
    type ParseError = FileRegistryParseError;

    fn try_parse(reader: &mut R) -> Result<FileRegistry, FileRegistryParseError> {
        // If the media info file ends with "EOFX", then the `.note` format is > 3001, and the
        // media info starts with that format value. For older note formats, this file ends with
        // "EOF", and does not start with the exact note format version.
        let last_four = {
            // Read the last four bytes in the stream, then seek back to the start.
            let start = reader.stream_position()?;
            reader.seek(SeekFrom::End(-4))?;
            let b = reader.read_4_bytes()?;
            reader.seek(SeekFrom::Start(start))?;
            b
        };

        let is_newer_format = match &last_four {
            b"EOFX" => {
                let _format_version = reader.read_u32_le()?;
                true
            }

            // Other option is only three bytes at the end, so ignore the first of the four.
            [.., b'E', b'O', b'F'] => false,

            bad => {
                return Err(FileRegistryParseError::BadEofStr(
                    String::from_utf8_lossy(bad).into(),
                ));
            }
        };

        Ok(FileRegistry {
            files: read_size_and_map!(reader, u16, {
                let mut reader: &mut dyn Read = if is_newer_format {
                    // For newer formats, the size of each entry is given at the start.
                    &mut reader.read_u32_le().map(|v| reader.take(v.into()))?
                } else {
                    reader
                };

                let bind_id = reader.read_u32_le()?;
                let name = reader.read_short_u16_string()?;
                let hash = reader.read_u8_string(64)?;

                let _ref_count = reader.read_u16_le()?;
                let _modified_time = reader.read_timestamp()?;

                let is_attached = if is_newer_format {
                    let is_attached = reader.read_u8()? != 0;

                    // Entry ends here, and as this is the newer format, `reader` is limited.
                    // We should have exhausted it:
                    if reader.read_u8().is_ok() {
                        return Err(FileRegistryParseError::UnfinishedEntry);
                    }

                    Some(is_attached)
                } else {
                    None
                };

                (
                    bind_id,
                    Rc::new(BoundFile {
                        name,
                        hash,
                        is_attached,
                    }),
                )
            }),
        })
    }
}
