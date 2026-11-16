//! Attachment previews: a thumbnail on each attachment card, and a large preview of images, PDFs and
//! text files inside Postie. Both are made on background threads, which read the attachment from the
//! store themselves, and kept for the session.
//!
//! PDFs are drawn by poppler's `pdftoppm`, when it's installed; without it they get a type tile like
//! any other file.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail};
use egui::{
    Align, Align2, Color32, CornerRadius, CursorIcon, Frame, Id, Key, Layout, Margin, Modifiers, Rect, Sense, Stroke,
    StrokeKind, TextureHandle, Ui, vec2,
};
use image::DynamicImage;

use super::icons::{self, Icon};
use super::theme::{self, *};
use super::widgets::{self, Kind as ButtonKind};
use super::{Action, App};
use crate::mail::parse::{self, Attachment};
use crate::store;

const CARD_WIDTH: f32 = 176.0;
const THUMB_HEIGHT: f32 = 108.0;
const CARD_HEIGHT: f32 = THUMB_HEIGHT + 46.0;
/// Thumbnails are made at twice the card's size, so they stay sharp on high-DPI screens.
const THUMB_PIXELS: (u32, u32) = (352, 216);
/// The longest side a previewed image keeps; bigger photos are scaled down to spare the GPU.
const FULL_PIXELS: u32 = 2560;
const PDF_PAGE_WIDTH: u32 = 1100;
const PDF_PAGES: usize = 30;
const TEXT_PREVIEW_BYTES: usize = 512 * 1024;
const TABLE_ROWS: usize = 2000;
/// Attachments bigger than this aren't previewed; they open in their own app.
const MAX_PREVIEW_BYTES: usize = 40 * 1024 * 1024;
const RENDER_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Image,
    Pdf,
    /// A spreadsheet saved as CSV or TSV, shown as a table.
    Table,
    Text,
    Other,
}

impl Kind {
    pub fn previewable(self) -> bool {
        self != Kind::Other
    }
}

fn extension(name: &str) -> String {
    name.rsplit_once('.').map(|(_, ext)| ext.to_ascii_lowercase()).unwrap_or_default()
}

fn pdftoppm_installed() -> bool {
    static INSTALLED: OnceLock<bool> = OnceLock::new();
    *INSTALLED
        .get_or_init(|| Command::new("pdftoppm").arg("-v").stdout(Stdio::null()).stderr(Stdio::null()).status().is_ok())
}

pub fn kind_of(attachment: &Attachment) -> Kind {
    let ext = extension(&attachment.name);
    let mime = attachment.mime.to_ascii_lowercase();
    if attachment.size > MAX_PREVIEW_BYTES {
        return Kind::Other;
    }
    if matches!(ext.as_str(), "png" | "jpg" | "jpeg" | "gif" | "webp")
        || matches!(mime.as_str(), "image/png" | "image/jpeg" | "image/gif" | "image/webp")
    {
        return Kind::Image;
    }
    if (ext == "pdf" || mime == "application/pdf") && pdftoppm_installed() {
        return Kind::Pdf;
    }
    if matches!(ext.as_str(), "csv" | "tsv") || matches!(mime.as_str(), "text/csv" | "text/tab-separated-values") {
        return Kind::Table;
    }
    const TEXT: &[&str] = &[
        "txt", "md", "log", "json", "xml", "yml", "yaml", "toml", "ini", "cfg", "conf", "ics", "vcf", "html", "htm",
        "css", "js", "ts", "rs", "py", "c", "h", "cpp", "hpp", "cs", "java", "go", "sh", "sql", "diff", "patch",
    ];
    if TEXT.contains(&ext.as_str()) || mime.starts_with("text/") {
        return Kind::Text;
    }
    Kind::Other
}

#[derive(Clone)]
enum Thumb {
    Loading,
    Image(TextureHandle),
    Text(String),
    Failed,
}

