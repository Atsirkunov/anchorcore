//! R46.16: pictures embedded in a PDF — extract what is there, render
//! nothing. Owner call 2026-10-23: "if a PDF has a picture, we extract; if
//! not, ignore". Vector drawings (most "save as PDF" charts) have no image
//! object and stay text-only; no renderer, no model on the ingest path.
//!
//! Two halves, both deterministic:
//! - `page_images`: per page, the image XObjects its content stream
//!   actually draws (`Do`), through Form XObjects up to `MAX_FORM_DEPTH`.
//!   Page resources are often shared (jsPDF lists every image on every
//!   page), so the resource dict alone would attribute every image to every
//!   page. Extraction writes one `![](embedded:pdf:<obj>-<gen>)` marker per
//!   drawn image at the end of its page block (`extract::join_pdf_pages`).
//! - `decode`: the image's bytes as a stored asset. JPEG (`DCTDecode`
//!   alone) passes through untouched; 8-bit Gray/RGB samples
//!   (Flate/LZW/ASCII85 or none, ICCBased N=1/3 included) are re-encoded as
//!   PNG, with an 8-bit `SMask` as alpha. Everything else (JPX, JBIG2,
//!   CCITT, CMYK, Indexed, stencil masks, other bit depths) is skipped.

use std::collections::{BTreeMap, HashSet};

use lopdf::{content::Content, Dictionary, Document, Object, ObjectId, Stream};

/// Marker part prefix after `assets::EMBEDDED_SCHEME`.
pub const PART_PREFIX: &str = "pdf:";
const MAX_FORM_DEPTH: usize = 4;
/// Decoded-sample guard: a hostile or broken PDF never allocates more than
/// this many pixels for one image.
const MAX_PIXELS: u64 = 64 * 1024 * 1024;

pub fn part_for(id: ObjectId) -> String {
    format!("{PART_PREFIX}{}-{}", id.0, id.1)
}

fn parse_part(part: &str) -> Option<ObjectId> {
    let (num, gen) = part.strip_prefix(PART_PREFIX)?.split_once('-')?;
    Some((num.parse().ok()?, gen.parse().ok()?))
}

// ----- which images each page draws -----

/// 1-based page number → drawn image objects, first-draw order, no repeats
/// within a page. Pages that draw no image are absent.
pub fn page_images(doc: &Document) -> BTreeMap<u32, Vec<ObjectId>> {
    let mut out = BTreeMap::new();
    for (page_no, page_id) in doc.get_pages() {
        let Ok(content) = doc.get_page_content(page_id) else { continue };
        let resources = page_resources(doc, page_id);
        let mut found = Vec::new();
        let mut seen = HashSet::new();
        collect_drawn(doc, &content, &resources, 0, &mut found, &mut seen);
        if !found.is_empty() {
            out.insert(page_no, found);
        }
    }
    out
}

fn page_resources(doc: &Document, page_id: ObjectId) -> Vec<&Dictionary> {
    let Ok((inline, inherited)) = doc.get_page_resources(page_id) else { return Vec::new() };
    inline.into_iter().chain(inherited.into_iter().filter_map(|id| doc.get_dictionary(id).ok())).collect()
}

/// The XObject `name` resolves to in the first resource dict that has it.
fn xobject_ref(doc: &Document, resources: &[&Dictionary], name: &[u8]) -> Option<ObjectId> {
    resources.iter().find_map(|res| {
        let xo = res.get(b"XObject").ok()?;
        let xo = doc.dereference(xo).ok()?.1.as_dict().ok()?;
        xo.get(name).ok()?.as_reference().ok()
    })
}

fn collect_drawn(doc: &Document, content: &[u8], resources: &[&Dictionary], depth: usize, found: &mut Vec<ObjectId>, seen: &mut HashSet<ObjectId>) {
    let Ok(ops) = Content::decode(content) else { return };
    for op in ops.operations.iter().filter(|o| o.operator == "Do") {
        let Some(name) = op.operands.first().and_then(|o| o.as_name().ok()) else { continue };
        let Some(id) = xobject_ref(doc, resources, name) else { continue };
        let Ok(stream) = doc.get_object(id).and_then(Object::as_stream) else { continue };
        match stream.dict.get(b"Subtype").and_then(Object::as_name) {
            Ok(b"Image") => {
                if seen.insert(id) {
                    found.push(id);
                }
            }
            Ok(b"Form") if depth < MAX_FORM_DEPTH => descend_form(doc, stream, resources, depth, found, seen),
            _ => {}
        }
    }
}

