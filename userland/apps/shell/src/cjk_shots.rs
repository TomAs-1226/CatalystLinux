//! Chinese, Japanese and Korean on the panel, as the simulator draws them:
//! Music with Japanese and Chinese songs, Files with CJK names, a note in
//! Japanese and its editor, Spotlight finding a CJK title, the Assistant
//! answering in Japanese, and mixed Latin and CJK at each text size in both
//! looks. `catalyst-shell --cjk-shots <dir>`.

use std::path::Path;
use std::time::{Duration, Instant};

use crate::gfx::TextSize;
use crate::look::Theme;
use crate::platform::Key::{self, *};
use crate::robot::Robot;
use crate::shell::Shell;

/// The two looks: Catalyst, dark; Bezel, light.
const LOOKS: [(&str, Theme, bool); 2] = [("catalyst", Theme::Catalyst, false), ("bezel-light", Theme::Bezel, true)];
const SIZES: [TextSize; 3] = [TextSize::Compact, TextSize::Standard, TextSize::Large];

fn settle(shell: &mut Shell, frames: usize) {
    for _ in 0..frames {
        if shell.tick(1.0 / 60.0) {
            shell.draw();
        }
    }
}

/// Frames at real speed, for what a thread finishes (a search, a reply).
fn wait(shell: &mut Shell, seconds: f32) {
    let until = Instant::now() + Duration::from_secs_f32(seconds);
    while Instant::now() < until {
        settle(shell, 4);
        std::thread::sleep(Duration::from_millis(25));
    }
    settle(shell, 40);
}

fn press(shell: &mut Shell, keys: &[Key], frames: usize) {
    for &key in keys {
        shell.key(key);
        settle(shell, frames);
    }
}

fn typed(shell: &mut Shell, text: &str) {
    for c in text.chars() {
        shell.key(Char(c));
        settle(shell, 2);
    }
    settle(shell, 40);
}

fn fresh(theme: Theme, light: bool, size: TextSize) -> Shell {
    let mut shell = Shell::new(Robot::demo());
    (shell.settings.theme, shell.settings.light, shell.settings.text) = (theme, light, size);
    shell.apply_settings();
    settle(&mut shell, 100);
    shell
}

/// Files and notes with Chinese, Japanese and Korean names and words.
fn fixtures() -> std::path::PathBuf {
    let files = crate::files::storage();
    let write = |name: &str, text: &str| std::fs::write(files.join(name), text).ok();
    std::fs::create_dir_all(files.join("我的文档")).ok();
    std::fs::create_dir_all(files.join("写真 2026")).ok();
    write("会議メモ.txt", "11月21日の会議\n\n・インテークのベルトを交換\n・バッテリー3は要点検\n");
    write("東京の写真.jpg", &"x".repeat(240_000));
    write("歌詞「夜明けのシグナル」.txt", "夜明けのシグナル\n灯台の光が\n");
    write("比赛记录.csv", "match,score\nq31,84\n");
    write("노트.md", "# 노트\n");
    write("Résumé – Thomas.pdf", &"x".repeat(52_000));
    let notes = crate::notes::dir();
    std::fs::create_dir_all(&notes).ok();
    let note = notes.join("2026-11-21-094500.txt");
    std::fs::write(
        &note,
        "練習メモ（11月21日）\n\nインテークの調子が悪かった。\nベルトを交換すること。\nバッテリー3は6 Vまで下がった：要交換。\nCAN bus「can_s0」が混んでいる。\n\n— Q31 · 11月21日 9:45\n",
    )
    .ok();
    std::fs::write(notes.join("2026-11-20-161000.txt"), "比赛笔记\n\n第三场：电池电压过低，换成5号电池。\n").ok();
    std::fs::write(notes.join("2026-11-19-120000.txt"), "Swerve offsets\n\nfront left 0.412 rad\n").ok();
    note
}

