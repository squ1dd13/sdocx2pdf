use byteorder::{LittleEndian, ReadBytesExt};
use color_eyre::eyre::OptionExt;
use std::io::Seek;

// todo: ("model") EndTag, as in SPen::EndTag::ParseImpl in _document.dll
// That specific end tag is the one in end_tag.bin.

#[derive(Debug)]
struct BoundFile {
    bind_id: u32,
    name: String,
    hash: String,
    ref_count: u16,
    ref_count_modified_time: chrono::DateTime<chrono::Utc>,
    is_attached: bool,
}

#[derive(Debug)]
struct MediaInfo {
    format_version: u32,
    bound_files: Vec<BoundFile>,
}

impl MediaInfo {
    fn try_parse<T: ReadBytesExt + Seek>(stream: &mut T) -> color_eyre::Result<MediaInfo> {
        Ok(MediaInfo {
            format_version: stream.read_u32::<LittleEndian>()?,
            bound_files: {
                let bound_file_count = stream.read_u16::<LittleEndian>()?;
                let mut bound_files = Vec::with_capacity(bound_file_count.into());

                for _ in 0..bound_file_count {
                    let data_size = stream.read_u32::<LittleEndian>()?;
                    let stream_pos_pre = stream.stream_position()?;
                    let expected_data_end = stream_pos_pre + data_size as u64;

                    let id = stream.read_u32::<LittleEndian>()?;
                    let filename_length: usize = stream.read_u16::<LittleEndian>()?.into();

                    // `filename_length` counts the characters, so *2 to get the number of bytes.
                    let mut filename_bytes = vec![0; 2 * filename_length];
                    stream.read_exact(&mut filename_bytes)?;

                    // Convert u8 -> u16.
                    let filename_u16s = (0..filename_length).map(|char_i| {
                        u16::from_le_bytes([
                            filename_bytes[char_i * 2],
                            filename_bytes[char_i * 2 + 1],
                        ])
                    });

                    let filename =
                        char::decode_utf16(filename_u16s).collect::<Result<String, _>>()?;

                    // This is always 64 bytes, but we need a `Vec` anyway because we ultimately
                    // want a `String`.
                    let mut file_hash_bytes = vec![0_u8; 64];
                    stream.read_exact(&mut file_hash_bytes)?;

                    let file_hash = String::from_utf8(file_hash_bytes)?;

                    let ref_count = stream.read_u16::<LittleEndian>()?;
                    let ref_count_modified_timestamp = stream.read_i64::<LittleEndian>()?;
                    let ref_count_modified_time =
                        chrono::DateTime::from_timestamp_micros(ref_count_modified_timestamp)
                            .ok_or_eyre("invalid timestamp")?;

                    let is_file_attached = stream.read_u8()? != 0;

                    let stream_pos_post = stream.stream_position()?;

                    if stream_pos_post != expected_data_end {
                        let actual_size = stream_pos_post - stream_pos_pre;

                        eprintln!(
                            "mismatch: declared size is {data_size}, but actual size is {actual_size}"
                        );

                        stream.seek(std::io::SeekFrom::Start(expected_data_end))?;
                    }

                    bound_files.push(BoundFile {
                        bind_id: id,
                        name: filename,
                        hash: file_hash,
                        ref_count,
                        ref_count_modified_time,
                        is_attached: is_file_attached,
                    })
                }

                bound_files
            },
        })
    }
}

fn main() -> color_eyre::Result<()> {
    let media_info_paths = [
        "/home/alex/projects/re/sdocx/sample_docs/Section2lectures-2_260218_125010/media/mediaInfo.dat",
        "/home/alex/projects/re/sdocx/sample_docs/Single drawn line fp17, inf scroll_260218_145754/media/mediaInfo.dat",
        "/home/alex/projects/re/sdocx/sample_docs/Has background colour, pattern cover, dots_260218_181735/media/mediaInfo.dat",
        "/home/alex/projects/re/sdocx/sample_docs/Empty, inf scroll_260218_145632/media/mediaInfo.dat",
    ];

    for path in media_info_paths {
        let mut media_info = std::fs::File::open(path)?;

        let info = MediaInfo::try_parse(&mut media_info)?;

        println!("{path}: {info:#?}");
    }

    Ok(())
}