#[derive(Clone)]
enum Full {
    Loading,
    Image(TextureHandle),
    /// Rendered pages, and how many the document has in all.
    Pages(Vec<TextureHandle>, usize),
    /// Rows of cells, and whether there were more rows than these.
    Table(Arc<Vec<Vec<String>>>, bool),
    Text(String),
    Failed(String),
}

/// Thumbnails and full previews by message and attachment, shared with the threads making them.
#[derive(Default)]
pub struct Previews {
    thumbs: Arc<Mutex<HashMap<(i64, usize), Thumb>>>,
    full: Arc<Mutex<HashMap<(i64, usize), Full>>>,
}

impl Previews {
    fn thumb(&self, ctx: &egui::Context, message: i64, attachment: &Attachment, kind: Kind) -> Thumb {
        let key = (message, attachment.index);
        if let Some(thumb) = self.thumbs.lock().unwrap().get(&key) {
            return thumb.clone();
        }
        self.thumbs.lock().unwrap().insert(key, Thumb::Loading);
        let (thumbs, ctx) = (self.thumbs.clone(), ctx.clone());
        std::thread::spawn(move || {
            let thumb = make_thumb(&ctx, message, key.1, kind).unwrap_or_else(|err| {
                log::info!("no thumbnail for attachment {key:?}: {err:#}");
                Thumb::Failed
            });
            thumbs.lock().unwrap().insert(key, thumb);
            ctx.request_repaint();
        });
        Thumb::Loading
    }

    fn full(&self, ctx: &egui::Context, message: i64, attachment: &Attachment, kind: Kind) -> Full {
        let key = (message, attachment.index);
        if let Some(full) = self.full.lock().unwrap().get(&key) {
            return full.clone();
        }
        self.full.lock().unwrap().insert(key, Full::Loading);
        let (full, ctx) = (self.full.clone(), ctx.clone());
        std::thread::spawn(move || {
            let made = make_full(&ctx, message, key.1, kind).unwrap_or_else(|err| Full::Failed(format!("{err:#}")));
            full.lock().unwrap().insert(key, made);
            ctx.request_repaint();
        });
        Full::Loading
    }

    /// Lets go of a full preview's textures once it's closed; thumbnails stay.
    pub fn release(&self, message: i64, index: usize) {
        self.full.lock().unwrap().remove(&(message, index));
    }

    /// Lets go of everything made for other messages, so reading through a mailbox doesn't pile
    /// up textures.
    pub fn keep_only(&self, message: i64) {
        self.thumbs.lock().unwrap().retain(|&(id, _), _| id == message);
        self.full.lock().unwrap().retain(|&(id, _), _| id == message);
    }
}

fn attachment_bytes(message: i64, index: usize) -> Result<Vec<u8>> {
    let db = store::open()?;
    let raw = store::raw(&db, message)?.ok_or_else(|| anyhow!("the message hasn't downloaded yet"))?;
    let (_, bytes) = parse::attachment(&raw, index).ok_or_else(|| anyhow!("the attachment is missing"))?;
    Ok(bytes)
}

fn texture(ctx: &egui::Context, name: String, image: &DynamicImage) -> TextureHandle {
    let rgba = image.to_rgba8();
    let size = [rgba.width() as usize, rgba.height() as usize];
    ctx.load_texture(name, egui::ColorImage::from_rgba_unmultiplied(size, rgba.as_raw()), egui::TextureOptions::LINEAR)
}

fn make_thumb(ctx: &egui::Context, message: i64, index: usize, kind: Kind) -> Result<Thumb> {
    let bytes = attachment_bytes(message, index)?;
    let (width, height) = THUMB_PIXELS;
    let name = format!("postie-thumb-{message}-{index}");
    Ok(match kind {
        Kind::Image => {
            let image = image::load_from_memory(&bytes)?;
            Thumb::Image(texture(
                ctx,
                name,
                &image.resize_to_fill(width, height, image::imageops::FilterType::Triangle),
            ))
        }
        Kind::Pdf => {
            let (pages, _) = render_pdf(&bytes, 1, width)?;
            let page = pages.into_iter().next().ok_or_else(|| anyhow!("the PDF has no pages"))?;
            // The top of the first page, where a document says what it is.
            let top = page.crop_imm(0, 0, page.width(), height.min(page.height()));
            Thumb::Image(texture(ctx, name, &top))
        }
        Kind::Table | Kind::Text => {
            let head = String::from_utf8_lossy(&bytes[..bytes.len().min(4096)]).into_owned();
            Thumb::Text(
                head.lines()
                    .take(9)
                    .map(|line| line.chars().take(60).collect::<String>())
                    .collect::<Vec<_>>()
                    .join("\n"),
            )
        }
        Kind::Other => Thumb::Failed,
    })
}

