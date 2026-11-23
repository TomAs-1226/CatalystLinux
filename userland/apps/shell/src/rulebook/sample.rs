//! A small manual in the form FIRST's HTML manual takes (Word's filtered
//! HTML, in the Mac's old character set: its sections as `<h1>`–`<h2>`,
//! rules as `RuleNumber-…` paragraphs with their anchors and headline
//! spans, violations, blue boxes, lettered lists, a table), with rules
//! written for Catalyst's tests and the simulator. None of it is FIRST's
//! text: FIRST's manual is downloaded on the handheld, never shipped.

use super::parse::encode_mac_roman;

const HEAD: &str = "<html>\r\n\r\n<head>\r\n<meta http-equiv=Content-Type content=\"text/html; charset=macintosh\">\r\n<meta name=Generator content=\"Microsoft Word 15 (filtered)\">\r\n<style>\r\n<!--\r\n /* Font Definitions */\r\n p.MsoNormal {margin:0in; font-size:11.0pt;}\r\n h1 {font-size:16.0pt;}\r\n-->\r\n</style>\r\n</head>\r\n\r\n<body lang=EN-US link=\"#0066B3\" vlink=\"#954F72\" style='word-wrap:break-word'>\r\n\r\n<div class=WordSection1>\r\n\r\n";

const COVER: &str = "<p class=MsoNormal align=center style='text-align:center'><span style='font-size:28.0pt'>2026 Sample</span></p>\r\n\r\n<p class=MsoNormal align=center style='text-align:center'><span style='font-size:28.0pt'>Game Manual</span></p>\r\n\r\n<p class=MsoNormal align=center style='text-align:center'>for Catalyst Linux’s simulator and tests</p>\r\n\r\n<p class=MsoToc1><a href=\"#_Toc100000001\">1<span style='font-size:11.0pt'>&nbsp;&nbsp;&nbsp;&nbsp;</span>Introduction<span style='color:windowtext;display:none'>. </span>4</a></p>\r\n\r\n<p class=MsoToc2><a href=\"#_Toc100000002\">1.1<span>&nbsp;&nbsp;&nbsp;</span>About This Sample<span style='display:none'>. </span>4</a></p>\r\n\r\n<span style='font-size:11.0pt;font-family:\"Arial\",sans-serif'><br clear=all style='page-break-before:always'>\r\n</span>\r\n\r\n";

const END: &str = "</div>\r\n\r\n</body>\r\n\r\n</html>\r\n";

fn heading(level: u8, number: &str, title: &str, toc: u32) -> String {
    let gap = if level == 1 { "&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;" } else { "&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;" };
    format!("<h{level}><a name=\"_Toc{toc}\">{number}<span style='font:7.0pt \"Times New Roman\"'>{gap}\r\n</span>{title}</a></h{level}>\r\n\r\n")
}

fn para(class: &str, text: &str) -> String {
    format!("<p class={class}>{text}</p>\r\n\r\n")
}

/// A rule's paragraph as Word writes it: its anchors, its ID in bold with
/// a run of spaces, its headline in a span, then its words.
fn rule(class: &str, id: &str, headline: &str, text: &str) -> String {
    format!(
        "<p class={class}><a name=\"_Ref{}\"></a><a name={id}></a><b>{id}<span\r\nstyle='font:7.0pt \"Times New Roman\"'>&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp; </span></b><span\r\nclass=Headline-Evergreen>{headline}</span> {text}</p>\r\n\r\n",
        100_000_000 + id.bytes().map(u32::from).sum::<u32>()
    )
}

fn violation(text: &str) -> String {
    para("Violation", &format!("Violation: {text}"))
}

fn blue_box(paragraphs: &[&str]) -> String {
    let mut out = "<div style='border:solid #0083AE 1.5pt;padding:1.0pt 4.0pt 1.0pt 4.0pt;\r\nbackground:#ABD8E7;margin-left:1.0in;margin-right:1.0in'>\r\n\r\n".to_owned();
    for p in paragraphs {
        let class = if p.starts_with("A. ") || p.starts_with("B. ") || p.starts_with("C. ") { "BlueBox-letteredlist" } else { "BlueBox" };
        out.push_str(&format!("<p class={class} style='margin-top:6.0pt;background:#ABD8E7'><span\r\nstyle='color:black'>{p}</span></p>\r\n\r\n"));
    }
    out.push_str("</div>\r\n\r\n");
    out
}

