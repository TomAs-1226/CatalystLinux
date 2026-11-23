//! The scanned pages, reviewed: the chosen page large at its paper's shape
//! and enhanced, a strip of every page (and one to add another), how the
//! page comes out (colour, grey, black and white) and its paper (auto, A4,
//! Letter, free). `,` and `.` move a page, `d` deletes it, `a` adds the
//! next (back to the camera), `s` saves the document.

use std::cell::RefCell;
use std::rc::Rc;

use bezel::{Align, Rect};
use detent::{role, Animated};

use super::{Doc, ENHANCES, PAPERS};
use crate::gfx::{Gfx, H, PAD, W};
use crate::photos::{draw_picture, whole};
use crate::platform::Key;
use crate::robot::Robot;
use crate::shell::{Cx, Nav, Screen};

const ROWS: [&str; 2] = ["enhance", "paper"];

pub struct Review {
    doc: Rc<RefCell<Doc>>,
    /// The page chosen (the strip's end is "add a page").
    at: usize,
    /// What the arrows move: the strip (0) or a row (1, 2).
    focus: usize,
    /// The strip's scroll, gliding to keep the chosen page in sight.
    scroll: Animated,
    deleting: bool,
    note: Option<(String, f32)>,
}

impl Review {
    pub fn new(doc: Rc<RefCell<Doc>>) -> Self {
        let at = doc.borrow().pages.len().saturating_sub(1);
        Review { doc, at, focus: 0, scroll: Animated::new(0.0, role::settle()).with_rest(0.05), deleting: false, note: None }
    }

    /// On a page and a row (the shots).
    pub fn at(mut self, page: usize, focus: usize) -> Self {
        self.at = page;
        self.focus = focus;
        self
    }

    fn say(&mut self, words: impl Into<String>) {
        self.note = Some((words.into(), 0.0));
    }

    fn pages(&self) -> usize {
        self.doc.borrow().pages.len()
    }

    /// ← → on the row in focus.
    fn step(&mut self, by: isize) {
        let n = self.pages();
        match self.focus {
            0 => self.at = (self.at as isize + by).clamp(0, n as isize) as usize,
            _ if self.at >= n => {}
            1 => {
                let now = self.doc.borrow().pages[self.at].enhance;
                let i = ENHANCES.iter().position(|e| *e == now).unwrap_or(0) as isize + by;
                self.doc.borrow_mut().set(self.at, Some(ENHANCES[i.rem_euclid(ENHANCES.len() as isize) as usize]), None);
            }
            _ => {
                let now = self.doc.borrow().pages[self.at].paper;
                let i = PAPERS.iter().position(|p| *p == now).unwrap_or(0) as isize + by;
                self.doc.borrow_mut().set(self.at, None, Some(PAPERS[i.rem_euclid(PAPERS.len() as isize) as usize]));
            }
        }
    }
}

impl Screen for Review {
    fn title(&self) -> &str {
        "Document"
    }

    fn app(&self) -> &str {
        "Camera"
    }

    fn full_screen(&self) -> bool {
        true
    }

    fn keep_in_recents(&self) -> bool {
        false
    }

    fn key(&mut self, key: Key, _cx: &mut Cx) -> Nav {
        let deleting = std::mem::take(&mut self.deleting);
        let n = self.pages();
        let saving = self.doc.borrow().saving().is_some();
        if n == 0 {
            // Saved (or emptied): any key goes back to the camera.
            return if matches!(key, Key::Back | Key::Leave | Key::Enter | Key::Char('a')) { Nav::Pop } else { Nav::Stay };
        }
        match key {
            Key::Back if deleting => {}
            Key::Back | Key::Leave => return Nav::Pop,
            _ if saving => self.say("saving the document…"),
            Key::Char('a') => return Nav::Pop,
            Key::Enter if self.focus == 0 && self.at >= n => return Nav::Pop,
            Key::Up => self.focus = self.focus.saturating_sub(1),
            Key::Down => self.focus = (self.focus + 1).min(ROWS.len()),
            Key::Left => self.step(-1),
            Key::Right => self.step(1),
            Key::Char('e') if self.at < n => {
                self.focus = 1;
                self.step(1);
            }
            Key::Char('p') if self.at < n => {
                self.focus = 2;
                self.step(1);
            }
            Key::Char(',' | '<') if self.at < n => self.at = self.doc.borrow_mut().shift(self.at, -1),
            Key::Char('.' | '>') if self.at < n => self.at = self.doc.borrow_mut().shift(self.at, 1),
            Key::Char('d') | Key::Delete if self.at < n => {
                if deleting {
                    self.doc.borrow_mut().remove(self.at);
                    self.at = self.at.min(self.pages().saturating_sub(1));
                    self.say("page deleted");
                    if self.pages() == 0 {
                        return Nav::Pop;
                    }
                } else {
                    self.deleting = true;
                }
            }
            Key::Char('s') => {
                if self.doc.borrow().working() {
                    self.say("a page is still being flattened");
                } else if self.doc.borrow_mut().save() {
                    self.say("saving…");
                }
            }
            _ => {}
        }
        Nav::Stay
    }