fn make_full(ctx: &egui::Context, message: i64, index: usize, kind: Kind) -> Result<Full> {
    let bytes = attachment_bytes(message, index)?;
    Ok(match kind {
        Kind::Image => {
            let image = image::load_from_memory(&bytes).context("couldn't read the image")?;
            let image = if image.width().max(image.height()) > FULL_PIXELS {
                image.resize(FULL_PIXELS, FULL_PIXELS, image::imageops::FilterType::CatmullRom)
            } else {
                image
            };
            Full::Image(texture(ctx, format!("postie-preview-{message}-{index}"), &image))
        }
        Kind::Pdf => {
            let (pages, total) = render_pdf(&bytes, PDF_PAGES, PDF_PAGE_WIDTH)?;
            let textures = pages
                .iter()
                .enumerate()
                .map(|(page, image)| texture(ctx, format!("postie-preview-{message}-{index}-{page}"), image))
                .collect();
            Full::Pages(textures, total)
        }
        Kind::Table => {
            let text = String::from_utf8_lossy(&bytes[..bytes.len().min(TEXT_PREVIEW_BYTES)]);
            let delimiter = if text.lines().next().is_some_and(|line| line.contains('\t')) { '\t' } else { ',' };
            let (rows, more) = parse_table(&text, delimiter, TABLE_ROWS);
            Full::Table(Arc::new(rows), more || bytes.len() > TEXT_PREVIEW_BYTES)
        }
        Kind::Text => {
            let text = String::from_utf8_lossy(&bytes[..bytes.len().min(TEXT_PREVIEW_BYTES)]).into_owned();
            Full::Text(text)
        }
        Kind::Other => bail!("there's no preview for this kind of file"),
    })
}

/// The first `pages` pages of a PDF, `width` pixels wide, and how many pages it has. Drawn by
/// `pdftoppm` from a private temporary copy, which is removed afterwards.
fn render_pdf(bytes: &[u8], pages: usize, width: u32) -> Result<(Vec<DynamicImage>, usize)> {
    let dir = scratch_dir()?;
    let result = (|| {
        let pdf = dir.join("attachment.pdf");
        std::fs::write(&pdf, bytes)?;
        let mut child = Command::new("pdftoppm")
            .args(["-png", "-f", "1", "-l", &pages.to_string(), "-scale-to-x", &width.to_string(), "-scale-to-y", "-1"])
            .arg(&pdf)
            .arg(dir.join("page"))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .context("couldn't run pdftoppm")?;
        let started = Instant::now();
        loop {
            if child.try_wait()?.is_some() {
                break;
            }
            if started.elapsed() > RENDER_TIMEOUT {
                let _ = child.kill();
                bail!("the PDF took too long to draw");
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)?
            .filter_map(|entry| entry.ok().map(|entry| entry.path()))
            .filter(|path| path.extension().is_some_and(|ext| ext == "png"))
            .collect();
        files.sort();
        let images = files.iter().map(image::open).collect::<Result<Vec<_>, _>>()?;
        if images.is_empty() {
            bail!("pdftoppm couldn't read the PDF");
        }
        let total = page_count(&pdf).unwrap_or(images.len()).max(images.len());
        Ok((images, total))
    })();
    let _ = std::fs::remove_dir_all(&dir);
    result
}

fn page_count(pdf: &Path) -> Option<usize> {
    let output = Command::new("pdfinfo").arg(pdf).stderr(Stdio::null()).output().ok()?;
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .find_map(|line| line.strip_prefix("Pages:").and_then(|count| count.trim().parse().ok()))
}

/// A new folder only the user can read, for one PDF at a time: in the session's runtime folder,
/// which is private and cleared at logout, or the temporary folder without one.
fn scratch_dir() -> Result<PathBuf> {
    use std::os::unix::fs::DirBuilderExt;
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let base = dirs::runtime_dir().unwrap_or_else(std::env::temp_dir);
    let name =
        format!("postie-preview-{}-{}", std::process::id(), NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed));
    let dir = base.join(name);
    // Not recursive, so this fails rather than reuse a folder someone else made.
    std::fs::DirBuilder::new().mode(0o700).create(&dir).context("couldn't make a folder to draw the PDF in")?;
    Ok(dir)
}