fn lettered(items: &[&str]) -> String {
    let n = items.len();
    items
        .iter()
        .enumerate()
        .map(|(k, item)| {
            let class = match k {
                0 => "Rule-LetteredListCxSpFirst",
                _ if k + 1 == n => "Rule-LetteredListCxSpLast",
                _ => "Rule-LetteredListCxSpMiddle",
            };
            let letter = (b'A' + k as u8) as char;
            para(class, &format!("{letter}.<span style='font:7.0pt \"Times New Roman\"'>&nbsp;&nbsp;&nbsp;&nbsp;&nbsp; </span>{item}"))
        })
        .collect()
}

fn table(caption: &str, rows: &[[&str; 3]]) -> String {
    let mut out = format!("<p class=MsoCaption>{caption}</p>\r\n\r\n<table class=REBUILTManualTable border=1 cellspacing=0 cellpadding=0>\r\n");
    for row in rows {
        out.push_str(" <tr>\r\n");
        for cell in row {
            out.push_str(&format!("  <td width=208 valign=top style='padding:0in 5.4pt'>\r\n  <p class=MsoNormal>{cell}</p>\r\n  </td>\r\n"));
        }
        out.push_str(" </tr>\r\n");
    }
    out.push_str("</table>\r\n\r\n");
    out
}

