use super::*;
use crate::gfx::TextSize;

/// The handheld's beat.
const BEAT: f32 = 0.025;
/// Home's apps, roughly: the built-in ones.
const COUNT: usize = 37;

/// Frames at the beat until the flow rests (or `most` seconds pass); every
/// position it was drawn at.
fn run(flow: &mut Flow, focus: usize, most: f32) -> Vec<f32> {
    let mut seen = Vec::new();
    let mut t = 0.0;
    while t < most {
        let moving = flow.tick(BEAT, focus);
        seen.push(flow.pos.value());
        t += BEAT;
        if !moving {
            break;
        }
    }
    seen
}

/// Right (or Left) held for `seconds`, repeating as the keyboard does: once
/// pressed, at 0.4 s, then every 75 ms. Positions at every beat.
fn hold(flow: &mut Flow, focus: &mut usize, dir: i32, seconds: f32, calm: bool) -> Vec<f32> {
    let (mut t, mut next, mut first) = (0.0f32, 0.0f32, true);
    let mut seen = Vec::new();
    while t < seconds - 1e-4 {
        if t + 1e-4 >= next {
            flow.step(focus, COUNT, dir, calm);
            next += if first { 0.4 } else { 0.075 };
            first = false;
        }
        flow.tick(BEAT, *focus);
        seen.push(flow.pos.value());
        t += BEAT;
    }
    seen
}

#[test]
fn every_built_in_app_has_its_own_colour_and_words() {
    let mut hues = Vec::new();
    for (_, icon, name) in crate::screens::builtin_apps() {
        assert!(about(name).is_some(), "{name} says what it does");
        hues.push(hue_of(name, icon));
    }
    // Neighbours on Home, as the apps come, are never the same colour.
    for pair in hues.windows(2) {
        assert_ne!(pair[0], pair[1], "two neighbours share a hue");
    }
    // Another app's colour comes from its icon, the same every time, and any
    // icon gets one of the 72.
    assert_eq!(hue_of("Sudoku", "grid_on"), hue_of("Sudoku", "grid_on"));
    assert!(hue_of("Sudoku", "grid_on") < 72 && hue_of("Thing", "") < 72);
}

#[test]
fn a_tap_turns_one_card_and_lands_like_a_dial_on_its_detent() {
    let mut flow = Flow::new();
    let mut focus = 3;
    flow.place(focus);
    assert_eq!(flow.step(&mut focus, COUNT, 1, false), Moved::Stay);
    assert_eq!(focus, 4);
    let seen = run(&mut flow, focus, 2.0);
    // It runs a hair past (the detent spring: 1.5%) and settles exactly,
    // within the spring's 0.57 s and a frame or two.
    let furthest = seen.iter().cloned().fold(f32::MIN, f32::max);
    assert!(furthest > 4.0 && furthest < 4.05, "past by {}", furthest - 4.0);
    assert_eq!(flow.pos.value(), 4.0);
    assert!(seen.len() as f32 * BEAT <= 0.65, "{} frames", seen.len());
    // Each frame a step on: it moves smoothly, never more than a third of a card.
    let mut last = 3.0;
    for p in seen {
        assert!((p - last).abs() < 0.34, "{last} to {p}");
        last = p;
    }
}

#[test]
fn a_held_key_gathers_speed_flies_through_every_app_and_glides_to_rest() {
    let mut flow = Flow::new();
    let mut focus = 0;
    flow.place(0);
    let seen = hold(&mut flow, &mut focus, 1, 1.5, false);
    // Every app passed in a second and a half.
    assert_eq!(focus, COUNT - 1, "reached {focus}");
    // It gathered speed: it moved further each tenth of a second, held,
    // than tapped (one card per 75 ms at most at first; far more later).
    let speed = |a: usize, b: usize| (seen[b] - seen[a]) / ((b - a) as f32 * BEAT);
    let early = speed(20, 26);
    let late = speed(44, 50);
    assert!(early > 5.0 && late > 2.0 * early, "early {early:.1} cards/s, late {late:.1}");
    // Let go mid-run (a fresh flow, held a second): it carries on at speed
    // and glides to rest on a card, without stopping first.
    let mut flow = Flow::new();
    let mut focus = 0;
    flow.place(0);
    hold(&mut flow, &mut focus, 1, 1.0, false);
    let at_release = flow.pos.velocity();
    assert!(at_release > 15.0, "running at {at_release:.1} cards/s");
    let glide = run(&mut flow, focus, 2.0);
    // The next frames keep most of the speed it had.
    let next = (glide[1] - glide[0]) / BEAT;
    assert!(next > 0.4 * at_release, "{next:.1} after {at_release:.1}");
    assert_eq!(flow.pos.value(), focus as f32, "at rest exactly on a card");
    let past = glide.iter().cloned().fold(f32::MIN, f32::max) - focus as f32;
    assert!(past < 0.15, "it lands without running far past: {past}");
    assert!(glide.len() as f32 * BEAT < 0.9, "{} frames to rest", glide.len());
}