/// CSV or TSV split into rows of cells, following quotes, up to `limit` rows; and whether there
/// were more.
fn parse_table(text: &str, delimiter: char, limit: usize) -> (Vec<Vec<String>>, bool) {
    let (mut rows, mut row, mut cell) = (Vec::new(), Vec::new(), String::new());
    let (mut quoted, mut chars) = (false, text.chars().peekable());
    while let Some(c) = chars.next() {
        match c {
            '"' if quoted && chars.peek() == Some(&'"') => {
                cell.push('"');
                chars.next();
            }
            '"' if quoted => quoted = false,
            '"' if cell.is_empty() => quoted = true,
            '\r' if !quoted => {}
            '\n' if !quoted => {
                row.push(std::mem::take(&mut cell));
                rows.push(std::mem::take(&mut row));
                if rows.len() == limit {
                    return (rows, chars.any(|c| !c.is_whitespace()));
                }
            }
            c if c == delimiter && !quoted => row.push(std::mem::take(&mut cell)),
            c => cell.push(c),
        }
    }
    if !cell.is_empty() || !row.is_empty() {
        row.push(cell);
        rows.push(row);
    }
    (rows, false)
}

fn kind_label(attachment: &Attachment) -> String {
    let ext = extension(&attachment.name);
    if ext.is_empty() { "File".into() } else { ext.to_uppercase() }
}

