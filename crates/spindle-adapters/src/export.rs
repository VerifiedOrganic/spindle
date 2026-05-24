use std::io::{Cursor, Write};

use zip::ZipWriter;
use zip::write::FileOptions;

/// Assembled project data ready for EPUB rendering.
pub struct EpubSource {
    pub title: String,
    pub author: Option<String>,
    pub language: String,
    pub books: Vec<EpubBook>,
}

pub struct EpubBook {
    pub number: i32,
    pub title: Option<String>,
    pub chapters: Vec<EpubChapter>,
}

pub struct EpubChapter {
    pub number: i32,
    pub title: Option<String>,
    /// Concatenated scene prose for this chapter.
    pub body: String,
}

/// Build an EPUB 3 archive from assembled project data and return the raw
/// bytes. The caller decides where to write them (disk, HTTP response, etc.).
pub fn build_epub(source: &EpubSource) -> anyhow::Result<Vec<u8>> {
    let buf = Cursor::new(Vec::new());
    let mut zip = ZipWriter::new(buf);
    let stored = FileOptions::default().compression_method(zip::CompressionMethod::Stored);
    let deflated = FileOptions::default();

    // ── mimetype (must be first entry, uncompressed, per EPUB spec) ──
    zip.start_file("mimetype", stored)?;
    zip.write_all(b"application/epub+zip")?;

    // ── META-INF/container.xml ──
    zip.start_file("META-INF/container.xml", deflated)?;
    zip.write_all(CONTAINER_XML)?;

    // ── stylesheet ──
    zip.start_file("OEBPS/style.css", deflated)?;
    zip.write_all(STYLESHEET)?;

    // Flatten chapters across all books into a sequential spine.
    let spine_items = collect_spine_items(source);

    // ── chapter XHTML files ──
    for item in &spine_items {
        zip.start_file(&item.path, deflated)?;
        zip.write_all(item.xhtml.as_bytes())?;
    }

    // ── title page ──
    let title_xhtml = render_title_page(&source.title, source.author.as_deref());
    zip.start_file("OEBPS/title.xhtml", deflated)?;
    zip.write_all(title_xhtml.as_bytes())?;

    // ── toc.xhtml (EPUB 3 navigation document) ──
    let toc_xhtml = render_nav_toc(source, &spine_items);
    zip.start_file("OEBPS/toc.xhtml", deflated)?;
    zip.write_all(toc_xhtml.as_bytes())?;

    // ── content.opf (package document) ──
    let opf = render_content_opf(source, &spine_items);
    zip.start_file("OEBPS/content.opf", deflated)?;
    zip.write_all(opf.as_bytes())?;

    let cursor = zip.finish()?;
    Ok(cursor.into_inner())
}

// ─── internal helpers ────────────────────────────────────────────────

struct SpineItem {
    /// Unique id used in the OPF manifest/spine (e.g. "ch_1_3").
    id: String,
    /// Path inside the ZIP (e.g. "OEBPS/ch_1_3.xhtml").
    path: String,
    /// Display label for the table of contents.
    toc_label: String,
    /// Full XHTML content.
    xhtml: String,
}

fn collect_spine_items(source: &EpubSource) -> Vec<SpineItem> {
    let multi_book = source.books.len() > 1;
    let mut items = Vec::new();

    for book in &source.books {
        for chapter in &book.chapters {
            let id = format!("ch_{}_{}", book.number, chapter.number);
            let path = format!("OEBPS/{id}.xhtml");

            let heading = chapter_heading(multi_book, book, chapter);
            let toc_label = heading.clone();
            let xhtml = render_chapter_xhtml(&heading, &chapter.body);

            items.push(SpineItem {
                id,
                path,
                toc_label,
                xhtml,
            });
        }
    }

    items
}

fn chapter_heading(multi_book: bool, book: &EpubBook, chapter: &EpubChapter) -> String {
    let prefix = if multi_book {
        let book_label = book
            .title
            .as_deref()
            .map(String::from)
            .unwrap_or_else(|| format!("Book {}", book.number));
        format!("{book_label} — ")
    } else {
        String::new()
    };

    match &chapter.title {
        Some(title) => format!("{prefix}Chapter {}: {title}", chapter.number),
        None => format!("{prefix}Chapter {}", chapter.number),
    }
}