#[test]
fn a_key_mid_glide_turns_it_from_where_it_is() {
    let mut flow = Flow::new();
    let mut focus = 10;
    flow.place(10);
    flow.step(&mut focus, COUNT, 1, false);
    for _ in 0..4 {
        flow.tick(BEAT, focus);
    }
    let (at, going) = (flow.pos.value(), flow.pos.velocity());
    assert!(at > 10.1 && at < 10.95 && going > 1.0, "on its way: {at} at {going}");
    // Left, long after: it turns round from where it is, at the speed it had.
    for _ in 0..8 {
        flow.tick(BEAT, focus);
    }
    let (at, going) = (flow.pos.value(), flow.pos.velocity());
    flow.step(&mut focus, COUNT, -1, false);
    assert_eq!(focus, 10);
    assert_eq!(flow.pos.value(), at, "nothing jumps");
    assert_eq!(flow.pos.velocity(), going, "it keeps its speed");
    let seen = run(&mut flow, focus, 2.0);
    let mut last = at;
    for p in &seen {
        assert!((p - last).abs() < 0.34, "{last} to {p}");
        last = *p;
    }
    assert_eq!(flow.pos.value(), 10.0);
}

#[test]
fn the_ends_give_and_spring_back_and_left_at_the_first_is_the_glance_page() {
    let last = COUNT - 1;
    let mut flow = Flow::new();
    let mut focus = last;
    flow.place(last);
    // Right at the last card: it gives a little past the end and comes back.
    assert_eq!(flow.step(&mut focus, COUNT, 1, false), Moved::Stay);
    assert_eq!(focus, last);
    let seen = run(&mut flow, focus, 2.0);
    let furthest = seen.iter().cloned().fold(f32::MIN, f32::max);
    assert!(furthest > last as f32 + 0.05, "it gave: {furthest}");
    // Drawn rubber-banded: never more than half a card past, whatever the push.
    assert!(rubber(furthest, COUNT) - (last as f32) < furthest - last as f32);
    assert!(rubber(last as f32 + 10.0, COUNT) - (last as f32) < 1.0);
    assert_eq!(flow.pos.value(), last as f32, "and is back exactly");
    // Left at the first card, pressed: the glance page.
    let mut focus = 0;
    flow.place(0);
    assert_eq!(flow.step(&mut focus, COUNT, -1, false), Moved::Glance);
    // But running into it, held: it gives, and the glance page waits for a press.
    let mut focus = 6;
    flow.place(6);
    let mut glance = false;
    let mut lowest = f32::MAX;
    let (mut t, mut next, mut first) = (0.0f32, 0.0f32, true);
    while t < 1.5 {
        if t + 1e-4 >= next {
            glance |= flow.step(&mut focus, COUNT, -1, false) == Moved::Glance;
            next += if first { 0.4 } else { 0.075 };
            first = false;
        }
        flow.tick(BEAT, focus);
        lowest = lowest.min(flow.pos.value());
        t += BEAT;
    }
    assert_eq!(focus, 0);
    assert!(!glance, "held into the first card: no glance page");
    assert!(lowest < 0.0, "pushed against the start, it gives: {lowest}");
    run(&mut flow, focus, 2.0);
    assert_eq!(flow.pos.value(), 0.0);
}

