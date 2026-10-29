//! Line icons after Lucide, drawn on a 24-unit grid with the painter, so nothing depends on a glyph
//! being in the font.

use egui::{Color32, Painter, Pos2, Rect, Shape, Stroke, pos2};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Icon {
    Inbox,
    Send,
    File,
    Archive,
    Trash,
    Junk,
    Folder,
    Flag,
    FlagFilled,
    Paperclip,
    Search,
    Settings,
    Compose,
    Reply,
    ReplyAll,
    Forward,
    Mail,
    MailOpen,
    Check,
    Close,
    Refresh,
    ChevronDown,
    ChevronRight,
    More,
    Clock,
    External,
    Download,
    Plus,
    Bell,
    BellOff,
    Alert,
    Layers,
    Menu,
    Minimize,
    Maximize,
    Restore,
}

struct Pen<'a> {
    painter: &'a Painter,
    rect: Rect,
    stroke: Stroke,
    color: Color32,
}

impl Pen<'_> {
    fn p(&self, x: f32, y: f32) -> Pos2 {
        let scale = self.rect.width() / 24.0;
        pos2(self.rect.left() + x * scale, self.rect.top() + y * scale)
    }

    fn line(&self, points: &[(f32, f32)]) {
        let points: Vec<Pos2> = points.iter().map(|&(x, y)| self.p(x, y)).collect();
        self.painter.add(Shape::line(points, self.stroke));
    }

    fn closed(&self, points: &[(f32, f32)]) {
        let points: Vec<Pos2> = points.iter().map(|&(x, y)| self.p(x, y)).collect();
        self.painter.add(Shape::closed_line(points, self.stroke));
    }

    fn circle(&self, x: f32, y: f32, r: f32) {
        let scale = self.rect.width() / 24.0;
        self.painter.circle_stroke(self.p(x, y), r * scale, self.stroke);
    }

    fn dot(&self, x: f32, y: f32, r: f32) {
        let scale = self.rect.width() / 24.0;
        self.painter.circle_filled(self.p(x, y), r * scale, self.color);
    }

    /// Points along an arc, in degrees with y pointing down, for joining into a line.
    fn arc(&self, cx: f32, cy: f32, r: f32, from: f32, to: f32) -> Vec<(f32, f32)> {
        let steps = 10;
        (0..=steps)
            .map(|i| {
                let a = (from + (to - from) * i as f32 / steps as f32).to_radians();
                (cx + r * a.cos(), cy + r * a.sin())
            })
            .collect()
    }

    fn rounded(&self, x0: f32, y0: f32, x1: f32, y1: f32, r: f32) {
        let mut points = Vec::new();
        points.extend(self.arc(x1 - r, y0 + r, r, -90.0, 0.0));
        points.extend(self.arc(x1 - r, y1 - r, r, 0.0, 90.0));
        points.extend(self.arc(x0 + r, y1 - r, r, 90.0, 180.0));
        points.extend(self.arc(x0 + r, y0 + r, r, 180.0, 270.0));
        self.closed(&points);
    }
}

