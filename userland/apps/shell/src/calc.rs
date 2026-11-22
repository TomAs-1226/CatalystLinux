//! Calculator: an expression typed on the keyboard and its value as it is
//! typed, with the last answers kept to recall.
//!
//! Two modes (Tab):
//! - standard: decimal numbers, + − × ÷ % ^, brackets, the usual functions
//!   (sqrt, sin…, ln, log, exp, abs, round…), pi and e; trigonometry in
//!   degrees or radians (Ctrl R);
//! - programmer: whole numbers at 8, 16, 32 or 64 bits (Ctrl W), two's
//!   complement, written in any base (0x1f, 0b1010, 0o17, or decimal), with
//!   & | ^ ~ << >>; the answer shown in hex, signed and unsigned decimal,
//!   octal and binary at once.
//!
//! `ans` is the last answer. History is kept beside the settings.

use std::path::PathBuf;

use bezel::{Align, Rect};

use crate::gfx::{Gfx, PAD, W};
use crate::platform::Key;
use crate::robot::Robot;
use crate::shell::{Cx, Nav, Screen};

const HISTORY_KEPT: usize = 40;
const INPUT_LIMIT: usize = 120;

// ------------------------------------------------------------------ modes ---

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Standard { degrees: bool },
    Programmer { bits: u32 },
}

impl Mode {
    fn words(self) -> String {
        match self {
            Mode::Standard { degrees: true } => "standard · degrees".into(),
            Mode::Standard { degrees: false } => "standard · radians".into(),
            Mode::Programmer { bits } => format!("programmer · {bits}-bit"),
        }
    }
}

/// A value: a real number, or a whole one at a width.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Value {
    Real(f64),
    Int(i64, u32),
}

// ---------------------------------------------------------------- parsing ---

#[derive(Clone, Debug, PartialEq)]
enum Token {
    Num(f64),
    Int(u128),
    Name(String),
    Op(&'static str),
    Open,
    Close,
    Comma,
}

fn tokens(text: &str, programmer: bool) -> Result<Vec<Token>, String> {
    let chars: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c.is_whitespace() || c == '_' {
            i += 1;
            continue;
        }
        if c.is_ascii_digit() || (c == '.' && !programmer) {
            // 0x / 0b / 0o prefixes in either mode.
            if c == '0' && i + 1 < chars.len() && matches!(chars[i + 1], 'x' | 'X' | 'b' | 'B' | 'o' | 'O') {
                let radix = match chars[i + 1].to_ascii_lowercase() {
                    'x' => 16,
                    'b' => 2,
                    _ => 8,
                };
                let start = i + 2;
                let mut end = start;
                while end < chars.len() && (chars[end].is_digit(radix) || chars[end] == '_') {
                    end += 1;
                }
                let digits: String = chars[start..end].iter().filter(|c| **c != '_').collect();
                let n = u128::from_str_radix(&digits, radix).map_err(|_| "a number too long".to_owned())?;
                out.push(if programmer { Token::Int(n) } else { Token::Num(n as f64) });
                i = end;
                continue;
            }
            let start = i;
            while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '.' || chars[i] == '_') {
                i += 1;
            }
            // An exponent: 1e3, 2.5e-4.
            if !programmer && i < chars.len() && matches!(chars[i], 'e' | 'E') && i + 1 < chars.len() && (chars[i + 1].is_ascii_digit() || (matches!(chars[i + 1], '-' | '+') && i + 2 < chars.len() && chars[i + 2].is_ascii_digit())) {
                i += 2;
                while i < chars.len() && chars[i].is_ascii_digit() {
                    i += 1;
                }
            }
            let text: String = chars[start..i].iter().filter(|c| **c != '_').collect();
            if programmer {
                if text.contains('.') {
                    return Err("whole numbers only here".into());
                }
                out.push(Token::Int(text.parse().map_err(|_| "a number too long".to_owned())?));
            } else {
                out.push(Token::Num(text.parse().map_err(|_| format!("{text} is not a number"))?));
            }
            continue;
        }
        if c.is_alphabetic() {
            let start = i;
            while i < chars.len() && (chars[i].is_alphanumeric()) {
                i += 1;
            }
            out.push(Token::Name(chars[start..i].iter().collect::<String>().to_lowercase()));
            continue;
        }
        let two: String = chars[i..(i + 2).min(chars.len())].iter().collect();
        let (op, width): (&'static str, usize) = match two.as_str() {
            "<<" => ("<<", 2),
            ">>" => (">>", 2),
            "**" => ("^", 2),
            _ => (match c {
                '+' => "+",
                '-' | '−' => "-",
                '*' | '×' => "*",
                '/' | '÷' => "/",
                '%' => "%",
                '^' => "^",
                '&' => "&",
                '|' => "|",
                '~' => "~",
                '(' => {
                    out.push(Token::Open);
                    i += 1;
                    continue;
                }
                ')' => {
                    out.push(Token::Close);
                    i += 1;
                    continue;
                }
                ',' => {
                    out.push(Token::Comma);
                    i += 1;
                    continue;
                }
                other => return Err(format!("{other} is not understood")),
            }, 1),
        };
        i += width;
        out.push(Token::Op(op));
    }
    Ok(out)
}