/// A Form XObject draws with its own resources (falling back to the
/// caller's, as viewers do for resource-less forms).
fn descend_form(doc: &Document, form: &Stream, parent: &[&Dictionary], depth: usize, found: &mut Vec<ObjectId>, seen: &mut HashSet<ObjectId>) {
    let Ok(content) = form.decompressed_content() else { return };
    let own = form.dict.get(b"Resources").ok().and_then(|r| doc.dereference(r).ok()).and_then(|(_, r)| r.as_dict().ok());
    match own {
        Some(res) => collect_drawn(doc, &content, &[res], depth + 1, found, seen),
        None => collect_drawn(doc, &content, parent, depth + 1, found, seen),
    }
}

// ----- decoding one image -----

/// One decoded image: `bytes` is `None` when the encoded size is over the
/// cap (metadata only, like every other asset).
#[derive(Debug, PartialEq)]
pub struct PdfImage {
    pub bytes: Option<Vec<u8>>,
    pub size: u64,
    pub mime: &'static str,
    pub w: u32,
    pub h: u32,
}

fn int(dict: &Dictionary, key: &[u8]) -> Option<i64> {
    dict.get(key).ok()?.as_i64().ok()
}

pub fn decode(doc: &Document, part: &str, cap: u64) -> Option<PdfImage> {
    let stream = doc.get_object(parse_part(part)?).ok()?.as_stream().ok()?;
    let d = &stream.dict;
    if d.get(b"ImageMask").and_then(Object::as_bool).unwrap_or(false) {
        return None;
    }
    let (w, h) = (u32::try_from(int(d, b"Width")?).ok()?, u32::try_from(int(d, b"Height")?).ok()?);
    let filters: Vec<&[u8]> = stream.filters().unwrap_or_default();
    if filters == [b"DCTDecode".as_slice()] {
        return Some(sized(stream.content.clone(), "image/jpeg", w, h, cap));
    }
    if !filters.iter().all(|f| matches!(*f, b"FlateDecode" | b"LZWDecode" | b"ASCII85Decode")) {
        return None;
    }
    let png = encode_png(doc, stream, w, h)?;
    Some(sized(png, "image/png", w, h, cap))
}

fn sized(bytes: Vec<u8>, mime: &'static str, w: u32, h: u32, cap: u64) -> PdfImage {
    let size = bytes.len() as u64;
    PdfImage { bytes: (size <= cap).then_some(bytes), size, mime, w, h }
}

/// Channels of a supported color space: Device/Cal Gray|RGB, ICCBased N.
fn channels(doc: &Document, cs: &Object) -> Option<usize> {
    let cs = doc.dereference(cs).ok()?.1;
    let name = match cs {
        Object::Name(n) => n.as_slice(),
        Object::Array(a) => a.first()?.as_name().ok()?,
        _ => return None,
    };
    match name {
        b"DeviceGray" | b"CalGray" | b"G" => Some(1),
        b"DeviceRGB" | b"CalRGB" | b"RGB" => Some(3),
        b"ICCBased" => {
            let icc = doc.dereference(cs.as_array().ok()?.get(1)?).ok()?.1.as_stream().ok()?;
            match int(&icc.dict, b"N")? {
                1 => Some(1),
                3 => Some(3),
                _ => None,
            }
        }
        _ => None,
    }
}

fn samples(stream: &Stream, w: u32, h: u32, ch: usize) -> Option<Vec<u8>> {
    if int(&stream.dict, b"BitsPerComponent") != Some(8) || u64::from(w) * u64::from(h) > MAX_PIXELS {
        return None;
    }
    let mut data = stream.decompressed_content().ok()?;
    let need = w as usize * h as usize * ch;
    if data.len() < need {
        return None;
    }
    data.truncate(need);
    Some(data)
}

