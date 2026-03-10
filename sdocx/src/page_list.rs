use std::io::Read;

use thiserror::Error;

use crate::{
    byte_stream::{ByteStreamLe, ReadStringError, TryParse},
    read_size_and_vec,
};

#[derive(Debug)]
pub struct PageRef {
    pub uuid: String,
    pub hash: [u8; 32],
}

/// The structure in the `pageIdInfo.dat` file.
#[derive(Debug)]
pub struct PageList {
    pub pages: Vec<PageRef>,

    /// The hash of the associated `note.note` file.
    pub note_hash: [u8; 32],
}

#[derive(Error, Debug)]
#[error(transparent)]
pub enum PageListParseError {
    Io(#[from] std::io::Error),
    Uuid(#[from] ReadStringError),
}

impl<R: Read> TryParse<R> for PageList {
    type ParseError = PageListParseError;

    fn try_parse(reader: &mut R) -> Result<PageList, PageListParseError> {
        Ok(PageList {
            note_hash: {
                let mut b = [0_u8; 32];
                reader.read_exact(&mut b)?;
                b
            },
            pages: read_size_and_vec!(
                reader,
                u16,
                PageRef {
                    uuid: reader.read_short_u16_string()?,
                    hash: {
                        let mut b = [0_u8; 32];
                        reader.read_exact(&mut b)?;
                        b
                    },
                }
            ),
        })
    }
}