/// Convert scene prose into XHTML. Double newlines become paragraph breaks;
/// single newlines become `<br/>` within a paragraph. LitRPG system UI blocks
/// can be marked with either pandoc-style fenced divs or backtick-fenced
/// blocks whose info string names a known system class:
///
/// ```text
/// ::: system-box                ```system-box
/// STAGE CRED EARNED: +2.        STAGE CRED EARNED: +2.
/// :::                           ```
/// ```
fn prose_to_xhtml_body(text: &str) -> String {
    let mut out = String::new();
    let mut paragraph_lines = Vec::new();
    let mut lines = text.lines();

    while let Some(line) = lines.next() {
        if let Some((class_name, fence)) = detect_system_fence(line) {
            flush_paragraph(&mut out, &mut paragraph_lines);

            let mut block_lines = Vec::new();
            let mut closed = false;
            for block_line in lines.by_ref() {
                if is_system_fence_close(block_line, &fence) {
                    closed = true;
                    break;
                }
                block_lines.push(block_line);
            }

            if closed {
                out.push_str(&render_system_block(class_name, &block_lines));
            } else {
                paragraph_lines.push(line);
                paragraph_lines.extend(block_lines);
            }
            continue;
        }

        if line.trim().is_empty() {
            flush_paragraph(&mut out, &mut paragraph_lines);
        } else {
            paragraph_lines.push(line);
        }
    }

    flush_paragraph(&mut out, &mut paragraph_lines);
    out
}

fn flush_paragraph(out: &mut String, paragraph_lines: &mut Vec<&str>) {
    let trimmed = trim_blank_edges(paragraph_lines);
    if trimmed.is_empty() {
        paragraph_lines.clear();
        return;
    }

    let rendered = xml_escape(&trimmed.join("\n")).replace('\n', "<br/>\n");
    out.push_str(&format!("<p>{rendered}</p>\n"));
    paragraph_lines.clear();
}

fn render_system_block(class_name: &str, lines: &[&str]) -> String {
    let trimmed = trim_blank_edges(lines);
    let body = render_inline_markdown(&trimmed.join("\n")).replace('\n', "<br/>\n");
    format!("<div class=\"{class_name}\">{body}</div>\n")
}

fn trim_blank_edges<'a>(lines: &[&'a str]) -> Vec<&'a str> {
    let start = lines
        .iter()
        .position(|line| !line.trim().is_empty())
        .unwrap_or(lines.len());
    let end = lines
        .iter()
        .rposition(|line| !line.trim().is_empty())
        .map(|index| index + 1)
        .unwrap_or(start);
    lines[start..end].to_vec()
}

enum SystemFence {
    Colon,
    Backtick,
}

fn detect_system_fence(line: &str) -> Option<(&'static str, SystemFence)> {
    let trimmed = line.trim();
    if let Some(rest) = trimmed.strip_prefix(":::") {
        return classify_system_class(rest.trim()).map(|c| (c, SystemFence::Colon));
    }
    if let Some(rest) = trimmed.strip_prefix("```") {
        return classify_system_class(rest.trim()).map(|c| (c, SystemFence::Backtick));
    }
    None
}

fn is_system_fence_close(line: &str, fence: &SystemFence) -> bool {
    let trimmed = line.trim();
    match fence {
        SystemFence::Colon => trimmed == ":::",
        SystemFence::Backtick => trimmed == "```",
    }
}

fn classify_system_class(name: &str) -> Option<&'static str> {
    match name {
        "system" => Some("system-box"),
        "system-box" => Some("system-box"),
        "system-notification" => Some("system-notification"),
        "system-pull" => Some("system-pull"),
        "system-quest" => Some("system-quest"),
        _ => None,
    }
}

fn render_inline_markdown(text: &str) -> String {
    let escaped = xml_escape(text);
    let escaped = render_delimited_inline(&escaped, "**", "strong");
    render_delimited_inline(&escaped, "*", "em")
}