/// An attachment as a card: a thumbnail, its name, and its type and size. Clicking previews it
/// when Postie can, otherwise opens it in its own app; a save button shows on hover.
pub fn card(app: &App, ui: &mut Ui, message: i64, attachment: &Attachment, out: &mut Vec<Action>) {
    let kind = kind_of(attachment);
    let (rect, response) = ui.allocate_exact_size(vec2(CARD_WIDTH, CARD_HEIGHT), Sense::click());
    if !ui.is_rect_visible(rect) {
        return;
    }
    let save = Rect::from_min_size(rect.right_top() + vec2(-32.0, 6.0), vec2(26.0, 26.0));
    let save_response = ui.interact(save, ui.scope_id().with(("save", message, attachment.index)), Sense::click());
    let hovered = response.hovered() || save_response.hovered();
    let painter = ui.painter();
    painter.rect(
        rect,
        CornerRadius::same(10),
        BG_PANEL,
        Stroke::new(1.0, if hovered { FG_FAINT } else { LINE_STRONG }),
        StrokeKind::Inside,
    );

    let top = CornerRadius { nw: 9, ne: 9, sw: 0, se: 0 };
    let thumb = Rect::from_min_size(rect.min, vec2(CARD_WIDTH, THUMB_HEIGHT)).shrink(1.0);
    painter.rect_filled(thumb, top, BG_SOFT);
    let state =
        if kind.previewable() { app.previews.thumb(ui.ctx(), message, attachment, kind) } else { Thumb::Failed };
    match state {
        Thumb::Image(texture) => {
            egui::Image::from_texture(egui::load::SizedTexture::from_handle(&texture))
                .corner_radius(top)
                .paint_at(ui, thumb);
        }
        Thumb::Text(text) => {
            let clip = ui.painter_at(thumb.shrink2(vec2(10.0, 8.0)));
            clip.text(
                thumb.left_top() + vec2(10.0, 8.0),
                Align2::LEFT_TOP,
                text,
                egui::FontId::monospace(9.5),
                FG_MUTED,
            );
        }
        Thumb::Loading => {
            let t = ui.input(|i| i.time) as f32;
            let center = thumb.center();
            let points: Vec<egui::Pos2> =
                (0..=16).map(|i| center + 8.0 * egui::Vec2::angled(t * 5.0 + i as f32 / 16.0 * 4.4)).collect();
            ui.painter().add(egui::Shape::line(points, Stroke::new(1.6, FG_SUBTLE)));
            ui.ctx().request_repaint();
        }
        Thumb::Failed => {
            let (bg, fg) = theme::tint(&kind_label(attachment));
            let tile = Rect::from_center_size(thumb.center(), vec2(64.0, 46.0));
            ui.painter().rect_filled(tile, CornerRadius::same(9), bg);
            ui.painter().text(tile.center(), Align2::CENTER_CENTER, kind_label(attachment), theme::semibold(15.0), fg);
        }
    }
    let painter = ui.painter();
    painter.hline(rect.x_range().shrink(1.0), thumb.bottom(), Stroke::new(1.0, LINE));
    widgets::paint_truncated(
        ui,
        egui::pos2(rect.left() + 11.0, thumb.bottom() + 7.0),
        &attachment.name,
        theme::medium(12.5),
        FG,
        CARD_WIDTH - 22.0,
    );
    let meta = format!("{} · {}", kind_label(attachment), super::human_size(attachment.size as u64));
    ui.painter().text(
        egui::pos2(rect.left() + 11.0, thumb.bottom() + 27.0),
        Align2::LEFT_TOP,
        meta,
        theme::regular(11.5),
        FG_SUBTLE,
    );

    if hovered {
        let fill =
            if save_response.hovered() { Color32::from_black_alpha(220) } else { Color32::from_black_alpha(160) };
        ui.painter().rect_filled(save, CornerRadius::same(6), fill);
        icons::paint(ui.painter(), save.shrink(6.0), Icon::Download, Color32::WHITE);
    }
    if save_response.on_hover_text("Save as…").on_hover_cursor(CursorIcon::PointingHand).clicked() {
        out.push(Action::SaveAttachment(message, attachment.index));
        return;
    }
    let tip = if kind.previewable() {
        format!("Preview {}", attachment.name)
    } else {
        format!("Open {} ({})", attachment.name, attachment.mime)
    };
    if response.on_hover_text(tip).on_hover_cursor(CursorIcon::PointingHand).clicked() {
        out.push(if kind.previewable() {
            Action::PreviewAttachment(message, attachment.index)
        } else {
            Action::OpenAttachment(message, attachment.index)
        });
    }
}