/// Evaluate `text` in `mode`, with `ans` the last answer.
pub fn eval(text: &str, mode: Mode, ans: Option<Value>) -> Result<Value, String> {
    let programmer = matches!(mode, Mode::Programmer { .. });
    let toks = tokens(text, programmer)?;
    if toks.is_empty() {
        return Err("nothing to work out".into());
    }
    let mut p = Parser { toks, at: 0, mode, ans };
    let v = p.expr(0)?;
    if p.at < p.toks.len() {
        return Err("something left over".into());
    }
    match (mode, v) {
        (Mode::Programmer { bits }, V::I(n)) => Ok(Value::Int(wrap(n, bits), bits)),
        (Mode::Standard { .. }, V::F(x)) if x.is_finite() => Ok(Value::Real(x)),
        (Mode::Standard { .. }, V::F(_)) => Err("not a number".into()),
        _ => Err("not understood".into()),
    }
}

/// A value while working: real (standard) or whole (programmer, unbounded
/// until the end, then wrapped to the width).
#[derive(Clone, Copy, Debug)]
enum V {
    F(f64),
    I(i128),
}

/// A whole number cut to `bits`, read as two's complement.
fn wrap(n: i128, bits: u32) -> i64 {
    let mask: u128 = if bits >= 64 { u64::MAX as u128 } else { (1u128 << bits) - 1 };
    let u = (n as u128) & mask;
    if bits < 64 && u & (1 << (bits - 1)) != 0 {
        (u as i128 - (1i128 << bits)) as i64
    } else {
        u as u64 as i64
    }
}

struct Parser {
    toks: Vec<Token>,
    at: usize,
    mode: Mode,
    ans: Option<Value>,
}

impl Parser {
    fn programmer(&self) -> bool {
        matches!(self.mode, Mode::Programmer { .. })
    }

    fn peek(&self) -> Option<&Token> {
        self.toks.get(self.at)
    }

    /// How tightly an infix operator binds (higher first), and whether it
    /// groups from the right.
    fn binding(&self, op: &str) -> Option<(u8, bool)> {
        Some(match (op, self.programmer()) {
            ("|", true) => (1, false),
            ("^", true) => (2, false),
            ("&", true) => (3, false),
            ("<<" | ">>", true) => (4, false),
            ("+" | "-", _) => (5, false),
            ("*" | "/" | "%", _) => (6, false),
            ("^", false) => (8, true),
            _ => return None,
        })
    }

    fn expr(&mut self, min: u8) -> Result<V, String> {
        let mut left = self.unary()?;
        while let Some(Token::Op(op)) = self.peek().cloned() {
            let Some((power, right_assoc)) = self.binding(op) else { break };
            if power < min {
                break;
            }
            self.at += 1;
            let right = self.expr(if right_assoc { power } else { power + 1 })?;
            left = self.apply(op, left, right)?;
        }
        Ok(left)
    }

    fn unary(&mut self) -> Result<V, String> {
        match self.peek().cloned() {
            Some(Token::Op("-")) => {
                self.at += 1;
                // −2^2 is −4: the power binds first.
                let v = if self.programmer() { self.unary()? } else { self.expr(7)? };
                Ok(match v {
                    V::F(x) => V::F(-x),
                    V::I(n) => V::I(-n),
                })
            }
            Some(Token::Op("+")) => {
                self.at += 1;
                self.unary()
            }
            Some(Token::Op("~")) if self.programmer() => {
                self.at += 1;
                match self.unary()? {
                    V::I(n) => Ok(V::I(!n)),
                    V::F(_) => Err("~ is for whole numbers".into()),
                }
            }
            _ => self.atom(),
        }
    }