fn render_delimited_inline(text: &str, delimiter: &str, tag: &str) -> String {
    let mut out = String::new();
    let mut rest = text;

    while let Some(index) = rest.find(delimiter) {
        out.push_str(&rest[..index]);
        let after_open = &rest[index + delimiter.len()..];
        let Some(close_index) = after_open.find(delimiter) else {
            out.push_str(delimiter);
            out.push_str(after_open);
            return out;
        };

        out.push('<');
        out.push_str(tag);
        out.push('>');
        out.push_str(&after_open[..close_index]);
        out.push_str("</");
        out.push_str(tag);
        out.push('>');
        rest = &after_open[close_index + delimiter.len()..];
    }

    out.push_str(rest);
    out
}

fn render_chapter_xhtml(heading: &str, body: &str) -> String {
    let body_html = prose_to_xhtml_body(body);
    format!(
        r#"<?xml version="1.0" encoding="utf-8"?>
<!DOCTYPE html>
<html xmlns="http://www.w3.org/1999/xhtml" xml:lang="en">
<head>
  <title>{heading}</title>
  <link rel="stylesheet" type="text/css" href="style.css"/>
</head>
<body>
  <h1>{heading}</h1>
{body_html}
</body>
</html>"#,
        heading = xml_escape(heading),
        body_html = body_html,
    )
}

fn render_title_page(title: &str, author: Option<&str>) -> String {
    let author_line = author
        .map(|a| format!("<p class=\"author\">{}</p>", xml_escape(a)))
        .unwrap_or_default();
    format!(
        r#"<?xml version="1.0" encoding="utf-8"?>
<!DOCTYPE html>
<html xmlns="http://www.w3.org/1999/xhtml" xml:lang="en">
<head>
  <title>{title}</title>
  <link rel="stylesheet" type="text/css" href="style.css"/>
</head>
<body>
  <div class="title-page">
    <h1>{title}</h1>
    {author_line}
  </div>
</body>
</html>"#,
        title = xml_escape(title),
    )
}

fn render_nav_toc(source: &EpubSource, items: &[SpineItem]) -> String {
    let mut entries = String::new();
    for item in items {
        let href = item.path.strip_prefix("OEBPS/").unwrap_or(&item.path);
        entries.push_str(&format!(
            "      <li><a href=\"{href}\">{label}</a></li>\n",
            label = xml_escape(&item.toc_label),
        ));
    }
    format!(
        r#"<?xml version="1.0" encoding="utf-8"?>
<!DOCTYPE html>
<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops" xml:lang="en">
<head>
  <title>{title}</title>
</head>
<body>
  <nav epub:type="toc" id="toc">
    <h1>Table of Contents</h1>
    <ol>
{entries}    </ol>
  </nav>
</body>
</html>"#,
        title = xml_escape(&source.title),
    )
}

fn render_content_opf(source: &EpubSource, items: &[SpineItem]) -> String {
    let uuid = simple_uuid(&source.title);
    let lang = xml_escape(&source.language);
    let title = xml_escape(&source.title);
    let author_meta = source
        .author
        .as_ref()
        .map(|a| format!("    <dc:creator>{}</dc:creator>", xml_escape(a)))
        .unwrap_or_default();

    let mut manifest = String::new();
    manifest.push_str("    <item id=\"style\" href=\"style.css\" media-type=\"text/css\"/>\n");
    manifest.push_str(
        "    <item id=\"title\" href=\"title.xhtml\" media-type=\"application/xhtml+xml\"/>\n",
    );
    manifest.push_str(
        "    <item id=\"toc\" href=\"toc.xhtml\" media-type=\"application/xhtml+xml\" properties=\"nav\"/>\n",
    );
    for item in items {
        let href = item.path.strip_prefix("OEBPS/").unwrap_or(&item.path);
        manifest.push_str(&format!(
            "    <item id=\"{id}\" href=\"{href}\" media-type=\"application/xhtml+xml\"/>\n",
            id = item.id,
        ));
    }

    let mut spine = String::new();
    spine.push_str("    <itemref idref=\"title\"/>\n");
    spine.push_str("    <itemref idref=\"toc\"/>\n");
    for item in items {
        spine.push_str(&format!("    <itemref idref=\"{}\"/>\n", item.id));
    }

    format!(
        r#"<?xml version="1.0" encoding="utf-8"?>
<package xmlns="http://www.idpf.org/2007/opf" unique-identifier="uid" version="3.0">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:identifier id="uid">urn:uuid:{uuid}</dc:identifier>
    <dc:title>{title}</dc:title>
    <dc:language>{lang}</dc:language>
{author_meta}
    <meta property="dcterms:modified">{modified}</meta>
  </metadata>
  <manifest>
{manifest}  </manifest>
  <spine>
{spine}  </spine>
</package>"#,
        modified = chrono_now(),
    )
}

