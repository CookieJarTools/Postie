//! Email bodies as styled text: HTML through html2text's rich output, plain text with its links and
//! quotes picked out.
//!
//! By default remote images and the sender's colours are left out, so nothing phones home and
//! nothing comes out dark-on-dark. The original view, chosen per message or per sender, keeps the
//! sender's colours for a white page and places the images in the flow.

use std::ops::Range;

use egui::text::{LayoutJob, TextFormat};
use egui::{Color32, FontId, Stroke};
use html2text::render::RichAnnotation;

use super::theme::{self, *};

const BODY: Color32 = Color32::from_rgb(0xdc, 0xdd, 0xe0);
const LINE_HEIGHT: f32 = 21.0;
/// The original view's page colours: dark text on white, as most mail is designed for.
pub const PAPER: Color32 = Color32::from_rgb(0xff, 0xff, 0xff);
const INK: Color32 = Color32::from_rgb(0x1f, 0x23, 0x28);
const INK_MUTED: Color32 = Color32::from_rgb(0x57, 0x60, 0x6a);
const INK_LINK: Color32 = Color32::from_rgb(0x09, 0x69, 0xda);
const INK_CODE: Color32 = Color32::from_rgb(0xf2, 0xf3, 0xf5);

pub struct Rendered {
    pub job: LayoutJob,
    /// Character ranges of links in `job.text`, with where each goes.
    pub links: Vec<(Range<usize>, String)>,
}

impl Rendered {
    pub fn link_at(&self, char_index: usize) -> Option<&str> {
        self.links.iter().find(|(range, _)| range.contains(&char_index)).map(|(_, url)| url.as_str())
    }
}

pub enum Piece {
    Text(Rendered),
    /// An image on a line of its own: where it comes from, and where clicking it goes.
    Image {
        src: String,
        link: Option<String>,
    },
}

pub struct Page {
    pub pieces: Vec<Piece>,
    /// Drawn for a white page, in the sender's colours.
    pub original: bool,
}

impl Page {
    pub fn is_blank(&self) -> bool {
        self.pieces.iter().all(|piece| matches!(piece, Piece::Text(text) if text.job.text.trim().is_empty()))
    }

    pub fn images(&self) -> impl Iterator<Item = &str> {
        self.pieces.iter().filter_map(|piece| match piece {
            Piece::Image { src, .. } => Some(src.as_str()),
            Piece::Text(_) => None,
        })
    }
}

#[derive(Default)]
struct Builder {
    job: LayoutJob,
    links: Vec<(Range<usize>, String)>,
    chars: usize,
    pieces: Vec<Piece>,
    /// Line breaks waiting for the next text, so none are left dangling before an image.
    newlines: usize,
    /// Everything in the fixed-width face, as the Monospace message text setting asks.
    mono: bool,
    original: bool,
}

#[derive(Clone, Copy, Default)]
struct Style {
    strong: bool,
    italic: bool,
    code: bool,
    strike: bool,
    link: bool,
    quote: bool,
    heading: bool,
    colour: Option<Color32>,
    background: Option<Color32>,
}

fn luminance(color: Color32) -> f32 {
    (0.2126 * color.r() as f32 + 0.7152 * color.g() as f32 + 0.0722 * color.b() as f32) / 255.0
}

impl Builder {
    fn push(&mut self, text: &str, style: Style, url: Option<&str>) {
        if text.is_empty() {
            return;
        }
        if !self.job.text.is_empty() && self.newlines > 0 {
            let breaks = "\n".repeat(self.newlines.min(2));
            self.append(&breaks, Style::default(), None);
        }
        self.newlines = 0;
        self.append(text, style, url);
    }

    fn append(&mut self, text: &str, style: Style, url: Option<&str>) {
        let font = if self.mono {
            FontId::monospace(if style.heading { 14.0 } else { 13.0 })
        } else if style.code {
            FontId::monospace(12.5)
        } else if style.heading {
            theme::semibold(15.0)
        } else if style.strong {
            theme::semibold(13.5)
        } else {
            theme::regular(13.5)
        };
        let color = if self.original {
            // The sender's colour, unless it would vanish on the white page without its own background.
            let own = style.colour.filter(|c| style.background.is_some() || luminance(*c) < 0.8);
            match (style.link, style.quote) {
                (true, _) => own.unwrap_or(INK_LINK),
                (_, true) => INK_MUTED,
                _ => own.unwrap_or(INK),
            }
        } else if style.link {
            ACCENT_TEXT
        } else if style.quote {
            FG_MUTED
        } else if self.mono && (style.strong || style.heading) {
            // The fixed-width face has no bold, so strong text is brighter instead.
            FG
        } else {
            BODY
        };
        let background = match (self.original, style.code) {
            (true, _) => style.background.unwrap_or(if style.code { INK_CODE } else { Color32::TRANSPARENT }),
            (false, true) => BG_ACTIVE,
            (false, false) => Color32::TRANSPARENT,
        };
        let format = TextFormat {
            font_id: font,
            color,
            italics: style.italic,
            line_height: Some(LINE_HEIGHT),
            underline: if style.link { Stroke::new(1.0, color.gamma_multiply(0.5)) } else { Stroke::NONE },
            strikethrough: if style.strike { Stroke::new(1.0, color) } else { Stroke::NONE },
            background,
            ..Default::default()
        };
        let count = text.chars().count();
        if let Some(url) = url {
            self.links.push((self.chars..self.chars + count, url.to_owned()));
        }
        self.chars += count;
        self.job.append(text, 0.0, format);
    }