#[test]
fn the_calm_version_lands_at_once_and_still_gathers_speed() {
    let mut flow = Flow::new();
    let mut focus = 0;
    flow.place(0);
    flow.step(&mut focus, COUNT, 1, true);
    assert_eq!((focus, flow.pos.value()), (1, 1.0), "at once");
    // No give at an end, no overshoot anywhere.
    let mut focus = COUNT - 1;
    flow.place(focus);
    flow.step(&mut focus, COUNT, 1, true);
    assert_eq!(flow.pos.value(), (COUNT - 1) as f32);
    // Held, it still goes further each step, so every app is in reach.
    let mut focus = 0;
    flow.place(0);
    let seen = hold(&mut flow, &mut focus, 1, 1.5, true);
    assert_eq!(focus, COUNT - 1);
    for (p, q) in seen.iter().zip(seen.iter().skip(1)) {
        assert!(q >= p && q.fract() == 0.0, "only whole cards, never back: {p} then {q}");
    }
    // The card turns over at once, and back.
    flow.enlarge(focus, true);
    assert_eq!(flow.openness(), 1.0);
    flow.close(true);
    assert_eq!(flow.openness(), 0.0);
}

#[test]
fn enter_turns_the_card_over_and_moving_on_turns_it_back() {
    let mut flow = Flow::new();
    let mut focus = 5;
    flow.place(5);
    flow.enlarge(focus, false);
    assert!(flow.is_open() && flow.open_on(5) && !flow.open_on(4));
    let seen: Vec<f32> = (0..40)
        .map(|_| {
            flow.tick(BEAT, focus);
            flow.openness()
        })
        .collect();
    assert!(seen[3] > 0.1 && seen[3] < 0.9, "on its way: {}", seen[3]);
    assert_eq!(flow.openness(), 1.0);
    // Right: it turns back as the flow moves on, from where it is.
    flow.step(&mut focus, COUNT, 1, false);
    assert!(!flow.is_open() && focus == 6);
    assert_eq!(flow.opened(), Some(5), "still turning back");
    run(&mut flow, focus, 2.0);
    assert_eq!((flow.openness(), flow.opened(), flow.pos.value()), (0.0, None, 6.0));
}

/// A Gfx at a text size and look, for laying the flow out.
fn gfx(size: TextSize, bezel: bool) -> Gfx {
    let mut g = Gfx::new(bezel::Tokens::dark());
    g.theme = if bezel { crate::look::Theme::Bezel } else { crate::look::Theme::Catalyst };
    g.set_text_size(size);
    g
}

#[test]
fn the_cards_stand_back_to_front_turned_toward_the_middle() {
    let mut g = gfx(TextSize::Standard, false);
    let mut flow = Flow::new();
    flow.place(10);
    let colours = vec![Color(200, 80, 80); COUNT];
    let scene = flow.scene(&mut g, COUNT, 10, &colours, 0.0);
    let middle = scene.cards.last().expect("cards");
    assert_eq!((middle.sprite, middle.angle, middle.z, middle.x), (10, 0.0, 0.0, 0.0), "the middle one last, facing you");
    // Farthest first.
    let away: Vec<f32> = scene.cards.iter().map(|c| (c.sprite as f32 - 10.0).abs()).collect();
    assert!(away.windows(2).all(|w| w[0] >= w[1]), "{away:?}");
    for c in &scene.cards[..scene.cards.len() - 1] {
        let left = c.sprite < 10;
        // Turned toward the middle: a card on the left turns its right edge away.
        assert_eq!(c.angle > 0.0, left, "card {} turned {}", c.sprite, c.angle);
        assert!((c.x < 0.0) == left && c.z > 0.0);
        assert!(c.shade <= 1.0 && c.fog > 0.0);
    }
    // Mirror images either side, but for the light from the left.
    let pair = |i: usize| scene.cards.iter().find(|c| c.sprite == i).copied().unwrap();
    let (l, r) = (pair(8), pair(12));
    assert!((l.x + r.x).abs() < 1e-3 && (l.angle + r.angle).abs() < 1e-5 && l.z == r.z);
    assert!(r.shade > l.shade, "the right one faces the light");
    // At rest the middle card is on the panel texel for pixel, where Home opens it from.
    assert_eq!(flow.middle(), Rect::new(112.0, scene.cy - 48.0, 96.0, 96.0));
    assert!(flow.middle().y.fract() == 0.0);
}

