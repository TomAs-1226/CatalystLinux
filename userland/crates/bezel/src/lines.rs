//! Text broken into lines that fit a width.
//!
//! Latin text breaks at its spaces. Chinese and Japanese have none, and break
//! between any two characters instead, except where the rules Japanese calls
//! kinsoku forbid it: no line starts with a closing bracket, a stop or a
//! comma, a small kana or the long-vowel mark (、。」ー っ), and none ends
//! with an opening bracket (「（). Korean is set with spaces between its
//! words and breaks there, as Latin does.

/// A character lines may break before or after with no space: Han
/// ideographs, kana, CJK punctuation and full-width forms. (Not Hangul:
/// Korean breaks at its spaces.)
pub fn breaks_anywhere(c: char) -> bool {
    matches!(c as u32,
        0x2E80..=0x2FDF     // CJK and Kangxi radicals
        | 0x3000..=0x303F   // CJK symbols and punctuation
        | 0x3040..=0x30FF   // hiragana, katakana
        | 0x3100..=0x312F   // bopomofo
        | 0x31A0..=0x31FF   // bopomofo extended, strokes, katakana extensions
        | 0x3200..=0x33FF   // enclosed letters, compatibility
        | 0x3400..=0x4DBF   // ideographs, extension A
        | 0x4E00..=0x9FFF   // ideographs
        | 0xF900..=0xFAFF   // compatibility ideographs
        | 0xFE30..=0xFE4F   // compatibility forms
        | 0xFF00..=0xFFEF   // full- and half-width forms
        | 0x20000..=0x3FFFF // ideographs, extensions B on
    )
}

/// A character set two columns wide (East Asian Width wide or full-width):
/// ideographs, kana, Hangul, full-width forms. A terminal gives each two
/// cells; so does Bezel's mono face.
pub fn is_wide(c: char) -> bool {
    matches!(c as u32,
        0x1100..=0x115F     // Hangul leading jamo
        | 0x2E80..=0x303E
        | 0x3041..=0x33FF
        | 0x3400..=0x4DBF
        | 0x4E00..=0x9FFF
        | 0xA000..=0xA4CF   // Yi
        | 0xA960..=0xA97F
        | 0xAC00..=0xD7A3   // Hangul syllables
        | 0xF900..=0xFAFF
        | 0xFE30..=0xFE4F
        | 0xFF00..=0xFF60
        | 0xFFE0..=0xFFE6
        | 0x20000..=0x3FFFD
    )
}

/// Not at the start of a line (行頭禁則): closing brackets and quotes, stops,
/// commas and the like, small kana, the long-vowel mark, iteration marks.
pub fn no_line_start(c: char) -> bool {
    matches!(
        c,
        '、' | '。' | '，' | '．' | '・' | '：' | '；' | '？' | '！' | '‼' | '⁇' | '⁈' | '⁉'
            | '）' | '」' | '』' | '】' | '〉' | '》' | '〕' | '］' | '｝' | '〙' | '〗' | '〟' | '〞'
            | '｣' | '｡' | '､' | '･' | '’' | '”' | '»' | '›'
            | 'ー' | 'ｰ' | '〜' | '～' | '゠' | '‐' | '–' | '゛' | '゜' | 'ゝ' | 'ゞ' | 'ヽ' | 'ヾ' | '々' | '〻'
            | 'ぁ' | 'ぃ' | 'ぅ' | 'ぇ' | 'ぉ' | 'っ' | 'ゃ' | 'ゅ' | 'ょ' | 'ゎ' | 'ゕ' | 'ゖ'
            | 'ァ' | 'ィ' | 'ゥ' | 'ェ' | 'ォ' | 'ッ' | 'ャ' | 'ュ' | 'ョ' | 'ヮ' | 'ヵ' | 'ヶ'
            | 'ㇰ'..='ㇿ' | 'ｧ'..='ｯ'
            | '…' | '‥' | '%' | '％' | '‰' | '℃' | '°' | '′' | '″'
            | ')' | ']' | '}' | ',' | '.' | ':' | ';' | '!' | '?'
    )
}

/// Not at the end of a line (行末禁則): opening brackets and quotes, and the
/// signs that go before a number.
pub fn no_line_end(c: char) -> bool {
    matches!(
        c,
        '（' | '「' | '『' | '【' | '〈' | '《' | '〔' | '［' | '｛' | '〘' | '〖' | '〝' | '｢'
            | '‘' | '“' | '«' | '‹' | '(' | '[' | '{' | '￥' | '＄' | '￡' | '＃' | '№'
    )
}

