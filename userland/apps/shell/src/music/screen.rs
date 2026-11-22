//! The Music app's pages: the start, search (songs, albums, artists,
//! playlists), what is playing and up next, the library on the card, an
//! album or playlist with its download, the downloads, and the account.
//!
//! The library is a page of Music rather than an app of its own: one app
//! owns the one queue and the one player (and the media keys, the tile and
//! the lock screen's card), so there is never a question of which "player"
//! is playing. Offline, Music opens straight on it, and nothing in it asks
//! the network for anything.

use bezel::{Align, Rect};

use super::downloads::{self, State};
use super::library::{self, Meta};
use super::player::{self, Phase, Repeat};
use super::{covers, polite, store, Collection, Kind, Opened, Track};
use crate::gfx::{Gfx, ListState, H, PAD, W};
use crate::git::Job;
use crate::platform::Key;
use crate::robot::Robot;
use crate::shell::{Cx, Nav, Screen};

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Filter {
    Songs,
    Albums,
    Artists,
    Playlists,
}

impl Filter {
    const ALL: [Filter; 4] = [Filter::Songs, Filter::Albums, Filter::Artists, Filter::Playlists];
    fn name(self) -> &'static str {
        match self {
            Filter::Songs => "songs",
            Filter::Albums => "albums",
            Filter::Artists => "artists",
            Filter::Playlists => "playlists",
        }
    }
}

/// Where a list of songs came from: what its first rows offer.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum ListKind {
    /// On the card: play, shuffle.
    Card,
    /// From YouTube Music: play, shuffle, download all.
    YouTube,
    /// Played and kept: play, shuffle, save all to the library.
    Played,
}

#[derive(Clone, Debug)]
pub struct SongList {
    pub title: String,
    pub byline: String,
    pub songs: Vec<Track>,
    pub metas: Vec<Meta>,
    pub kind: ListKind,
    /// Downloaded into a playlist of this name too.
    pub playlist: Option<String>,
}