    fn newline(&mut self) {
        self.newlines += 1;
    }

    fn flush(&mut self) {
        if self.job.text.is_empty() {
            return;
        }
        let mut job = std::mem::take(&mut self.job);
        job.wrap.break_anywhere = false;
        self.pieces.push(Piece::Text(Rendered { job, links: std::mem::take(&mut self.links) }));
        self.chars = 0;
    }

    fn image(&mut self, src: &str, link: Option<&str>) {
        self.flush();
        self.newlines = 0;
        self.pieces.push(Piece::Image { src: src.to_owned(), link: link.map(str::to_owned) });
    }

    fn finish(mut self) -> Page {
        self.flush();
        Page { pieces: self.pieces, original: self.original }
    }
}

/// The text with no styling, for quoting in replies when a message only has HTML.
pub fn to_plain(html: &str) -> String {
    html2text::config::plain()
        .raw_mode(true)
        .link_footnotes(false)
        .string_from_read(html.as_bytes(), 78)
        .unwrap_or_default()
}

pub fn from_html(html: &str, mono: bool, original: bool) -> Page {
    let config = html2text::config::rich().raw_mode(true).link_footnotes(false).no_link_wrapping();
    // The sender's styles, colours included, only matter on the original view's white page.
    let config = if original { config.use_doc_css() } else { config };
    let html = if original { give_images_alt(html) } else { html.to_owned() };
    let Ok(lines) = config.lines_from_read(html.as_bytes(), 400) else {
        return from_text(&to_plain(&html), mono);
    };
    let mut builder = Builder { mono, original, ..Default::default() };
    let mut started = false;
    for line in &lines {
        let text: String = line.tagged_strings().map(|piece| piece.s.as_str()).collect();
        if text.trim().is_empty() {
            if started {
                builder.newline();
            }
            continue;
        }
        if started {
            builder.newline();
        }
        started = true;
        // html2text marks headings the Markdown way; show them as headings instead.
        let marks = text.chars().take_while(|c| *c == '#').count();
        let heading = (1..=6).contains(&marks) && text[marks..].starts_with(' ');
        let mut skip = if heading { marks + 1 } else { 0 };
        for piece in line.tagged_strings() {
            let shown: &str = if skip > 0 {
                let cut = piece.s.char_indices().nth(skip).map_or(piece.s.len(), |(at, _)| at);
                skip -= piece.s[..cut].chars().count();
                &piece.s[cut..]
            } else {
                &piece.s
            };
            let mut style = Style { heading, ..Style::default() };
            let mut url = None;
            let mut image = None;
            for annotation in &piece.tag {
                match annotation {
                    RichAnnotation::Strong => style.strong = true,
                    RichAnnotation::Emphasis => style.italic = true,
                    RichAnnotation::Code | RichAnnotation::Preformat(_) => style.code = true,
                    RichAnnotation::Strikeout => style.strike = true,
                    RichAnnotation::Link(target) => {
                        style.link = true;
                        url = Some(target.as_str());
                    }
                    RichAnnotation::Image(src) => image = Some(src.as_str()),
                    RichAnnotation::Colour(c) => style.colour = Some(Color32::from_rgb(c.r, c.g, c.b)),
                    RichAnnotation::BgColour(c) => style.background = Some(Color32::from_rgb(c.r, c.g, c.b)),
                    _ => {}
                }
            }
            let url = url.filter(|url| url.starts_with("http") || url.starts_with("mailto:"));
            match image {
                Some(src) if original => builder.image(src, url),
                _ => builder.push(shown, style, url),
            }
        }
    }
    builder.finish()
}

/// html2text drops an image with no alt text when it's inside a link, which is how most
/// newsletters make their banners clickable. An alt put first wins over the sender's own, and the
/// original view shows the image rather than its alt, so nothing is lost.
fn give_images_alt(html: &str) -> String {
    let lower = html.to_ascii_lowercase();
    let mut out = String::with_capacity(html.len() + 64);
    let mut last = 0;
    for (at, _) in lower.match_indices("<img") {
        let after = lower.as_bytes().get(at + 4).copied();
        if matches!(after, Some(b' ' | b'\t' | b'\n' | b'\r' | b'/' | b'>')) {
            out.push_str(&html[last..at + 4]);
            out.push_str(" alt=\"image\"");
            last = at + 4;
        }
    }
    out.push_str(&html[last..]);
    out
}

