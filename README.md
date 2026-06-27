# sdocx2pdf&nbsp;&ndash;&#32;convert Samsung Notes documents to vector PDFs

<p style="background-color: yellow">"sdocx2pdf" written in SNotes, converted to PDF using sdocx2pdf, and then to an image</p>

sdocx2pdf is a tool for converting Samsung Notes documents to vector PDFs. Vector PDFs represent
handwriting using smooth curves that appear crisp at any resolution. This is unlike the PDFs that
the Samsung Notes apps themselves export, which are _raster_ PDFs: they represent the handwriting
using finite-resolution images, making it appear pixelated. The aim of sdocx2pdf is to produce a
vector PDF that looks close to how the unexported note looks in the app.

This repository contains `sdocx`, a library crate, and `sdocx2pdf`, a binary crate. The former
implements a library for parsing Samsung's proprietary SDOCX format; the latter uses that library
to implement the sdocx2pdf tool. The library is fairly complete in that it is able to parse the
vast majority[^1] of objects that exist in the SDOCX format. **sdocx2pdf is incomplete** because it
does not make use of all the parsed data. **At present, it reproduces handwriting and embedded
PDFs, but ignores all other features.** Currently, you can

- write notes by hand in SNotes and get a vector PDF of your handwriting using sdocx2pdf; and
- annotate PDFs by hand in SNotes and get a vector PDF with your handwriting on top of the original
  PDF (with searchable/selectable text) using sdocx2pdf.

sdocx2pdf **does not yet produce PDFs containing images, shapes, typed text, web links or
paintings**. If those features are used in the input document, they will be ignored. I intend to
implement them over time. Another limitation is that sdocx2pdf currently does not exactly replicate
the appearances of some of the writing tools. All tools are usable&nbsp;&ndash;&#32;in particular,
the fountain pen and highlighters work very well&nbsp;&ndash;&#32;but

- the exact ways in which stroke width and colour vary with pressure/direction/speed/pen tilt
  across the different tools,
- the shapes at the ends of strokes, and
- the textures used for the pencils and calligraphy brush

do not match SNotes perfectly. Thus, in its current form, sdocx2pdf is good for producing PDFs of
handwriting, but is not ideal if you are doing anything more artistic. Again, I expect this to
improve.

## Usage

1. Download the correct version of sdocx2pdf for your computer.
2. On your device with Samsung Notes, open a note you wish to convert.
3. Use either 'Save note as Samsung Notes file' or 'Share note as Samsung Notes file' to get an
   SDOCX file for the note onto your computer.
4. Run `/path/to/sdocx2pdf your.sdocx output.pdf` at your computer's command line, replacing the
   paths respectively with the location of the sdocx2pdf binary you downloaded, the location of the
   SDOCX file you just created, and the path you'd like the new PDF to be written to.

For example, I'd do the following:

1. Download the Linux x86-64 version of sdocx2pdf on my laptop.
2. Open the note in SNotes on my Galaxy Tab S11.
3. Use 'Share note as Samsung Notes file' (via the three-dot menu at the top-right of the English
   interface, which has a 'Share' button in the row of five icons at the bottom) and select Quick
   Share to send the SDOCX file to my laptop (which is running
   [rquickshare](https://github.com/Martichou/rquickshare)).
4. Run `sdocx2pdf /my/download/folder/note.sdocx /tmp/out.pdf`, and enjoy the nice crisp
   handwriting in `/tmp/out.pdf`.

The process is even easier if you're using the Samsung Notes app on a Windows computer; then, you
can just 'Save note as Samsung Notes file' and immediately feed the SDOCX file into sdocx2pdf
without sending it between devices.[^2]

## Device support

I only own a Galaxy Tab S11 and the S Pen it came with, so have done all my testing of handwriting
features with those. It is possible that differences in polling rate for different device/pen
combinations could affect the stored handwriting data, and therefore the appearance of the
converted PDF. However, I expect sdocx2pdf to work to some degree on all

<p style="background-color: yellow">technical details</p>

<p style="background-color: yellow">right now it is very error-sensitive by design in order to make it easy to keep the library up-to-date - not ideal for a tool, so will make it more resilient</p>

<p style="background-color: yellow">collapsible help text</p>

[^1]:
    Technically, there are some types of objects that could exist but which the apps never create
    (as far as I can tell). These include plots and tables. The library does not parse these
    objects because I have never been able to create them. As of June 2026, the library can parse
    any object that you can add to a document in the Android app.

[^2]:
    In fact, the Windows app stores documents in an extracted SDOCX format which sdocx2pdf
    supports. Once you've found them, you can just give sdocx2pdf the path to the folder
    corresponding to the note you'd like to convert.
