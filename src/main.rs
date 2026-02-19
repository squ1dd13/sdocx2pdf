use byteorder::{LittleEndian, ReadBytesExt};
use chrono::{DateTime, Utc};
use color_eyre::eyre::{OptionExt, eyre};
use std::io::Seek;

fn read_u8_buf(stream: &mut impl ReadBytesExt, n: usize) -> color_eyre::Result<Vec<u8>> {
    let mut bytes = vec![0u8; n];
    stream.read_exact(&mut bytes)?;

    Ok(bytes)
}

fn read_u8_string(stream: &mut impl ReadBytesExt, n_chars: usize) -> color_eyre::Result<String> {
    String::from_utf8(read_u8_buf(stream, n_chars)?).map_err(From::from)
}

fn read_u16_string(stream: &mut impl ReadBytesExt, n_chars: usize) -> color_eyre::Result<String> {
    let bytes = read_u8_buf(stream, 2 * n_chars)?;

    char::decode_utf16((0..n_chars).map(|i| u16::from_le_bytes([bytes[2 * i], bytes[2 * i + 1]])))
        .collect::<color_eyre::Result<String, _>>()
        .map_err(From::from)
}

fn read_short_u16_string(stream: &mut impl ReadBytesExt) -> color_eyre::Result<String> {
    let n_chars: usize = stream.read_u16::<LittleEndian>()?.into();
    read_u16_string(stream, n_chars)
}

fn read_long_u16_string(stream: &mut impl ReadBytesExt) -> color_eyre::Result<String> {
    let n_chars: usize = stream.read_u32::<LittleEndian>()?.try_into()?;
    read_u16_string(stream, n_chars)
}

fn read_timestamp(stream: &mut impl ReadBytesExt) -> color_eyre::Result<DateTime<Utc>> {
    DateTime::from_timestamp_micros(stream.read_i64::<LittleEndian>()?)
        .ok_or_eyre("invalid timestamp")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NoteSdkType {
    /// `SAMSUNG S-Pen PAINTING SDK`; 0x1 in DLL
    SamsungSPenPainting,

    /// `S-Pen SDK`; 0x2 in DLL
    SPen,

    /// `S-Pen PAINTING SDK`; 0x3 in DLL
    SPenPainting,

    /// `SAMSUNG S-Pen SDK`; 0x4 in DLL
    SamsungSPen,
}

impl NoteSdkType {
    fn ident(self) -> &'static str {
        match self {
            NoteSdkType::SamsungSPenPainting => "Document for SAMSUNG S-Pen PAINTING SDK",
            NoteSdkType::SPen => "Document for S-Pen SDK",
            NoteSdkType::SPenPainting => "Document for S-Pen PAINTING SDK",
            NoteSdkType::SamsungSPen => "Document for SAMSUNG S-Pen SDK",
        }
    }

    /// Checks whether the next `block_size` bytes in `stream` ends with an ident string
    /// that matches `self`.
    ///
    /// The returned `bool` is `true` if the ident is successfully read and matches `self`,
    /// and `false` if the read was successful but the ident doesn't match, or if `block_size`
    /// is smaller than the length of the expected ident. If the read is successful,
    /// the string constructed is returned as well.
    ///
    /// If `Ok` is returned, `stream` will be at the same position as it was when this method was
    /// called.
    fn verify_block_end<T: ReadBytesExt + Seek>(
        self,
        stream: &mut T,
        block_size: usize,
    ) -> color_eyre::Result<(bool, Option<String>)> {
        let ident = self.ident();

        let Some(ident_offset) = block_size.checked_sub(ident.len()) else {
            // The block cannot contain the ident, because it isn't big enough.
            return Ok((false, None));
        };

        // Seek to where the start of the ident should be.
        stream.seek_relative(ident_offset.try_into()?)?;

        let maybe_ident = read_u8_string(stream, ident.len())?;

        // Return to the start of the block.
        stream.seek_relative(-i64::try_from(block_size)?)?;

        Ok((maybe_ident == ident, Some(maybe_ident)))
    }
}

#[derive(Debug)]
struct EncryptionInfo {
    encryption_size: u32,
    encryption_salt: Vec<u8>,
    encryption_iv: Vec<u8>,
    encryption_key: Vec<u8>,
}

