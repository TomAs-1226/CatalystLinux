//! The player end to end, to standard output: one song running on into
//! the next with nothing between them, a crossfade between two albums'
//! songs, and none within one album.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

fn dir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("catalyst-play-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// A 16-bit stereo WAV of `frames` frames, each sample from `f`.
fn wav(path: &Path, rate: u32, frames: usize, f: impl Fn(usize) -> i16) {
    let data = frames * 4;
    let mut b = Vec::with_capacity(44 + data);
    b.extend_from_slice(b"RIFF");
    b.extend_from_slice(&(36 + data as u32).to_le_bytes());
    b.extend_from_slice(b"WAVEfmt ");
    b.extend_from_slice(&16u32.to_le_bytes());
    b.extend_from_slice(&1u16.to_le_bytes());
    b.extend_from_slice(&2u16.to_le_bytes());
    b.extend_from_slice(&rate.to_le_bytes());
    b.extend_from_slice(&(rate * 4).to_le_bytes());
    b.extend_from_slice(&4u16.to_le_bytes());
    b.extend_from_slice(&16u16.to_le_bytes());
    b.extend_from_slice(b"data");
    b.extend_from_slice(&(data as u32).to_le_bytes());
    for i in 0..frames {
        let s = f(i).to_le_bytes();
        b.extend_from_slice(&s);
        b.extend_from_slice(&s);
    }
    std::fs::write(path, b).unwrap();
}

/// The player, as built for these tests: under emulation (scripts/test-arm.sh)
/// through the runner the tests themselves run under, so it finds the
/// image's loader.
fn player() -> Command {
    let bin = env!("CARGO_BIN_EXE_catalyst-play");
    match std::env::var_os("CARGO_TARGET_AARCH64_UNKNOWN_LINUX_MUSL_RUNNER").filter(|_| cfg!(target_arch = "aarch64")) {
        Some(runner) => {
            let mut c = Command::new(runner);
            c.arg(bin);
            c
        }
        None => Command::new(bin),
    }
}

/// The player's sound (left channel) and what it said.
fn play(d: &Path, fx: &str, first: &Path, album: &str, next: &str) -> (Vec<i16>, String) {
    let fx_path = d.join("audio-fx");
    std::fs::write(&fx_path, fx).unwrap();
    let mut child = player()
        .args(["--raw", "--album", album])
        .arg(first)
        .env("CATALYST_AUDIO_FX", &fx_path)
        .env("CATALYST_LOUDNESS", d.join("loudness"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(next.as_bytes()).unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let left = out.stdout.chunks_exact(4).map(|f| i16::from_le_bytes([f[0], f[1]])).collect();
    (left, String::from_utf8_lossy(&out.stderr).into_owned())
}

#[test]
fn the_next_song_follows_without_a_sample_between() {
    let d = dir("gapless");
    let (a, b) = (d.join("a.wav"), d.join("b.wav"));
    wav(&a, 48_000, 48_000, |i| (i % 1000) as i16 + 1);
    wav(&b, 48_000, 24_000, |i| -((i % 700) as i16) - 1);
    let next = format!("next\t{}\talbum=x\n", b.display());
    let (left, said) = play(&d, "loudness=off\ncrossfade=0\n", &a, "x", &next);
    assert_eq!(left.len(), 72_000, "{said}");
    // Every sample as it was in the files: the first song's last, then the second's first.
    assert!(left[..48_000].iter().enumerate().all(|(i, &s)| s == (i % 1000) as i16 + 1));
    assert_eq!((left[47_999], left[48_000]), (1000, -1));
    assert!(left[48_000..].iter().enumerate().all(|(i, &s)| s == -((i % 700) as i16) - 1));
    assert!(said.contains(&format!("track {}", b.display())), "{said}");
    assert!(said.trim_end().ends_with("end"));
    std::fs::remove_dir_all(d).ok();
}

#[test]
fn songs_of_other_albums_crossfade_and_one_albums_do_not() {
    let d = dir("crossfade");
    let (a, b) = (d.join("a.wav"), d.join("b.wav"));
    let tone = |freq: f32, amp: f32| move |i: usize| (amp * (i as f32 * freq * 6.2832 / 48_000.0).sin()) as i16;
    wav(&a, 48_000, 48_000 * 10, tone(440.0, 12_000.0));
    wav(&b, 48_000, 48_000 * 10, tone(660.0, 12_000.0));
    let next = |album: &str| format!("next\t{}\talbum={album}\n", b.display());
    // Two seconds' overlap: 18 seconds in all, the join at equal power.
    let (left, said) = play(&d, "loudness=off\ncrossfade=2\n", &a, "one", &next("two"));
    assert_eq!(left.len(), 864_000, "{said}");
    assert!(said.contains("at 5.0\ntrack ") && said.contains("at 6.0\nend"), "{said}");
    let rms = |s: &[i16]| (s.iter().map(|&x| (x as f64).powi(2)).sum::<f64>() / s.len() as f64).sqrt();
    let alone = rms(&left[48_000..96_000]);
    let mid = rms(&left[48_000 * 8 + 12_000..48_000 * 9 - 12_000]);
    assert!((mid / alone - 1.0).abs() < 0.25, "{alone} {mid}");
    // The same album: back to back, twenty seconds.
    let (left, _) = play(&d, "loudness=off\ncrossfade=2\n", &a, "one", &next("one"));
    assert_eq!(left.len(), 960_000);
    std::fs::remove_dir_all(d).ok();
}

#[test]
fn spatial_sound_and_the_equalizer_change_the_sound_and_off_they_leave_it_be() {
    let d = dir("effects");
    let a = d.join("a.wav");
    let tone = |i: usize| (8_000.0 * (i as f32 * 300.0 * 6.2832 / 48_000.0).sin()) as i16;
    wav(&a, 48_000, 48_000, tone);
    let play_with = |fx: &str, headphones: bool| {
        let fx_path = d.join("audio-fx");
        std::fs::write(&fx_path, fx).unwrap();
        let mut cmd = player();
        cmd.arg("--raw").arg(&a).env("CATALYST_AUDIO_FX", &fx_path).env("CATALYST_LOUDNESS", d.join("loudness")).env("CATALYST_AUDIO_RUN", &d);
        if headphones {
            cmd.env("CATALYST_PLAY_HEADPHONES", "1");
        }
        let out = cmd.stdin(Stdio::null()).output().unwrap();
        assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
        out.stdout.chunks_exact(2).map(|b| i16::from_le_bytes([b[0], b[1]])).collect::<Vec<i16>>()
    };
    let as_made: Vec<i16> = (0..48_000).flat_map(|i| [tone(i), tone(i)]).collect();
    assert_eq!(play_with("spatial=off\neq=flat\nloudness=off\n", true), as_made, "all off: bit for bit");
    // Spatial sound is for headphones only: through a speaker, nothing.
    assert_eq!(play_with("spatial=on\neq=flat\nloudness=off\n", false), as_made);
    let spatial = play_with("spatial=on\neq=flat\nloudness=off\n", true);
    assert_eq!(spatial.len(), as_made.len());
    assert_ne!(spatial, as_made);
    let bass = play_with("spatial=off\neq=bass\nloudness=off\n", false);
    assert_eq!(bass.len(), as_made.len());
    // 300 Hz sits under the bass shelf's lift and the 250 Hz bell: changed, never clipped.
    assert_ne!(bass, as_made);
    assert!(bass.iter().all(|s| s.unsigned_abs() < 32_000));
    std::fs::remove_dir_all(d).ok();
}

#[test]
fn a_cd_rate_pair_is_resampled_across_the_join_without_a_dip() {
    let d = dir("resampled");
    let (a, b) = (d.join("a.wav"), d.join("b.wav"));
    let tone = |i: usize| (10_000.0 * (i as f32 * 441.0 * 6.2832 / 44_100.0).sin()) as i16;
    wav(&a, 44_100, 44_100, tone);
    wav(&b, 44_100, 44_100, move |i| tone(i + 44_100));
    let (left, _) = play(&d, "loudness=off\n", &a, "", &format!("next\t{}\n", b.display()));
    assert!((95_990..=96_040).contains(&left.len()), "{}", left.len());
    // One continuous tone: no stretch near the join is quiet.
    let join = 48_000;
    for w in left[join - 2000..join + 2000].chunks(200) {
        let peak = w.iter().map(|s| s.unsigned_abs()).max().unwrap();
        assert!(peak > 9_000, "{peak}");
    }
    std::fs::remove_dir_all(d).ok();
}