/// An 8-bit gray soft mask of the same size becomes the alpha channel.
fn soft_mask(doc: &Document, stream: &Stream, w: u32, h: u32) -> Option<Vec<u8>> {
    let mask = doc.get_object(stream.dict.get(b"SMask").ok()?.as_reference().ok()?).ok()?.as_stream().ok()?;
    if (int(&mask.dict, b"Width"), int(&mask.dict, b"Height")) != (Some(w.into()), Some(h.into())) {
        return None;
    }
    samples(mask, w, h, 1)
}

fn encode_png(doc: &Document, stream: &Stream, w: u32, h: u32) -> Option<Vec<u8>> {
    let ch = channels(doc, stream.dict.get(b"ColorSpace").ok()?)?;
    let pixels = samples(stream, w, h, ch)?;
    let (color, data) = match soft_mask(doc, stream, w, h) {
        Some(alpha) => (if ch == 1 { png::ColorType::GrayscaleAlpha } else { png::ColorType::Rgba }, interleave(&pixels, &alpha, ch)),
        None => (if ch == 1 { png::ColorType::Grayscale } else { png::ColorType::Rgb }, pixels),
    };
    let mut out = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut out, w, h);
        enc.set_color(color);
        enc.set_depth(png::BitDepth::Eight);
        enc.write_header().ok()?.write_image_data(&data).ok()?;
    }
    Some(out)
}