    fn atom(&mut self) -> Result<V, String> {
        let tok = self.peek().cloned().ok_or("it ends too soon")?;
        self.at += 1;
        match tok {
            Token::Num(x) => Ok(V::F(x)),
            Token::Int(n) => Ok(V::I(n as i128)),
            Token::Open => {
                let v = self.expr(0)?;
                self.close()?;
                Ok(v)
            }
            Token::Name(name) => {
                if self.peek() == Some(&Token::Open) {
                    self.at += 1;
                    let mut args = vec![self.expr(0)?];
                    while self.peek() == Some(&Token::Comma) {
                        self.at += 1;
                        args.push(self.expr(0)?);
                    }
                    self.close()?;
                    return self.call(&name, &args);
                }
                self.constant(&name)
            }
            Token::Close => Err("a ) with no (".into()),
            Token::Comma => Err("a , out of place".into()),
            Token::Op(op) => Err(format!("{op} needs a number before it")),
        }
    }

    fn close(&mut self) -> Result<(), String> {
        if self.peek() == Some(&Token::Close) {
            self.at += 1;
            Ok(())
        } else {
            Err("a ( is not closed".into())
        }
    }

    fn constant(&self, name: &str) -> Result<V, String> {
        if name == "ans" {
            return match (self.ans, self.programmer()) {
                (Some(Value::Real(x)), false) => Ok(V::F(x)),
                (Some(Value::Int(n, _)), true) => Ok(V::I(n as i128)),
                (Some(Value::Int(n, _)), false) => Ok(V::F(n as f64)),
                (Some(Value::Real(x)), true) if x.fract() == 0.0 => Ok(V::I(x as i128)),
                _ => Err("no answer yet".into()),
            };
        }
        if self.programmer() {
            return Err(format!("{name} is not known here"));
        }
        match name {
            "pi" | "π" => Ok(V::F(std::f64::consts::PI)),
            "tau" => Ok(V::F(std::f64::consts::TAU)),
            "e" => Ok(V::F(std::f64::consts::E)),
            other => Err(format!("{other} is not known")),
        }
    }

    fn call(&self, name: &str, args: &[V]) -> Result<V, String> {
        if self.programmer() {
            return Err(format!("{name}() is for the standard mode"));
        }
        let xs: Vec<f64> = args.iter().map(|v| if let V::F(x) = v { *x } else { 0.0 }).collect();
        let one = |f: fn(f64) -> f64| -> Result<V, String> {
            match xs.as_slice() {
                [x] => Ok(V::F(f(*x))),
                _ => Err(format!("{name}() takes one number")),
            }
        };
        let degrees = matches!(self.mode, Mode::Standard { degrees: true });
        let to_rad = |x: f64| if degrees { x.to_radians() } else { x };
        let from_rad = |x: f64| if degrees { x.to_degrees() } else { x };
        let x = xs.first().copied().unwrap_or(0.0);
        match name {
            "sqrt" => {
                if x < 0.0 {
                    Err("no square root of a negative".into())
                } else {
                    one(f64::sqrt)
                }
            }
            "abs" => one(f64::abs),
            "sin" => Ok(V::F(clean(to_rad(x).sin()))),
            "cos" => Ok(V::F(clean(to_rad(x).cos()))),
            "tan" => Ok(V::F(clean(to_rad(x).tan()))),
            "asin" => Ok(V::F(from_rad(x.asin()))),
            "acos" => Ok(V::F(from_rad(x.acos()))),
            "atan" => match xs.as_slice() {
                [y, x] => Ok(V::F(from_rad(y.atan2(*x)))),
                _ => Ok(V::F(from_rad(x.atan()))),
            },
            "ln" => one(f64::ln),
            "log" => one(f64::log10),
            "log2" => one(f64::log2),
            "exp" => one(f64::exp),
            "floor" => one(f64::floor),
            "ceil" => one(f64::ceil),
            "round" => one(f64::round),
            "min" | "max" | "hypot" => {
                if xs.len() < 2 {
                    return Err(format!("{name}() takes two or more numbers"));
                }
                Ok(V::F(match name {
                    "min" => xs.iter().copied().fold(f64::INFINITY, f64::min),
                    "max" => xs.iter().copied().fold(f64::NEG_INFINITY, f64::max),
                    _ => xs.iter().map(|v| v * v).sum::<f64>().sqrt(),
                }))
            }
            other => Err(format!("{other}() is not known")),
        }
    }