/// Deterministic UUID derived from the title so the same project always
/// produces the same identifier (useful for EPUB readers tracking position).
fn simple_uuid(seed: &str) -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut hasher = DefaultHasher::new();
    seed.hash(&mut hasher);
    let h = hasher.finish();
    format!(
        "{:08x}-{:04x}-{:04x}-{:04x}-{:012x}",
        (h >> 32) as u32,
        (h >> 16) as u16,
        (h & 0xffff) as u16 | 0x4000,
        ((h >> 48) as u16 & 0x3fff) | 0x8000,
        h & 0xffffffffffff,
    )
}

fn chrono_now() -> String {
    // EPUB requires W3CDTF (ISO 8601 subset).
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    // Simple UTC timestamp without pulling in chrono crate.
    let secs_per_day = 86400u64;
    let days = now / secs_per_day;
    let secs = now % secs_per_day;
    let hours = secs / 3600;
    let mins = (secs % 3600) / 60;
    let secs = secs % 60;
    // Approximate date from days since epoch (good enough for modified stamp).
    let (year, month, day) = days_to_ymd(days);
    format!("{year:04}-{month:02}-{day:02}T{hours:02}:{mins:02}:{secs:02}Z")
}

fn days_to_ymd(mut days: u64) -> (u64, u64, u64) {
    // Civil days algorithm (adapted from Howard Hinnant).
    days += 719468;
    let era = days / 146097;
    let doe = days - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y, m, d)
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

const CONTAINER_XML: &[u8] = br#"<?xml version="1.0" encoding="utf-8"?>
<container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container">
  <rootfiles>
    <rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/>
  </rootfiles>
</container>"#;