#[derive(Clone, Debug)]
enum Page {
    Start,
    Search,
    Playing,
    Queue,
    Account,
    Library,
    Artists,
    Albums(Option<String>),
    Songs(Box<SongList>),
    Downloads,
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum StartRow {
    NowPlaying,
    Search,
    Library,
    Downloads,
    Likes,
    Song(usize),
    Account,
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum LibRow {
    Artists,
    Albums,
    Songs,
    Played,
    Playlist(usize),
    Shuffle,
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum ListRow {
    Play,
    Shuffle,
    Download,
    Song(usize),
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum DlRow {
    Pause,
    WifiOnly,
    Charging,
    RetryFailed,
    ClearSaved,
    Item(usize),
}

/// What a search found.
#[derive(Clone)]
enum Found {
    Songs(Vec<Track>),
    Collections(Vec<Collection>),
}

/// The Music app: what is playing, search, the queue, the library, downloads.
pub struct MusicScreen {
    page: Page,
    list: ListState,
    /// Pages to go back to, with where their lists were.
    back: Vec<(Page, ListState)>,
    /// The start page's songs (quick picks, or likes), and their loading.
    picks: Vec<Track>,
    picks_are: String,
    picks_job: Option<Job<Result<(String, Vec<Track>), String>>>,
    picks_error: Option<String>,
    /// Search: what is typed, whether typing, what kind, what was found.
    query: String,
    typing: bool,
    filter: Filter,
    results: Vec<Track>,
    collections: Vec<Collection>,
    searched: String,
    search_job: Option<Job<Result<Found, String>>>,
    /// Offline, search looks on the card.
    searched_card: bool,
    /// An album, playlist or artist being opened.
    open_job: Option<Job<Result<Opened, String>>>,
    /// Typing in a library list narrows it.
    narrow: String,
    /// Account: the stream finder's version, and work on it.
    version: Option<Job<Option<String>>>,
    version_seen: Option<String>,
    updating: Option<Job<Result<String, String>>>,
    armed: bool,
    /// The progress bar's fill, gliding.
    bar: detent::Animated,
    gliding: bool,
    /// What was last drawn, to draw again only when it changes.
    covers_seen: u64,
    library_seen: u64,
    downloads_seen: u64,
    second_seen: u32,
    last_phase: Phase,
    changed: bool,
}

impl MusicScreen {
    /// Opened on a search for `query` (from Spotlight).
    pub fn searching(query: &str) -> Self {
        let mut screen = Self::new();
        screen.go(Page::Search);
        screen.query = query.to_owned();
        screen.run_search();
        screen
    }

    pub fn new() -> Self {
        let playing = player::now().track().is_some();
        // Offline, with nothing playing: straight to what is on the card.
        let offline = super::on_device() && !polite::online();
        let page = if playing {
            Page::Playing
        } else if offline {
            Page::Library
        } else {
            Page::Start
        };
        let mut screen = MusicScreen {
            page: Page::Start,
            list: ListState::default(),
            back: Vec::new(),
            picks: Vec::new(),
            picks_are: "quick picks".into(),
            picks_job: (!offline).then(|| Job::start(super::start_page)),
            picks_error: offline.then(|| "offline · your library and played songs are here".to_owned()),
            query: String::new(),
            typing: false,
            filter: Filter::Songs,
            results: Vec::new(),
            collections: Vec::new(),
            searched: String::new(),
            search_job: None,
            searched_card: false,
            open_job: None,
            narrow: String::new(),
            version: None,
            version_seen: None,
            updating: None,
            armed: false,
            bar: detent::Animated::new(0.0, detent::role::hold()).with_rest(0.002),
            gliding: false,
            covers_seen: 0,
            library_seen: 0,
            downloads_seen: 0,
            second_seen: u32::MAX,
            last_phase: Phase::Empty,
            changed: true,
        };
        screen.go(page);
        screen
    }

    fn go(&mut self, page: Page) {
        self.page = page;
        self.list = ListState::default();
        self.armed = false;
        self.narrow.clear();
        match &self.page {
            Page::Start => self.list.focus = if player::now().track().is_some() { 0 } else { 1 },
            Page::Queue => self.list.focus = player::now().at,
            Page::Account => {
                if self.version_seen.is_none() && self.version.is_none() {
                    self.version = Some(Job::start(super::resolver_version));
                }
            }
            _ => {}
        }
        self.changed = true;
    }

    /// Into a page below this one; Back returns here.
    fn push(&mut self, page: Page) {
        let here = std::mem::replace(&mut self.page, Page::Start);
        self.back.push((here, std::mem::take(&mut self.list)));
        self.go(page);
    }

    /// Back to the page above (or to `fallback` if there is none).
    fn pop(&mut self, fallback: Page) {
        match self.back.pop() {
            Some((page, list)) => {
                self.page = page;
                self.list = list;
                self.armed = false;
                self.narrow.clear();
                self.changed = true;
            }
            None => self.go(fallback),
        }
    }

    fn start_rows(&self) -> Vec<StartRow> {
        let mut rows = Vec::new();
        if player::now().track().is_some() {
            rows.push(StartRow::NowPlaying);
        }
        rows.push(StartRow::Search);
        rows.push(StartRow::Library);
        if !downloads::items().is_empty() {
            rows.push(StartRow::Downloads);
        }
        if super::signed_in() && polite::online() {
            rows.push(StartRow::Likes);
        }
        rows.extend((0..self.picks.len()).map(StartRow::Song));
        rows.push(StartRow::Account);
        rows
    }

    fn lib_rows(&self) -> Vec<LibRow> {
        let mut rows = vec![LibRow::Artists, LibRow::Albums, LibRow::Songs, LibRow::Played];
        rows.extend((0..library::playlists().len()).map(LibRow::Playlist));
        if library::count() + store::cache_count() > 0 {
            rows.push(LibRow::Shuffle);
        }
        rows
    }

    fn list_rows(list: &SongList, narrow: &str) -> Vec<ListRow> {
        let mut rows = Vec::new();
        if narrow.is_empty() && !list.songs.is_empty() {
            rows.push(ListRow::Play);
            rows.push(ListRow::Shuffle);
            if list.kind != ListKind::Card {
                rows.push(ListRow::Download);
            }
        }
        rows.extend(Self::shown(list, narrow).into_iter().map(ListRow::Song));
        rows
    }

    /// The songs of a list that hold what is typed.
    fn shown(list: &SongList, narrow: &str) -> Vec<usize> {
        let q = narrow.to_lowercase();
        (0..list.songs.len()).filter(|&i| q.is_empty() || format!("{} {} {}", list.songs[i].title, list.songs[i].artist, list.songs[i].album).to_lowercase().contains(&q)).collect()
    }

    fn dl_rows(&self) -> Vec<DlRow> {
        let items = downloads::items();
        let mut rows = vec![DlRow::Pause, DlRow::WifiOnly, DlRow::Charging];
        if items.iter().any(|i| matches!(i.state, State::Failed(_))) {
            rows.push(DlRow::RetryFailed);
        }
        if items.iter().any(|i| matches!(i.state, State::Done(_))) {
            rows.push(DlRow::ClearSaved);
        }
        rows.extend((0..items.len()).map(DlRow::Item));
        rows
    }

    fn run_search(&mut self) {
        let q = self.query.trim().to_owned();
        if q.is_empty() {
            return;
        }
        self.searched = q.clone();
        self.results.clear();
        self.collections.clear();
        self.list = ListState::default();
        // Offline: what is on the card.
        if super::on_device() && !polite::online() {
            self.searched_card = true;
            self.results = card_search(&q);
            self.changed = true;
            return;
        }
        self.searched_card = false;
        let filter = self.filter;
        self.search_job = Some(Job::start(move || match filter {
            Filter::Songs => super::search(&q).map(Found::Songs),
            Filter::Albums => super::search_collections(&q, Kind::Album).map(Found::Collections),
            Filter::Artists => super::search_collections(&q, Kind::Artist).map(Found::Collections),
            Filter::Playlists => super::search_collections(&q, Kind::Playlist).map(Found::Collections),
        }));
    }

    fn open(&mut self, c: Collection) {
        let title = c.title.clone();
        self.push(Page::Songs(Box::new(SongList { title, byline: c.byline.clone(), songs: Vec::new(), metas: Vec::new(), kind: ListKind::YouTube, playlist: None })));
        self.open_job = Some(Job::start(move || super::open_collection(&c)));
    }

    fn songs_page(title: &str, byline: String, songs: Vec<Track>, kind: ListKind) -> Page {
        let metas = vec![Meta::default(); songs.len()];
        Page::Songs(Box::new(SongList { title: title.into(), byline, songs, metas, kind, playlist: None }))
    }
}

/// Offline search: the library's songs, then songs played and kept.
fn card_search(q: &str) -> Vec<Track> {
    let mut found = library::search(q);
    let words: Vec<String> = q.to_lowercase().split_whitespace().map(str::to_owned).collect();
    for p in store::played() {
        let hay = format!("{} {} {}", p.track.title, p.track.artist, p.track.album).to_lowercase();
        if words.iter().all(|w| hay.contains(w.as_str())) && !found.iter().any(|t| t.id == p.track.id) {
            found.push(p.track);
        }
    }
    found
}

/// Songs played and kept, as a list to play (the latest first).
fn played_list() -> SongList {
    let played = store::played();
    let bytes: u64 = played.iter().map(|p| p.bytes).sum();
    let songs: Vec<Track> = played.into_iter().map(|p| p.track).collect();
    SongList { title: "Played".into(), byline: format!("{} · {}", count_words(songs.len(), "song"), store::size_words(bytes)), metas: vec![Meta::default(); songs.len()], songs, kind: ListKind::Played, playlist: None }
}

fn count_words(n: usize, what: &str) -> String {
    if n == 1 {
        format!("1 {what}")
    } else {
        format!("{n} {what}s")
    }
}

impl Screen for MusicScreen {
    fn title(&self) -> &str {
        // The title is borrowed; the page's own is kept in a field each change.
        match &self.page {
            Page::Start | Page::Playing => "Music",
            Page::Search => "Search",
            Page::Queue => "Up next",
            Page::Account => "Account",
            Page::Library => "Library",
            Page::Artists => "Artists",
            Page::Albums(None) => "Albums",
            Page::Albums(Some(a)) => a,
            Page::Songs(l) => &l.title,
            Page::Downloads => "Downloads",
        }
    }

    /// The start page, search and the library's lists take letters: typing
    /// there searches or narrows (the shell would otherwise make f, x, z
    /// and c arrows).
    fn wants_text(&self) -> bool {
        self.typing || matches!(self.page, Page::Start | Page::Search | Page::Artists | Page::Albums(_) | Page::Songs(_))
    }

    fn key(&mut self, key: Key, cx: &mut Cx) -> Nav {
        self.changed = true;
        let n = player::now();
        // Typing a search: letters into the field, Enter looks.
        if self.typing {
            match key {
                Key::Char(c) => self.query.push(c),
                Key::Backspace => {
                    self.query.pop();
                }
                Key::Enter => {
                    self.typing = false;
                    self.run_search();
                }
                Key::Tab => {
                    self.filter = Filter::ALL[(Filter::ALL.iter().position(|f| *f == self.filter).unwrap_or(0) + 1) % 4];
                }
                Key::Back | Key::Leave => {
                    self.typing = false;
                    if self.results.is_empty() && self.collections.is_empty() {
                        self.pop(Page::Start);
                    }
                }
                Key::Down if !self.results.is_empty() || !self.collections.is_empty() => self.typing = false,
                _ => {}
            }
            return Nav::Stay;
        }
        let page = self.page.clone();
        match page {
            Page::Playing => match key {
                Key::Back => return Nav::Pop,
                Key::Enter | Key::Char(' ') => player::toggle(),
                Key::Left => player::previous(),
                Key::Right => player::next(),
                Key::Char(',') => player::seek(-10.0),
                Key::Char('.') => player::seek(10.0),
                Key::Char('s') => player::shuffle(),
                Key::Char('r') => player::repeat(),
                Key::Char('d') => {
                    if let Some(t) = n.track().cloned() {
                        *cx.say = Some(save_one(&t));
                    }
                }
                Key::Down | Key::Char('u') => self.push(Page::Queue),
                Key::Up | Key::Tab => self.go(Page::Start),
                Key::Char('l') => self.push(Page::Library),
                Key::Char('/') => {
                    self.push(Page::Search);
                    self.typing = true;
                }
                _ => {}
            },
            Page::Start => {
                let rows = self.start_rows();
                let row = rows.get(self.list.focus).copied().unwrap_or(StartRow::Search);
                match key {
                    Key::Back => return Nav::Pop,
                    Key::Tab if n.track().is_some() => self.go(Page::Playing),
                    Key::Enter | Key::Right => match row {
                        StartRow::NowPlaying => self.go(Page::Playing),
                        StartRow::Search => {
                            self.push(Page::Search);
                            self.typing = true;
                        }
                        StartRow::Library => self.push(Page::Library),
                        StartRow::Downloads => self.push(Page::Downloads),
                        StartRow::Likes => self.open(Collection { kind: Kind::Liked, id: "FEmusic_liked_videos".into(), title: "Your likes".into(), byline: "signed in".into(), art: String::new(), year: 0 }),
                        StartRow::Song(i) => {
                            // Quick picks are a mix already: played as a list; likes too.
                            player::play(self.picks.clone(), i, false);
                            self.go(Page::Playing);
                        }
                        StartRow::Account => self.push(Page::Account),
                    },
                    Key::Ctrl('n') => {
                        if let StartRow::Song(i) = row {
                            player::play_next(self.picks[i].clone());
                            *cx.say = Some(format!("next: {}", self.picks[i].title));
                        }
                    }
                    Key::Char('d') => {
                        if let StartRow::Song(i) = row {
                            *cx.say = Some(save_one(&self.picks[i]));
                        }
                    }
                    // Typing anywhere on the start page starts a search.
                    Key::Char(c) if c.is_alphanumeric() => {
                        self.push(Page::Search);
                        self.query = c.to_string();
                        self.typing = true;
                    }
                    other => crate::screens::list_key(&mut self.list, other, rows.len()),
                }
            }
            Page::Search => {
                let count = if self.filter == Filter::Songs || self.searched_card { self.results.len() } else { self.collections.len() };
                match key {
                    Key::Back => self.pop(Page::Start),
                    Key::Up if self.list.focus == 0 => self.typing = true,
                    Key::Char('/') => self.typing = true,
                    Key::Tab => {
                        self.filter = Filter::ALL[(Filter::ALL.iter().position(|f| *f == self.filter).unwrap_or(0) + 1) % 4];
                        self.run_search();
                    }
                    Key::Enter | Key::Right if self.filter == Filter::Songs || self.searched_card => {
                        if let Some(t) = self.results.get(self.list.focus) {
                            // A song alone: its radio after it (online).
                            player::play(vec![t.clone()], 0, true);
                            self.go(Page::Playing);
                        }
                    }
                    Key::Enter | Key::Right => {
                        if let Some(c) = self.collections.get(self.list.focus).cloned() {
                            self.open(c);
                        }
                    }
                    Key::Ctrl('n') => {
                        if let Some(t) = self.results.get(self.list.focus).cloned() {
                            *cx.say = Some(format!("next: {}", t.title));
                            player::play_next(t);
                        }
                    }
                    Key::Char('d') if self.filter == Filter::Songs => {
                        if let Some(t) = self.results.get(self.list.focus) {
                            *cx.say = Some(save_one(t));
                        }
                    }
                    Key::Char(c) if c.is_alphanumeric() => {
                        self.query = c.to_string();
                        self.typing = true;
                    }
                    other => crate::screens::list_key(&mut self.list, other, count),
                }
            }
            Page::Queue => match key {
                Key::Back => self.pop(Page::Playing),
                Key::Enter | Key::Right => {
                    player::jump(self.list.focus);
                    self.back.clear();
                    self.go(Page::Playing);
                }
                Key::Delete => player::remove(self.list.focus),
                other => crate::screens::list_key(&mut self.list, other, n.queue.len()),
            },
            Page::Account => {
                let armed = std::mem::take(&mut self.armed);
                match key {
                    Key::Back => self.pop(Page::Start),
                    Key::Enter if self.list.focus == 1 && self.updating.is_none() => self.updating = Some(Job::start(super::update_resolver)),
                    Key::Enter if self.list.focus == 2 && super::signed_in() => {
                        if armed {
                            match super::sign_out() {
                                Ok(()) => *cx.say = Some("signed out".into()),
                                Err(why) => *cx.say = Some(why),
                            }
                            self.picks_job = Some(Job::start(super::start_page));
                        } else {
                            self.armed = true;
                        }
                    }
                    other => crate::screens::list_key(&mut self.list, other, if super::signed_in() { 3 } else { 2 }),
                }
            }
            Page::Library => {
                let rows = self.lib_rows();
                match key {
                    Key::Back => self.pop(Page::Start),
                    Key::Enter | Key::Right => match rows.get(self.list.focus).copied() {
                        Some(LibRow::Artists) => self.push(Page::Artists),
                        Some(LibRow::Albums) => self.push(Page::Albums(None)),
                        Some(LibRow::Songs) => {
                            let songs: Vec<Track> = library::songs().iter().map(library::Song::track).collect();
                            let byline = count_words(songs.len(), "song");
                            self.push(Self::songs_page("Songs", byline, songs, ListKind::Card));
                        }
                        Some(LibRow::Played) => self.push(Page::Songs(Box::new(played_list()))),
                        Some(LibRow::Playlist(i)) => {
                            if let Some((name, songs)) = library::playlists().into_iter().nth(i) {
                                let songs: Vec<Track> = songs.iter().map(library::Song::track).collect();
                                let byline = count_words(songs.len(), "song");
                                self.push(Self::songs_page(&name, byline, songs, ListKind::Card));
                            }
                        }
                        Some(LibRow::Shuffle) => {
                            let mut all: Vec<Track> = library::songs().iter().map(library::Song::track).collect();
                            all.extend(store::played().into_iter().map(|p| p.track).filter(|t| library::by_youtube(&t.id).is_none()));
                            if !all.is_empty() {
                                player::play(all, 0, false);
                                player::shuffle();
                                self.go(Page::Playing);
                            }
                        }
                        None => {}
                    },
                    Key::Char('/') => {
                        self.push(Page::Search);
                        self.typing = true;
                    }
                    other => crate::screens::list_key(&mut self.list, other, rows.len()),
                }
            }
            Page::Artists => {
                let artists: Vec<_> = library::artists().into_iter().filter(|a| narrowed(&self.narrow, &a.0)).collect();
                match key {
                    Key::Back if !self.narrow.is_empty() => self.narrow.clear(),
                    Key::Back => self.pop(Page::Library),
                    Key::Enter | Key::Right => {
                        if let Some((a, ..)) = artists.get(self.list.focus).cloned() {
                            self.push(Page::Albums(Some(a)));
                        }
                    }
                    Key::Backspace => {
                        self.narrow.pop();
                        self.list = ListState::default();
                    }
                    Key::Char(c) if !c.is_control() => {
                        self.narrow.push(c);
                        self.list = ListState::default();
                    }
                    other => crate::screens::list_key(&mut self.list, other, artists.len()),
                }
            }
            Page::Albums(artist) => {
                let albums: Vec<_> = library::albums(artist.as_deref()).into_iter().filter(|a| narrowed(&self.narrow, &format!("{} {}", a.name, a.artist))).collect();
                let extra = usize::from(artist.is_some());
                match key {
                    Key::Back if !self.narrow.is_empty() => self.narrow.clear(),
                    Key::Back => self.pop(Page::Library),
                    Key::Enter | Key::Right => {
                        // An artist's page starts with all their songs.
                        if extra == 1 && self.list.focus == 0 {
                            let songs: Vec<Track> = albums.iter().flat_map(|a| a.songs.iter().map(library::Song::track)).collect();
                            let byline = count_words(songs.len(), "song");
                            self.push(Self::songs_page(artist.as_deref().unwrap_or("Songs"), byline, songs, ListKind::Card));
                        } else if let Some(a) = albums.get(self.list.focus - extra) {
                            let songs: Vec<Track> = a.songs.iter().map(library::Song::track).collect();
                            let byline = if a.year > 0 { format!("{} · {}", a.artist, a.year) } else { a.artist.clone() };
                            self.push(Self::songs_page(&a.name, byline, songs, ListKind::Card));
                        }
                    }
                    Key::Backspace => {
                        self.narrow.pop();
                        self.list = ListState::default();
                    }
                    Key::Char(c) if !c.is_control() => {
                        self.narrow.push(c);
                        self.list = ListState::default();
                    }
                    other => crate::screens::list_key(&mut self.list, other, albums.len() + extra),
                }
            }
            Page::Songs(list) => {
                let rows = Self::list_rows(&list, &self.narrow);
                let armed = std::mem::take(&mut self.armed);
                match key {
                    Key::Back if !self.narrow.is_empty() => self.narrow.clear(),
                    Key::Back => {
                        self.open_job = None;
                        self.pop(Page::Library);
                    }
                    Key::Enter | Key::Right => match rows.get(self.list.focus).copied() {
                        Some(ListRow::Play) => {
                            player::play(list.songs.clone(), 0, false);
                            self.back.clear();
                            self.go(Page::Playing);
                        }
                        Some(ListRow::Shuffle) => {
                            player::play(list.songs.clone(), 0, false);
                            player::shuffle();
                            self.back.clear();
                            self.go(Page::Playing);
                        }
                        Some(ListRow::Download) if armed => {
                            let songs: Vec<(Track, Meta)> = list.songs.iter().cloned().zip(list.metas.iter().cloned()).collect();
                            let added = downloads::add(songs, &list.title, list.playlist.as_deref());
                            *cx.say = Some(if added == 0 { "all of it is saved already".into() } else { format!("{} to download · Downloads shows them", count_words(added, "song")) });
                            if added > 0 {
                                self.push(Page::Downloads);
                            }
                        }
                        Some(ListRow::Download) => self.armed = true,
                        Some(ListRow::Song(i)) => {
                            player::play(list.songs.clone(), i, false);
                            self.back.clear();
                            self.go(Page::Playing);
                        }
                        None => {}
                    },
                    Key::Ctrl('n') => {
                        if let Some(ListRow::Song(i)) = rows.get(self.list.focus).copied() {
                            player::play_next(list.songs[i].clone());
                            *cx.say = Some(format!("next: {}", list.songs[i].title));
                        }
                    }
                    Key::Backspace => {
                        self.narrow.pop();
                        self.list = ListState::default();
                    }
                    Key::Char(c) if !c.is_control() && list.songs.len() > 8 => {
                        self.narrow.push(c);
                        self.list = ListState::default();
                    }
                    other => crate::screens::list_key(&mut self.list, other, rows.len()),
                }
            }
            Page::Downloads => {
                let rows = self.dl_rows();
                let items = downloads::items();
                let mut c = store::choices();
                match key {
                    Key::Back => self.pop(Page::Start),
                    Key::Enter | Key::Right => match rows.get(self.list.focus).copied() {
                        Some(DlRow::Pause) => {
                            c.paused = !c.paused;
                            store::set_choices(c);
                            *cx.say = Some((if c.paused { "downloads paused" } else { "downloads go on" }).into());
                        }
                        Some(DlRow::WifiOnly) => {
                            c.wifi_only = !c.wifi_only;
                            store::set_choices(c);
                        }
                        Some(DlRow::Charging) => {
                            c.charging_only = !c.charging_only;
                            store::set_choices(c);
                        }
                        Some(DlRow::RetryFailed) => downloads::retry_failed(),
                        Some(DlRow::ClearSaved) => {
                            downloads::clear_finished();
                            self.list = ListState::default();
                        }
                        Some(DlRow::Item(i)) => {
                            if let Some(item) = items.get(i) {
                                match &item.state {
                                    State::Failed(_) => downloads::retry(item.n),
                                    State::Done(path) => *cx.say = Some(format!("saved in {}", short_place(path))),
                                    _ => {}
                                }
                            }
                        }
                        None => {}
                    },
                    Key::Delete => {
                        if let Some(DlRow::Item(i)) = rows.get(self.list.focus).copied() {
                            if let Some(item) = items.get(i) {
                                downloads::cancel(item.n);
                                *cx.say = Some(format!("taken out: {}", item.track.title));
                            }
                        }
                    }
                    other => crate::screens::list_key(&mut self.list, other, rows.len()),
                }
            }
        }
        Nav::Stay
    }

    fn update(&mut self, cx: &mut Cx) -> Nav {
        if let Some(done) = self.picks_job.as_ref().and_then(Job::get) {
            self.picks_job = None;
            match done {
                Ok((what, t)) => {
                    super::know(&t);
                    self.picks = t;
                    self.picks_are = what;
                    self.picks_error = None;
                }
                Err(why) => self.picks_error = Some(why),
            }
            self.changed = true;
        }
        if let Some(done) = self.search_job.as_ref().and_then(Job::get) {
            self.search_job = None;
            match done {
                Ok(Found::Songs(t)) if t.is_empty() => *cx.say = Some("no songs found".into()),
                Ok(Found::Songs(t)) => {
                    super::know(&t);
                    self.results = t;
                }
                Ok(Found::Collections(c)) if c.is_empty() => *cx.say = Some(format!("no {} found", self.filter.name())),
                Ok(Found::Collections(c)) => self.collections = c,
                Err(why) if why == super::lookup::NO_INTERNET => {
                    // Offline after all: what is on the card.
                    self.searched_card = true;
                    self.results = card_search(&self.searched);
                    *cx.say = Some("offline · searched what is on the card".into());
                }
                Err(why) => *cx.say = Some(why),
            }
            self.changed = true;
        }
        if let Some(done) = self.open_job.as_ref().and_then(Job::get) {
            self.open_job = None;
            match done {
                Ok(o) => {
                    super::know(&o.tracks);
                    if let Page::Songs(list) = &mut self.page {
                        list.byline = o.byline.clone();
                        list.metas = o.metas.clone();
                        list.songs = o.tracks;
                        list.playlist = o.playlist;
                    }
                }
                Err(why) => *cx.say = Some(why),
            }
            self.changed = true;
        }
        if let Some(done) = self.version.as_ref().and_then(Job::get) {
            self.version = None;
            self.version_seen = done.or(Some("–".into()));
            self.changed = true;
        }
        if let Some(done) = self.updating.as_ref().and_then(Job::get) {
            self.updating = None;
            match done {
                Ok(v) => {
                    *cx.say = Some(format!("stream finder {v}"));
                    self.version_seen = Some(v);
                }
                Err(why) => *cx.say = Some(why),
            }
            self.changed = true;
        }
        // A problem playing is said once, as it happens.
        let phase = player::now().phase;
        if phase != self.last_phase {
            if let Phase::Failed(why) = &phase {
                *cx.say = Some(why.clone());
            }
            self.last_phase = phase;
            self.changed = true;
        }
        Nav::Stay
    }

    fn tick(&mut self, dt: f32) -> bool {
        let n = player::now();
        let fraction = match n.length {
            Some(len) if len > 0.0 => n.position / len,
            _ => 0.0,
        };
        if (self.bar.target() - fraction).abs() > 0.0005 {
            // A jump (a new song, a seek) glides; play itself moves on in steps too small to see.
            if (self.bar.target() - fraction).abs() > 0.05 {
                self.bar.set(fraction);
            } else {
                self.bar.jump(fraction);
            }
        }
        self.gliding = self.bar.tick(dt);
        // Drawn again only when what shows changes: the second shown, a
        // cover arriving, the library or the downloads moving on.
        let mut changed = std::mem::take(&mut self.changed) | self.gliding;
        let second = n.position as u32;
        if second != self.second_seen && matches!(self.page, Page::Playing | Page::Start) {
            self.second_seen = second;
            changed = true;
        }
        for (seen, now) in [(&mut self.covers_seen, covers::ARRIVED.load(std::sync::atomic::Ordering::Relaxed)), (&mut self.library_seen, library::stamp() + store::played_stamp()), (&mut self.downloads_seen, downloads::stamp())] {
            if *seen != now {
                *seen = now;
                changed = true;
            }
        }
        changed
    }

    /// Kept to the frame beat only while the bar glides; otherwise looked
    /// at four times a second, which the clock's seconds need.
    fn busy(&self) -> bool {
        self.gliding
    }

    fn keeps_awake(&self) -> bool {
        false
    }

    fn draw(&mut self, g: &mut Gfx, _robot: &Robot) {
        match self.page.clone() {
            Page::Playing => self.draw_playing(g),
            Page::Start => self.draw_start(g),
            Page::Search => self.draw_search(g),
            Page::Queue => self.draw_queue(g),
            Page::Account => self.draw_account(g),
            Page::Library => self.draw_library(g),
            Page::Artists => self.draw_artists(g),
            Page::Albums(a) => self.draw_albums(g, a.as_deref()),
            Page::Songs(l) => self.draw_songs(g, &l),
            Page::Downloads => self.draw_downloads(g),
        }
    }
}

fn narrowed(narrow: &str, hay: &str) -> bool {
    narrow.is_empty() || hay.to_lowercase().contains(&narrow.to_lowercase())
}

/// One song saved to the library (Playing, search, quick picks: D).
fn save_one(t: &Track) -> String {
    if t.local.is_some() || library::by_youtube(&t.id).is_some() {
        return "on the card already".into();
    }
    let meta = Meta { album_artist: String::new(), track_no: 0, year: 0 };
    match downloads::add(vec![(t.clone(), meta)], &t.title, None) {
        0 => "waiting to download already".into(),
        _ => format!("saving {} to the library", t.title),
    }
}

/// "/data/music/library/Mara Ellis/Night Bus/01 Slow Signal.opus" → "Mara Ellis/Night Bus".
fn short_place(path: &str) -> String {
    let p = std::path::Path::new(path);
    let lib = store::library_dir();
    let rel = p.strip_prefix(&lib).unwrap_or(p);
    rel.parent().map_or_else(|| rel.display().to_string(), |d| d.display().to_string().replace('\\', "/"))
}

impl MusicScreen {
    fn draw_playing(&mut self, g: &mut Gfx) {
        let t = g.tokens;
        let ty = g.ty;
        let n = player::now();
        let Some(track) = n.track().cloned() else {
            g.empty("music_note", "nothing playing · esc, then pick a song");
            return;
        };
        let top = g.top() + 2.0;
        let side = 96.0;
        let art = Rect::new(PAD, top, side, side);
        match covers::cover(&track) {
            Some(p) if !p.is_empty() => crate::photos::draw_picture(g, art, &p, [0.0, 0.0, p.w as f32, p.h as f32], 14.0, 1.0),
            _ => {
                g.surface(art, 14.0, t.surface2);
                g.icon("music_note", true, art.cx(), art.cy(), 34.0, t.faint);
            }
        }
        // To the right: the song, the artist and album, where it is, the transport.
        let x = art.right() + 12.0;
        let w = W - PAD - x;
        g.text_fit(x, top + ty.name.size * 0.6, &track.title, ty.name, t.ink, w);
        g.text_fit(x, top + ty.name.size + 4.0 + ty.body.size * 0.55, &track.artist, ty.body, t.dim, w);
        let here = track.local.is_some() || library::by_youtube(&track.id).is_some();
        let album = match (track.album.is_empty(), here) {
            (false, true) => format!("{} · on the card", track.album.to_lowercase()),
            (false, false) => track.album.to_lowercase(),
            (true, true) => "on the card".into(),
            (true, false) => String::new(),
        };
        if !album.is_empty() {
            g.text_fit(x, top + ty.name.size + ty.body.size + 8.0 + ty.caption.size * 0.5, &album, ty.caption, t.faint, w);
        }
        let bar_y = top + 56.0;
        g.meter(Rect::new(x, bar_y, w, 5.0), self.bar.value(), t.ice);
        let left = super::clock(n.position);
        let right = match n.length {
            Some(l) => format!("-{}", super::clock(l - n.position)),
            None => String::new(),
        };
        let times_y = bar_y + 7.0 + ty.caption.size * 0.5;
        g.text_at(x, times_y, Align::Left, &left, ty.caption, t.dim);
        g.text_at(W - PAD, times_y, Align::Right, &right, ty.caption, t.dim);
        // The transport: previous, play or pause (the primary action, in
        // signal), next; shuffle and repeat at its ends, lit when on.
        let cy = times_y + 25.0;
        let mid = x + w * 0.5;
        let playing = matches!(n.phase, Phase::Playing | Phase::Loading);
        for (dx, icon, r) in [(-46.0, "skip_previous", 13.0), (0.0, if playing { "pause" } else { "play_arrow" }, 16.0), (46.0, "skip_next", 13.0)] {
            let primary = dx == 0.0;
            g.dot(mid + dx, cy, r, if primary { t.signal } else { t.surface2 });
            g.icon(icon, true, mid + dx, cy, if primary { 20.0 } else { 17.0 }, if primary { t.on_signal } else { t.ink });
        }
        let repeat_icon = if n.repeat == Repeat::One { "repeat_one" } else { "repeat" };
        g.icon("shuffle", false, x + 8.0, cy, 15.0, if n.shuffle { t.ink } else { t.faint });
        g.icon(repeat_icon, false, W - PAD - 8.0, cy, 15.0, if n.repeat != Repeat::Off { t.ink } else { t.faint });
        let status = if n.buffering {
            Some("buffering…".to_owned())
        } else if n.phase == Phase::Loading {
            Some(n.loading.clone().unwrap_or_else(|| "loading…".into()))
        } else {
            None
        };
        if let Some(s) = status {
            g.text_at(mid, cy - 25.0, Align::Center, &s, ty.caption, t.dim);
        }
        let upcoming = n.queue.get(n.at + 1).map(|t| format!("next: {}", t.title.to_lowercase()));
        let hint = match (&n.phase, upcoming) {
            (Phase::Failed(_), _) => vec!["enter: try again · ←→: skip".to_owned(), "enter: try again".to_owned()],
            (_, Some(next)) => vec![format!("{next} · ↓ up next"), "enter: pause · s: shuffle · r: repeat".to_owned(), "enter · ←→ · ↓".to_owned()],
            _ => vec!["enter: pause · l: library · /: search".to_owned(), "enter · ← · /".to_owned()],
        };
        let choices: Vec<&str> = hint.iter().map(String::as_str).collect();
        g.hint_any(&choices);
    }

    fn draw_start(&mut self, g: &mut Gfx) {
        let t = g.tokens;
        let rows = self.start_rows();
        let (top, row_h, bottom) = (g.top(), g.row_h(), H - PAD - g.hint_h());
        let width = W - 2.0 * PAD - 5.0;
        let n = player::now();
        let focus = self.list.focus;
        let dl = downloads::summary();
        for (i, y) in self.list.layout(g, rows.len(), top, row_h, 3.0, bottom + 2.0) {
            let r = Rect::new(PAD, y, width, row_h);
            match rows[i] {
                StartRow::NowPlaying => {
                    let track = n.track().cloned().unwrap_or_default();
                    let icon = if n.phase == Phase::Playing { "graphic_eq" } else { "pause" };
                    crate::bluetooth::icon_row(g, r, icon, n.phase == Phase::Playing, &track.title, &super::clock(n.position), t.ink, i == focus);
                }
                StartRow::Search => crate::bluetooth::icon_row(g, r, "search", false, "Search", "", t.dim, i == focus),
                StartRow::Library => {
                    let count = library::count() + store::cache_count();
                    crate::bluetooth::icon_row(g, r, "library_music", false, "Library", &format!("{count} ›"), t.dim, i == focus)
                }
                StartRow::Downloads => {
                    let active = dl.done < dl.total;
                    crate::bluetooth::icon_row(g, r, if active { "downloading" } else { "download_done" }, active, "Downloads", &format!("{} of {} ›", dl.done, dl.total), t.dim, i == focus)
                }
                StartRow::Likes => crate::bluetooth::icon_row(g, r, "favorite", false, "Your likes", "›", t.dim, i == focus),
                StartRow::Song(s) => {
                    let track = &self.picks[s];
                    song_row(g, r, track, i == focus);
                }
                StartRow::Account => g.row(r, "Account", if super::signed_in() { "signed in ›" } else { "signed out ›" }, t.dim, i == focus),
            }
        }
        self.list.scrollbar(g, rows.len(), top, bottom);
        let row = rows.get(focus).copied();
        let songs = format!("{} · enter: play · d: save", self.picks_are);
        let hint: &[&str] = match row {
            Some(StartRow::Song(_)) => &[&songs, "enter: play · ctrl n: next", "enter: play"],
            Some(StartRow::NowPlaying) => &["enter: now playing · tab, anywhere here", "enter: now playing"],
            Some(StartRow::Library) => &["on the card · plays without the internet", "on the card"],
            Some(StartRow::Downloads) => &["saving to the library, one at a time", "saving to the library"],
            Some(StartRow::Account) if super::signed_in() => &["signed in with your browser's cookies", "signed in"],
            Some(StartRow::Account) => &["signed out unless you sign in", "signed out"],
            _ => &["type to search youtube music", "type to search"],
        };
        if self.picks_job.is_some() && matches!(row, Some(StartRow::Search)) {
            g.hint("loading…");
        } else if let (Some(why), Some(StartRow::Search | StartRow::Library)) = (&self.picks_error, row) {
            let why = why.clone();
            g.hint_any(&[&why, "offline"]);
        } else {
            g.hint_any(hint);
        }
    }

    fn draw_search(&mut self, g: &mut Gfx) {
        let t = g.tokens;
        let ty = g.ty;
        let top = g.top();
        // The field: what is typed, with a caret while typing; the kind of
        // thing looked for at its right.
        let field = Rect::new(PAD, top, W - 2.0 * PAD, g.row_h());
        g.surface(field, field.h * 0.5, if self.typing { t.surface2 } else { t.surface1 });
        g.icon("search", false, field.x + 14.0, field.cy(), 14.0, t.dim);
        let kind = if self.searched_card && !self.typing { "on the card" } else { self.filter.name() };
        let kind_w = g.measure(kind, ty.caption);
        g.text_at(field.right() - 12.0, field.cy(), Align::Right, kind, ty.caption, if self.typing { t.ink } else { t.dim });
        let shown = if self.query.is_empty() && !self.typing { "songs, albums, artists".to_owned() } else { self.query.clone() };
        let room = field.w - 40.0 - kind_w - 10.0;
        let text_w = g.text_fit(field.x + 27.0, field.cy(), &shown, ty.body, if self.query.is_empty() { t.faint } else { t.ink }, room);
        if self.typing {
            let caret_x = field.x + 27.0 + text_w.min(room) + 2.0;
            g.surface(Rect::new(caret_x, field.cy() - ty.body.size * 0.45, 1.5, ty.body.size * 0.9), 0.5, t.signal);
            g.focus_ring(field, field.h * 0.5);
        }
        let (list_top, row_h, bottom) = (field.bottom() + 5.0, g.row_h(), H - PAD - g.hint_h());
        let width = W - 2.0 * PAD - 5.0;
        if self.search_job.is_some() {
            g.text_at(W * 0.5, (list_top + bottom) * 0.5, Align::Center, "looking…", ty.caption, t.dim);
        }
        let focus = if self.typing { usize::MAX } else { self.list.focus };
        let songs = self.filter == Filter::Songs || self.searched_card;
        let count = if songs { self.results.len() } else { self.collections.len() };
        for (i, y) in self.list.layout(g, count, list_top, row_h, 3.0, bottom + 2.0) {
            let r = Rect::new(PAD, y, width, row_h);
            if songs {
                let track = self.results[i].clone();
                song_row(g, r, &track, i == focus);
            } else {
                let c = self.collections[i].clone();
                collection_row(g, r, &c, i == focus);
            }
        }
        self.list.scrollbar(g, count, list_top, bottom);
        if self.typing {
            g.hint_any(&["enter: search · tab: songs, albums, artists", "enter: search · tab: kind", "enter: search"]);
        } else if count > 0 && songs {
            g.hint_any(&["enter: play, then its radio · d: save", "enter: play · d: save", "enter: play"]);
        } else if count > 0 {
            g.hint_any(&["enter: open · tab: another kind", "enter: open"]);
        } else {
            g.hint("/: search again");
        }
    }

    fn draw_queue(&mut self, g: &mut Gfx) {
        let t = g.tokens;
        let n = player::now();
        let (top, row_h, bottom) = (g.top(), g.row_h(), H - PAD - g.hint_h());
        let width = W - 2.0 * PAD - 5.0;
        if n.queue.is_empty() {
            g.empty("queue_music", "nothing in the queue");
            return;
        }
        let focus = self.list.focus;
        for (i, y) in self.list.layout(g, n.queue.len(), top, row_h, 3.0, bottom + 2.0) {
            let r = Rect::new(PAD, y, width, row_h);
            let track = &n.queue[i];
            if i == n.at {
                let icon = if n.phase == Phase::Playing { "graphic_eq" } else { "pause" };
                crate::bluetooth::icon_row(g, r, icon, true, &track.title, "now", t.ink, i == focus);
            } else {
                song_row(g, r, track, i == focus);
            }
        }
        self.list.scrollbar(g, n.queue.len(), top, bottom);
        g.hint_any(&["enter: play this · del: take it out", "enter: play · del: out"]);
    }

    fn draw_library(&mut self, g: &mut Gfx) {
        let t = g.tokens;
        let rows = self.lib_rows();
        let (top, row_h, bottom) = (g.top(), g.row_h(), H - PAD - g.hint_h());
        let width = W - 2.0 * PAD - 5.0;
        let playlists = library::playlists();
        let (artists, albums, songs, played) = (library::artists().len(), library::albums(None).len(), library::count(), store::cache_count());
        for (i, y) in self.list.layout(g, rows.len(), top, row_h, 3.0, bottom + 2.0) {
            let r = Rect::new(PAD, y, width, row_h);
            let f = i == self.list.focus;
            match rows[i] {
                LibRow::Artists => crate::bluetooth::icon_row(g, r, "artist", false, "Artists", &format!("{artists} ›"), t.dim, f),
                LibRow::Albums => crate::bluetooth::icon_row(g, r, "album", false, "Albums", &format!("{albums} ›"), t.dim, f),
                LibRow::Songs => crate::bluetooth::icon_row(g, r, "music_note", false, "Songs", &format!("{songs} ›"), t.dim, f),
                LibRow::Played => crate::bluetooth::icon_row(g, r, "history", false, "Played", &format!("{played} ›"), t.dim, f),
                LibRow::Playlist(p) => {
                    let (name, list) = &playlists[p];
                    crate::bluetooth::icon_row(g, r, "playlist_play", false, name, &format!("{} ›", list.len()), t.dim, f)
                }
                LibRow::Shuffle => crate::bluetooth::icon_row(g, r, "shuffle", false, "Shuffle all", "", t.dim, f),
            }
        }
        self.list.scrollbar(g, rows.len(), top, bottom);
        let hint: &[&str] = match rows.get(self.list.focus) {
            Some(LibRow::Played) => &["every song played, kept to play offline", "kept to play offline"],
            Some(LibRow::Shuffle) => &["the library and played songs, shuffled", "all, shuffled"],
            _ if library::refreshing() => &["looking for new music on the card…", "looking for new music…"],
            _ if songs == 0 => &["save songs from search, or copy your own in", "nothing saved yet"],
            _ => &["on the card · plays without the internet", "on the card"],
        };
        g.hint_any(hint);
    }

    fn draw_artists(&mut self, g: &mut Gfx) {
        let t = g.tokens;
        let artists: Vec<_> = library::artists().into_iter().filter(|a| narrowed(&self.narrow, &a.0)).collect();
        let (top, row_h, bottom) = (g.top(), g.row_h(), H - PAD - g.hint_h());
        let width = W - 2.0 * PAD - 5.0;
        if artists.is_empty() {
            g.empty("artist", if self.narrow.is_empty() { "no artists on the card yet" } else { "no artist has that in their name" });
            self.narrow_hint(g);
            return;
        }
        for (i, y) in self.list.layout(g, artists.len(), top, row_h, 3.0, bottom + 2.0) {
            let (name, albums, songs) = &artists[i];
            let value = format!("{} · {}", count_words(*albums, "album"), songs);
            g.row(Rect::new(PAD, y, width, row_h), name, &value, t.dim, i == self.list.focus);
        }
        self.list.scrollbar(g, artists.len(), top, bottom);
        self.narrow_hint(g);
    }

    fn narrow_hint(&self, g: &mut Gfx) {
        if self.narrow.is_empty() {
            g.hint_any(&["enter: open · type to narrow", "enter: open"]);
        } else {
            let s = format!("“{}” · esc: all", self.narrow);
            g.hint_any(&[&s, "esc: all"]);
        }
    }

    fn draw_albums(&mut self, g: &mut Gfx, artist: Option<&str>) {
        let t = g.tokens;
        let albums: Vec<_> = library::albums(artist).into_iter().filter(|a| narrowed(&self.narrow, &format!("{} {}", a.name, a.artist))).collect();
        let extra = usize::from(artist.is_some());
        let (top, row_h, bottom) = (g.top(), g.row_h(), H - PAD - g.hint_h());
        let width = W - 2.0 * PAD - 5.0;
        if albums.is_empty() && extra == 0 {
            g.empty("album", if self.narrow.is_empty() { "no albums on the card yet" } else { "no album has that in its name" });
            self.narrow_hint(g);
            return;
        }
        for (i, y) in self.list.layout(g, albums.len() + extra, top, row_h, 3.0, bottom + 2.0) {
            let r = Rect::new(PAD, y, width, row_h);
            let f = i == self.list.focus;
            if extra == 1 && i == 0 {
                let n: usize = albums.iter().map(|a| a.songs.len()).sum();
                crate::bluetooth::icon_row(g, r, "music_note", false, "All songs", &n.to_string(), t.dim, f);
                continue;
            }
            let a = &albums[i - extra];
            let first = a.songs[0].track();
            let byline = if artist.is_some() { if a.year > 0 { a.year.to_string() } else { String::new() } } else { a.artist.to_lowercase() };
            art_row(g, r, &first, &a.name, &byline, &a.songs.len().to_string(), f);
        }
        self.list.scrollbar(g, albums.len() + extra, top, bottom);
        self.narrow_hint(g);
    }

    fn draw_songs(&mut self, g: &mut Gfx, list: &SongList) {
        let t = g.tokens;
        let ty = g.ty;
        let rows = Self::list_rows(list, &self.narrow);
        let (top, row_h, bottom) = (g.top(), g.row_h(), H - PAD - g.hint_h());
        let width = W - 2.0 * PAD - 5.0;
        if list.songs.is_empty() {
            let what = if self.open_job.is_some() { "opening…" } else { "no songs here" };
            g.text_at(W * 0.5, (top + bottom) * 0.5, Align::Center, what, ty.caption, t.dim);
            g.hint(if self.open_job.is_some() { "asking youtube music…" } else { "esc: back" });
            return;
        }
        let bytes = match list.kind {
            ListKind::Played => list.songs.iter().filter_map(|s| store::cached(&s.id)).map(|p| p.bytes).sum(),
            _ => downloads::estimate(&list.songs),
        };
        let saved = list.songs.iter().filter(|s| s.local.is_some() || library::by_youtube(&s.id).is_some()).count();
        for (i, y) in self.list.layout(g, rows.len(), top, row_h, 3.0, bottom + 2.0) {
            let r = Rect::new(PAD, y, width, row_h);
            let f = i == self.list.focus;
            match rows[i] {
                ListRow::Play => crate::bluetooth::icon_row(g, r, "play_arrow", false, "Play", &count_words(list.songs.len(), "song"), t.dim, f),
                ListRow::Shuffle => crate::bluetooth::icon_row(g, r, "shuffle", false, "Shuffle", "", t.dim, f),
                ListRow::Download => {
                    let (name, value) = match (list.kind, self.armed && f) {
                        (_, true) => ("Enter again to download", format!("~{}", store::size_words(bytes))),
                        (ListKind::Played, false) => ("Save all to the library", store::size_words(bytes)),
                        (_, false) if saved == list.songs.len() => ("Downloaded", "on the card".to_owned()),
                        (_, false) => ("Download all", format!("~{}", store::size_words(bytes))),
                    };
                    crate::bluetooth::icon_row(g, r, if saved == list.songs.len() { "download_done" } else { "download" }, self.armed && f, name, &value, t.dim, f)
                }
                ListRow::Song(s) => {
                    let track = list.songs[s].clone();
                    song_row(g, r, &track, f);
                }
            }
        }
        self.list.scrollbar(g, rows.len(), top, bottom);
        let byline = list.byline.to_lowercase();
        match rows.get(self.list.focus) {
            Some(ListRow::Download) if self.armed => g.hint_any(&["enter again: download · esc: not now", "enter again: download"]),
            Some(ListRow::Download) => g.hint_any(&["into the library, over wi-fi, one at a time", "into the library"]),
            _ if !self.narrow.is_empty() => {
                let s = format!("“{}” · esc: all", self.narrow);
                g.hint_any(&[&s, "esc: all"]);
            }
            _ if !byline.is_empty() => g.hint_any(&[&byline, "enter: play"]),
            _ => g.hint_any(&["enter: play · ctrl n: next", "enter: play"]),
        }
    }

    fn draw_downloads(&mut self, g: &mut Gfx) {
        let t = g.tokens;
        let ty = g.ty;
        let rows = self.dl_rows();
        let items = downloads::items();
        let s = downloads::summary();
        let c = store::choices();
        // The whole queue at a glance: a meter, how many and how much.
        let top = g.top();
        let line_y = top + ty.caption.size * 0.5 + 1.0;
        let words = format!("{} of {} saved · {} of {}", s.done, s.total, store::size_words(s.bytes_done), store::size_words(s.bytes_total.max(1)));
        g.text_fit(PAD + 2.0, line_y, &words, ty.caption, t.dim, W - 2.0 * PAD - 4.0);
        let fraction = if s.bytes_total > 0 { s.bytes_done as f32 / s.bytes_total as f32 } else { 0.0 };
        g.meter(Rect::new(PAD + 2.0, line_y + ty.caption.size * 0.5 + 4.0, W - 2.0 * PAD - 4.0, 4.0), fraction.min(1.0), t.ice);
        let list_top = line_y + ty.caption.size * 0.5 + 13.0;
        let (row_h, bottom) = (g.row_h(), H - PAD - g.hint_h());
        let width = W - 2.0 * PAD - 5.0;
        for (i, y) in self.list.layout(g, rows.len(), list_top, row_h, 3.0, bottom + 2.0) {
            let r = Rect::new(PAD, y, width, row_h);
            let f = i == self.list.focus;
            match rows[i] {
                DlRow::Pause => crate::bluetooth::icon_row(g, r, if c.paused { "play_arrow" } else { "pause" }, false, if c.paused { "Resume all" } else { "Pause all" }, "", t.dim, f),
                DlRow::WifiOnly => g.row(r, "Only over Wi-Fi", if c.wifi_only { "on" } else { "off" }, if c.wifi_only { t.ink } else { t.dim }, f),
                DlRow::Charging => g.row(r, "Only while charging", if c.charging_only { "on" } else { "off" }, if c.charging_only { t.ink } else { t.dim }, f),
                DlRow::RetryFailed => crate::bluetooth::icon_row(g, r, "sync", false, "Try the failed again", &s.failed.to_string(), t.dim, f),
                DlRow::ClearSaved => crate::bluetooth::icon_row(g, r, "check_circle", false, "Clear the saved", &s.done.to_string(), t.dim, f),
                DlRow::Item(k) => {
                    let item = &items[k];
                    let value = match &item.state {
                        State::Waiting if item.have > 0 => format!("{}%", item.have * 100 / item.total.max(1)),
                        State::Waiting => "waiting".to_owned(),
                        State::Active => format!("{}%", item.have * 100 / item.total.max(1)),
                        State::Done(_) => "saved".to_owned(),
                        State::Failed(_) => "failed".to_owned(),
                    };
                    g.row(r, "", &value, if matches!(item.state, State::Failed(_)) { t.signal } else { t.dim }, f);
                    let side = r.h - 8.0;
                    let art = Rect::new(r.x + 5.0, r.cy() - side * 0.5, side, side);
                    match covers::cover(&item.track) {
                        Some(p) if !p.is_empty() => crate::photos::draw_picture(g, art, &p, [0.0, 0.0, p.w as f32, p.h as f32], 5.0, 1.0),
                        _ => g.surface(art, 5.0, t.surface2),
                    }
                    let x = art.right() + 7.0;
                    let room = r.right() - 14.0 - g.measure(&value, if g.theme == crate::look::Theme::Bezel { ty.body } else { ty.label }) - x;
                    g.text_fit(x, r.cy() - if item.state == State::Active { 2.0 } else { 0.0 }, &item.track.title, ty.body, t.ink, room);
                    if item.state == State::Active {
                        g.meter(Rect::new(x, r.bottom() - 6.0, room, 2.5), (item.have as f32 / item.total.max(1) as f32).min(1.0), t.ice);
                    }
                }
            }
        }
        self.list.scrollbar(g, rows.len(), list_top, bottom);
        let waits = super::fetch::waits(true);
        let place = "saved in /data/music/library".to_owned();
        let focused = rows.get(self.list.focus).copied();
        match focused {
            Some(DlRow::Item(k)) => {
                let item = &items[k];
                let line = match &item.state {
                    State::Failed(why) => format!("{why} · enter: again · del: out"),
                    State::Done(p) => format!("in {}", short_place(p)),
                    _ => format!("{} · del: take it out", item.from.to_lowercase()),
                };
                g.hint_any(&[&line, "del: take it out"]);
            }
            _ => match waits {
                Some(w) if s.done < s.total => g.hint_any(&[w, "waiting"]),
                _ => g.hint_any(&[&place, "in the library"]),
            },
        }
    }

    fn draw_account(&mut self, g: &mut Gfx) {
        let t = g.tokens;
        let ty = g.ty;
        let (top, row_h, bottom) = (g.top(), g.row_h(), H - PAD - g.hint_h());
        let width = W - 2.0 * PAD - 5.0;
        let signed = super::signed_in();
        let version = match (&self.updating, &self.version_seen) {
            (Some(_), _) => "updating…".to_owned(),
            (_, Some(v)) => v.clone(),
            _ => "…".to_owned(),
        };
        let mut rows: Vec<(&str, String, bool)> = vec![("YouTube Music", (if signed { "signed in" } else { "signed out" }).into(), signed), ("Stream finder", version, true)];
        if signed {
            rows.push(("Sign out", (if self.armed { "enter again" } else { "" }).into(), true));
        }
        let mut below = top;
        for (i, y) in self.list.layout(g, rows.len(), top, row_h, 3.0, bottom + 2.0) {
            let (name, value, lit) = &rows[i];
            let r = Rect::new(PAD, y, width, row_h);
            g.row(r, name, value, if *lit { t.ink } else { t.dim }, i == self.list.focus);
            below = below.max(r.bottom());
        }
        self.list.scrollbar(g, rows.len(), top, bottom);
        // The warning first, so it is the part that always shows.
        let sentences: &[&str] = if signed {
            &["Third-party players like this one can break YouTube's terms, which could put this account at risk.", "Sign out to keep it out of reach."]
        } else {
            &["Third-party players like this one can break YouTube's terms.", "Signed out, nothing is at risk.", "To sign in, put cookies.txt from your browser in /data/music."]
        };
        // Whole sentences, the warning first: as many as there is room for.
        let mut y = below + 9.0 + ty.caption.size * 0.5;
        let step = ty.caption.size + 4.0;
        let mut lines: Vec<String> = Vec::new();
        for sentence in sentences {
            let joined = if lines.is_empty() { (*sentence).to_owned() } else { format!("{} {sentence}", lines.join(" ")) };
            let with = crate::help::wrap(g, &joined, ty.caption, width - 8.0);
            if y + step * (with.len() as f32 - 1.0) + ty.caption.size * 0.5 > bottom {
                break;
            }
            lines = with;
        }
        for line in &lines {
            g.text_at(PAD + 4.0, y, Align::Left, line, ty.caption, t.dim);
            y += step;
        }
        let hint: &[&str] = match self.list.focus {
            1 => &["enter: fetch the newest yt-dlp", "enter: update"],
            2 if self.armed => &["enter again: sign out · esc: stay", "enter again: sign out"],
            2 => &["forgets the cookies on this handheld", "enter: sign out"],
            _ if signed => &["signed in with your browser's cookies", "signed in"],
            _ => &["signed out unless you sign in", "signed out"],
        };
        g.hint_any(hint);
    }
}

/// A song in a list: its cover small, its title, and artist with length.
pub fn song_row(g: &mut Gfx, r: Rect, track: &Track, focused: bool) {
    let length = track.seconds.map(|s| super::clock(s as f32)).unwrap_or_default();
    let byline = track.artist.to_lowercase();
    art_row(g, r, track, &track.title, &byline, &length, focused);
}

/// A row with a cover: its name, a byline after it if there is room, a
/// value at the right.
fn art_row(g: &mut Gfx, r: Rect, art_of: &Track, name: &str, byline: &str, value: &str, focused: bool) {
    let t = g.tokens;
    let ty = g.ty;
    g.row(r, "", value, t.dim, focused);
    let side = r.h - 8.0;
    let art = Rect::new(r.x + 5.0, r.cy() - side * 0.5, side, side);
    match covers::cover(art_of) {
        Some(p) if !p.is_empty() => crate::photos::draw_picture(g, art, &p, [0.0, 0.0, p.w as f32, p.h as f32], 5.0, 1.0),
        _ => g.surface(art, 5.0, t.surface2),
    }
    let x = art.right() + 7.0;
    let room = r.right() - 14.0 - g.measure(value, if g.theme == crate::look::Theme::Bezel { ty.body } else { ty.label }) - x;
    let title_w = g.text_fit(x, r.cy(), name, ty.body, t.ink, room);
    if title_w + 30.0 < room && !byline.is_empty() {
        g.text_fit(x + title_w + 6.0, r.cy(), byline, ty.caption, t.dim, room - title_w - 6.0);
    }
}

/// An album, artist or playlist found.
fn collection_row(g: &mut Gfx, r: Rect, c: &Collection, focused: bool) {
    let art_of = Track { id: c.id.clone(), art: c.art.clone(), title: c.title.clone(), ..Track::default() };
    let value = if c.year > 0 { c.year.to_string() } else { String::new() };
    art_row(g, r, &art_of, &c.title, &c.byline.to_lowercase(), &value, focused);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_hint_fits_whole_at_every_text_size() {
        let mut g = Gfx::new(bezel::Tokens::dark());
        let shortest = [
            "enter · ←→ · ↓",
            "enter · ← · /",
            "enter: try again",
            "enter: play",
            "enter: now playing",
            "type to search",
            "signed out unless you sign in",
            "enter: search",
            "enter: play · del: out",
            "enter: update",
            "enter again: sign out",
            "enter: sign out",
            "signed in",
            "signed out",
            "on the card",
            "saving to the library",
            "kept to play offline",
            "all, shuffled",
            "looking for new music…",
            "nothing saved yet",
            "enter: open",
            "esc: all",
            "enter again: download",
            "into the library",
            "del: take it out",
            "waiting",
            "in the library",
            "offline",
        ];
        for size in [crate::gfx::TextSize::Standard, crate::gfx::TextSize::Large] {
            g.set_text_size(size);
            let caption = g.ty.caption;
            for s in shortest {
                assert!(g.measure(s, caption) <= W - 2.0 * PAD, "{s:?} at {size:?}");
            }
        }
    }

    #[test]
    fn offline_search_finds_the_library_and_what_was_played() {
        let found = card_search("orbit");
        assert!(found.iter().any(|t| t.album == "Orbit Songs" && t.local.is_some()), "{found:?}");
        assert!(card_search("nothing like this anywhere").is_empty());
    }
}
