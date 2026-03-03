use crate::byte_stream::ByteStreamLe;
use chrono::{DateTime, Utc};
use color_eyre::Result;
use std::io::{Seek, SeekFrom};

#[derive(Debug)]
#[expect(dead_code)]
struct BoundFile {
    bind_id: u32,
    name: String,
    hash: String,
    ref_count: u16,
    ref_count_modified_time: DateTime<Utc>,
    is_attached: bool,
}

#[derive(Debug)]
#[expect(dead_code)]
pub struct MediaInfo {
    format_version: u32,
    bound_files: Vec<BoundFile>,
}

impl MediaInfo {
    /// Based on `SPen::MediaFileManagerNew::Load` in `libSpen_document.dll`.
    pub fn try_parse<T: ByteStreamLe + Seek>(stream: &mut T) -> Result<MediaInfo> {
        Ok(MediaInfo {
            format_version: stream.read_u32_le()?,
            bound_files: {
                let bound_file_count = stream.read_u16_le()?;
                let mut bound_files = Vec::with_capacity(bound_file_count.into());

                for _ in 0..bound_file_count {
                    let data_size: u64 = stream.read_u32_le()?.into();
                    let stream_pos_pre = stream.stream_position()?;
                    let expected_data_end = stream_pos_pre + data_size;

                    let id = stream.read_u32_le()?;
                    let filename = stream.read_short_u16_string()?;

                    let file_hash = stream.read_u8_string(64)?;

                    let ref_count = stream.read_u16_le()?;
                    let ref_count_modified_time = stream.read_timestamp()?;

                    let is_file_attached = stream.read_u8()? != 0;

                    let stream_pos_post = stream.stream_position()?;

                    if stream_pos_post != expected_data_end {
                        let actual_size = stream_pos_post - stream_pos_pre;

                        eprintln!(
                            "mismatch: declared size is {data_size}, but actual size is {actual_size}"
                        );

                        stream.seek(SeekFrom::Start(expected_data_end))?;
                    }

                    bound_files.push(BoundFile {
                        bind_id: id,
                        name: filename,
                        hash: file_hash,
                        ref_count,
                        ref_count_modified_time,
                        is_attached: is_file_attached,
                    });
                }

                bound_files
            },
        })
    }
}
