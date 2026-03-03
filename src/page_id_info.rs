use crate::byte_stream::ByteStreamLe;
use color_eyre::Result;

#[derive(Debug)]
pub struct PageIdInfoPage {
    pub page_id: String,
    #[expect(dead_code)]
    hash: [u8; 32],
}

#[derive(Debug)]
pub struct PageIdInfo {
    /// The SHA256 digest from the associated `note.note` file.
    #[expect(dead_code)]
    note_doc_sha256: [u8; 32],
    pub pages: Vec<PageIdInfoPage>,
}

impl PageIdInfo {
    pub fn try_parse<T: ByteStreamLe>(stream: &mut T) -> Result<PageIdInfo> {
        let mut note_doc_sha256 = [0_u8; 32];
        stream.read_exact(&mut note_doc_sha256)?;

        let page_count = stream.read_u16_le()?;

        let mut pages = Vec::with_capacity(page_count.into());

        for _ in 0..page_count {
            pages.push(PageIdInfoPage {
                page_id: stream.read_short_u16_string()?,
                hash: {
                    let mut buf = [0_u8; 32];
                    stream.read_exact(&mut buf)?;
                    buf
                },
            });
        }

        Ok(PageIdInfo {
            note_doc_sha256,
            pages,
        })
    }
}