    fn apply(&self, op: &str, a: V, b: V) -> Result<V, String> {
        match (a, b) {
            (V::F(a), V::F(b)) => Ok(V::F(match op {
                "+" => a + b,
                "-" => a - b,
                "*" => a * b,
                "/" if b == 0.0 => return Err("÷ 0".into()),
                "/" => a / b,
                "%" if b == 0.0 => return Err("% 0".into()),
                "%" => a % b,
                "^" => a.powf(b),
                _ => return Err(format!("{op} is for whole numbers")),
            })),
            (V::I(a), V::I(b)) => Ok(V::I(match op {
                "+" => a.wrapping_add(b),
                "-" => a.wrapping_sub(b),
                "*" => a.wrapping_mul(b),
                "/" if b == 0 => return Err("÷ 0".into()),
                "/" => a / b,
                "%" if b == 0 => return Err("% 0".into()),
                "%" => a % b,
                "&" => a & b,
                "|" => a | b,
                "^" => a ^ b,
                "<<" => {
                    if !(0..128).contains(&b) {
                        return Err("a shift too far".into());
                    }
                    a.wrapping_shl(b as u32)
                }
                ">>" => {
                    if !(0..128).contains(&b) {
                        return Err("a shift too far".into());
                    }
                    a >> b
                }
                _ => return Err(format!("{op} is not understood")),
            })),
            _ => Err("numbers of two kinds".into()),
        }
    }
}

/// Rounding's dust (sin 180° = 1.2e-16) swept away.
fn clean(x: f64) -> f64 {
    if x.abs() < 1e-12 {
        0.0
    } else {
        x
    }
}

// ------------------------------------------------------------- formatting ---

/// A real number as a calculator shows it: whole when it is, otherwise up to
/// twelve significant figures, and powers of ten for the very large or small.
pub fn format_real(x: f64) -> String {
    if x == 0.0 {
        return "0".into();
    }
    if x.fract() == 0.0 && x.abs() < 1e15 {
        return format!("{}", x as i64);
    }
    let magnitude = x.abs().log10().floor() as i32;
    if !(-6..15).contains(&magnitude) {
        let s = format!("{x:.10e}");
        let (mantissa, exponent) = s.split_once('e').unwrap_or((&s, "0"));
        let mantissa = mantissa.trim_end_matches('0').trim_end_matches('.');
        return format!("{mantissa}e{exponent}");
    }
    let decimals = (11 - magnitude).clamp(0, 15) as usize;
    let s = format!("{x:.decimals$}");
    if s.contains('.') {
        s.trim_end_matches('0').trim_end_matches('.').to_owned()
    } else {
        s
    }
}

/// Binary digits in fours: 1010 0001.
pub fn grouped_binary(n: u64, bits: u32) -> String {
    let s = format!("{n:0width$b}", width = bits as usize);
    let s = &s[s.len() - bits as usize..];
    s.as_bytes().chunks(4).map(|c| std::str::from_utf8(c).unwrap_or("")).collect::<Vec<_>>().join(" ")
}