const STYLESHEET: &[u8] = br#"body {
  font-family: Georgia, "Times New Roman", serif;
  margin: 1.5em;
  line-height: 1.6;
}
h1 {
  font-size: 1.4em;
  margin-bottom: 1em;
  text-align: center;
}
p {
  text-indent: 1.5em;
  margin: 0.3em 0;
}
p:first-of-type {
  text-indent: 0;
}
.title-page {
  text-align: center;
  margin-top: 30%;
}
.title-page h1 {
  font-size: 2em;
  margin-bottom: 0.5em;
}
.author {
  font-size: 1.2em;
  font-style: italic;
}
.system-box {
  border: 1.5px solid #8888aa;
  border-radius: 3px;
  padding: 0.5em 0.9em;
  margin: 1em 0;
  font-family: "Courier New", Courier, monospace;
  font-size: 0.88em;
  line-height: 1.3;
  background-color: rgba(26, 26, 46, 0.08);
}
.system-notification {
  border-left: 3px solid #b8960b;
  padding: 0.3em 0.8em;
  margin: 0.7em 0;
  font-family: "Courier New", Courier, monospace;
  font-size: 0.85em;
  font-style: italic;
  line-height: 1.3;
}
.system-pull {
  border: 2px solid #b8960b;
  border-radius: 4px;
  padding: 0.7em 1em;
  margin: 1.2em auto;
  max-width: 80%;
  text-align: center;
  font-family: "Courier New", Courier, monospace;
  font-size: 0.9em;
  line-height: 1.4;
  background-color: rgba(42, 26, 10, 0.06);
}
.system-quest {
  border: 1.5px dashed #8888aa;
  border-radius: 3px;
  padding: 0.5em 0.9em;
  margin: 1em 0;
  font-family: "Courier New", Courier, monospace;
  font-size: 0.88em;
  line-height: 1.35;
}
@media (prefers-color-scheme: dark) {
  .system-box {
    border-color: #aaaacc;
    background-color: rgba(200, 200, 255, 0.08);
  }
  .system-notification {
    border-left-color: #ffd700;
  }
  .system-pull {
    border-color: #ffd700;
    background-color: rgba(255, 215, 0, 0.06);
  }
  .system-quest {
    border-color: #aaaacc;
  }
}
"#;

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_source() -> EpubSource {
        EpubSource {
            title: "Test Novel".to_string(),
            author: Some("Jane Doe".to_string()),
            language: "en".to_string(),
            books: vec![EpubBook {
                number: 1,
                title: None,
                chapters: vec![
                    EpubChapter {
                        number: 1,
                        title: Some("The Beginning".to_string()),
                        body: "It was a dark and stormy night.\n\nThe rain fell in sheets."
                            .to_string(),
                    },
                    EpubChapter {
                        number: 2,
                        title: None,
                        body: "Morning came slowly.\n\nShe opened her eyes.".to_string(),
                    },
                ],
            }],
        }
    }

    #[test]
    fn builds_valid_epub_archive() {
        let bytes = build_epub(&sample_source()).unwrap();
        let mut archive = zip::ZipArchive::new(Cursor::new(&bytes)).unwrap();

        // Required EPUB entries exist.
        assert!(archive.by_name("mimetype").is_ok());
        assert!(archive.by_name("META-INF/container.xml").is_ok());
        assert!(archive.by_name("OEBPS/content.opf").is_ok());
        assert!(archive.by_name("OEBPS/toc.xhtml").is_ok());
        assert!(archive.by_name("OEBPS/title.xhtml").is_ok());
        assert!(archive.by_name("OEBPS/style.css").is_ok());

        // Chapter files present.
        assert!(archive.by_name("OEBPS/ch_1_1.xhtml").is_ok());
        assert!(archive.by_name("OEBPS/ch_1_2.xhtml").is_ok());
    }

    #[test]
    fn mimetype_is_first_and_uncompressed() {
        let bytes = build_epub(&sample_source()).unwrap();
        let mut archive = zip::ZipArchive::new(Cursor::new(&bytes)).unwrap();
        let entry = archive.by_index(0).unwrap();
        assert_eq!(entry.name(), "mimetype");
        assert_eq!(entry.compression(), zip::CompressionMethod::Stored);
    }

    #[test]
    fn chapter_xhtml_contains_prose() {
        let bytes = build_epub(&sample_source()).unwrap();
        let mut archive = zip::ZipArchive::new(Cursor::new(&bytes)).unwrap();
        let mut content = String::new();
        std::io::Read::read_to_string(
            &mut archive.by_name("OEBPS/ch_1_1.xhtml").unwrap(),
            &mut content,
        )
        .unwrap();
        assert!(content.contains("dark and stormy night"));
        assert!(content.contains("<p>"));
        assert!(content.contains("The Beginning"));
    }

    #[test]
    fn multi_book_headings_include_book_name() {
        let source = EpubSource {
            title: "Epic".to_string(),
            author: None,
            language: "en".to_string(),
            books: vec![
                EpubBook {
                    number: 1,
                    title: Some("Winter".to_string()),
                    chapters: vec![EpubChapter {
                        number: 1,
                        title: None,
                        body: "Snow.".to_string(),
                    }],
                },
                EpubBook {
                    number: 2,
                    title: Some("Summer".to_string()),
                    chapters: vec![EpubChapter {
                        number: 1,
                        title: None,
                        body: "Sun.".to_string(),
                    }],
                },
            ],
        };
        let bytes = build_epub(&source).unwrap();
        let mut archive = zip::ZipArchive::new(Cursor::new(&bytes)).unwrap();
        let mut c1 = String::new();
        std::io::Read::read_to_string(&mut archive.by_name("OEBPS/ch_1_1.xhtml").unwrap(), &mut c1)
            .unwrap();
        assert!(c1.contains("Winter"));
        let mut c2 = String::new();
        std::io::Read::read_to_string(&mut archive.by_name("OEBPS/ch_2_1.xhtml").unwrap(), &mut c2)
            .unwrap();
        assert!(c2.contains("Summer"));
    }

    #[test]
    fn xml_special_chars_are_escaped() {
        let source = EpubSource {
            title: "A <Bold> & \"New\" Story".to_string(),
            author: None,
            language: "en".to_string(),
            books: vec![EpubBook {
                number: 1,
                title: None,
                chapters: vec![EpubChapter {
                    number: 1,
                    title: None,
                    body: "She said \"hello\" & waved.".to_string(),
                }],
            }],
        };
        let bytes = build_epub(&source).unwrap();
        let mut archive = zip::ZipArchive::new(Cursor::new(&bytes)).unwrap();
        let mut opf = String::new();
        std::io::Read::read_to_string(&mut archive.by_name("OEBPS/content.opf").unwrap(), &mut opf)
            .unwrap();
        assert!(opf.contains("&amp;"));
        assert!(opf.contains("&lt;Bold&gt;"));
    }

    #[test]
    fn system_fenced_blocks_render_as_styled_divs() {
        let source = EpubSource {
            title: "Drop D".to_string(),
            author: None,
            language: "en".to_string(),
            books: vec![EpubBook {
                number: 1,
                title: None,
                chapters: vec![EpubChapter {
                    number: 1,
                    title: Some("The Board".to_string()),
                    body: concat!(
                        "Viola blinked.\n\n",
                        "::: system-box\n",
                        "STAGE CRED EARNED: +2. Total: **6**.\n",
                        "POST-PERFORMANCE BANNER unlocked.\n",
                        ":::\n\n",
                        "::: system-notification\n",
                        "+1 Clout. Different girl, different context.\n",
                        ":::\n\n",
                        "::: system-pull\n",
                        "TIER 3: NOW WE'RE TALKING\n",
                        "THE COUNTERSTEER\n",
                        "Intimate Technique -- *Passive/Active hybrid*\n",
                        ":::\n\n",
                        "::: system-quest\n",
                        "Today's board:\n",
                        "- Practice (30+ min)\n",
                        "- Write something new\n",
                        ":::\n\n",
                        "Then the room came back."
                    )
                    .to_string(),
                }],
            }],
        };

        let bytes = build_epub(&source).unwrap();
        let mut archive = zip::ZipArchive::new(Cursor::new(&bytes)).unwrap();
        let mut chapter = String::new();
        std::io::Read::read_to_string(
            &mut archive.by_name("OEBPS/ch_1_1.xhtml").unwrap(),
            &mut chapter,
        )
        .unwrap();

        assert!(chapter.contains("<div class=\"system-box\">"));
        assert!(chapter.contains("<div class=\"system-notification\">"));
        assert!(chapter.contains("<div class=\"system-pull\">"));
        assert!(chapter.contains("<div class=\"system-quest\">"));
        assert!(chapter.contains("Total: <strong>6</strong>."));
        assert!(chapter.contains("<em>Passive/Active hybrid</em>"));
        assert!(chapter.contains("Total: <strong>6</strong>.<br/>\nPOST-PERFORMANCE"));
        assert!(chapter.contains("<p>Viola blinked.</p>"));
        assert!(chapter.contains("<p>Then the room came back.</p>"));
    }

    #[test]
    fn backtick_fenced_system_blocks_render_as_styled_divs() {
        let source = EpubSource {
            title: "Drop D".to_string(),
            author: None,
            language: "en".to_string(),
            books: vec![EpubBook {
                number: 1,
                title: None,
                chapters: vec![EpubChapter {
                    number: 1,
                    title: Some("The Board".to_string()),
                    body: concat!(
                        "Viola blinked.\n\n",
                        "```system-box\n",
                        "STAGE CRED EARNED: +2. Total: **6**.\n",
                        "POST-PERFORMANCE BANNER unlocked.\n",
                        "```\n\n",
                        "```system-notification\n",
                        "+1 Clout. Different girl, different context.\n",
                        "```\n\n",
                        "```system-pull\n",
                        "TIER 3: NOW WE'RE TALKING\n",
                        "THE COUNTERSTEER\n",
                        "Intimate Technique -- *Passive/Active hybrid*\n",
                        "```\n\n",
                        "```system-quest\n",
                        "Today's board:\n",
                        "- Practice (30+ min)\n",
                        "- Write something new\n",
                        "```\n\n",
                        "Then the room came back."
                    )
                    .to_string(),
                }],
            }],
        };

        let bytes = build_epub(&source).unwrap();
        let mut archive = zip::ZipArchive::new(Cursor::new(&bytes)).unwrap();
        let mut chapter = String::new();
        std::io::Read::read_to_string(
            &mut archive.by_name("OEBPS/ch_1_1.xhtml").unwrap(),
            &mut chapter,
        )
        .unwrap();

        assert!(chapter.contains("<div class=\"system-box\">"));
        assert!(chapter.contains("<div class=\"system-notification\">"));
        assert!(chapter.contains("<div class=\"system-pull\">"));
        assert!(chapter.contains("<div class=\"system-quest\">"));
        assert!(chapter.contains("Total: <strong>6</strong>."));
        assert!(chapter.contains("<em>Passive/Active hybrid</em>"));
        assert!(chapter.contains("Total: <strong>6</strong>.<br/>\nPOST-PERFORMANCE"));
        assert!(chapter.contains("<p>Viola blinked.</p>"));
        assert!(chapter.contains("<p>Then the room came back.</p>"));
        assert!(!chapter.contains("```"));
    }

    #[test]
    fn plain_system_fence_renders_as_default_system_box() {
        let source = EpubSource {
            title: "Drop D".to_string(),
            author: None,
            language: "en".to_string(),
            books: vec![EpubBook {
                number: 1,
                title: None,
                chapters: vec![EpubChapter {
                    number: 1,
                    title: None,
                    body: concat!(
                        "The air chimed.\n\n",
                        "```system\n",
                        "QUEST UPDATED: Hold the line.\n",
                        "```\n\n",
                        "She tightened her grip."
                    )
                    .to_string(),
                }],
            }],
        };

        let bytes = build_epub(&source).unwrap();
        let mut archive = zip::ZipArchive::new(Cursor::new(&bytes)).unwrap();
        let mut chapter = String::new();
        std::io::Read::read_to_string(
            &mut archive.by_name("OEBPS/ch_1_1.xhtml").unwrap(),
            &mut chapter,
        )
        .unwrap();

        assert!(chapter.contains("<div class=\"system-box\">QUEST UPDATED: Hold the line.</div>"));
        assert!(!chapter.contains("```system"));
        assert!(!chapter.contains("```"));
    }

    #[test]
    fn unknown_backtick_fence_falls_through_unchanged() {
        let source = EpubSource {
            title: "Drop D".to_string(),
            author: None,
            language: "en".to_string(),
            books: vec![EpubBook {
                number: 1,
                title: None,
                chapters: vec![EpubChapter {
                    number: 1,
                    title: None,
                    body: "```rust\nfn main() {}\n```".to_string(),
                }],
            }],
        };

        let bytes = build_epub(&source).unwrap();
        let mut archive = zip::ZipArchive::new(Cursor::new(&bytes)).unwrap();
        let mut chapter = String::new();
        std::io::Read::read_to_string(
            &mut archive.by_name("OEBPS/ch_1_1.xhtml").unwrap(),
            &mut chapter,
        )
        .unwrap();

        // Unknown info string ("rust") is NOT a system class, so the fence is
        // left alone and rendered as ordinary prose.
        assert!(!chapter.contains("<div class=\"rust\""));
        assert!(chapter.contains("```rust"));
    }

    #[test]
    fn system_styles_are_included_in_epub_stylesheet() {
        let bytes = build_epub(&sample_source()).unwrap();
        let mut archive = zip::ZipArchive::new(Cursor::new(&bytes)).unwrap();
        let mut css = String::new();
        std::io::Read::read_to_string(&mut archive.by_name("OEBPS/style.css").unwrap(), &mut css)
            .unwrap();

        assert!(css.contains(".system-box"));
        assert!(css.contains("border: 1.5px solid #8888aa"));
        assert!(css.contains(".system-notification"));
        assert!(css.contains("border-left: 3px solid #b8960b"));
        assert!(css.contains(".system-pull"));
        assert!(css.contains("max-width: 80%"));
        assert!(css.contains(".system-quest"));
        assert!(css.contains("border: 1.5px dashed #8888aa"));
        assert!(css.contains("@media (prefers-color-scheme: dark)"));
    }
}