    fn tick(&mut self, dt: f32) -> bool {
        let mut changed = self.doc.borrow_mut().poll() | self.scroll.tick(dt);
        if self.doc.borrow().saving().is_some() {
            changed = true;
        }
        if let Some((_, age)) = &mut self.note {
            *age += dt;
            changed = true;
            if *age > 2.6 {
                self.note = None;
            }
        }
        changed
    }

    fn busy(&self) -> bool {
        self.doc.borrow().working()
    }

    fn draw(&mut self, g: &mut Gfx, _robot: &Robot) {
        let t = g.tokens;
        let ty = g.ty;
        g.canvas.fill_rect(Rect::new(g.ox, 0.0, W, H), t.ground, 1.0);
        let bottom = g.above_hint();
        let mut doc = self.doc.borrow_mut();
        let n = doc.pages.len();
        if n == 0 {
            // Saved: what and where.
            // Its name on a line of its own, then what it holds and where.
            let lines: Vec<String> = match &doc.saved {
                Some((pdf, pages)) => {
                    let name = pdf.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
                    vec!["Saved".to_owned(), name, format!("{pages} page{} · pdf and png in scans", if *pages == 1 { "" } else { "s" })]
                }
                None => vec!["No pages".to_owned(), "a: back to the camera to scan one".to_owned()],
            };
            let title = lines[0].clone();
            let cy = (bottom * 0.5 - ty.caption.size * 0.5).round();
            g.canvas.circle(W * 0.5 + g.ox, cy - 22.0, 17.0, t.ice, 1.0);
            g.icon(if doc.saved.is_some() { "picture_as_pdf" } else { "document_scanner" }, true, W * 0.5, cy - 22.0, 18.0, t.on_ice);
            g.text_at(W * 0.5, cy + 8.0, Align::Center, &title, ty.name, t.ink);
            for (k, line) in lines.iter().skip(1).enumerate() {
                let line = g.text.fit(line, &ty.caption, W - 2.0 * PAD);
                let y = cy + 8.0 + ty.name.size * 0.5 + 6.0 + ty.caption.size * 0.5 + k as f32 * (ty.caption.size + 4.0);
                g.text_at(W * 0.5, y, Align::Center, &line, ty.caption, if k == 0 { t.ink } else { t.dim });
            }
            g.hint("esc: back to the camera");
            return;
        }
        self.at = self.at.min(n);
        let on_page = self.at < n;
        // The page, large: as tall as the panel allows, at its shape.
        let room = Rect::new(PAD, PAD, ((bottom - PAD) / 1.414).round(), bottom - PAD);
        let shown_at = self.at.min(n - 1);
        let page = &mut doc.pages[shown_at];
        let shape = page.shape();
        let (pw, ph) = if room.h / room.w > shape { (room.w, room.w * shape) } else { (room.h / shape, room.h) };
        let pr = Rect::new((room.cx() - pw * 0.5).round(), (room.cy() - ph * 0.5).round(), pw.round(), ph.round());
        g.shadow(pr, 3.0, 0.8);
        match page.shown() {
            Some(pic) => draw_picture(g, pr, pic, whole(pic), 3.0, 1.0),
            None => {
                g.surface(pr, 3.0, t.surface2);
                g.icon("document_scanner", false, pr.cx(), pr.cy() - 10.0, 18.0, t.faint);
                g.text_at(pr.cx(), pr.cy() + 10.0, Align::Center, "flattening", ty.caption, t.dim);
            }
        }
        // The column beside it.
        let x0 = room.right() + 12.0;
        let cw = W - PAD - x0;
        let mut y = PAD + ty.name.size * 0.5 + 1.0;
        let head = if on_page { format!("Page {} of {n}", self.at + 1) } else { format!("{n} page{}", if n == 1 { "" } else { "s" }) };
        g.text_at(x0, y, Align::Left, &head, ty.name, t.ink);
        y += ty.name.size * 0.5 + 4.0 + ty.caption.size * 0.5;
        let p = &doc.pages[shown_at];
        let detail = if p.preview.is_none() {
            "flattening the page…".to_owned()
        } else {
            let paper = p.paper_now();
            let paper = if p.paper == super::Paper::Auto && paper != super::Paper::Free { format!("{} (found)", paper.name()) } else { paper.name().to_owned() };
            format!("{paper} · {} × {}", p.size.0, (p.size.0 as f32 * p.shape()).round())
        };
        let detail = if on_page { detail } else { "a new page from the camera".to_owned() };
        // Under the heading: how the save goes, what was said, else the page.
        if let Some(f) = doc.saving() {
            let words = format!("saving {:.0}%", f * 100.0);
            let mw = g.measure("saving 100%", ty.caption) + 8.0;
            g.text_at(x0, y, Align::Left, &words, ty.caption, t.dim);
            g.meter(Rect::new(x0 + mw, y - 2.0, (cw - mw).max(10.0), 4.0), f, t.signal);
        } else if let Some(why) = doc.trouble.clone() {
            g.text_fit(x0, y, &why, ty.caption, g.fault(), cw);
        } else if let Some((words, _)) = &self.note {
            g.text_fit(x0, y, words, ty.caption, t.ink, cw);
        } else {
            g.text_fit(x0, y, &detail, ty.caption, t.dim, cw);
        }
        // The strip: every page, then one to add.
        // As tall as the room above the two rows leaves, at most three captions'.
        let row_h = g.row_h();
        let room_h = bottom - (y + ty.caption.size * 0.5 + 9.0) - 10.0 - 2.0 * row_h - 4.0;
        let th = (ty.caption.size * 3.2).min(room_h).max(20.0).round();
        let tw = (th * 0.76).round();
        let gap = 6.0;
        let strip_y = y + ty.caption.size * 0.5 + 9.0;
        let target = ((self.at as f32 + 1.0) * (tw + gap) - cw).max(0.0);
        self.scroll.set(target);
        if g.calm {
            self.scroll.jump(target);
        }
        let sx = self.scroll.value();
        g.canvas.set_clip(Rect::new(x0 - 4.0 + g.ox, strip_y - 4.0, cw + 8.0, th + 8.0));
        let mut ring = None;
        for i in 0..=n {
            let r = Rect::new(x0 + i as f32 * (tw + gap) - sx, strip_y, tw, th);
            if r.right() < x0 - 6.0 || r.x > x0 + cw + 6.0 {
                continue;
            }
            if i < n {
                let page = &mut doc.pages[i];
                match page.shown() {
                    Some(pic) => {
                        let (w, h) = crate::editor::fit_size(pic.w as f32, pic.h as f32, r.w, r.h);
                        let pr = Rect::new((r.cx() - w * 0.5).round(), (r.cy() - h * 0.5).round(), w.round(), h.round());
                        g.surface(r, 4.0, t.surface1);
                        // Pictures go past the clip: only one wholly in the column.
                        if pr.x >= x0 - 1.0 && pr.right() <= x0 + cw + 1.0 {
                            draw_picture(g, pr, pic, whole(pic), 2.0, 1.0);
                        }
                    }
                    None => g.surface(r, 4.0, t.surface2),
                }
                let label = (i + 1).to_string();
                let lw = g.measure(&label, ty.caption) + 8.0;
                let chip = Rect::new(r.right() - lw - 2.0, r.bottom() - ty.caption.size - 6.0, lw, ty.caption.size + 4.0);
                g.surface(chip, chip.h * 0.5, if i == self.at { t.ice } else { t.surface3 });
                g.text_at(chip.cx(), chip.cy(), Align::Center, &label, ty.caption, if i == self.at { t.on_ice } else { t.ink });
            } else {
                g.surface(r, 4.0, t.surface2);
                g.icon("add", false, r.cx(), r.cy(), 16.0, t.ink);
            }
            if i == self.at {
                ring = Some(r);
            }
        }
        g.canvas.clear_clip();
        // The rows: how the page comes out, and its paper.
        let rows_y = strip_y + th + 10.0;
        for (k, name) in ROWS.iter().enumerate() {
            let r = Rect::new(x0, rows_y + k as f32 * (row_h + 4.0), cw, row_h);
            let value = if on_page {
                let p = &doc.pages[self.at];
                if k == 0 {
                    p.enhance.name().to_owned()
                } else {
                    p.paper.name().to_owned()
                }
            } else {
                "–".to_owned()
            };
            g.row(r, name, &value, if on_page { t.ink } else { t.faint }, self.focus == k + 1);
        }
        if self.focus == 0 {
            if let Some(r) = ring {
                g.focus_ring(r, 4.0);
            }
        }
        drop(doc);
        if self.deleting {
            g.hint_any(&["d again deletes this page · esc: keep it", "d again: delete"]);
        } else if self.focus == 0 {
            g.hint_any(&["a: add page · s: save pdf · d: delete · , . move", "a: add page · s: save · d: delete", "a: add · s: save"]);
        } else {
            g.hint_any(&["←→ change · ↑ the pages · a: add page · s: save pdf", "←→ change · s: save pdf", "←→ · s: save"]);
        }
    }
}