pub fn paint(painter: &Painter, rect: Rect, icon: Icon, color: Color32) {
    let width = (rect.width() / 24.0 * 1.9).max(1.15);
    let pen = Pen { painter, rect, stroke: Stroke::new(width, color), color };
    match icon {
        Icon::Inbox => {
            pen.line(&[(22.0, 12.0), (16.0, 12.0), (14.0, 15.0), (10.0, 15.0), (8.0, 12.0), (2.0, 12.0)]);
            pen.closed(&[
                (2.0, 12.0),
                (5.5, 5.0),
                (18.5, 5.0),
                (22.0, 12.0),
                (22.0, 18.0),
                (20.0, 20.0),
                (4.0, 20.0),
                (2.0, 18.0),
            ]);
        }
        Icon::Send => {
            pen.closed(&[(22.0, 2.0), (15.0, 22.0), (11.0, 13.0), (2.0, 9.0)]);
            pen.line(&[(22.0, 2.0), (11.0, 13.0)]);
        }
        Icon::File => {
            pen.closed(&[
                (14.0, 2.0),
                (6.0, 2.0),
                (4.0, 4.0),
                (4.0, 20.0),
                (6.0, 22.0),
                (18.0, 22.0),
                (20.0, 20.0),
                (20.0, 8.0),
            ]);
            pen.line(&[(14.0, 2.0), (14.0, 8.0), (20.0, 8.0)]);
        }
        Icon::Archive => {
            pen.rounded(2.0, 3.0, 22.0, 8.0, 1.0);
            pen.line(&[(4.0, 8.0), (4.0, 19.0), (6.0, 21.0), (18.0, 21.0), (20.0, 19.0), (20.0, 8.0)]);
            pen.line(&[(10.0, 12.0), (14.0, 12.0)]);
        }
        Icon::Trash => {
            pen.line(&[(3.0, 6.0), (21.0, 6.0)]);
            pen.line(&[(19.0, 6.0), (19.0, 20.0), (17.0, 22.0), (7.0, 22.0), (5.0, 20.0), (5.0, 6.0)]);
            pen.line(&[(8.0, 6.0), (8.0, 4.0), (10.0, 2.0), (14.0, 2.0), (16.0, 4.0), (16.0, 6.0)]);
            pen.line(&[(10.0, 11.0), (10.0, 17.0)]);
            pen.line(&[(14.0, 11.0), (14.0, 17.0)]);
        }
        Icon::Junk => {
            pen.closed(&[
                (7.9, 2.0),
                (16.1, 2.0),
                (22.0, 7.9),
                (22.0, 16.1),
                (16.1, 22.0),
                (7.9, 22.0),
                (2.0, 16.1),
                (2.0, 7.9),
            ]);
            pen.line(&[(12.0, 8.0), (12.0, 12.5)]);
            pen.dot(12.0, 16.0, 1.2);
        }
        Icon::Folder => {
            pen.closed(&[
                (2.0, 5.0),
                (4.0, 3.0),
                (8.0, 3.0),
                (10.0, 6.0),
                (20.0, 6.0),
                (22.0, 8.0),
                (22.0, 18.0),
                (20.0, 20.0),
                (4.0, 20.0),
                (2.0, 18.0),
            ]);
        }
        Icon::Flag | Icon::FlagFilled => {
            let outline = [
                (4.0, 15.0),
                (8.0, 14.0),
                (12.0, 15.5),
                (16.0, 17.0),
                (20.0, 15.0),
                (20.0, 3.0),
                (16.0, 5.0),
                (12.0, 3.5),
                (8.0, 2.0),
                (4.0, 3.0),
            ];
            if icon == Icon::FlagFilled {
                pen.painter.add(Shape::Path(egui::epaint::PathShape {
                    points: outline.iter().map(|&(x, y)| pen.p(x, y)).collect(),
                    closed: true,
                    fill: color,
                    stroke: pen.stroke.into(),
                }));
            } else {
                pen.closed(&outline);
            }
            pen.line(&[(4.0, 22.0), (4.0, 15.0)]);
        }
        Icon::Paperclip => {
            let mut points = vec![(21.44, 11.05), (12.25, 20.24)];
            points.extend(pen.arc(8.0, 16.0, 6.0, 45.0, 225.0));
            points.push((12.33, 3.18));
            points.extend(pen.arc(15.17, 6.01, 4.0, 225.0, 405.0));
            points.push((9.41, 17.41));
            points.extend(pen.arc(8.0, 16.0, 2.0, 45.0, 225.0));
            points.push((15.07, 6.1));
            pen.line(&points);
        }
        Icon::Search => {
            pen.circle(11.0, 11.0, 7.5);
            pen.line(&[(21.0, 21.0), (16.4, 16.4)]);
        }
        Icon::Settings => {
            pen.circle(12.0, 12.0, 3.0);
            pen.circle(12.0, 12.0, 7.0);
            for i in 0..8 {
                let a = (i as f32 * 45.0).to_radians();
                pen.line(&[(12.0 + 7.0 * a.cos(), 12.0 + 7.0 * a.sin()), (12.0 + 9.8 * a.cos(), 12.0 + 9.8 * a.sin())]);
            }
        }
        Icon::Compose => {
            pen.line(&[
                (12.0, 3.0),
                (5.0, 3.0),
                (3.0, 5.0),
                (3.0, 19.0),
                (5.0, 21.0),
                (19.0, 21.0),
                (21.0, 19.0),
                (21.0, 12.0),
            ]);
            pen.closed(&[(18.4, 2.6), (21.4, 5.6), (12.0, 15.0), (8.0, 16.0), (9.0, 12.0)]);
        }
        Icon::Reply => {
            pen.line(&[(9.0, 17.0), (4.0, 12.0), (9.0, 7.0)]);
            let mut tail = vec![(20.0, 18.0)];
            tail.extend(pen.arc(16.0, 16.0, 4.0, 0.0, -90.0));
            tail.push((4.0, 12.0));
            pen.line(&tail);
        }
        Icon::ReplyAll => {
            pen.line(&[(7.0, 17.0), (2.0, 12.0), (7.0, 7.0)]);
            pen.line(&[(12.0, 17.0), (7.0, 12.0), (12.0, 7.0)]);
            let mut tail = vec![(22.0, 18.0)];
            tail.extend(pen.arc(18.0, 16.0, 4.0, 0.0, -90.0));
            tail.push((7.0, 12.0));
            pen.line(&tail);
        }
        Icon::Forward => {
            pen.line(&[(15.0, 17.0), (20.0, 12.0), (15.0, 7.0)]);
            let mut tail = vec![(4.0, 18.0)];
            tail.extend(pen.arc(8.0, 16.0, 4.0, 180.0, 270.0));
            tail.push((20.0, 12.0));
            pen.line(&tail);
        }
        Icon::Mail => {
            pen.rounded(2.0, 4.0, 22.0, 20.0, 2.0);
            pen.line(&[(22.0, 7.0), (12.0, 13.0), (2.0, 7.0)]);
        }
        Icon::MailOpen => {
            pen.closed(&[(2.0, 9.0), (12.0, 3.0), (22.0, 9.0), (22.0, 19.0), (20.0, 21.0), (4.0, 21.0), (2.0, 19.0)]);
            pen.line(&[(22.0, 9.0), (12.0, 15.0), (2.0, 9.0)]);
        }
        Icon::Check => pen.line(&[(20.0, 6.0), (9.0, 17.0), (4.0, 12.0)]),
        Icon::Close => {
            pen.line(&[(18.0, 6.0), (6.0, 18.0)]);
            pen.line(&[(6.0, 6.0), (18.0, 18.0)]);
        }
        Icon::Refresh => {
            let mut arc = pen.arc(12.0, 12.0, 9.0, 0.0, 315.0);
            arc.push((21.0, 8.0));
            pen.line(&arc);
            pen.line(&[(21.0, 3.0), (21.0, 8.0), (16.0, 8.0)]);
        }
        Icon::ChevronDown => pen.line(&[(6.0, 9.0), (12.0, 15.0), (18.0, 9.0)]),
        Icon::ChevronRight => pen.line(&[(9.0, 18.0), (15.0, 12.0), (9.0, 6.0)]),
        Icon::More => {
            pen.dot(5.0, 12.0, 1.6);
            pen.dot(12.0, 12.0, 1.6);
            pen.dot(19.0, 12.0, 1.6);
        }
        Icon::Clock => {
            pen.circle(12.0, 12.0, 10.0);
            pen.line(&[(12.0, 6.0), (12.0, 12.0), (16.0, 14.0)]);
        }
        Icon::External => {
            pen.line(&[(15.0, 3.0), (21.0, 3.0), (21.0, 9.0)]);
            pen.line(&[(10.0, 14.0), (21.0, 3.0)]);
            pen.line(&[
                (18.0, 13.0),
                (18.0, 19.0),
                (16.0, 21.0),
                (5.0, 21.0),
                (3.0, 19.0),
                (3.0, 8.0),
                (5.0, 6.0),
                (11.0, 6.0),
            ]);
        }
        Icon::Download => {
            pen.line(&[(21.0, 15.0), (21.0, 19.0), (19.0, 21.0), (5.0, 21.0), (3.0, 19.0), (3.0, 15.0)]);
            pen.line(&[(7.0, 10.0), (12.0, 15.0), (17.0, 10.0)]);
            pen.line(&[(12.0, 15.0), (12.0, 3.0)]);
        }
        Icon::Plus => {
            pen.line(&[(5.0, 12.0), (19.0, 12.0)]);
            pen.line(&[(12.0, 5.0), (12.0, 19.0)]);
        }
        Icon::Bell | Icon::BellOff => {
            let mut bell = pen.arc(12.0, 8.0, 6.0, 180.0, 360.0);
            bell.extend([(18.0, 10.0), (19.0, 14.0), (21.0, 17.0), (3.0, 17.0), (5.0, 14.0), (6.0, 10.0), (6.0, 8.0)]);
            pen.line(&bell);
            pen.line(&pen.arc(12.0, 19.5, 2.0, 20.0, 160.0));
            if icon == Icon::BellOff {
                pen.line(&[(2.0, 2.0), (22.0, 22.0)]);
            }
        }
        Icon::Alert => {
            pen.closed(&[(12.0, 3.0), (22.0, 20.0), (2.0, 20.0)]);
            pen.line(&[(12.0, 9.0), (12.0, 13.5)]);
            pen.dot(12.0, 16.8, 1.1);
        }
        Icon::Layers => {
            pen.closed(&[(12.0, 2.5), (2.0, 7.5), (12.0, 12.5), (22.0, 7.5)]);
            pen.line(&[(2.0, 12.0), (12.0, 17.0), (22.0, 12.0)]);
            pen.line(&[(2.0, 16.5), (12.0, 21.5), (22.0, 16.5)]);
        }
        Icon::Menu => {
            pen.line(&[(4.0, 6.0), (20.0, 6.0)]);
            pen.line(&[(4.0, 12.0), (20.0, 12.0)]);
            pen.line(&[(4.0, 18.0), (20.0, 18.0)]);
        }
        Icon::Minimize => pen.line(&[(5.0, 12.0), (19.0, 12.0)]),
        Icon::Maximize => pen.rounded(5.0, 5.0, 19.0, 19.0, 1.5),
        Icon::Restore => {
            pen.rounded(4.0, 8.0, 16.0, 20.0, 1.5);
            pen.line(&[
                (8.0, 8.0),
                (8.0, 5.5),
                (9.5, 4.0),
                (18.5, 4.0),
                (20.0, 5.5),
                (20.0, 14.5),
                (18.5, 16.0),
                (16.0, 16.0),
            ]);
        }
    }
}