/// The large preview over the window, for the attachment `App::preview_open` names.
pub fn window(app: &App, ctx: &egui::Context, out: &mut Vec<Action>) {
    let Some((message, index)) = app.preview_open else { return };
    let attachments = app
        .reader
        .as_ref()
        .filter(|reader| reader.id() == message)
        .map(|reader| reader.attachments())
        .unwrap_or_default();
    let Some(position) = attachments.iter().position(|attachment| attachment.index == index) else {
        out.push(Action::ClosePreview);
        return;
    };
    let attachment = &attachments[position];
    let kind = kind_of(attachment);
    let step = |delta: isize| {
        let next = (position as isize + delta).rem_euclid(attachments.len() as isize) as usize;
        Action::PreviewAttachment(message, attachments[next].index)
    };
    if attachments.len() > 1 {
        if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::ArrowRight)) {
            out.push(step(1));
        }
        if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::ArrowLeft)) {
            out.push(step(-1));
        }
    }

    let screen = ctx.content_rect();
    let size = vec2((screen.width() - 120.0).clamp(480.0, 1280.0), (screen.height() - 100.0).clamp(360.0, 1000.0));
    let frame = Frame::new().fill(BG_PANEL).stroke(Stroke::new(1.0, LINE_STRONG)).corner_radius(12);
    let response = egui::Modal::new(Id::unique("attachment-preview"))
        .backdrop_color(Color32::from_black_alpha(170))
        .frame(frame)
        .show(ctx, |ui| {
            ui.set_width(size.x);
            ui.spacing_mut().item_spacing.y = 0.0;
            Frame::new().inner_margin(Margin { left: 18, right: 10, top: 10, bottom: 10 }).show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 1.0;
                        widgets::label(ui, &attachment.name, theme::semibold(14.0), FG);
                        let mut meta =
                            format!("{} · {}", kind_label(attachment), super::human_size(attachment.size as u64));
                        if attachments.len() > 1 {
                            meta.push_str(&format!(" · {} of {}", position + 1, attachments.len()));
                        }
                        widgets::label(ui, meta, theme::regular(12.0), FG_SUBTLE);
                    });
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if widgets::icon_button(ui, Icon::Close, "Close (Esc)").clicked() {
                            out.push(Action::ClosePreview);
                        }
                        ui.add_space(4.0);
                        if widgets::icon_text_button(ui, Icon::External, "Open", ButtonKind::Secondary)
                            .on_hover_text("Open in its own app")
                            .clicked()
                        {
                            out.push(Action::OpenAttachment(message, index));
                        }
                        if widgets::icon_text_button(ui, Icon::Download, "Save", ButtonKind::Ghost).clicked() {
                            out.push(Action::SaveAttachment(message, index));
                        }
                        if attachments.len() > 1 {
                            ui.add_space(6.0);
                            if widgets::icon_button(ui, Icon::ChevronRight, "Next (→)").clicked() {
                                out.push(step(1));
                            }
                            if widgets::icon_button(ui, Icon::ChevronLeft, "Previous (←)").clicked() {
                                out.push(step(-1));
                            }
                        }
                    });
                });
            });
            let line = ui.available_rect_before_wrap();
            ui.painter().hline(line.x_range(), line.top(), Stroke::new(1.0, LINE));
            let body = vec2(size.x, size.y - 58.0);
            let (area, _) = ui.allocate_exact_size(body, Sense::hover());
            ui.painter().rect_filled(area, CornerRadius { nw: 0, ne: 0, sw: 11, se: 11 }, BG_SOFT);
            let mut content = ui
                .new_child(egui::UiBuilder::new().max_rect(area.shrink(16.0)).layout(Layout::top_down(Align::Center)));
            let state = if kind.previewable() {
                app.previews.full(ctx, message, attachment, kind)
            } else {
                Full::Failed("There's no preview for this kind of file.".into())
            };
            show_full(&mut content, state, message, index, out);
        });
    if response.should_close() {
        out.push(Action::ClosePreview);
    }
}