/// A whole number's forms: hex, signed and unsigned decimal, octal, binary.
pub fn forms(n: i64, bits: u32) -> [(&'static str, String); 5] {
    let mask: u64 = if bits >= 64 { u64::MAX } else { (1u64 << bits) - 1 };
    let u = (n as u64) & mask;
    let digits = (bits / 4) as usize;
    [
        ("hex", format!("0x{u:0digits$X}")),
        ("dec", n.to_string()),
        ("unsigned", u.to_string()),
        ("oct", format!("0o{u:o}")),
        ("bin", grouped_binary(u, bits)),
    ]
}

/// The answer as the big line shows it.
pub fn shown(v: Value) -> String {
    match v {
        Value::Real(x) => format_real(x),
        Value::Int(n, bits) => forms(n, bits)[0].1.clone(),
    }
}

// ---------------------------------------------------------------- history ---

fn history_path() -> PathBuf {
    crate::config_dir().join("calculator")
}

fn load_history() -> Vec<(String, String)> {
    std::fs::read_to_string(history_path())
        .unwrap_or_default()
        .lines()
        .filter_map(|l| l.split_once('\t'))
        .map(|(a, b)| (a.to_owned(), b.to_owned()))
        .collect()
}

fn save_history(history: &[(String, String)]) {
    let text: String = history.iter().map(|(a, b)| format!("{a}\t{b}\n")).collect();
    std::fs::create_dir_all(crate::config_dir()).ok();
    crate::sys::write_atomic(&history_path(), text.as_bytes()).ok();
}

// ----------------------------------------------------------------- screen ---

pub struct Calculator {
    mode: Mode,
    input: String,
    /// The last answer worked out (Enter), and what it was worked out from.
    answer: Option<(String, Value)>,
    error: Option<String>,
    /// Newest last: (expression, answer as shown).
    history: Vec<(String, String)>,
    /// Going back through the history: how far back.
    recall: Option<usize>,
    ans: Option<Value>,
}

impl Calculator {
    pub fn new() -> Self {
        Calculator { mode: Mode::Standard { degrees: true }, input: String::new(), answer: None, error: None, history: load_history(), recall: None, ans: None }
    }

    /// Opened with an expression already typed (from search), in the mode
    /// it is worked out in.
    pub fn with_input(text: &str, mode: Mode) -> Self {
        let mut calc = Self::new();
        calc.input = text.chars().take(INPUT_LIMIT).collect();
        calc.mode = mode;
        calc
    }

    /// What the expression comes to so far, if anything (shown faintly).
    fn preview(&self) -> Option<Value> {
        if self.input.trim().is_empty() {
            return None;
        }
        eval(&self.input, self.mode, self.ans).ok()
    }

    fn work_out(&mut self) {
        if self.input.trim().is_empty() {
            return;
        }
        match eval(&self.input, self.mode, self.ans) {
            Ok(v) => {
                self.history.push((self.input.trim().to_owned(), shown(v)));
                if self.history.len() > HISTORY_KEPT {
                    self.history.remove(0);
                }
                save_history(&self.history);
                self.answer = Some((self.input.trim().to_owned(), v));
                self.ans = Some(v);
                self.error = None;
                self.input.clear();
            }
            Err(why) => self.error = Some(why),
        }
        self.recall = None;
    }

    fn switch(&mut self, mode: Mode) {
        self.mode = mode;
        self.answer = None;
        self.error = None;
        self.ans = None;
    }
}

impl Screen for Calculator {
    fn title(&self) -> &str {
        "Calculator"
    }

    fn wants_text(&self) -> bool {
        true
    }

    /// Copy: what is typed, else the answer as it shows.
    fn copy(&mut self, _robot: &Robot) -> Option<String> {
        if !self.input.trim().is_empty() {
            return Some(self.input.clone());
        }
        self.answer.as_ref().map(|(_, v)| shown(*v))
    }

    fn key(&mut self, key: Key, _cx: &mut Cx) -> Nav {
        match key {
            Key::Back if self.input.is_empty() && self.answer.is_none() => return Nav::Pop,
            Key::Back => {
                self.input.clear();
                self.answer = None;
                self.error = None;
                self.recall = None;
            }
            Key::Leave => return Nav::Pop,
            Key::Enter | Key::Char('=') => self.work_out(),
            Key::Tab => {
                let next = match self.mode {
                    Mode::Standard { .. } => Mode::Programmer { bits: 32 },
                    Mode::Programmer { .. } => Mode::Standard { degrees: true },
                };
                self.switch(next);
            }
            Key::Ctrl('r') => {
                if let Mode::Standard { degrees } = self.mode {
                    self.mode = Mode::Standard { degrees: !degrees };
                }
            }
            Key::Ctrl('w') => {
                if let Mode::Programmer { bits } = self.mode {
                    let next = match bits {
                        8 => 16,
                        16 => 32,
                        32 => 64,
                        _ => 8,
                    };
                    self.mode = Mode::Programmer { bits: next };
                    if let Some((_, Value::Int(n, _))) = self.answer {
                        let v = Value::Int(wrap(n as i128, next), next);
                        self.answer = self.answer.take().map(|(e, _)| (e, v));
                        self.ans = Some(v);
                    }
                }
            }
            Key::Up => {
                let back = self.recall.map_or(1, |r| r + 1).min(self.history.len());
                if back > 0 {
                    self.recall = Some(back);
                    self.input = self.history[self.history.len() - back].0.clone();
                }
            }
            Key::Down => match self.recall {
                Some(1) | None => {
                    self.recall = None;
                    self.input.clear();
                }
                Some(r) => {
                    self.recall = Some(r - 1);
                    self.input = self.history[self.history.len() - (r - 1)].0.clone();
                }
            },
            Key::Backspace => {
                self.input.pop();
                self.error = None;
            }
            Key::Char(c) if !c.is_control() && self.input.chars().count() < INPUT_LIMIT => {
                // An operator first carries on from the last answer.
                if self.input.is_empty() && self.answer.is_some() && "+-*/%^&|<>×÷".contains(c) {
                    self.input.push_str("ans");
                }
                self.input.push(c);
                self.error = None;
            }
            _ => {}
        }
        Nav::Stay
    }

    fn draw(&mut self, g: &mut Gfx, _robot: &Robot) {
        let t = g.tokens;
        let ty = g.ty;
        let top = g.top();
        let right = W - PAD - 8.0;
        let programmer = matches!(self.mode, Mode::Programmer { .. });
        // The answer, large (hex in mono, so 0 and O never mix); at 64 bits
        // less so, leaving room below for both lines of bits.
        let big = match self.mode {
            Mode::Programmer { bits } => bezel::TextStyle::label((ty.label.size * if bits >= 64 { 1.45 } else { 1.75 }).round()),
            Mode::Standard { .. } => ty.display(28.0),
        };
        // The mode, small, over the display, and the key that changes it;
        // the display as tall as its three lines at this text size.
        let height = 6.0 + ty.caption.size + 4.0 + ty.label.size + if programmer { 4.0 } else { 10.0 } + big.size * 0.9 + 8.0;
        let display = Rect::new(PAD, top, W - 2.0 * PAD, height);
        g.surface(display, g.tile_radius(), t.surface1);
        let mode_cy = display.y + 6.0 + ty.caption.size * 0.5;
        g.text_at(display.x + 9.0, mode_cy, Align::Left, &self.mode.words(), ty.caption, t.dim);
        let switch = match self.mode {
            Mode::Standard { degrees: true } => "ctrl r: radians".to_owned(),
            Mode::Standard { degrees: false } => "ctrl r: degrees".to_owned(),
            Mode::Programmer { bits } => format!("ctrl w: {}-bit", if bits >= 64 { 8 } else { bits * 2 }),
        };
        g.text_at(right, mode_cy, Align::Right, &switch, ty.caption, t.faint);
        // What is being typed, its end in view, and the caret.
        let typed_cy = display.y + 10.0 + ty.caption.size + ty.label.size * 0.5;
        let shown_input = if self.input.is_empty() { self.answer.as_ref().map_or(String::new(), |(e, _)| format!("{e} =")) } else { self.input.clone() };
        let room = display.w - 20.0;
        let fitted = tail_fit(g, &shown_input, ty.label, room);
        let w = g.text_at(right, typed_cy, Align::Right, &fitted, ty.label, if self.input.is_empty() { t.dim } else { t.ink });
        if !self.input.is_empty() || self.answer.is_none() {
            let x = if self.input.is_empty() { right - 1.0 } else { right + 1.0 };
            let _ = w;
            g.surface(Rect::new(x, typed_cy - ty.label.size * 0.5, 1.5, ty.label.size), 0.5, t.signal);
        }
        // The answer; or what it would be, faint; or why not.
        let big_cy = display.bottom() - 8.0 - big.size * 0.45;
        let (text, color) = match (&self.error, self.preview(), &self.answer) {
            (Some(why), _, _) => (why.clone(), g.fault()),
            (None, Some(v), _) => (shown(v), t.dim),
            (None, None, Some((_, v))) => (shown(*v), t.ink),
            _ => ("0".into(), t.faint),
        };
        let style = if self.error.is_some() { ty.body } else { big };
        let fitted = tail_fit(g, &text, style, room);
        g.text_at(right, big_cy, Align::Right, &fitted, style, color);
        // Below: a whole number's other forms, or the latest answers.
        let below = display.bottom() + 6.0;
        let line = ty.caption.size + 4.0;
        let bottom = g.above_hint();
        let value = self.error.is_none().then(|| self.preview().or(self.answer.as_ref().map(|(_, v)| *v))).flatten();
        if let (Mode::Programmer { bits }, Some(Value::Int(n, _))) = (self.mode, value) {
            // Decimal, then every bit (32 to a line); unsigned and octal as
            // room allows, in that order. Unsigned sits beside decimal when
            // both fit there, and is left out where it is the same.
            let line = ty.caption.size + 3.0;
            let below = display.bottom() + 4.0;
            let f = forms(n, bits);
            let groups: Vec<&str> = f[4].1.split(' ').collect();
            let bin_lines = groups.len().div_ceil(8);
            // Thirty-two bits to a line, smaller where they would reach "bin".
            let mut bits_style = ty.caption;
            let room = right - (PAD + 6.0 + g.measure("bin ", ty.caption));
            let across = g.measure(&["0000"; 8].join(" "), bits_style);
            if across > room {
                bits_style.size *= room / across;
            }
            let bin_line = (bits_style.size + 3.0).min(line);
            // Lines to spare once decimal and the bits have theirs.
            let mut spare = ((bottom + 2.0 - below - line - bin_lines as f32 * bin_line) / line).floor().max(0.0) as usize;
            let half = W * 0.5;
            let tag = g.measure("uns ", ty.caption) + 6.0;
            let differ = f[1].1 != f[2].1;
            let beside = differ
                && g.measure(&f[1].1, ty.caption) <= half - 6.0 - (PAD + 6.0 + tag)
                && g.measure(&f[2].1, ty.caption) <= right - (half + 6.0 + tag);
            let unsigned_line = differ && !beside && spare > 0;
            spare -= usize::from(unsigned_line);
            let octal = spare > 0;
            let mut y = below + line * 0.5;
            g.text_at(PAD + 6.0, y, Align::Left, "dec", ty.caption, t.dim);
            if beside {
                g.text_at(half - 6.0, y, Align::Right, &f[1].1, ty.caption, t.ink);
                g.text_at(half + 6.0, y, Align::Left, "uns", ty.caption, t.dim);
            }
            g.text_at(right, y, Align::Right, &f[if beside { 2 } else { 1 }].1, ty.caption, t.ink);
            y += line;
            for (shown, tag, value) in [(unsigned_line, "uns", &f[2].1), (octal, "oct", &f[3].1)] {
                if shown {
                    g.text_at(PAD + 6.0, y, Align::Left, tag, ty.caption, t.dim);
                    g.text_at(right, y, Align::Right, value, ty.caption, t.ink);
                    y += line;
                }
            }
            let mut y = y - line * 0.5 + bin_line * 0.5;
            for chunk in groups.chunks(8) {
                g.text_at(PAD + 6.0, y, Align::Left, "bin", ty.caption, t.dim);
                g.text_at(right, y, Align::Right, &chunk.join(" "), bits_style, t.ink);
                y += bin_line;
            }
        } else {
            let mut y = below + line * 0.5;
            for (expr, answer) in self.history.iter().rev() {
                if y + line * 0.5 > bottom {
                    break;
                }
                let answer_w = g.measure(answer, ty.caption);
                let expr = g.text.fit(&format!("{expr} ="), &ty.caption, (W - 2.0 * PAD - 24.0 - answer_w).max(30.0));
                g.text_at(PAD + 6.0, y, Align::Left, &expr, ty.caption, t.faint);
                g.text_at(right, y, Align::Right, answer, ty.caption, t.dim);
                y += line;
            }
        }
        g.hint_any(match self.mode {
            Mode::Standard { .. } => &["enter: = · ↑: earlier · tab: programmer", "↑: earlier · tab: programmer"],
            Mode::Programmer { .. } => &["0x 0b 0o · & | ^ ~ << >> · tab: standard", "& | ^ ~ << >> · tab: standard"],
        });
    }
}

/// The end of `text` that fits in `room` (where the typing is).
fn tail_fit(g: &mut Gfx, text: &str, style: bezel::TextStyle, room: f32) -> String {
    if g.measure(text, style) <= room {
        return text.to_owned();
    }
    let mut chars: Vec<char> = text.chars().collect();
    while !chars.is_empty() && g.measure(&format!("…{}", chars.iter().collect::<String>()), style) > room {
        chars.remove(0);
    }
    format!("…{}", chars.iter().collect::<String>())
}

#[cfg(test)]
mod tests {
    use super::*;

    const DEG: Mode = Mode::Standard { degrees: true };
    const RAD: Mode = Mode::Standard { degrees: false };

    fn real(text: &str, mode: Mode) -> f64 {
        match eval(text, mode, None).unwrap_or_else(|e| panic!("{text}: {e}")) {
            Value::Real(x) => x,
            other => panic!("{text}: {other:?}"),
        }
    }

    fn int(text: &str, bits: u32) -> i64 {
        match eval(text, Mode::Programmer { bits }, None).unwrap_or_else(|e| panic!("{text}: {e}")) {
            Value::Int(n, _) => n,
            other => panic!("{text}: {other:?}"),
        }
    }

    #[test]
    fn expressions_work_out_with_the_usual_precedence() {
        assert_eq!(real("1 + 2 * 3", DEG), 7.0);
        assert_eq!(real("(1 + 2) * 3", DEG), 9.0);
        assert_eq!(real("2^3^2", DEG), 512.0, "powers group from the right");
        assert_eq!(real("-2^2", DEG), -4.0);
        assert_eq!(real("10 % 4", DEG), 2.0);
        assert_eq!(real("1.5e3 / 3", DEG), 500.0);
        assert_eq!(real("3 × 4 ÷ 2", DEG), 6.0);
        assert_eq!(real("2 ** 10", DEG), 1024.0);
        assert_eq!(real("sqrt(16) + abs(-3)", DEG), 7.0);
        assert_eq!(real("max(1, 7, 3)", DEG), 7.0);
        assert!((real("hypot(3, 4)", DEG) - 5.0).abs() < 1e-12);
        assert!((real("pi", DEG) - std::f64::consts::PI).abs() < 1e-15);
        assert!(eval("1/0", DEG, None).is_err());
        assert!(eval("(1+2", DEG, None).is_err());
        assert!(eval("1+", DEG, None).is_err());
        assert!(eval("sqrt(-1)", DEG, None).is_err());
        assert!(eval("frobnicate(2)", DEG, None).is_err());
    }

    #[test]
    fn trigonometry_follows_degrees_or_radians() {
        assert!((real("sin(30)", DEG) - 0.5).abs() < 1e-12);
        assert_eq!(real("sin(180)", DEG), 0.0, "no rounding dust");
        assert!((real("sin(pi/2)", RAD) - 1.0).abs() < 1e-12);
        assert!((real("atan(1)", DEG) - 45.0).abs() < 1e-9);
        assert!((real("atan(1, -1)", DEG) - 135.0).abs() < 1e-9);
    }

    #[test]
    fn the_last_answer_carries_on() {
        let ans = Some(Value::Real(12.0));
        assert_eq!(eval("ans / 4", DEG, ans), Ok(Value::Real(3.0)));
        assert!(eval("ans", DEG, None).is_err());
    }

    #[test]
    fn programmer_mode_wraps_at_its_width_as_twos_complement() {
        assert_eq!(int("0xff + 1", 8), 0);
        assert_eq!(int("0x7f + 1", 8), -128);
        assert_eq!(int("-1", 16), -1);
        assert_eq!(int("0b1010 | 0b0101", 8), 15);
        assert_eq!(int("0xf0 & 0x3c", 8), 0x30);
        assert_eq!(int("6 ^ 3", 32), 5, "^ is exclusive or here");
        assert_eq!(int("~0", 32), -1);
        assert_eq!(int("1 << 4", 32), 16);
        assert_eq!(int("-16 >> 2", 32), -4);
        assert_eq!(int("7 / 2", 32), 3);
        assert_eq!(int("0o17", 32), 15);
        assert_eq!(int("1 + 2 * 3 | 8", 32), 15, "| binds loosest");
        assert!(eval("1.5", Mode::Programmer { bits: 32 }, None).is_err());
        assert!(eval("1 << 200", Mode::Programmer { bits: 64 }, None).is_err());
        assert_eq!(int("0xffffffffffffffff", 64), -1);
    }

    #[test]
    fn a_whole_number_reads_in_every_base() {
        let f = forms(-1, 8);
        assert_eq!(f[0].1, "0xFF");
        assert_eq!(f[1].1, "-1");
        assert_eq!(f[2].1, "255");
        assert_eq!(f[3].1, "0o377");
        assert_eq!(f[4].1, "1111 1111");
        assert_eq!(forms(10, 16)[4].1, "0000 0000 0000 1010");
        assert_eq!(forms(255, 32)[0].1, "0x000000FF");
    }

    #[test]
    fn reals_read_as_a_calculator_shows_them() {
        assert_eq!(format_real(42.0), "42");
        assert_eq!(format_real(0.1 + 0.2), "0.3");
        assert_eq!(format_real(1.0 / 3.0), "0.333333333333");
        assert_eq!(format_real(-2.5), "-2.5");
        assert_eq!(format_real(1.0e20), "1e20");
        assert_eq!(format_real(1.5e-9), "1.5e-9");
        assert_eq!(format_real(123456.789), "123456.789");
    }
}