/// Whether a line may break between `a` and `b`, which have no space
/// between them.
pub fn may_break_between(a: char, b: char) -> bool {
    (breaks_anywhere(a) || breaks_anywhere(b))
        && !no_line_start(b)
        && !no_line_end(a)
        // A dash or an ellipsis written twice ("——", "……") stays whole.
        && !(a == b && matches!(a, '—' | '―' | '…' | '‥'))
}

/// A space lines break at: any but the no-break ones, and the ideographic
/// space, which is a character of CJK text (an indent, a gap in a title) and
/// is kept as it is.
fn breaking_space(c: char) -> bool {
    c.is_whitespace() && !matches!(c, '\u{a0}' | '\u{202f}' | '\u{2007}' | '\u{2060}' | '\u{3000}')
}

/// A piece of text no line breaks inside, and whether a space comes before it.
#[derive(Debug, PartialEq)]
struct Piece {
    text: String,
    space: bool,
}

fn pieces(text: &str) -> Vec<Piece> {
    let mut out = Vec::new();
    let mut piece = Piece { text: String::new(), space: false };
    let mut spaced = false;
    let mut prev: Option<char> = None;
    for c in text.chars() {
        if breaking_space(c) {
            if !piece.text.is_empty() {
                out.push(std::mem::replace(&mut piece, Piece { text: String::new(), space: false }));
            }
            spaced = !out.is_empty();
            prev = None;
            continue;
        }
        if piece.text.is_empty() {
            piece.space = spaced;
        } else if prev.is_some_and(|p| may_break_between(p, c)) {
            out.push(std::mem::replace(&mut piece, Piece { text: String::new(), space: false }));
        }
        piece.text.push(c);
        spaced = false;
        prev = Some(c);
    }
    if !piece.text.is_empty() {
        out.push(piece);
    }
    out
}

/// How close to the width an estimate must be to be measured exactly. A
/// line's width is estimated as the sum of its pieces (kerning and tracking
/// across a join move it by a fraction of a pixel).
const CLOSE: f32 = 2.0;

