<p align="center"><img src="./sdocx2pdf.svg" width="40%" alt="'sdocx2pdf' in handwriting"></p>

# sdocx2pdf&nbsp;&ndash;&#32;convert Samsung Notes documents to vector PDFs

sdocx2pdf is a tool for converting Samsung Notes documents to vector PDFs. Vector PDFs represent
handwriting using smooth curves that appear crisp at any resolution. This is unlike the PDFs that
the Samsung Notes apps themselves export, which are _raster_ PDFs: they represent the handwriting
using finite-resolution images, making it appear pixelated. The aim of sdocx2pdf is to produce a
vector PDF that looks close to how the unexported note looks in the app.

The name comes from Samsung's proprietary SDOCX file format. It is the format used by Samsung Notes
internally to store notes on the disk, and also externally when you export a note as a 'Samsung
Notes file' (giving you a `.sdocx` file).

## Capabilities

sdocx2pdf runs on Linux, macOS and Windows and can work with SDOCX files produced by the Samsung
Notes apps for Android and Windows. It reads the handwriting data in the files, represents it
mathematically using smooth curves, and produces a PDF containing those curves. Currently, you can

- write notes by hand in SNotes and get a vector PDF of your handwriting using sdocx2pdf; and
- annotate PDFs by hand in SNotes and get a vector PDF with your handwriting on top of the original
  PDF (with searchable/selectable text) using sdocx2pdf.

There are options for splitting pageless documents into pages based either on page length or on the
page breaks in any embedded PDFs. You can also choose to convert a pageless document to a long
single-page PDF.

These capabilities precisely fit my personal use case: I annotate documents and take notes in
lectures using SNotes. I'd like to be able to read my work on devices that do not run SNotes, so
exporting to PDF is ideal. However, using the native PDF export feature in SNotes produces
pixelated PDFs that are unpleasant to read. sdocx2pdf solves this problem.

<p>
<details>

<summary>Sample PDF page</summary>

To give you an idea of what this all means in practice, here is a page from a PDF produced by
sdocx2pdf v0.1.1. It has been converted to SVG so that it can be included in this README. (Note
that the green marker at the bottom has been outlined manually using the fountain pen.) You may be
able to tell that the fountain pen is the first-class citizen here.

<p align="center">
   <img src="./demo.svg" width="60%" alt="A sample PDF page produced by the tool">
</p>

</details>
</p>

At present, sdocx2pdf understands but ignores all note features other than handwriting and embedded
PDFs. Thus, it does not yet produce PDFs containing images, shapes, typed text, web links or
paintings. It can still process documents that use these features, but it will not include them in
the PDF. I intend to improve sdocx2pdf by adding support for these things in the future.

Another current limitation of sdocx2pdf is that it does not precisely replicate the features of all
the various writing tools. For example, the calligraphy pen looks exactly the same as the fountain
pen because I have not yet invested any time in trying to make it look like it does in SNotes.
Similarly, the pencil, calligraphy brush and ink pen are not yet distinguished from the fountain
pen (though they have different default widths, and width is something that sdocx2pdf _does_
reproduce). These too will hopefully improve with time.

Finally, handwriting that has been modified by the 'Handwriting Help' features is less
information-dense than usual, and at the moment gets smoothed out a bit too much.

## Usage

