use std::{
    borrow::Cow,
    fs::File,
    io::{BufReader, Read},
    path::Path,
};

use thiserror::Error;
use zip::{HasZipMetadata, ZipArchive};

use crate::{
    byte_stream::TryParse,
    context::{DocumentContext, TryParseWithContext},
    end_tag::{EndTag, EndTagParseError, PageModel},
    media_info::{FileRegistry, FileRegistryParseError},
    note_doc::{NoteDoc, NoteDocParseError},
    page::{Page, PageParseError, object::text::Text},
    page_list::{PageList, PageListParseError, PageRef},
};

#[derive(Error, Debug)]
#[error(transparent)]
pub enum DocumentError {
    Io(#[from] std::io::Error),
    Zip(#[from] zip::result::ZipError),

    FileRegistry(#[from] FileRegistryParseError),
    NoteDoc(#[from] NoteDocParseError),
    PageList(#[from] PageListParseError),
    Page(#[from] PageParseError),
    EndTag(#[from] EndTagParseError),

    #[error("'{0}' is not in the zip archive")]
    MissingArchiveEntry(Cow<'static, str>),

    #[error("hash in page list does not match note file")]
    PageListHashMismatch,

    #[error("incorrect hash for page '{0}'")]
    PageHashMismatch(String),

    #[error("uncompressed size {0} is too large")]
    FileTooBig(u64),
}

#[derive(Debug)]
pub struct Document {
    note: NoteDoc,
    pages: Vec<Page>,
    end_tag: EndTag,
}

macro_rules! entry_cursor {
    ($archive:expr, $name:expr $(,)?) => {
        // Closure drops the name and wraps the reader so it can be used outside.
        entry_cursor!($archive, $name, |_name, rdr| Ok(rdr))
    };

    ($archive:expr, $name:expr, $and_then:expr $(,)?) => {
        match $name {
            name => match $archive.by_name(&name) {
                Ok(mut reader) => {
                    let n = reader.get_metadata().uncompressed_size;

                    // Try to convert u64 -> usize.
                    usize::try_from(n)
                        .map_err(|_| DocumentError::FileTooBig(n))
                        // Allocate enouch space for the uncompressed data.
                        .map(Vec::with_capacity)
                        .and_then(|mut buf| {
                            // Decompress the file into the buffer and wrap it in a cursor.
                            reader
                                .read_to_end(&mut buf)
                                .map_err(From::from)
                                .map(|_| std::io::Cursor::new(buf))
                        })
                        // Do something with the file name (which we have not consumed) and data.
                        .and_then(|rdr| $and_then(name, rdr))
                }

                // Create a more informative error if the entry doesn't exist.
                Err(zip::result::ZipError::FileNotFound) => {
                    Err(DocumentError::MissingArchiveEntry(name.into()))
                }

                Err(e) => Err(DocumentError::from(e)),
            },
        }
    };
}

impl Document {
    /// Constructs a `Document` from a zipped note file (typically `.sdocx`) at `path`.
    pub fn from_zip(path: impl AsRef<Path>) -> Result<Document, DocumentError> {
        let mut archive = ZipArchive::new(BufReader::new(File::open(path)?))?;

        let file_registry =
            FileRegistry::try_parse(&mut entry_cursor!(archive, "media/mediaInfo.dat")?)?;

        let note =
            NoteDoc::try_parse_with_ctx(&mut entry_cursor!(archive, "note.note")?, &file_registry)?;

        let PageList { pages, note_hash } =
            PageList::try_parse(&mut entry_cursor!(archive, "pageIdInfo.dat")?)?;

        let page_ctx = DocumentContext {
            file_registry: &file_registry,
            string_registry: note.string_registry(),
        };

        if &note_hash != note.hash() {
            return Err(DocumentError::PageListHashMismatch);
        }

        // Parse the file corresponding to each page reference.
        let pages = pages
            .into_iter()
            .map(|PageRef { uuid, hash }| {
                let mut file_name = uuid;
                file_name.push_str(".page");

                entry_cursor!(archive, file_name, |name, mut reader| {
                    Page::try_parse_with_ctx(&mut reader, &page_ctx)
                        .map_err(From::from)
                        .and_then(|p| {
                            if &hash != p.hash() {
                                Err(DocumentError::PageHashMismatch(name))
                            } else {
                                Ok(p)
                            }
                        })
                })
            })
            .collect::<Result<Vec<_>, _>>()?;

        let end_tag = EndTag::try_parse_with_ctx(
            &mut entry_cursor!(archive, "end_tag.bin")?,
            &crate::end_tag::NoteSdkType::SPen,
        )?;

        Ok(Document {
            note,
            pages,
            end_tag,
        })
    }

    /// Constructs a `Document` from the contents of the directory at `path`.
    ///
    /// The directory could be an extracted `.sdocx`, or possibly an unexported note from the
    /// Windows app.
    pub fn from_dir(path: impl AsRef<Path>) -> Result<Document, DocumentError> {
        let path: &Path = path.as_ref();

        let file_registry =
            FileRegistry::try_parse(&mut File::open(path.join("media/mediaInfo.dat"))?)?;

        let note =
            NoteDoc::try_parse_with_ctx(&mut File::open(path.join("note.note"))?, &file_registry)?;

        let PageList { pages, note_hash } =
            PageList::try_parse(&mut File::open(path.join("pageIdInfo.dat"))?)?;

        let page_ctx = DocumentContext {
            file_registry: &file_registry,
            string_registry: note.string_registry(),
        };

        if &note_hash != note.hash() {
            return Err(DocumentError::PageListHashMismatch);
        }

        // Parse the file corresponding to each page reference.
        let pages = pages
            .into_iter()
            .map(|PageRef { uuid, hash }| {
                let mut file_name = path.join(uuid);
                file_name.add_extension("page");

                // Use a `BufReader` for pages because they can get very large (several MB), and
                // are parsed using loads of tiny reads.
                Page::try_parse_with_ctx(&mut BufReader::new(File::open(&file_name)?), &page_ctx)
                    .map_err(From::from)
                    .and_then(|p| {
                        if &hash != p.hash() {
                            Err(DocumentError::PageHashMismatch(
                                file_name.display().to_string(),
                            ))
                        } else {
                            Ok(p)
                        }
                    })
            })
            .collect::<Result<Vec<_>, _>>()?;

        let end_tag = EndTag::try_parse_with_ctx(
            &mut File::open(path.join("end_tag.bin"))?,
            &crate::end_tag::NoteSdkType::SPen,
        )?;

        Ok(Document {
            note,
            pages,
            end_tag,
        })
    }

    pub fn pages(&self) -> &[Page] {
        &self.pages
    }

    pub const fn page_model(&self) -> PageModel {
        self.end_tag.page_model
    }

    /// Returns the width and height of the document.
    ///
    /// # Panics
    /// Panics if the different components of the document disagree on the values.
    pub fn width_height(&self) -> (u32, u32) {
        // Width is always stored as an integer, so we can check for exact equality.
        assert_eq!(self.end_tag.note_width, self.note.width);

        // Height is stored in the end tag as `f32` and in the note doc as `u32`. Assert that they
        // agree up to floor/ceil/round.
        let h_diff = (f64::from(self.end_tag.note_height) - f64::from(self.note.height)).abs();
        assert!(h_diff < 1.0);

        (self.note.width, self.note.height)
    }

    pub const fn title_text(&self) -> &Text {
        &self.note.title_text
    }

    pub const fn body_text(&self) -> &Text {
        &self.note.body_text
    }
}
