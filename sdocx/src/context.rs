use crate::{media_info::FileRegistry, note_doc::StringRegistry};

#[derive(Clone, Copy)]
pub struct DocumentContext<'fr, 'sr> {
    pub file_registry: &'fr FileRegistry,
    pub string_registry: &'sr StringRegistry,
}

pub trait TryParseWithContext<R: ?Sized, C: ?Sized>: Sized {
    type ParseError;

    fn try_parse_with_ctx(reader: &mut R, ctx: &C) -> Result<Self, Self::ParseError>;
}