/// The sample as text; `revised` is its next edition (one rule changed,
/// one added, one taken out), for what an update shows.
fn html(revised: bool) -> String {
    let mut s = String::from(HEAD);
    s.push_str(COVER);
    s += &heading(1, "1", "Introduction", 100000001);
    s += &heading(2, "1.1", "About This Sample", 100000002);
    s += &para("MsoNormal", "This sample stands in for the game manual in Catalyst’s simulator. Its rules are written for the handheld’s tests and pictures; they are not FIRST’s rules. On a handheld, Rulebook downloads FIRST’s own manual.");
    s += &para("MsoNormal", "Words in CAPITALS are defined in the glossary. A rule’s ID gives its section: G for game rules, R for ROBOT rules, E for event rules.");
    s += &heading(1, "6", "Game Details", 100000006);
    s += &heading(2, "6.1", "MATCH Periods", 100000061);
    s += &para("MsoNormal", "A MATCH has two periods. AUTO is the first 15 seconds, when ROBOTS run on their own with no input from their DRIVERS. TELEOP is the 2 minutes and 15 seconds after it, when DRIVERS steer.");
    s += &heading(2, "6.2", "Scoring", 100000062);
    s += &para("MsoNormal", "Points are counted by the FIELD as the MATCH runs, and confirmed by the REFEREES once it ends.");
    s += &table("Table 6&#8209;1 Points for each task", &[["Task", "AUTO", "TELEOP"], ["GAME PIECE in the low GOAL", "3", "2"], ["GAME PIECE in the high GOAL", "6", "4"], ["ROBOT parked in its ZONE", "–", "3"]]);
    s += &para("MsoCaption", "Figure 6&#8209;1 The FIELD from above");
    s += &heading(1, "7", "Game Rules (G)", 100000007);
    s += &heading(2, "7.1", "Personal Safety", 100000071);
    s += &rule("RuleNumber-Game", "G101", "*Stay outside the FIELD.", "A team member may not reach into the FIELD with any part of their body during a MATCH.");
    s += &violation("VERBAL WARNING. YELLOW CARD for each one after it at the event.");
    s += &blue_box(&["A violation counts against the whole team, not the person. If one student reaches in during MATCH 3 and another during MATCH 25, the team’s first is a VERBAL WARNING and its second a YELLOW CARD."]);
    s += &rule("RuleNumber-Game", "G102", "*Use the gates.", "Team members go onto the FIELD only through an open gate, and only when the FIELD’s lights are green, unless a REFEREE says otherwise.");
    s += &violation("VERBAL WARNING. YELLOW CARD for each one after it at the event.");
    s += &heading(2, "7.2", "Conduct", 100000072);
    s += &rule("RuleNumber-Game", "G201", "*Be kind.", "Teams treat everyone at the event with respect. Examples of what is not allowed:");
    s += &lettered(&["taunting another team or its ROBOT,", "arguing with a REFEREE about a call,", "leaving trash in the stands or the pit."]);
    s += &violation("VERBAL WARNING, or a YELLOW CARD if it was meant or is serious.");
    s += &rule("RuleNumber-Game", "G202", "Only DRIVE TEAMS on the ALLIANCE wall.", "During a MATCH, only a team’s DRIVE TEAM of at most 4 people may stand at its ALLIANCE wall.");
    s += &violation("MINOR FOUL.");
    s += &heading(2, "7.3", "Gameplay", 100000073);
    s += &rule("RuleNumber-Game", "G301", "Hold at most 2 GAME PIECES.", "A ROBOT may control no more than 2 GAME PIECES at a time.");
    s += &violation("MINOR FOUL for each GAME PIECE over the limit. MAJOR FOUL if it goes on for more than 5 seconds.");
    s += &blue_box(&["Pushing a GAME PIECE along the carpet without holding it is not control. Carrying one inside the BUMPERS, or pinning one against a wall, is."]);
    s += &rule("RuleNumber-Game", "G302", "No pinning.", "A ROBOT may not pin an opposing ROBOT against a FIELD element or another ROBOT for more than 3 seconds.");
    s += &violation("MINOR FOUL, and a MINOR FOUL for every 3 seconds after that.");
    if revised {
        s += &rule("RuleNumber-Game", "G304", "Stay off the RAMP in AUTO.", "In AUTO, no part of a ROBOT may touch the RAMP of the other ALLIANCE.");
        s += &violation("MAJOR FOUL.");
    } else {
        s += &rule("RuleNumber-Game", "G303", "Leave the BUMP alone.", "A ROBOT may not move or lift the BUMP in the middle of the FIELD.");
        s += &violation("MAJOR FOUL.");
    }
    s += &heading(1, "8", "ROBOT Construction Rules (R)", 100000008);
    s += &heading(2, "8.1", "General", 100000081);
    s += &rule("RuleNumber-Robot", "R101", "*Starting size.", "A ROBOT in its starting configuration fits within a 30 in. by 30 in. square, and is no taller than 3 ft. 6 in.");
    s += &blue_box(&["Teams should leave room for BUMPERS, which are counted in the size, and for the cables that stick out of motor controllers."]);
    s += &rule("RuleNumber-Robot", "R102", "*Weight limit.", "A ROBOT weighs no more than 115 lb., without its BUMPERS and battery.");
    s += &heading(2, "8.2", "BUMPERS", 100000082);
    s += &rule("RuleNumber-Robot", "R201", "*BUMPERS all around.", "A ROBOT has BUMPERS that protect every outside corner and at least 6 in. of each side next to it.");
    s += &rule("RuleNumber-Robot", "R202", "*What BUMPERS are made of.", "Each BUMPER is made of:");
    s += &lettered(&["a backing of 3⁄4 in. plywood,", "padding of two stacked pool noodles, 2.5 in. across,", "a cloth cover in the ALLIANCE’s colour, red or blue."]);
    s += &blue_box(&["Swapping covers between MATCHES is fine, as long as the cover shows the right colour when the MATCH starts."]);
    s += &rule("RuleNumber-Robot", "R203", "*Team number on the BUMPERS.", "The team’s number is shown on the BUMPERS on at least two opposite sides, in numerals at least 4 in. tall.");
    s += &heading(2, "8.3", "Power", 100000083);
    if revised {
        s += &rule("RuleNumber-Robot", "R301", "*One battery.", "The ROBOT is powered by a single 12 V sealed lead-acid battery of 17–18.2 Ah, held in with a strap or a bracket.");
    } else {
        s += &rule("RuleNumber-Robot", "R301", "*One battery.", "The ROBOT is powered by a single 12 V sealed lead-acid battery of about 18 Ah.");
    }
    s += &rule("RuleNumber-Robot", "R302", "*A main breaker.", "All power passes through one 120 A main breaker, which is easy to reach when the ROBOT is on the FIELD.");
    s += &heading(1, "9", "Event Rules (E)", 100000009);
    s += &heading(2, "9.1", "Safety", 100000091);
    s += &rule("RuleNumber-Event", "E101", "*Safety glasses.", "Everyone in the pit and on the FIELD wears safety glasses.");
    s += &rule("RuleNumber-Event", "E102", "*No running.", "Nobody runs in the pit, the queue or the stands.");
    s += &heading(1, "10", "Glossary", 100000010);
    s += &table("Table 10&#8209;1 Terms", &[["Term", "Meaning", "Used in"], ["AUTO", "The first 15 seconds of a MATCH.", "6.1"], ["BUMPER", "Padding around a ROBOT’s frame.", "8.2"], ["MATCH", "A game of 2 minutes and 30 seconds.", "6.1"]]);
    s.push_str(END);
    s
}

/// The sample as its bytes would come from the server.
pub fn bytes() -> Vec<u8> {
    encode_mac_roman(&html(false))
}

/// The sample's next edition.
pub fn revised() -> Vec<u8> {
    encode_mac_roman(&html(true))
}