fn interleave(pixels: &[u8], alpha: &[u8], ch: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(pixels.len() + alpha.len());
    for (px, a) in pixels.chunks_exact(ch).zip(alpha) {
        out.extend_from_slice(px);
        out.push(*a);
    }
    out
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use lopdf::{dictionary, Stream};

    /// A PDF whose page 1 has a line of text and draws a 40×40 RGB Flate
    /// image with a soft mask (via a Form XObject), page 2 draws a JPEG, page 3 only lists both
    /// in its shared resources and draws nothing, page 4 holds a CMYK image.
    pub(crate) fn fixture() -> Vec<u8> {
        let mut doc = Document::with_version("1.5");
        let pages_id = doc.new_object_id();
        let rgb: Vec<u8> = (0..40 * 40).flat_map(|i| [(i % 256) as u8, 80, 160]).collect();
        let alpha = vec![200u8; 40 * 40];
        let mut mask = Stream::new(dictionary! {"Type" => "XObject", "Subtype" => "Image", "Width" => 40, "Height" => 40, "ColorSpace" => "DeviceGray", "BitsPerComponent" => 8}, alpha);
        mask.compress().unwrap();
        let mask = doc.add_object(mask);
        let mut img = Stream::new(dictionary! {"Type" => "XObject", "Subtype" => "Image", "Width" => 40, "Height" => 40, "ColorSpace" => "DeviceRGB", "BitsPerComponent" => 8, "SMask" => mask}, rgb);
        img.compress().unwrap();
        let flate = doc.add_object(img);
        // SOI, APP0, SOF0 640×480, EOI: enough for `assets::sniff`.
        let jpg = vec![0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x04, 0x00, 0x00, 0xFF, 0xC0, 0x00, 0x11, 0x08, 0x01, 0xE0, 0x02, 0x80, 0xFF, 0xD9];
        let jpeg = doc.add_object(Stream::new(dictionary! {"Type" => "XObject", "Subtype" => "Image", "Width" => 640, "Height" => 480, "ColorSpace" => "DeviceRGB", "BitsPerComponent" => 8, "Filter" => "DCTDecode"}, jpg));
        let cmyk = doc.add_object(Stream::new(dictionary! {"Type" => "XObject", "Subtype" => "Image", "Width" => 40, "Height" => 40, "ColorSpace" => "DeviceCMYK", "BitsPerComponent" => 8}, vec![0u8; 40 * 40 * 4]));
        let form = doc.add_object(Stream::new(
            dictionary! {"Type" => "XObject", "Subtype" => "Form", "BBox" => vec![0.into(), 0.into(), 40.into(), 40.into()], "Resources" => dictionary! {"XObject" => dictionary! {"Inner" => flate}}},
            b"q 40 0 0 40 0 0 cm /Inner Do Q".to_vec(),
        ));
        let font = doc.add_object(dictionary! {"Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica"});
        let shared = doc.add_object(dictionary! {"XObject" => dictionary! {"Fm" => form, "Jp" => jpeg, "Ck" => cmyk}, "Font" => dictionary! {"F1" => font}});
        let mut kids = Vec::new();
        for body in ["BT /F1 12 Tf 10 60 Td (Page one words) Tj ET /Fm Do", "q 64 0 0 48 0 0 cm /Jp Do Q /Jp Do", "0 0 m 10 10 l S", "/Ck Do"] {
            let content = doc.add_object(Stream::new(dictionary! {}, body.as_bytes().to_vec()));
            let page = doc.add_object(dictionary! {"Type" => "Page", "Parent" => pages_id, "Contents" => content, "Resources" => shared, "MediaBox" => vec![0.into(), 0.into(), 100.into(), 100.into()]});
            kids.push(page.into());
        }
        let count = kids.len() as i64;
        doc.objects.insert(pages_id, Object::Dictionary(dictionary! {"Type" => "Pages", "Kids" => kids, "Count" => count}));
        let catalog = doc.add_object(dictionary! {"Type" => "Catalog", "Pages" => pages_id});
        doc.trailer.set("Root", catalog);
        let mut out = Vec::new();
        doc.save_to(&mut out).unwrap();
        out
    }

    /// `n` image-only pages, each drawing its own JPEG object.
    pub(crate) fn image_pages(n: usize) -> Vec<u8> {
        let mut doc = Document::with_version("1.5");
        let pages_id = doc.new_object_id();
        let mut kids: Vec<Object> = Vec::new();
        for i in 0..n {
            let jpg = vec![0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x04, 0x00, 0x00, 0xFF, 0xC0, 0x00, 0x11, 0x08, 0x01, 0xE0, 0x02, 0x80, i as u8, 0xFF, 0xD9];
            let img = doc.add_object(Stream::new(dictionary! {"Type" => "XObject", "Subtype" => "Image", "Width" => 640, "Height" => 480, "ColorSpace" => "DeviceRGB", "BitsPerComponent" => 8, "Filter" => "DCTDecode"}, jpg));
            let content = doc.add_object(Stream::new(dictionary! {}, b"q 640 0 0 480 0 0 cm /I Do Q".to_vec()));
            let res = dictionary! {"XObject" => dictionary! {"I" => img}};
            let page = doc.add_object(dictionary! {"Type" => "Page", "Parent" => pages_id, "Contents" => content, "Resources" => res, "MediaBox" => vec![0.into(), 0.into(), 640.into(), 480.into()]});
            kids.push(page.into());
        }
        let count = kids.len() as i64;
        doc.objects.insert(pages_id, Object::Dictionary(dictionary! {"Type" => "Pages", "Kids" => kids, "Count" => count}));
        let catalog = doc.add_object(dictionary! {"Type" => "Catalog", "Pages" => pages_id});
        doc.trailer.set("Root", catalog);
        let mut out = Vec::new();
        doc.save_to(&mut out).unwrap();
        out
    }

    #[test]
    fn pages_list_only_the_images_they_draw() {
        let doc = Document::load_mem(&fixture()).unwrap();
        let pages = page_images(&doc);
        assert_eq!(pages.keys().copied().collect::<Vec<_>>(), vec![1, 2, 4], "page 3 draws nothing; shared resources don't count");
        assert_eq!(pages[&2].len(), 1, "a JPEG drawn twice on one page is one image");
        let flate = decode(&doc, &part_for(pages[&1][0]), u64::MAX).unwrap();
        assert_eq!((flate.mime, flate.w, flate.h), ("image/png", 40, 40));
        let info = crate::assets::sniff(flate.bytes.as_deref().unwrap()).unwrap();
        assert_eq!((info.mime, info.w, info.h), ("image/png", Some(40), Some(40)), "valid PNG");
        let jpeg = decode(&doc, &part_for(pages[&2][0]), u64::MAX).unwrap();
        assert_eq!((jpeg.mime, jpeg.w), ("image/jpeg", 640));
        assert_eq!(&jpeg.bytes.as_deref().unwrap()[..2], &[0xFF, 0xD8], "JPEG bytes pass through");
        assert_eq!(decode(&doc, &part_for(pages[&4][0]), u64::MAX), None, "CMYK samples are skipped");
        let over = decode(&doc, &part_for(pages[&2][0]), 10).unwrap();
        assert_eq!((over.bytes, over.size > 10), (None, true), "over the cap keeps metadata only");
        assert_eq!(decode(&doc, "pdf:999-0", u64::MAX), None);
    }
}