pub fn shots(dir: &Path) {
    std::fs::create_dir_all(dir).expect("create the screenshot folder");
    let save = |shell: &mut Shell, name: &str| {
        shell.draw();
        crate::platform::sim::save_png(&shell.gfx.canvas, &dir.join(format!("{name}.png")), 3);
        println!("{name}");
    };
    if !bezel::text::cjk_available() {
        eprintln!("no CJK font: run scripts/cjk-font.py first (it makes userland/assets/fonts/cjk/{})", bezel::text::CJK_FILE);
        std::process::exit(1);
    }
    let note = fixtures();

    for (look, theme, light) in LOOKS {
        // Music: a search by an artist's name in Latin letters finds her
        // Japanese songs; another, a Chinese artist's.
        let mut shell = fresh(theme, light, TextSize::Standard);
        shell.show(Box::new(crate::music::MusicScreen::searching("hoshino")));
        wait(&mut shell, 1.2);
        save(&mut shell, &format!("01-music-japanese-{look}"));
        let mut shell = fresh(theme, light, TextSize::Standard);
        shell.show(Box::new(crate::music::MusicScreen::searching("chen")));
        wait(&mut shell, 1.2);
        save(&mut shell, &format!("02-music-chinese-{look}"));

        // Files with CJK names (after the folders and the Latin names).
        let mut shell = fresh(theme, light, TextSize::Standard);
        shell.show(Box::new(crate::files::Files::new()));
        settle(&mut shell, 60);
        press(&mut shell, &[Down; 7], 6);
        settle(&mut shell, 60);
        save(&mut shell, &format!("06-files-{look}"));

        // Notes: the list, and a note in Japanese open in the editor.
        let mut shell = fresh(theme, light, TextSize::Standard);
        shell.show(Box::new(crate::notes::Notes::new()));
        settle(&mut shell, 60);
        save(&mut shell, &format!("07-notes-{look}"));
        let editor = crate::files::Editor::open(&note).expect("the note");
        shell.show(Box::new(editor));
        settle(&mut shell, 40);
        press(&mut shell, &[Down, Down, Right, Right, Right, Right], 4);
        settle(&mut shell, 30);
        save(&mut shell, &format!("08-note-japanese-{look}"));

        // Spotlight: the songs Music listed, the note and the files, found
        // by their characters (typed here as dictation or a paste would give them).
        let mut shell = fresh(theme, light, TextSize::Standard);
        crate::spotlight::index().refresh(true);
        shell.key(Ctrl('\n'));
        settle(&mut shell, 50);
        typed(&mut shell, "夜明け");
        save(&mut shell, &format!("09-spotlight-yoake-{look}"));
        shell.key(Delete);
        typed(&mut shell, "東京");
        save(&mut shell, &format!("10-spotlight-tokyo-{look}"));
        shell.key(Delete);
        typed(&mut shell, "晴");
        save(&mut shell, &format!("11-spotlight-one-character-{look}"));
        shell.key(Back);
        settle(&mut shell, 60);

        // The Assistant asked in Japanese: its answer broken into lines
        // between characters, no line starting with 、 or 。.
        crate::keys::Keys::default().save().ok();
        let mut held = crate::keys::Keys::default();
        held.set("openai", "sk-proj-demo-7f3a");
        held.save().ok();
        std::fs::remove_file(crate::config_dir().join("assistant")).ok();
        std::fs::remove_dir_all(crate::config_dir().join("assistant-chats")).ok();
        let mut shell = fresh(theme, light, TextSize::Standard);
        press(&mut shell, &vec![Right; crate::steps_to("Assistant")], 12);
        press(&mut shell, &[Enter], 90);
        typed(&mut shell, "ロボットの調子は？");
        press(&mut shell, &[Enter], 20);
        wait(&mut shell, 1.4);
        save(&mut shell, &format!("12-assistant-japanese-{look}"));
        press(&mut shell, &[Up, Up], 30);
        save(&mut shell, &format!("13-assistant-japanese-scrolled-{look}"));
    }

    // Mixed Latin and CJK at each text size, in both looks.
    for (look, theme, light) in LOOKS {
        for size in SIZES {
            let name = size.name();
            let mut shell = fresh(theme, light, size);
            shell.show(Box::new(crate::music::MusicScreen::searching("hoshino")));
            wait(&mut shell, 1.2);
            save(&mut shell, &format!("20-sizes-music-{name}-{look}"));
            let mut shell = fresh(theme, light, size);
            shell.show(Box::new(crate::files::Files::new()));
            settle(&mut shell, 60);
            press(&mut shell, &[Down; 7], 6);
            settle(&mut shell, 60);
            save(&mut shell, &format!("21-sizes-files-{name}-{look}"));
            let mut shell = fresh(theme, light, size);
            shell.show(Box::new(crate::files::Editor::open(&note).expect("the note")));
            settle(&mut shell, 40);
            save(&mut shell, &format!("22-sizes-note-{name}-{look}"));
        }
    }

    // Played, last (the player goes on between shells): a Japanese song
    // and what comes after it, then a Chinese one; paused after.
    for (look, theme, light) in LOOKS {
        let mut shell = fresh(theme, light, TextSize::Standard);
        shell.show(Box::new(crate::music::MusicScreen::searching("hoshino")));
        wait(&mut shell, 1.2);
        press(&mut shell, &[Enter], 30);
        wait(&mut shell, 3.4);
        save(&mut shell, &format!("30-music-playing-japanese-{look}"));
        press(&mut shell, &[Down], 60);
        save(&mut shell, &format!("31-music-up-next-{look}"));
        let mut shell = fresh(theme, light, TextSize::Standard);
        shell.show(Box::new(crate::music::MusicScreen::searching("chen")));
        wait(&mut shell, 1.2);
        press(&mut shell, &[Down, Enter], 30);
        wait(&mut shell, 3.4);
        save(&mut shell, &format!("32-music-playing-chinese-{look}"));
        shell.key(Media(crate::platform::Media::PlayPause));
        settle(&mut shell, 30);
    }
}