impl EncryptionInfo {
    fn try_parse<T: ReadBytesExt + Seek>(stream: &mut T) -> color_eyre::Result<EncryptionInfo> {
        Ok(EncryptionInfo {
            encryption_size: stream.read_u32::<LittleEndian>()?,

            encryption_salt: {
                let salt_size: usize = stream.read_u32::<LittleEndian>()?.try_into()?;
                read_u8_buf(stream, salt_size)?
            },

            encryption_iv: {
                let iv_size: usize = stream.read_u32::<LittleEndian>()?.try_into()?;
                read_u8_buf(stream, iv_size)?
            },

            encryption_key: {
                let key_size: usize = stream.read_u32::<LittleEndian>()?.try_into()?;
                read_u8_buf(stream, key_size)?
            },
        })
    }
}

/// The structure in `end_tag.bin`.
#[derive(Debug)]
struct ModelEndTag {
    sdk_type: NoteSdkType,

    format_version: u32,
    note_id: String,
    last_modified_time: DateTime<Utc>,
    property_flags: u32,
    cover_image: String,
    note_width: u32,
    note_height: f32,
    app_name: String,
    app_version_major: u32,
    app_version_minor: u32,
    app_patch_name: String,
    min_format_version: u32,
    created_time: DateTime<Utc>,
    last_viewed_page_index: u32,
    page_model: u16,
    document_type: u16,
    owner_id: String,
    encryption_info: Option<EncryptionInfo>,
    display_created_time: DateTime<Utc>,
    display_modified_time: DateTime<Utc>,
    last_recognised_data_modified_time: DateTime<Utc>,
    fixed_font: String,
    fixed_text_direction: u32,
    fixed_background_theme: u32,
    server_check_point: i64,
    new_orientation: u32,
    min_unknown_version: u32,
    app_custom_data: String,
}

impl ModelEndTag {
    fn try_parse<T: ReadBytesExt + Seek>(
        stream: &mut T,
        sdk_type: NoteSdkType,
    ) -> color_eyre::Result<ModelEndTag> {
        let tag_size: usize = stream.read_u16::<LittleEndian>()?.into();

        // Make sure the tag specifies the SDK type we are expecting.
        let (ident_matches, ident_found) = sdk_type.verify_block_end(stream, tag_size)?;

        let expected_ident = sdk_type.ident();

        if !ident_matches {
            return Err(match ident_found {
                Some(found) => eyre!("ident '{found}' does not match expected '{expected_ident}'"),
                None => eyre!("not enough space for ident '{expected_ident}'"),
            });
        }

        let format_version = stream.read_u32::<LittleEndian>()?;

        let note_id = read_short_u16_string(stream)?;
        let last_modified_time = read_timestamp(stream)?;
        let property_flags = stream.read_u32::<LittleEndian>()?;
        let cover_image = read_short_u16_string(stream)?;

        let note_width = stream.read_u32::<LittleEndian>()?;
        let note_height = stream.read_f32::<LittleEndian>()?;

        let app_name = read_short_u16_string(stream)?;
        let app_version_major = stream.read_u32::<LittleEndian>()?;
        let app_version_minor = stream.read_u32::<LittleEndian>()?;
        let app_patch_name = read_short_u16_string(stream)?;

        let min_format_version = stream.read_u32::<LittleEndian>()?;

        let created_time = read_timestamp(stream)?;
        let last_viewed_page_index = stream.read_u32::<LittleEndian>()?;

        let page_model = stream.read_u16::<LittleEndian>()?;
        let document_type = stream.read_u16::<LittleEndian>()?;

        let owner_id = read_short_u16_string(stream)?;

        let n_to_skip: i64 = stream.read_u32::<LittleEndian>()?.into();
        stream.seek_relative(n_to_skip)?;

        let encryption_data_size = stream.read_u32::<LittleEndian>()?;

        let encryption_info = if encryption_data_size == 0 {
            None
        } else {
            Some(EncryptionInfo::try_parse(stream)?)
        };

        let display_created_time = read_timestamp(stream)?;
        let display_modified_time = read_timestamp(stream)?;
        let last_recognised_data_modified_time = read_timestamp(stream)?;

        let fixed_font = read_short_u16_string(stream)?;
        let fixed_text_direction = stream.read_u32::<LittleEndian>()?;
        let fixed_background_theme = stream.read_u32::<LittleEndian>()?;

        let server_check_point = stream.read_i64::<LittleEndian>()?;

        let new_orientation = stream.read_u32::<LittleEndian>()?;
        let min_unknown_version = stream.read_u32::<LittleEndian>()?;

        let app_custom_data = read_long_u16_string(stream)?;

        // We know that the real ident and expected ident match, so to seek past the
        // real ident we can just skip the size of the expected ident.
        stream.seek_relative(expected_ident.len().try_into()?)?;

        Ok(ModelEndTag {
            sdk_type,
            format_version,
            note_id,
            last_modified_time,
            property_flags,
            cover_image,
            note_width,
            note_height,
            app_name,
            app_version_major,
            app_version_minor,
            app_patch_name,
            min_format_version,
            created_time,
            last_viewed_page_index,
            page_model,
            document_type,
            owner_id,
            encryption_info,
            display_created_time,
            display_modified_time,
            last_recognised_data_modified_time,
            fixed_font,
            fixed_text_direction,
            fixed_background_theme,
            server_check_point,
            new_orientation,
            min_unknown_version,
            app_custom_data,
        })
    }
}