fn show_full(ui: &mut Ui, state: Full, message: i64, index: usize, out: &mut Vec<Action>) {
    let available = ui.available_size();
    match state {
        Full::Loading => {
            ui.add_space(available.y / 2.0 - 20.0);
            ui.horizontal(|ui| {
                ui.add_space(available.x / 2.0 - 70.0);
                widgets::spinner(ui, 14.0);
                widgets::label(ui, "Making a preview…", theme::regular(13.0), FG_MUTED);
            });
        }
        Full::Image(texture) => {
            let native = texture.size_vec2();
            let scale = (available.x / native.x).min(available.y / native.y).min(1.0);
            let shown = native * scale;
            ui.add_space(((available.y - shown.y) / 2.0).max(0.0));
            ui.add(egui::Image::from_texture(egui::load::SizedTexture::new(texture.id(), shown)).corner_radius(4));
        }
        Full::Pages(pages, total) => {
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                ui.vertical_centered(|ui| {
                    for page in &pages {
                        let native = page.size_vec2();
                        let width = native.x.min(ui.available_width() - 8.0);
                        let shown = vec2(width, native.y * width / native.x);
                        ui.add(egui::Image::from_texture(egui::load::SizedTexture::new(page.id(), shown)));
                        ui.add_space(14.0);
                    }
                    if total > pages.len() {
                        widgets::note(
                            ui,
                            format!("Showing the first {} of {} pages. Open it to read the rest.", pages.len(), total),
                        );
                        ui.add_space(8.0);
                        if widgets::button(ui, "Open", ButtonKind::Secondary).clicked() {
                            out.push(Action::OpenAttachment(message, index));
                        }
                    }
                });
            });
        }
        Full::Table(rows, more) => {
            egui::ScrollArea::both().auto_shrink([false, false]).show(ui, |ui| {
                ui.with_layout(Layout::top_down(Align::Min), |ui| {
                    egui::Grid::new("attachment-table").striped(true).spacing(vec2(22.0, 7.0)).show(ui, |ui| {
                        for (number, row) in rows.iter().enumerate() {
                            let (font, color) = if number == 0 {
                                (theme::semibold(12.5), FG)
                            } else {
                                (theme::regular(12.5), FG_MUTED)
                            };
                            for cell in row {
                                let text: String = cell.chars().take(120).collect();
                                widgets::label(ui, text, font.clone(), color);
                            }
                            ui.end_row();
                        }
                    });
                    if more {
                        ui.add_space(12.0);
                        widgets::note(ui, format!("Showing the first {} rows. Open it to see the rest.", rows.len()));
                    }
                });
            });
        }
        Full::Text(text) => {
            egui::ScrollArea::both().auto_shrink([false, false]).show(ui, |ui| {
                ui.with_layout(Layout::top_down(Align::Min), |ui| {
                    ui.add(
                        egui::Label::new(egui::RichText::new(text).font(egui::FontId::monospace(12.5)).color(FG))
                            .selectable(true)
                            .extend(),
                    );
                });
            });
        }
        Full::Failed(reason) => {
            ui.add_space(available.y / 2.0 - 80.0);
            if widgets::empty_state(ui, Icon::File, "No preview", &reason, Some("Open in its app")) {
                out.push(Action::OpenAttachment(message, index));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn attachment(name: &str, mime: &str, size: usize) -> Attachment {
        Attachment { index: 0, name: name.into(), mime: mime.into(), size, inline: false }
    }

    #[test]
    fn picks_a_preview_by_type() {
        assert_eq!(kind_of(&attachment("Photo.JPG", "application/octet-stream", 10)), Kind::Image);
        assert_eq!(kind_of(&attachment("scan", "image/png", 10)), Kind::Image);
        assert_eq!(kind_of(&attachment("notes.md", "application/octet-stream", 10)), Kind::Text);
        assert_eq!(kind_of(&attachment("invite", "text/calendar", 10)), Kind::Text);
        assert_eq!(kind_of(&attachment("Expenses.CSV", "application/octet-stream", 10)), Kind::Table);
        assert_eq!(kind_of(&attachment("report.docx", "application/msword", 10)), Kind::Other);
        assert_eq!(kind_of(&attachment("huge.png", "image/png", 50 * 1024 * 1024)), Kind::Other, "too big to preview");
        if pdftoppm_installed() {
            assert_eq!(kind_of(&attachment("Invoice.pdf", "application/pdf", 10)), Kind::Pdf);
        }
    }

    #[test]
    fn reads_csv_with_quotes() {
        let text = "Name,Note,Amount\r\n\"Shah, Priya\",\"Said \"\"hi\"\"\nthen left\",12.50\r\nTom,,3\n";
        let (rows, more) = parse_table(text, ',', 100);
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[1], ["Shah, Priya", "Said \"hi\"\nthen left", "12.50"]);
        assert_eq!(rows[2], ["Tom", "", "3"]);
        assert!(!more);
        let (rows, more) = parse_table("a\nb\nc\n", ',', 2);
        assert_eq!((rows.len(), more), (2, true));
        assert_eq!(parse_table("a\tb", '\t', 10).0, [["a", "b"]]);
    }
}
