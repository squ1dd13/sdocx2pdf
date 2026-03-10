fn main() {
    let document = sdocx::Document::from_zip(
        "/home/alex/projects/re/sdocx/sample_docs/Single drawn line fp17, inf scroll_260218_145754.sdocx",
    )
    .unwrap();

    eprintln!(
        "Title is '{}'",
        document.title_text().raw_string().unwrap_or("")
    );

    match document.page_model() {
        sdocx::PageModel::Paged => eprintln!("This is a paged document"),
        sdocx::PageModel::Pageless => eprintln!("This is a pageless document"),
    };

    let (w, h) = document.width_height();
    eprintln!("w = {w}, h = {h}");

    let mut event_count = 0_usize;

    for page in document.pages() {
        for layer in page.layers() {
            for object in layer.objects() {
                let sdocx::DocObject::Stroke(stroke) = object else {
                    continue;
                };

                eprintln!(
                    "Stroke pen is {}",
                    stroke.pen_name().unwrap_or("(unspecified)")
                );

                event_count += stroke.events().len();
            }
        }
    }

    eprintln!("Document contains {event_count} stroke events");
}
