use std::io::Read;

use thiserror::Error;

use crate::{
    byte_stream::{ByteStreamLe, ReadStringError, TryParse},
    read_size_and_vec,
};

#[derive(Debug)]
pub struct PageRef {
    uuid: String,

    #[expect(dead_code)]
    hash: [u8; 32],
}

impl PageRef {
    pub fn uuid(&self) -> &str {
        &self.uuid
    }
}

/// The structure in the `pageIdInfo.dat` file.
#[derive(Debug)]
pub struct PageList {
    pages: Vec<PageRef>,

    /// The hash of the associated `note.note` file.
    #[expect(dead_code)]
    note_hash: [u8; 32],
}

impl PageList {
    pub fn pages(&self) -> &[PageRef] {
        &self.pages
    }
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