/// `text` broken into lines no wider than `width`, by `measure`. Spaces
/// between words become single spaces, and none starts or ends a line.
/// A piece wider than a whole line (a long address) is broken between its
/// characters if `split_long`, and otherwise left whole on a line of its own.
pub fn wrap(text: &str, width: f32, split_long: bool, measure: &mut dyn FnMut(&str) -> f32) -> Vec<String> {
    let mut lines = Vec::new();
    let mut line = String::new();
    let mut line_w = 0.0f32;
    let mut space_w: Option<f32> = None;
    for piece in pieces(text) {
        let w = measure(&piece.text);
        if !line.is_empty() {
            let join = if piece.space { *space_w.get_or_insert_with(|| measure(" ")) } else { 0.0 };
            let estimate = line_w + join + w;
            let candidate = || {
                let mut s = String::with_capacity(line.len() + piece.text.len() + 1);
                s.push_str(&line);
                if piece.space {
                    s.push(' ');
                }
                s.push_str(&piece.text);
                s
            };
            if estimate <= width - CLOSE {
                line = candidate();
                line_w = estimate;
                continue;
            }
            if estimate <= width + CLOSE {
                let tried = candidate();
                let exact = measure(&tried);
                if exact <= width {
                    line = tried;
                    line_w = exact;
                    continue;
                }
            }
            lines.push(std::mem::take(&mut line));
        }
        if split_long && w > width {
            // Broken where it must be: as many characters as fit, then on.
            let mut chars = piece.text.chars().peekable();
            while let Some(c) = chars.next() {
                line.push(c);
                if chars.peek().is_some() && measure(&line) > width && line.chars().count() > 1 {
                    line.pop();
                    lines.push(std::mem::take(&mut line));
                    line.push(c);
                }
            }
            line_w = measure(&line);
        } else {
            line = piece.text;
            line_w = w;
        }
    }
    if !line.is_empty() {
        lines.push(line);
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A measure for tests: Latin letters and spaces half an em, wide
    /// characters a whole one, at 10 px.
    fn width_of(s: &str) -> f32 {
        s.chars().map(|c| if is_wide(c) { 10.0 } else { 5.0 }).sum()
    }

    fn wrap_at(text: &str, width: f32) -> Vec<String> {
        wrap(text, width, true, &mut width_of)
    }

    #[test]
    fn latin_breaks_at_spaces_and_collapses_them() {
        assert_eq!(wrap_at("the quick  brown fox", 50.0), ["the quick", "brown fox"]);
        assert_eq!(wrap_at("  lead and trail  ", 200.0), ["lead and trail"]);
        assert!(wrap_at("", 50.0).is_empty());
    }

    #[test]
    fn chinese_breaks_between_any_two_characters() {
        // Ten characters, four to a line.
        assert_eq!(wrap_at("我们一起去看电影吧好", 40.0), ["我们一起", "去看电影", "吧好"]);
    }

    #[test]
    fn no_line_starts_with_a_stop_a_closing_bracket_or_a_small_kana() {
        // Five to a line, 「。」 would start the second: it stays with the
        // character before it, which goes down with it.
        assert_eq!(wrap_at("今日は晴れ。明日は雨", 50.0), ["今日は晴", "れ。明日は", "雨"]);
        // At every width from three characters to ten: no line starts with
        // 、。」っー, none ends with 「, and nothing is lost or added.
        let text = "「東京」は、とても大きい都市です。ちょっと待って、コーヒーを飲もう。";
        for chars in 3..=10 {
            let limit = chars as f32 * 10.0;
            let lines = wrap_at(text, limit);
            assert_eq!(lines.concat(), text, "at {chars}");
            for (i, line) in lines.iter().enumerate() {
                assert!(width_of(line) <= limit, "{line} too wide at {chars}");
                if i > 0 {
                    assert!(!no_line_start(line.chars().next().unwrap()), "line {i} starts {line:?} at {chars}: {lines:?}");
                }
                if i + 1 < lines.len() {
                    assert!(!no_line_end(line.chars().last().unwrap()), "line {i} ends {line:?} at {chars}: {lines:?}");
                }
            }
        }
    }

    #[test]
    fn an_opening_bracket_goes_down_with_what_it_opens() {
        assert_eq!(wrap_at("歌「夜に駆ける」", 20.0), ["歌", "「夜", "に駆", "け", "る」"]);
        assert_eq!(wrap_at("（笑）です", 30.0), ["（笑）", "です"]);
    }

    #[test]
    fn a_doubled_dash_or_ellipsis_stays_whole() {
        let lines = wrap_at("我——你……他", 30.0);
        assert!(lines.iter().all(|l| !l.starts_with('—') && !l.starts_with('…')), "{lines:?}");
        assert_eq!(lines.concat(), "我——你……他");
    }

    #[test]
    fn mixed_latin_and_cjk_break_between_scripts_and_keep_latin_words_whole() {
        assert_eq!(wrap_at("YOASOBI夜に駆ける", 50.0), ["YOASOBI夜", "に駆ける"]);
        // A Latin word in the middle of Japanese is not split.
        assert_eq!(wrap_at("今日はCatalystで", 50.0), ["今日は", "Catalystで"]);
    }

    #[test]
    fn korean_breaks_at_spaces_like_latin() {
        assert_eq!(wrap_at("안녕하세요 반갑습니다", 60.0), ["안녕하세요", "반갑습니다"]);
    }

    #[test]
    fn an_ideographic_space_is_kept() {
        assert_eq!(wrap_at("夜に駆ける\u{3000}YOASOBI", 200.0), ["夜に駆ける\u{3000}YOASOBI"]);
    }

    #[test]
    fn a_long_word_is_cut_only_when_asked() {
        assert_eq!(wrap("abcdefghij", 25.0, true, &mut width_of), ["abcde", "fghij"]);
        assert_eq!(wrap("abcdefghij", 25.0, false, &mut width_of), ["abcdefghij"]);
    }

    #[test]
    fn widths_near_the_edge_are_measured_exactly() {
        // A measure where a whole line is narrower than its parts (as
        // negative tracking makes it): the exact width decides.
        let mut tight = |s: &str| width_of(s) - 1.5 * (s.chars().count().saturating_sub(1)) as f32;
        let lines = wrap("aa bb cc", 20.0, false, &mut tight);
        assert_eq!(lines, ["aa bb", "cc"]);
    }
}
