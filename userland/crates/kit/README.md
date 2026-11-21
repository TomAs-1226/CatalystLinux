# Catalyst Kit

Write an app for Catalyst Linux that looks and moves like the shell's own
screens: the same rows, type, glass island and focus ring, Detent's
springs, the handheld's keys, and whatever look the handheld is wearing
(accent, light or dark, text size, theme, corners). The shell hands your app
the panel when its tile is opened and takes it back when it ends, or when
Ctrl-Q is pressed or Esc held for a second.

Two ways to write one:

- **In Rust**, with this crate. A full app, with pages and motion.
- **As a shell script**, with `catalyst-ui` (built from this crate, in the
  image): one native page at a time, its answer on standard output. Quick,
  and what `ai app` writes for you.

## A Rust app

`userland/catalogue/hello` is the one to copy. In short:

```rust
use catalyst_kit::prelude::*;

struct Hello { list: List }

impl Page for Hello {
    fn title(&self) -> &str { "Hello" }

    fn key(&mut self, key: Key, cx: &mut Cx) -> Flow {
        match key {
            Key::Back => Flow::Quit,
            Key::Enter => { cx.say("hello"); Flow::Stay }
            other => { self.list.key(other, 3); Flow::Stay }
        }
    }

    fn draw(&mut self, ui: &mut Ui) {
        let (top, h, bottom) = (ui.top(), ui.row_h(), ui.bottom());
        for (i, y) in self.list.layout(ui, 3, top, h, 3.0, bottom) {
            let r = Rect::new(PAD, y, W - 2.0 * PAD - 5.0, h);
            ui.icon_row(r, "star", ["one", "two", "three"][i], "", i == self.list.focus);
        }
        self.list.end(ui, 3, top, bottom);
        ui.hint("enter: say hello · esc: leave");
    }
}

fn main() {
    catalyst_kit::launch(|| Hello { list: List::default() }, &[("1-list", &[Key::Down])]);
}
```

**Pages.** An app is a stack of [`Page`]s. `key` answers a key with a
[`Flow`]: `Stay`, `Push` (a page slides in), `Replace` (one slides in in this
one's place), `Pop`, or `Quit`. Pages may also `tick` (move on their own:
true redraws), keep time without redrawing (`running`), act each frame
(`update`, which can say something or move pages), take text (`wants_text`),
or draw the whole panel (`full_screen`).

**Keys.** Outside text, F X Z C are the arrows and Backspace is Back, as
everywhere on Catalyst. Held keys repeat on the shell's clock. Brightness
(Fn + U, I) and the way out are handled for you.

**Drawing.** Lay out from the type, never from fixed pixels: `ui.top()` is
where a page starts under the island, `ui.row_h()` a row, `ui.bottom()` where
the hint line begins. Then the large text size fits as the standard one
does. Controls: `row`, `icon_row`, `switch_row`, `button`, `chip`, `keycap`,
`meter`, `stat`, `paragraph`, `hint` / `hint_any`, `empty`, and
`focus_ring`, which glides from the last thing focused. Colours come from
`ui.tokens` (Bezel's roles: `ink`, `dim`, `signal`, `ice`, surfaces) and
`ui.fault()` / `ui.good()`; never hand-pick a hex colour. `ui.canvas` draws
anything else. `ui.calm()` asks for no overshoot and no flourish.

**Icons.** Material Symbols by name (`userland/assets/icons`). An icon the
kit does not carry is drawn as nothing and fails the gallery.

**Keeping things.** `Saved::open("scores")` is a small `key = value` file in
the app's own folder (`$CATALYST_APP_DATA`), written whole or not at all.
That folder outlives updates of the app. `Rng::seeded()` for games.

**Seeing it.** On Windows, `cargo run -p your-app --profile sim` opens it in
a window at 4×. `--shots DIR` draws its gallery (the scenes passed to
`launch`, each a run of keys) to PNGs, in any look: `--light`, `--text
large`, `--theme bezel`, `--calm`, `--accent blue`. Look at every one.
The gallery deals with a fixed seed and uses a folder of its own.

**For the handheld.** Add the crate to `userland/Cargo.toml` and to the app
list in `scripts/build-shell.sh`; give it an app folder under
`catalyst-apps/src/<id>/` with `binary = <program>`. See
`catalyst-apps/README.md` for packing, signing and publishing.

**Memory.** An app costs nothing until it is opened and nothing after it
ends. Open, it holds about 1.8 MB of its own (the frame, the wallpaper, the
glass, glyphs) plus about 3 MB of its program's pages, which the kernel
shares and can drop; 6.8 MB resident at most, measured. The program is
about 3 MB (fonts and symbols included), 1.4 MB packed.

## A script app

```sh
#!/bin/sh
while choice=$(catalyst-ui menu "Pit" "timer:Match timer" "battery_full:Battery check"); do
	case "$choice" in
	"Battery check")
		n=$(catalyst-ui input "Battery" "Which battery is in?") || continue
		catalyst-ui confirm "Battery" "Mark battery $n ready?" && echo "$n" >> "$CATALYST_APP_DATA/ready"
		;;
	esac
done
```

with an app file saying `run = sh ./run.sh` and `screen = own`.

| Page | Says |
| --- | --- |
| `catalyst-ui menu TITLE ITEM...` | the item chosen (`icon:Label` gives it a symbol) |
| `catalyst-ui input TITLE [PROMPT] [--value TEXT]` | what was typed |
| `catalyst-ui confirm TITLE QUESTION [--yes W] [--no W]` | exit status 0 for yes |
| `catalyst-ui message TITLE TEXT` | nothing, once Enter or Esc is pressed |
| `catalyst-ui view TITLE [FILE]` | nothing: a file, or what is piped in, to scroll |

Esc on any page exits with status 1. `catalyst-pack new <id> "<Name>"`
writes a folder to start from; `ai app <what it should do>`, in the
Terminal, writes one for you, shows it whole, and puts it on Home only on
a yes.
