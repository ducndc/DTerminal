use alacritty_terminal::vte::ansi::{Color, NamedColor};
use egui::Color32;

/// Ported from the Electron app's `themes.js` — same five presets, same hex values.
#[derive(Clone, Copy)]
pub struct Palette {
    pub background: Color32,
    pub foreground: Color32,
    pub cursor: Color32,
    pub selection: Color32,
    pub black: Color32,
    pub red: Color32,
    pub green: Color32,
    pub yellow: Color32,
    pub blue: Color32,
    pub magenta: Color32,
    pub cyan: Color32,
    pub white: Color32,
}

fn hex(s: &str) -> Color32 {
    let s = s.trim_start_matches('#');
    let r = u8::from_str_radix(&s[0..2], 16).unwrap_or(0);
    let g = u8::from_str_radix(&s[2..4], 16).unwrap_or(0);
    let b = u8::from_str_radix(&s[4..6], 16).unwrap_or(0);
    Color32::from_rgb(r, g, b)
}

impl Palette {
    pub fn dark() -> Self {
        Self {
            background: hex("#1e1e1e"),
            foreground: hex("#d4d4d4"),
            cursor: hex("#d4d4d4"),
            selection: hex("#264f78"),
            black: hex("#1e1e1e"),
            red: hex("#f44747"),
            green: hex("#6a9955"),
            yellow: hex("#d7ba7d"),
            blue: hex("#569cd6"),
            magenta: hex("#c586c0"),
            cyan: hex("#4ec9b0"),
            white: hex("#d4d4d4"),
        }
    }

    pub fn light() -> Self {
        Self {
            background: hex("#ffffff"),
            foreground: hex("#1e1e1e"),
            cursor: hex("#1e1e1e"),
            selection: hex("#add6ff"),
            black: hex("#1e1e1e"),
            red: hex("#cd3131"),
            green: hex("#00802b"),
            yellow: hex("#795e26"),
            blue: hex("#0451a5"),
            magenta: hex("#a31515"),
            cyan: hex("#0598bc"),
            white: hex("#ffffff"),
        }
    }

    pub fn solarized_dark() -> Self {
        Self {
            background: hex("#002b36"),
            foreground: hex("#839496"),
            cursor: hex("#93a1a1"),
            selection: hex("#073642"),
            black: hex("#073642"),
            red: hex("#dc322f"),
            green: hex("#859900"),
            yellow: hex("#b58900"),
            blue: hex("#268bd2"),
            magenta: hex("#d33682"),
            cyan: hex("#2aa198"),
            white: hex("#eee8d5"),
        }
    }

    pub fn monokai() -> Self {
        Self {
            background: hex("#272822"),
            foreground: hex("#f8f8f2"),
            cursor: hex("#f8f8f0"),
            selection: hex("#49483e"),
            black: hex("#272822"),
            red: hex("#f92672"),
            green: hex("#a6e22e"),
            yellow: hex("#f4bf75"),
            blue: hex("#66d9ef"),
            magenta: hex("#ae81ff"),
            cyan: hex("#a1efe4"),
            white: hex("#f8f8f2"),
        }
    }

    pub fn dracula() -> Self {
        Self {
            background: hex("#282a36"),
            foreground: hex("#f8f8f2"),
            cursor: hex("#f8f8f0"),
            selection: hex("#44475a"),
            black: hex("#21222c"),
            red: hex("#ff5555"),
            green: hex("#50fa7b"),
            yellow: hex("#f1fa8c"),
            blue: hex("#bd93f9"),
            magenta: hex("#ff79c6"),
            cyan: hex("#8be9fd"),
            white: hex("#f8f8f2"),
        }
    }

    pub fn by_name(name: &str) -> Self {
        match name {
            "light" => Self::light(),
            "solarizedDark" => Self::solarized_dark(),
            "monokai" => Self::monokai(),
            "dracula" => Self::dracula(),
            _ => Self::dark(),
        }
    }

    fn basic(&self, i: u8) -> Color32 {
        // No separate "bright" set in the ported themes (xterm.js fell back to
        // defaults too), so normal and bright share the same 8 colors.
        match i % 8 {
            0 => self.black,
            1 => self.red,
            2 => self.green,
            3 => self.yellow,
            4 => self.blue,
            5 => self.magenta,
            6 => self.cyan,
            _ => self.white,
        }
    }

    fn named(&self, n: NamedColor) -> Color32 {
        use NamedColor::*;
        match n {
            Black | BrightBlack | DimBlack => self.black,
            Red | BrightRed | DimRed => self.red,
            Green | BrightGreen | DimGreen => self.green,
            Yellow | BrightYellow | DimYellow => self.yellow,
            Blue | BrightBlue | DimBlue => self.blue,
            Magenta | BrightMagenta | DimMagenta => self.magenta,
            Cyan | BrightCyan | DimCyan => self.cyan,
            White | BrightWhite | DimWhite => self.white,
            Foreground | BrightForeground | DimForeground => self.foreground,
            Background => self.background,
            Cursor => self.cursor,
        }
    }

    /// 256-color indexed palette: 0-15 basic/bright, 16-231 the 6x6x6 cube,
    /// 232-255 the grayscale ramp — the standard xterm formula.
    fn indexed(&self, i: u8) -> Color32 {
        if i < 16 {
            return self.basic(i);
        }
        if i >= 232 {
            let level = 8 + (i - 232) as u16 * 10;
            let level = level as u8;
            return Color32::from_rgb(level, level, level);
        }
        const STEPS: [u8; 6] = [0, 95, 135, 175, 215, 255];
        let idx = i - 16;
        let r = STEPS[(idx / 36) as usize];
        let g = STEPS[((idx / 6) % 6) as usize];
        let b = STEPS[(idx % 6) as usize];
        Color32::from_rgb(r, g, b)
    }

    pub fn resolve(&self, color: Color) -> Color32 {
        match color {
            Color::Spec(rgb) => Color32::from_rgb(rgb.r, rgb.g, rgb.b),
            Color::Named(n) => self.named(n),
            Color::Indexed(i) => self.indexed(i),
        }
    }
}