1. Go to the [latest release](https://github.com/squ1dd13/sdocx2pdf/releases/latest) and download
   the correct version of sdocx2pdf for your computer.
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
without sending it between devices.[^1]

<details>

<summary>sdocx2pdf help message</summary>

```
A tool for converting Samsung Notes documents to vector PDFs. "Vector" means that handwriting data is stored mathematically (as equations for curves) rather than as pixel data (an image). This makes writing clearer and easier to read.

Usage: sdocx2pdf [OPTIONS] <IN> <OUT>

Arguments:
  <IN>
          The path to the Samsung Notes document to be converted. This is typically an SDOCX file (.sdocx).

          The Windows app stores unexported documents as directories that have the same internal structure as SDOCX files. You can also pass the path to one of these directories, or to a directory containing the contents of an unzipped SDOCX file.

  <OUT>
          The path to which the produced PDF will be written. If it already exists, the file will be overwritten.

Options:
      --auto-split
          Inserts page breaks into pageless documents between pages of any embedded PDFs. Disabled by default.

          By default, a pageless document will be converted to a PDF containing a long single page. With auto-splitting enabled, if a pageless document embeds any PDFs, page breaks are inserted to match the page breaks in the embedded PDFs. For example, if you import a five-page PDF into a blank pageless document and annotate it, auto-splitting will give you a five-page PDF
          rather than a single-page PDF.

          This option does nothing when converting a paged document. It also does nothing for pageless documents that do not embed any PDFs; see the basic splitting option.

      --basic-split <BASIC_SPLIT>
          Specifies the page-splitting behaviour used for pageless documents when auto-splitting is not in effect, either because it is disabled or because the document being converted does not embed any PDFs.

          Basic splitting is disabled by default, resulting in long single-page PDFs when auto-splitting is not used. To use basic splitting only, specify a mode and do not enable auto-splitting. When basic splitting and auto-splitting are both enabled, basic splitting is used as a fallback when there are no PDFs embedded in the document. If auto-splitting is enabled but
          basic splitting is not, documents that embed PDFs will be auto-split, but those that don't will not be split at all.

          Possible values:
          - a4-portrait:  Split the document into portrait A4 pages
          - a4-landscape: Split the document into landscape A4 pages
```

</details>

## Device compatibility

I only own a Galaxy Tab S11 and the S Pen it came with, so I have done all my testing of
handwriting features with those. It is possible that differences in polling rate for different
device/pen combinations could affect the stored handwriting data, and therefore the appearance of
the converted PDF. I expect sdocx2pdf will work to some degree regardless of the hardware used to
produce the note, but I don't know yet.

## Technical details

This repository contains two crates: `sdocx`, a library crate, and `sdocx2pdf`, the binary crate
implementing the tool. Both were written by a human (me :wave:). sdocx2pdf is built on the library,
which parses the SDOCX format almost completely.[^2] The limitations described above are due to
sdocx2pdf not implementing the output logic for all the features of SNotes documents. Note that
`sdocx2pdf` was written over a long time while I was experimenting with different ways to draw the
handwriting, so the code is very messy (right now). `sdocx`, on the other hand, was easier to
write, and the code is fairly clean. The library features extensive error reporting to make it
easier to update when Samsung updates the file format.

I have mostly focused on handwriting. (Accordingly, the way it is converted is stupidly
complicated.) SDOCX represents handwriting as a list of events. A single event gives a position, a
pressure value, and optionally information about the tilt of the S Pen. It is possible to produce a
PDF that displays the handwriting by drawing a line between each pair of consecutive events, but,
at least with the hardware I use, there are far too many events to be able to do this without the
resulting PDF being very large (we're talking hundreds of MB for a couple of pages of sparse
writing). It is therefore necessary to pick out the events that are most important to the shape of
the handwriting and discard the rest.

sdocx2pdf finds the [curvature](https://en.wikipedia.org/wiki/Curvature) along the stroke and uses
it to identify the [inflection points](https://en.wikipedia.org/wiki/Inflection_point) and
[vertices](<https://en.wikipedia.org/wiki/Vertex_(curve)>). Along with the positions of the first
and last events, these are considered the 'key features' of the curve and are always included in
the output. Some points between these features are also included in order to ensure the difference
in tangent angle between adjacent sampled points stays below a certain threshold. Otherwise, it
becomes difficult to connect the points nicely using cubic Bézier curves, which is what sdocx2pdf
does.

The actual handwriting processing pipeline is roughly this: clean the events; interpolate the
position and pressure data; upsample; eliminate jitter by applying a Gaussian filter to the
upsampled position and pressure in the time domain, finding the first and second time derivatives
of position along the way; calculate the curvature of the filtered stroke from the derivatives;
interpolate the upsampled and filtered position, pressure and curvature to obtain continuous
functions, now of arc length, not time; find the key features using the curvature; and place points
strategically between the key features to reduce the maximum angle change between points.

To draw the stroke in the PDF, sdocx2pdf joins adjacent points by filling between Bézier curves
that are combined to form the shape of an
['idealised bean'](https://math.stackexchange.com/questions/256937/what-shape-is-a-bean) (or cashew
nut).[^3] The control points for the Bézier curves on either side of the bean (along the long axis)
are calculated so that the body of the bean roughly follows the shape of the stroke between the two
points. For some pens, the width of the bean is determined using pressure.

[^1]:
    In fact, the Windows app stores documents in an extracted SDOCX format that sdocx2pdf supports.
    Once you've found them, you can just give sdocx2pdf the path to the folder corresponding to the
    note you'd like to convert.

[^2]:
    Technically, there are some types of objects that could exist but which the apps never create
    (as far as I can tell). These include plots and tables. The library does not parse these
    objects because I have never been able to create them. As of June 2026, the library can parse
    any object that you can add to a document in the Android app.

[^3]:
    The full bean is not always created. The round ends make for better connections, because the
    overlap prevents hairline gaps between beans, but there is still an overlap if the end of one
    bean is round and the start of the next bean is flat. Except for sharp corners, where the
    rounding on the beans produces a nice rounded corner, sdocx2pdf will avoid rounding both ends
    of the beans to save on file size. The best way to see this is to change the filling operations
    to stroking operations in the code.