#[derive(Debug)]
struct BoundFile {
    bind_id: u32,
    name: String,
    hash: String,
    ref_count: u16,
    ref_count_modified_time: DateTime<Utc>,
    is_attached: bool,
}

#[derive(Debug)]
struct MediaInfo {
    format_version: u32,
    bound_files: Vec<BoundFile>,
}

impl MediaInfo {
    /// Based on `SPen::MediaFileManagerNew::Load` in `libSpen_document.dll`.
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
                    let filename = read_short_u16_string(stream)?;

                    let file_hash = read_u8_string(stream, 64)?;

                    let ref_count = stream.read_u16::<LittleEndian>()?;
                    let ref_count_modified_time = read_timestamp(stream)?;

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

fn demo_media_info() -> color_eyre::Result<()> {
    let media_info_paths = [
        "/home/alex/projects/re/sdocx/sample_docs/Section2lectures-2_260218_125010/media/mediaInfo.dat",
        "/home/alex/projects/re/sdocx/sample_docs/Single drawn line fp17, inf scroll_260218_145754/media/mediaInfo.dat",
        "/home/alex/projects/re/sdocx/sample_docs/Has background colour, pattern cover, dots_260218_181735/media/mediaInfo.dat",
        "/home/alex/projects/re/sdocx/sample_docs/Empty, inf scroll_260218_145632/media/mediaInfo.dat",
        "/home/alex/projects/re/sdocx/sample_docs/empty encrypted_260219_125722/media/mediaInfo.dat",
    ];

    for path in media_info_paths {
        let mut media_info = std::fs::File::open(path)?;

        let info = MediaInfo::try_parse(&mut media_info)?;

        println!("{path}: {info:#?}");
    }

    Ok(())
}

fn demo_end_tag() -> color_eyre::Result<()> {
    let end_tag_paths = [
        "/home/alex/projects/re/sdocx/sample_docs/Section2lectures-2_260218_125010/end_tag.bin",
        "/home/alex/projects/re/sdocx/sample_docs/Single drawn line fp17, inf scroll_260218_145754/end_tag.bin",
        "/home/alex/projects/re/sdocx/sample_docs/Has background colour, pattern cover, dots_260218_181735/end_tag.bin",
        "/home/alex/projects/re/sdocx/sample_docs/Empty, inf scroll_260218_145632/end_tag.bin",
        "/home/alex/projects/re/sdocx/sample_docs/empty encrypted_260219_125722/end_tag.bin",
    ];

    for path in end_tag_paths {
        let mut end_tag_file = std::fs::File::open(path)?;

        let end_tag = ModelEndTag::try_parse(&mut end_tag_file, NoteSdkType::SPen)?;

        println!("{path}: {end_tag:#?}");
    }

    Ok(())
}

fn main() -> color_eyre::Result<()> {
    color_eyre::install()?;

    demo_media_info()?;
    demo_end_tag()?;

    Ok(())
}