#[test]
fn turned_over_it_shows_its_back_texel_for_pixel_where_the_app_opens_from() {
    let mut g = gfx(TextSize::Standard, false);
    let mut flow = Flow::new();
    flow.place(4);
    flow.enlarge(4, true);
    let colours = vec![Color(80, 80, 200); COUNT];
    let scene = flow.scene(&mut g, COUNT, 4, &colours, 0.0);
    let c = scene.cards.last().unwrap();
    assert_eq!((c.atlas, c.mirror, c.w, c.h), (1, true, FACE_W as f32, FACE_H as f32), "its back, whole");
    assert!((c.angle - std::f32::consts::PI).abs() < 1e-5 && c.z.abs() < 1e-3);
    // Straight on, its edges on whole pixels: exactly the rectangle Enter opens the app from.
    let r = flow.enlarged();
    let face_cy = scene.cy + c.y;
    assert_eq!(r, Rect::new(160.0 - FACE_W as f32 * 0.5, face_cy - FACE_H as f32 * 0.5, FACE_W as f32, FACE_H as f32));
    assert!(r.x.fract() == 0.0 && r.y.fract() == 0.0 && r.bottom() < crate::gfx::H - 13.0, "clear of the page track: {r:?}");
    // The others made way, and its shadow is under it.
    assert!(scene.cards.iter().filter(|o| o.atlas == 0).all(|o| o.x.abs() > FACE_W as f32 * 0.5 - 30.0), "the others made way");
    assert!(scene.shadow.is_some());
}

#[test]
fn a_card_at_rest_is_drawn_texel_for_pixel_and_reflected_fading() {
    let (mut scene, cards, face) = render::test_scene();
    // One card, at rest in the middle, no glow, no fog.
    scene.cards = vec![Placed { atlas: 0, sprite: 1, x: 0.0, y: 0.0, z: 0.0, angle: 0.0, w: 96.0, h: 96.0, mirror: false, shade: 1.0, fog: 0.0, reflect: 1.0 }];
    scene.shadow = None;
    let mut px = vec![0u32; (crate::gfx::W * crate::gfx::H) as usize];
    render::render(&mut px, &scene, [&cards, &face]);
    let (x0, y0, _) = cards.level(1, 0);
    let top = (scene.cy - 48.0) as usize;
    // Well inside its corners, every pixel is its texel.
    for y in 12..84 {
        for x in 12..84 {
            let texel = cards.texel(x0 + 1 + x, y0 + 1 + y) & 0x00ff_ffff;
            assert_eq!(px[(top + y) * 320 + 112 + x], texel, "({x}, {y})");
        }
    }
    // Under it, its reflection: fainter further down, gone within a third of its height.
    let floor = top + 96;
    let lum = |p: u32| ((p >> 16) & 0xff) + ((p >> 8) & 0xff) + (p & 0xff);
    let bare = {
        let mut b = vec![0u32; px.len()];
        let empty = Scene { cards: Vec::new(), ..scene.clone() };
        render::render(&mut b, &empty, [&cards, &face]);
        b
    };
    let lift = |y: usize| (112 + 20..112 + 76).map(|x| lum(px[y * 320 + x]).abs_diff(lum(bare[y * 320 + x]))).sum::<u32>();
    assert!(lift(floor + 2) > lift(floor + 14) && lift(floor + 14) > lift(floor + 26), "{} {} {}", lift(floor + 2), lift(floor + 14), lift(floor + 26));
    assert_eq!(lift(floor + 34), 0, "gone");
}