pub fn from_text(text: &str, mono: bool) -> Page {
    let mut builder = Builder { mono, ..Default::default() };
    let mut started = false;
    for line in text.trim_end().lines() {
        let line = line.trim_end_matches('\r');
        if line.trim().is_empty() {
            if started {
                builder.newline();
            }
            continue;
        }
        if started {
            builder.newline();
        }
        started = true;
        let style = Style { quote: line.trim_start().starts_with('>'), ..Default::default() };
        push_linkified(&mut builder, line, style);
    }
    builder.finish()
}

/// Plain text with its web addresses made into links.
fn push_linkified(builder: &mut Builder, line: &str, style: Style) {
    let mut rest = line;
    while let Some(start) = find_url(rest) {
        builder.push(&rest[..start], style, None);
        let tail = &rest[start..];
        let end = tail.find(|c: char| c.is_whitespace() || matches!(c, '<' | '>' | '"')).unwrap_or(tail.len());
        let url = tail[..end].trim_end_matches(['.', ',', ')', ';', ':', '!', '?', '\'']);
        let target = if url.starts_with("www.") { format!("https://{url}") } else { url.to_owned() };
        builder.push(url, Style { link: true, ..style }, Some(&target));
        rest = &tail[url.len()..];
    }
    builder.push(rest, style, None);
}

fn find_url(text: &str) -> Option<usize> {
    ["https://", "http://", "www."].iter().filter_map(|prefix| text.find(prefix)).min()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn only_text(page: &Page) -> &Rendered {
        match page.pieces.as_slice() {
            [Piece::Text(text)] => text,
            _ => panic!("expected one text piece, got {}", page.pieces.len()),
        }
    }

    fn section_colour(text: &Rendered, wanted: &str) -> Color32 {
        text.job
            .sections
            .iter()
            .find(|s| text.job.text[usize::from(s.byte_range.start)..usize::from(s.byte_range.end)].contains(wanted))
            .map(|s| s.format.color)
            .expect("section")
    }

    #[test]
    fn links_in_plain_text_are_styled() {
        let page = from_text("Try https://example.com/menu today.", false);
        let rendered = only_text(&page);
        assert_eq!(rendered.links.len(), 1);
        assert_eq!(rendered.links[0].1, "https://example.com/menu");
        assert_eq!(section_colour(rendered, "https://example.com/menu"), ACCENT_TEXT);
        assert_eq!(rendered.link_at(6), Some("https://example.com/menu"));
    }

    #[test]
    fn monospace_setting_covers_everything() {
        let page = from_html("<p>Hello <b>there</b></p>", true, false);
        let rendered = only_text(&page);
        assert!(
            rendered.job.sections.iter().all(|section| section.format.font_id.family == egui::FontFamily::Monospace)
        );
    }

    #[test]
    fn html_links_and_bold() {
        let page = from_html(
            "<p>We shipped <b>rules</b>. <a href='https://linear.app/changelog'>Read more</a></p>",
            false,
            false,
        );
        let rendered = only_text(&page);
        assert!(rendered.links.iter().any(|(_, url)| url == "https://linear.app/changelog"), "{:?}", rendered.links);
        assert!(rendered.job.text.contains("Read more"), "{}", rendered.job.text);
    }

    const NEWSLETTER: &str = "<p>Hello</p><a href='https://x.com/go'><img src='https://x.com/hero.png'></a>\
        <p style='color:#ff0000'>Red news</p><p style='color:#ffffff'>Ghost</p><img src='cid:logo@x' alt='Logo'>";

    #[test]
    fn original_view_places_images_in_the_flow() {
        let page = from_html(NEWSLETTER, false, true);
        let images: Vec<&str> = page.images().collect();
        assert_eq!(images, ["https://x.com/hero.png", "cid:logo@x"]);
        assert!(matches!(&page.pieces[1], Piece::Image { link: Some(link), .. } if link == "https://x.com/go"));
        let Piece::Text(after) = &page.pieces[2] else { panic!("text after the image") };
        assert!(!after.job.text.starts_with('\n'), "no stray line break after an image");
        assert_eq!(section_colour(after, "Red news"), Color32::from_rgb(0xff, 0, 0));
        assert_eq!(section_colour(after, "Ghost"), INK, "white text would vanish on the white page");
    }

    #[test]
    fn headings_lose_their_markdown_marks() {
        let page = from_html("<h2>Autumn sale</h2><p>Hi</p><h3><b>Bold</b> heading</h3>", false, false);
        let text = &only_text(&page).job.text;
        assert!(text.starts_with("Autumn sale"), "{text:?}");
        assert!(text.contains("Bold heading") && !text.contains('#'), "{text:?}");
    }

    #[test]
    fn safe_view_never_places_images() {
        let page = from_html(NEWSLETTER, false, false);
        assert_eq!(page.images().count(), 0);
        let rendered = only_text(&page);
        assert_eq!(section_colour(rendered, "Red news"), BODY, "the sender's colours stay out by default");
    }
}
