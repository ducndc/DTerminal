use super::theme::Palette;
use super::TerminalSurface;
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::vte::ansi::CursorShape;
use egui::{Color32, FontId, Key, Rect, Sense, Stroke, StrokeKind, TextStyle, Vec2};

/// What the UI layer should do after painting one frame of a terminal.
pub struct TermFrameResult {
    /// Raw bytes to write to the backend (PTY/SSH channel), already translated
    /// from keyboard input.
    pub input_bytes: Vec<u8>,
    /// New (cols, rows) if the available area implies a different grid size.
    pub resize_to: Option<(usize, usize)>,
}

pub fn show(
    ui: &mut egui::Ui,
    id: egui::Id,
    surface: &TerminalSurface,
    palette: &Palette,
    focus_now: bool,
) -> TermFrameResult {
    let font_id = FontId::monospace(ui.text_style_height(&TextStyle::Monospace).max(13.0));
    let ctx = ui.ctx().clone();
    let (char_w, row_h) = ctx.fonts_mut(|f| {
        let w = f.glyph_width(&font_id, 'M');
        let h = f.row_height(&font_id);
        (w, h)
    });

    let available = ui.available_size();
    let rect = Rect::from_min_size(ui.cursor().min, available);
    let response = ui.interact(rect, id, Sense::click());
    if focus_now {
        response.request_focus();
    }
    let focused = response.has_focus();

    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 0.0, palette.background);

    let content = surface.renderable();
    let cursor_point = content.cursor.point;
    let cursor_visible = !matches!(content.cursor.shape, CursorShape::Hidden);

    for indexed in content.display_iter {
        let line = indexed.point.line.0;
        if line < 0 {
            continue;
        }
        let (row, col) = (line as usize, indexed.point.column.0);
        let cell = indexed.cell;
        if cell.c == ' ' && cell.bg == alacritty_terminal::vte::ansi::Color::Named(
            alacritty_terminal::vte::ansi::NamedColor::Background,
        ) {
            continue;
        }

        let mut fg = palette.resolve(cell.fg);
        let mut bg = palette.resolve(cell.bg);
        if cell.flags.contains(Flags::INVERSE) {
            std::mem::swap(&mut fg, &mut bg);
        }
        if cell.flags.contains(Flags::DIM) {
            fg = dim(fg);
        }

        let cell_min = rect.min + Vec2::new(col as f32 * char_w, row as f32 * row_h);
        let cell_rect = Rect::from_min_size(cell_min, Vec2::new(char_w, row_h));

        if bg != palette.background {
            painter.rect_filled(cell_rect, 0.0, bg);
        }

        if cell.c != ' ' && cell.c != '\0' {
            painter.text(
                cell_min,
                egui::Align2::LEFT_TOP,
                cell.c,
                font_id.clone(),
                fg,
            );
        }

        if cell.flags.contains(Flags::UNDERLINE) {
            let y = cell_rect.max.y - 1.0;
            painter.line_segment(
                [egui::pos2(cell_rect.min.x, y), egui::pos2(cell_rect.max.x, y)],
                Stroke::new(1.0, fg),
            );
        }
        if cell.flags.contains(Flags::STRIKEOUT) {
            let y = cell_rect.center().y;
            painter.line_segment(
                [egui::pos2(cell_rect.min.x, y), egui::pos2(cell_rect.max.x, y)],
                Stroke::new(1.0, fg),
            );
        }
    }

    if focused && cursor_visible && cursor_point.line.0 >= 0 {
        let row = cursor_point.line.0 as usize;
        let col = cursor_point.column.0;
        let cell_min = rect.min + Vec2::new(col as f32 * char_w, row as f32 * row_h);
        let cell_rect = Rect::from_min_size(cell_min, Vec2::new(char_w, row_h));
        match content.cursor.shape {
            CursorShape::Beam => {
                painter.line_segment(
                    [cell_rect.left_top(), cell_rect.left_bottom()],
                    Stroke::new(2.0, palette.cursor),
                );
            }
            CursorShape::Underline => {
                let y = cell_rect.max.y - 1.0;
                painter.line_segment(
                    [egui::pos2(cell_rect.min.x, y), egui::pos2(cell_rect.max.x, y)],
                    Stroke::new(2.0, palette.cursor),
                );
            }
            _ => {
                painter.rect_stroke(cell_rect, 0.0, Stroke::new(1.5, palette.cursor), StrokeKind::Inside);
            }
        }
    }

    ui.allocate_rect(rect, Sense::hover());

    let mut input_bytes = Vec::new();
    if focused {
        ui.input(|i| {
            for event in &i.events {
                match event {
                    egui::Event::Text(text) => {
                        input_bytes.extend_from_slice(text.as_bytes());
                    }
                    egui::Event::Key {
                        key,
                        pressed: true,
                        modifiers,
                        ..
                    } => {
                        append_key_bytes(*key, *modifiers, &mut input_bytes);
                    }
                    egui::Event::Paste(text) => {
                        input_bytes.extend_from_slice(text.as_bytes());
                    }
                    _ => {}
                }
            }
        });
    }

    let cols = (available.x / char_w).floor().max(1.0) as usize;
    let rows = (available.y / row_h).floor().max(1.0) as usize;
    let resize_to = if cols != surface.cols() || rows != surface.rows() {
        Some((cols, rows))
    } else {
        None
    };

    TermFrameResult {
        input_bytes,
        resize_to,
    }
}

fn dim(c: Color32) -> Color32 {
    Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), (c.a() as f32 * 0.7) as u8)
}

fn append_key_bytes(key: Key, modifiers: egui::Modifiers, out: &mut Vec<u8>) {
    if modifiers.ctrl && !modifiers.shift && !modifiers.alt {
        let letter = match key {
            Key::A => Some(1u8),
            Key::B => Some(2),
            Key::C => Some(3),
            Key::D => Some(4),
            Key::E => Some(5),
            Key::F => Some(6),
            Key::G => Some(7),
            Key::H => Some(8),
            Key::I => Some(9),
            Key::J => Some(10),
            Key::K => Some(11),
            Key::L => Some(12),
            Key::M => Some(13),
            Key::N => Some(14),
            Key::O => Some(15),
            Key::P => Some(16),
            Key::Q => Some(17),
            Key::R => Some(18),
            Key::S => Some(19),
            Key::T => Some(20),
            Key::U => Some(21),
            Key::V => Some(22),
            Key::W => Some(23),
            Key::X => Some(24),
            Key::Y => Some(25),
            Key::Z => Some(26),
            _ => None,
        };
        if let Some(b) = letter {
            out.push(b);
            return;
        }
    }

    match key {
        Key::Enter => out.push(b'\r'),
        Key::Backspace => out.push(0x7f),
        Key::Tab => out.push(b'\t'),
        Key::Escape => out.push(0x1b),
        Key::ArrowUp => out.extend_from_slice(b"\x1b[A"),
        Key::ArrowDown => out.extend_from_slice(b"\x1b[B"),
        Key::ArrowRight => out.extend_from_slice(b"\x1b[C"),
        Key::ArrowLeft => out.extend_from_slice(b"\x1b[D"),
        Key::Home => out.extend_from_slice(b"\x1b[H"),
        Key::End => out.extend_from_slice(b"\x1b[F"),
        Key::PageUp => out.extend_from_slice(b"\x1b[5~"),
        Key::PageDown => out.extend_from_slice(b"\x1b[6~"),
        Key::Delete => out.extend_from_slice(b"\x1b[3~"),
        _ => {}
    }
}