#[test]
fn a_card_turned_away_reads_its_narrower_levels() {
    assert_eq!(render::levels(0.8), (0, 0.0), "magnified: the whole picture");
    assert_eq!(render::levels(1.0), (0, 0.0));
    let (l, between) = render::levels(1.5);
    assert!(l == 0 && between > 0.5 && between < 0.6, "between the whole and the half: {between}");
    assert_eq!(render::levels(2.0), (1, 0.0));
    assert_eq!(render::levels(9.0), (2, 0.0), "never past the quarter");
    // A side card in the flow is read from its narrower levels.
    let (scene, cards, _) = render::test_scene();
    let c = scene.cards[0];
    let (_, _, half) = render::span(&scene, &c, &cards).unwrap();
    let most = (0..320).filter_map(|x| render::column(&scene, &c, &cards, half, x as f32 + 0.5)).map(|k| k.texels).fold(0.0, f32::max);
    assert!(most > 1.5, "{most}");
    // The level pictures are the whole one, averaged in pairs across.
    let (x0, y0, _) = cards.level(0, 0);
    let (x1, _, _) = cards.level(0, 1);
    let row = y0 + 40;
    let (a, b) = (cards.texel(x0 + 1 + 20, row), cards.texel(x0 + 1 + 21, row));
    let half = cards.texel(x1 + 1 + 10, row);
    for shift in [0, 8, 16, 24] {
        let avg = (((a >> shift) & 0xff) + ((b >> shift) & 0xff)) / 2;
        assert!(((half >> shift) & 0xff).abs_diff(avg) <= 1);
    }
}

#[test]
fn the_test_frame_draws_the_same_every_time() {
    let (scene, cards, face) = render::test_scene();
    let mut a = vec![0u32; 320 * 170];
    let mut b = vec![7u32; 320 * 170];
    render::render(&mut a, &scene, [&cards, &face]);
    render::render(&mut b, &scene, [&cards, &face]);
    assert!(a == b, "the ground covers everything, the cards draw the same");
    assert!(a.iter().all(|p| p >> 24 == 0), "the panel's top byte stays clear");
}

#[test]
fn every_name_and_back_fits_at_every_text_size_in_both_looks() {
    let names: Vec<&str> = crate::screens::builtin_apps().map(|(_, _, n)| n).collect();
    for size in [TextSize::Compact, TextSize::Standard, TextSize::Large] {
        for bezel in [false, true] {
            let mut g = gfx(size, bezel);
            let lettering = lettering(&g);
            let room = CARD as f32 - 2.0 * lettering.margin;
            for name in &names {
                let name = if bezel { name.to_lowercase() } else { name.to_string() };
                let (lines, style, whole) = art::name_lines(&mut g.text, &name, &lettering.names, room);
                assert!(whole, "{name} is cut on its card at {size:?}, bezel {bezel}");
                assert!(lines.len() <= 2, "{name} takes {} lines at {size:?}", lines.len());
                // Never smaller than the type scale's caption.
                assert!(style.size >= g.ty.caption.size);
            }
            // The back: the name beside the icon, what it does in two lines at most.
            let x = 12.0 + 15.0 + 15.0 + 9.0;
            let room = FACE_W as f32 - x - 12.0;
            for name in &names {
                let title = if bezel { name.to_lowercase() } else { name.to_string() };
                assert!(g.text.measure(&title, &lettering.title) <= room, "{title}'s title at {size:?}");
                let about = about(name).unwrap();
                let lines = art::wrap(&mut g.text, about, &lettering.caption, room, 2);
                assert!(lines.join(" ") == about, "{name}: {about:?} runs past two lines at {size:?}: {lines:?}");
                // The lines fit between the name and the reading.
                let bottom = 12.0 + 15.0 + 9.0 + lines.len() as f32 * (lettering.caption.size + 3.0);
                assert!(bottom <= FACE_H as f32 - 12.0 - lettering.reading.size * 0.75, "{name} at {size:?}: words reach {bottom}");
            }
            assert!(g.text.measure("enter · open", &lettering.caption) < FACE_W as f32 * 0.4);
        }
    }
}

#[test]
fn the_atlas_keeps_every_card_within_what_the_gpu_takes() {
    for count in [1, 37, 60, 200] {
        let atlas = art::Atlas::new(CARD, CARD, count);
        assert!(atlas.w <= 2048 && atlas.h <= 2048, "{count} cards: {} × {}", atlas.w, atlas.h);
        // No two cards overlap, and each has its transparent edge.
        let (xa, ya, _) = atlas.level(count - 1, 2);
        assert!(xa + CARD / 4 + 2 < atlas.w && ya + CARD + 2 < atlas.h);
    }
    let face = art::Atlas::new(FACE_W, FACE_H, 1);
    assert!(face.w <= 2048);
}
