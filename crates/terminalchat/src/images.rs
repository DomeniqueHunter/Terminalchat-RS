//! Inline images (eicons, avatars, profile thumbnails) drawn with Sixel, Kitty or iTerm2.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::Cursor;
use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime};

use image::codecs::gif::GifDecoder;
use image::imageops::FilterType;
use image::{AnimationDecoder, DynamicImage, ImageFormat, RgbaImage};
use ratatui::layout::{Rect, Size};
use ratatui::Frame;
use ratatui_image::picker::{Picker, ProtocolType};
use ratatui_image::protocol::Protocol;
use ratatui_image::{FontSize, Image, Resize};
use tokio::sync::mpsc::UnboundedSender;

use crate::app::AppEvent;
use crate::bbcode::{eicon_url, percent_encode};
use crate::config::Config;
use crate::paths;

pub const MAX_PER_MESSAGE: usize = 36;
pub const AVATAR_ROWS: u16 = 6;
pub const THUMB_ROWS: u16 = 8;
pub const THUMB_COLUMNS: usize = 5;
const MAX_MEMORY_IMAGES: usize = 100;
const MAX_FRAMES: usize = 60;
/// More animated images than this on screen: none of them animates (saves bandwidth, e.g. over SSH).
const MAX_ANIMATED: usize = 4;
const MIN_FRAME_MILLIS: u32 = 100;
const MAX_ASPECT: u32 = 4;
const TIMEOUT: Duration = Duration::from_secs(15);
const DEFAULT_FONT: (u16, u16) = (10, 20);
const AVATAR_URL: &str = "https://static.f-list.net/images/avatar/";
const THUMB_URL: &str = "https://static.f-list.net/images/charthumb/";
const FULL_IMAGE_URL: &str = "https://static.f-list.net/images/charimage/";

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Kind {
    Eicon,
    Avatar,
    Thumb,
}

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct Key {
    pub kind: Kind,
    pub name: String,
}

impl Key {
    pub fn new(kind: Kind, name: &str) -> Key {
        Key { kind, name: name.trim().to_lowercase() }
    }

    pub fn label(&self) -> String {
        match self.kind {
            Kind::Eicon => format!(":{}:", self.name),
            Kind::Avatar | Kind::Thumb => format!("[{}]", self.name),
        }
    }

    fn url(&self) -> String {
        match self.kind {
            Kind::Eicon => eicon_url(&self.name),
            Kind::Avatar => format!("{AVATAR_URL}{}.png", percent_encode(&self.name)),
            Kind::Thumb => format!("{THUMB_URL}{}", percent_encode(&self.name)),
        }
    }

    fn cache_path(&self) -> PathBuf {
        let (dir, extension) = match self.kind {
            Kind::Eicon => ("eicons", ".gif"),
            Kind::Avatar => ("avatars", ".png"),
            Kind::Thumb => ("thumbs", ""),
        };
        paths::cache_dir().join(dir).join(format!("{}{extension}", percent_encode(&self.name)))
    }
}

pub fn full_image_url(file_name: &str) -> String {
    format!("{FULL_IMAGE_URL}{}", percent_encode(file_name.trim().to_lowercase().as_str()))
}

/// Replaces `[eicon]name[/eicon]` and `[icon]name[/icon]` (outside `[noparse]`) by nothing and
/// returns the images found. Tags beyond `limit` stay in the text.
pub fn extract(line: &str, limit: usize) -> (String, Vec<Key>) {
    extract_with(line, limit, None)
}

/// Like `extract`, but every removed tag leaves `marker` in the text.
pub fn extract_marked(line: &str, limit: usize, marker: char) -> (String, Vec<Key>) {
    extract_with(line, limit, Some(marker))
}

fn extract_with(line: &str, limit: usize, marker: Option<char>) -> (String, Vec<Key>) {
    let lower = line.to_ascii_lowercase();
    let mut text = String::with_capacity(line.len());
    let mut keys = Vec::new();
    let mut position = 0;
    while position < line.len() {
        let candidates = [("[noparse]", 0), ("[eicon]", 1), ("[icon]", 2)];
        let next = candidates
            .iter()
            .filter_map(|(tag, id)| lower[position..].find(tag).map(|at| (position + at, *tag, *id)))
            .min_by_key(|(at, _, _)| *at);
        let Some((start, tag, id)) = next else {
            text.push_str(&line[position..]);
            break;
        };
        text.push_str(&line[position..start]);
        let body = start + tag.len();
        let closing = match id {
            0 => "[/noparse]",
            1 => "[/eicon]",
            _ => "[/icon]",
        };
        let Some(close) = lower[body..].find(closing).map(|at| body + at) else {
            text.push_str(&line[start..body]);
            position = body;
            continue;
        };
        let end = close + closing.len();
        let name = line[body..close].trim();
        if id == 0 || keys.len() >= limit || name.is_empty() || name.contains(['[', '\n']) {
            text.push_str(&line[start..end]);
        } else {
            keys.push(Key::new(if id == 1 { Kind::Eicon } else { Kind::Avatar }, name));
            text.extend(marker);
        }
        position = end;
    }
    (text, keys)
}

pub type Frames = Vec<(RgbaImage, Duration)>;

struct Frame2 {
    image: DynamicImage,
    delay: Duration,
}

struct Loaded {
    frames: Vec<Frame2>,
    total: Duration,
    started: Instant,
    protocols: HashMap<usize, Protocol>,
    used: u64,
}

enum Slot {
    Loading,
    Missing,
    Ready(Box<Loaded>),
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Status {
    Loading,
    Missing,
    Ready,
}

pub struct ImageStore {
    picker: Option<Picker>,
    pub animate: bool,
    pub attached: bool,
    eicon_rows: u16,
    max_age: Duration,
    slots: HashMap<Key, Slot>,
    drawn: HashSet<Key>,
    visible: HashSet<Key>,
    animating: HashSet<Key>,
    clock: u64,
    tx: UnboundedSender<AppEvent>,
    client: Option<reqwest::Client>,
}

fn window_font() -> Option<(u16, u16)> {
    let size = ratatui::crossterm::terminal::window_size().ok()?;
    if size.width == 0 || size.height == 0 || size.columns == 0 || size.rows == 0 {
        return None;
    }
    Some(((size.width / size.columns).max(1), (size.height / size.rows).max(1)))
}

fn protocol_by_name(name: &str) -> Option<ProtocolType> {
    match name {
        "sixel" => Some(ProtocolType::Sixel),
        "kitty" => Some(ProtocolType::Kitty),
        "iterm2" => Some(ProtocolType::Iterm2),
        _ => None,
    }
}

/// Asks the terminal for its graphics support. Call after entering the alternate screen and before
/// the input thread starts. `TERMINALCHAT_IMAGES` = off|sixel|kitty|iterm2 overrides the detection.
pub fn detect_picker(config: &Config) -> Option<Picker> {
    if config.eicons != "inline" {
        return None;
    }
    let forced = std::env::var("TERMINALCHAT_IMAGES").ok().map(|value| value.trim().to_lowercase());
    if matches!(forced.as_deref(), Some("off" | "0" | "no" | "false")) {
        return None;
    }
    let mut picker = match Picker::from_query_stdio() {
        Ok(picker) => picker,
        Err(_) if forced.is_some() => {
            let (width, height) = window_font().unwrap_or(DEFAULT_FONT);
            let mut picker = Picker::halfblocks();
            picker = rebuilt(&picker, FontSize::new(width, height));
            picker
        }
        Err(_) => return None,
    };
    if let Some(protocol) = forced.as_deref().and_then(protocol_by_name) {
        picker.set_protocol_type(protocol);
    }
    let konsole = std::env::var_os("KONSOLE_VERSION").is_some_and(|value| !value.is_empty());
    if konsole && forced.is_none() && picker.protocol_type() != ProtocolType::Halfblocks {
        // Konsole claims Kitty support but cannot show its placeholders; Sixel works there
        picker.set_protocol_type(ProtocolType::Sixel);
    }
    if picker.protocol_type() == ProtocolType::Halfblocks {
        return None;
    }
    Some(picker)
}

#[allow(deprecated)]
fn rebuilt(old: &Picker, font: FontSize) -> Picker {
    let mut picker = Picker::from_fontsize(font);
    picker.set_protocol_type(old.protocol_type());
    picker
}

impl ImageStore {
    pub fn new(tx: UnboundedSender<AppEvent>) -> ImageStore {
        ImageStore {
            picker: None,
            animate: true,
            attached: true,
            eicon_rows: 4,
            max_age: Duration::from_secs(24 * 3600),
            slots: HashMap::new(),
            drawn: HashSet::new(),
            visible: HashSet::new(),
            animating: HashSet::new(),
            clock: 0,
            tx,
            client: None,
        }
    }

    pub fn configure(&mut self, picker: Option<Picker>, config: &Config) {
        self.picker = picker;
        // GIFs resend their pixels with every frame: off by default over SSH, /gif on enables them
        self.animate = config.eicon_animate && std::env::var_os("SSH_CONNECTION").is_none();
        self.eicon_rows = config.eicon_height.clamp(1, 20) as u16;
        self.max_age = Duration::from_secs_f64(config.eicon_max_age_hours.max(0.0) * 3600.0);
        if self.picker.is_some() {
            self.client = reqwest::Client::builder().timeout(TIMEOUT).user_agent("terminalchat").build().ok();
        }
    }

    pub fn enabled(&self) -> bool {
        self.picker.is_some()
    }

    pub fn eicon_rows(&self) -> u16 {
        self.eicon_rows
    }

    pub fn font(&self) -> (u16, u16) {
        self.picker.as_ref().map(|picker| (picker.font_size().width, picker.font_size().height)).unwrap_or(DEFAULT_FONT)
    }

    pub fn protocol_name(&self) -> &'static str {
        match self.picker.as_ref().map(Picker::protocol_type) {
            Some(ProtocolType::Sixel) => "sixel",
            Some(ProtocolType::Kitty) => "kitty",
            Some(ProtocolType::Iterm2) => "iterm2",
            _ => "none",
        }
    }

    /// Measures the cell size again. Returns true when it changed (all images are loaded again).
    pub fn remeasure(&mut self) -> bool {
        let Some((width, height)) = window_font() else { return false };
        let Some(picker) = &self.picker else { return false };
        if (picker.font_size().width, picker.font_size().height) == (width, height) {
            return false;
        }
        self.picker = Some(rebuilt(picker, FontSize::new(width, height)));
        self.slots.clear();
        true
    }

    fn rows_of(&self, kind: Kind) -> u16 {
        match kind {
            Kind::Eicon => self.eicon_rows,
            Kind::Avatar => AVATAR_ROWS,
            Kind::Thumb => THUMB_ROWS,
        }
    }

    pub fn status(&self, key: &Key) -> Status {
        match self.slots.get(key) {
            Some(Slot::Ready(_)) => Status::Ready,
            Some(Slot::Missing) => Status::Missing,
            _ => Status::Loading,
        }
    }

    /// Size of a loaded image in cells.
    pub fn size_cells(&self, key: &Key) -> Option<(u16, u16)> {
        let Some(Slot::Ready(loaded)) = self.slots.get(key) else { return None };
        let (font_width, font_height) = self.font();
        let image = &loaded.frames[0].image;
        let columns = image.width().div_ceil(u32::from(font_width)).max(1);
        let rows = image.height().div_ceil(u32::from(font_height)).max(1);
        Some((columns.min(u32::from(u16::MAX)) as u16, rows.min(u32::from(u16::MAX)) as u16))
    }

    /// Columns an image takes in a line: its real size once loaded, otherwise an estimate.
    pub fn slot_width(&self, key: &Key) -> u16 {
        if let Some(Slot::Ready(_)) = self.slots.get(key) {
            return self.size_cells(key).map_or(1, |size| size.0);
        }
        if let Some(Slot::Missing) = self.slots.get(key) {
            return crate::rows::text_width(&key.label()).min(u16::MAX as usize) as u16;
        }
        let (font_width, font_height) = self.font();
        (u32::from(self.rows_of(key.kind)) * u32::from(font_height)).div_ceil(u32::from(font_width.max(1))).max(1) as u16
    }

    pub fn request(&mut self, key: &Key) {
        if !self.enabled() || self.slots.contains_key(key) {
            return;
        }
        let Some(client) = self.client.clone() else { return };
        self.slots.insert(key.clone(), Slot::Loading);
        let (_, font_height) = self.font();
        let height = u32::from(self.rows_of(key.kind)) * u32::from(font_height);
        let (key, tx, max_age) = (key.clone(), self.tx.clone(), self.max_age);
        tokio::spawn(async move {
            let frames = load(&client, &key, max_age, height * MAX_ASPECT, height).await;
            let _ = tx.send(AppEvent::Image(key, frames));
        });
    }

    pub fn on_loaded(&mut self, key: Key, frames: Option<Frames>) {
        if !self.slots.contains_key(&key) {
            return;
        }
        let slot = match frames {
            Some(frames) if !frames.is_empty() => {
                let total = frames.iter().map(|(_, delay)| *delay).sum();
                self.clock += 1;
                Slot::Ready(Box::new(Loaded {
                    frames: frames.into_iter().map(|(image, delay)| Frame2 { image: DynamicImage::ImageRgba8(image), delay }).collect(),
                    total,
                    started: Instant::now(),
                    protocols: HashMap::new(),
                    used: self.clock,
                }))
            }
            _ => Slot::Missing,
        };
        self.slots.insert(key, slot);
        self.evict();
    }

    fn evict(&mut self) {
        let mut ready: Vec<(u64, Key)> =
            self.slots.iter().filter_map(|(key, slot)| if let Slot::Ready(loaded) = slot { Some((loaded.used, key.clone())) } else { None }).collect();
        if ready.len() <= MAX_MEMORY_IMAGES {
            return;
        }
        ready.sort_by_key(|(used, _)| *used);
        let excess = ready.len() - MAX_MEMORY_IMAGES;
        for (_, key) in ready.into_iter().filter(|(_, key)| !self.visible.contains(key)).take(excess) {
            self.slots.remove(&key);
        }
    }

    pub fn begin_frame(&mut self) {
        self.drawn.clear();
    }

    pub fn end_frame(&mut self) {
        self.visible = std::mem::take(&mut self.drawn);
        let candidates: HashSet<Key> = self
            .visible
            .iter()
            .filter(|key| matches!(self.slots.get(*key), Some(Slot::Ready(loaded)) if loaded.frames.len() > 1 && !loaded.total.is_zero()))
            .cloned()
            .collect();
        self.animating = if candidates.len() > MAX_ANIMATED { HashSet::new() } else { candidates };
        for (key, slot) in &mut self.slots {
            if let Slot::Ready(loaded) = slot {
                if !self.visible.contains(key) {
                    loaded.protocols.clear();
                }
            }
        }
    }

    fn animated(&self, key: &Key, loaded: &Loaded) -> bool {
        self.animate && self.attached && loaded.frames.len() > 1 && !loaded.total.is_zero() && self.animating.contains(key)
    }

    fn frame_index(&self, key: &Key, loaded: &Loaded) -> usize {
        if !self.animated(key, loaded) {
            return 0;
        }
        let total = loaded.total.as_millis().max(1);
        let mut at = loaded.started.elapsed().as_millis() % total;
        for (index, frame) in loaded.frames.iter().enumerate() {
            let delay = frame.delay.as_millis();
            if at < delay {
                return index;
            }
            at -= delay;
        }
        0
    }

    /// When the next animation frame of a visible image is due.
    pub fn next_wake(&self) -> Option<Instant> {
        let now = Instant::now();
        let mut best: Option<Duration> = None;
        for key in &self.visible {
            let Some(Slot::Ready(loaded)) = self.slots.get(key) else { continue };
            if !self.animated(key, loaded) {
                continue;
            }
            let total = loaded.total.as_millis().max(1);
            let mut at = loaded.started.elapsed().as_millis() % total;
            for frame in &loaded.frames {
                let delay = frame.delay.as_millis();
                if at < delay {
                    let wait = Duration::from_millis((delay - at).max(1) as u64);
                    best = Some(best.map_or(wait, |current| current.min(wait)));
                    break;
                }
                at -= delay;
            }
        }
        best.map(|wait| now + wait)
    }

    /// Draws the image into `area` (exactly its size in cells). Returns false if it could not.
    pub fn render(&mut self, frame: &mut Frame, key: &Key, area: Rect) -> bool {
        let Some(picker) = &self.picker else { return false };
        let index = match self.slots.get(key) {
            Some(Slot::Ready(loaded)) => self.frame_index(key, loaded),
            _ => return false,
        };
        self.clock += 1;
        let clock = self.clock;
        let Some(Slot::Ready(loaded)) = self.slots.get_mut(key) else { return false };
        loaded.used = clock;
        if !loaded.protocols.contains_key(&index) {
            let image = loaded.frames[index].image.clone();
            match picker.new_protocol(image, Size::new(area.width, area.height), Resize::Fit(None)) {
                Ok(protocol) => {
                    loaded.protocols.insert(index, protocol);
                }
                Err(_) => return false,
            }
        }
        frame.render_widget(Image::new(&loaded.protocols[&index]), area);
        self.drawn.insert(key.clone());
        true
    }
}

fn http_date(time: SystemTime) -> String {
    chrono::DateTime::<chrono::Utc>::from(time).format("%a, %d %b %Y %H:%M:%S GMT").to_string()
}

async fn load(client: &reqwest::Client, key: &Key, max_age: Duration, max_width: u32, max_height: u32) -> Option<Frames> {
    let bytes = fetch(client, key, max_age).await?;
    let fill = key.kind != Kind::Thumb;
    tokio::task::spawn_blocking(move || decode(&bytes, max_width, max_height, fill)).await.ok().flatten()
}

async fn fetch(client: &reqwest::Client, key: &Key, max_age: Duration) -> Option<Vec<u8>> {
    let path = key.cache_path();
    let modified = tokio::fs::metadata(&path).await.ok().and_then(|meta| meta.modified().ok());
    if let Some(modified) = modified {
        let age = SystemTime::now().duration_since(modified).unwrap_or_default();
        if age < max_age {
            if let Ok(bytes) = tokio::fs::read(&path).await {
                return Some(bytes);
            }
        }
    }
    let mut request = client.get(key.url());
    if let Some(modified) = modified {
        request = request.header("If-Modified-Since", http_date(modified));
    }
    let stale = || async { tokio::fs::read(&path).await.ok() };
    let response = match request.send().await {
        Ok(response) => response,
        Err(_) => return stale().await,
    };
    let status = response.status();
    if status.as_u16() == 304 {
        let touch = path.clone();
        let _ = tokio::task::spawn_blocking(move || {
            fs::File::options().write(true).open(&touch).and_then(|file| file.set_modified(SystemTime::now()))
        })
        .await;
        return stale().await;
    }
    if status.as_u16() == 404 || status.as_u16() == 410 {
        let _ = tokio::fs::remove_file(&path).await;
        return None;
    }
    if !status.is_success() {
        return stale().await;
    }
    let Ok(bytes) = response.bytes().await else { return stale().await };
    if let Some(parent) = path.parent() {
        let _ = tokio::fs::create_dir_all(parent).await;
    }
    let mut temporary = path.clone().into_os_string();
    temporary.push(".part");
    let temporary = PathBuf::from(temporary);
    if tokio::fs::write(&temporary, &bytes).await.is_ok() && tokio::fs::rename(&temporary, &path).await.is_err() {
        let _ = tokio::fs::remove_file(&temporary).await;
    }
    Some(bytes.to_vec())
}

fn flatten_and_fit(mut image: RgbaImage, max_width: u32, max_height: u32, fill: bool) -> RgbaImage {
    for pixel in image.pixels_mut() {
        let alpha = u32::from(pixel[3]);
        for channel in 0..3 {
            pixel[channel] = (u32::from(pixel[channel]) * alpha / 255) as u8;
        }
        pixel[3] = 255;
    }
    let (width, height) = image.dimensions();
    if !fill && width <= max_width && height <= max_height {
        return image;
    }
    let scale = (max_width as f64 / width as f64).min(max_height as f64 / height as f64);
    let new_width = ((width as f64 * scale).round() as u32).max(1);
    let new_height = ((height as f64 * scale).round() as u32).max(1);
    if (new_width, new_height) == (width, height) {
        return image;
    }
    let filter = if scale > 1.0 { FilterType::CatmullRom } else { FilterType::Triangle };
    image::imageops::resize(&image, new_width, new_height, filter)
}

/// `fill` scales the image up as well, so that it uses the full height.
pub fn decode(bytes: &[u8], max_width: u32, max_height: u32, fill: bool) -> Option<Frames> {
    let format = image::guess_format(bytes).ok()?;
    let mut frames: Frames = Vec::new();
    if format == ImageFormat::Gif {
        let decoder = GifDecoder::new(Cursor::new(bytes)).ok()?;
        for frame in decoder.into_frames().take(MAX_FRAMES) {
            let frame = frame.ok()?;
            let (numerator, denominator) = frame.delay().numer_denom_ms();
            let millis = numerator.checked_div(denominator).unwrap_or(100);
            let millis = if millis < 20 { 100 } else { millis.max(MIN_FRAME_MILLIS) };
            frames.push((flatten_and_fit(frame.into_buffer(), max_width, max_height, fill), Duration::from_millis(u64::from(millis))));
        }
    } else {
        let image = image::load_from_memory(bytes).ok()?.to_rgba8();
        frames.push((flatten_and_fit(image, max_width, max_height, fill), Duration::ZERO));
    }
    if frames.is_empty() {
        None
    } else {
        Some(frames)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_eicons_and_icons() {
        let (text, keys) = extract("hi [eicon]Cat Hug[/eicon] und [icon]Bob[/icon]!", 36);
        assert_eq!(text, "hi  und !");
        assert_eq!(keys, vec![Key::new(Kind::Eicon, "cat hug"), Key::new(Kind::Avatar, "bob")]);
    }

    #[test]
    fn marked_extraction_keeps_the_position() {
        let (text, keys) = extract_marked("a [eicon]x[/eicon] b", 36, '\u{E000}');
        assert_eq!(text, "a \u{E000} b");
        assert_eq!(keys.len(), 1);
    }

    #[test]
    fn noparse_and_limit_keep_the_text() {
        let (text, keys) = extract("[noparse][eicon]a[/eicon][/noparse][eicon]b[/eicon][eicon]c[/eicon]", 1);
        assert_eq!(text, "[noparse][eicon]a[/eicon][/noparse][eicon]c[/eicon]");
        assert_eq!(keys.len(), 1);
        assert_eq!(keys[0].name, "b");
    }

    #[test]
    fn unclosed_tags_stay() {
        let (text, keys) = extract("a [eicon]b", 36);
        assert_eq!(text, "a [eicon]b");
        assert!(keys.is_empty());
    }

    #[test]
    fn urls_and_cache_paths() {
        assert_eq!(Key::new(Kind::Avatar, "Max Muster").url(), "https://static.f-list.net/images/avatar/max%20muster.png");
        assert!(Key::new(Kind::Eicon, "x").cache_path().ends_with("eicons/x.gif"));
        assert_eq!(full_image_url("12.JPG"), "https://static.f-list.net/images/charimage/12.jpg");
    }

    #[test]
    fn decodes_png_and_scales_down() {
        let mut png = Vec::new();
        let image = RgbaImage::from_pixel(100, 50, image::Rgba([255, 0, 0, 255]));
        image.write_to(&mut Cursor::new(&mut png), ImageFormat::Png).unwrap();
        let frames = decode(&png, 40, 40, false).unwrap();
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].0.dimensions(), (40, 20));
    }

    #[test]
    fn small_images_fill_the_height_when_asked() {
        let mut png = Vec::new();
        let image = RgbaImage::from_pixel(50, 50, image::Rgba([0, 255, 0, 255]));
        image.write_to(&mut Cursor::new(&mut png), ImageFormat::Png).unwrap();
        assert_eq!(decode(&png, 288, 72, true).unwrap()[0].0.dimensions(), (72, 72));
        assert_eq!(decode(&png, 288, 72, false).unwrap()[0].0.dimensions(), (50, 50));
        let wide = RgbaImage::from_pixel(400, 10, image::Rgba([0, 0, 255, 255]));
        let mut data = Vec::new();
        wide.write_to(&mut Cursor::new(&mut data), ImageFormat::Png).unwrap();
        assert_eq!(decode(&data, 288, 72, true).unwrap()[0].0.dimensions(), (288, 7));
    }

    #[test]
    fn renders_a_loaded_image_as_sixel() {
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let mut store = ImageStore::new(tx);
        let mut picker = Picker::halfblocks();
        picker.set_protocol_type(ProtocolType::Sixel);
        store.picker = Some(picker);
        let key = Key::new(Kind::Eicon, "cat");
        store.slots.insert(key.clone(), Slot::Loading);
        let frames = vec![
            (RgbaImage::from_pixel(20, 40, image::Rgba([255, 0, 0, 255])), Duration::from_millis(100)),
            (RgbaImage::from_pixel(20, 40, image::Rgba([0, 255, 0, 255])), Duration::from_millis(100)),
        ];
        store.on_loaded(key.clone(), Some(frames));
        assert_eq!(store.size_cells(&key), Some((2, 2)));
        let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(10, 5)).unwrap();
        store.begin_frame();
        let mut drawn = false;
        terminal.draw(|frame| drawn = store.render(frame, &key, Rect::new(1, 1, 2, 2))).unwrap();
        store.end_frame();
        assert!(drawn);
        let cell = terminal.backend().buffer().cell((1, 1)).unwrap();
        assert!(cell.symbol().contains("\x1bP"), "no sixel data in the first cell");
        assert!(store.next_wake().is_some());
        store.animate = false;
        assert!(store.next_wake().is_none());
    }

    #[test]
    fn too_many_animated_images_stay_still() {
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let mut store = ImageStore::new(tx);
        let mut picker = Picker::halfblocks();
        picker.set_protocol_type(ProtocolType::Sixel);
        store.picker = Some(picker);
        let keys: Vec<Key> = (0..5).map(|index| Key::new(Kind::Eicon, &format!("e{index}"))).collect();
        for key in &keys {
            store.slots.insert(key.clone(), Slot::Loading);
            let frames = vec![
                (RgbaImage::from_pixel(20, 40, image::Rgba([255, 0, 0, 255])), Duration::from_millis(100)),
                (RgbaImage::from_pixel(20, 40, image::Rgba([0, 255, 0, 255])), Duration::from_millis(100)),
            ];
            store.on_loaded(key.clone(), Some(frames));
        }
        let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(30, 5)).unwrap();
        for shown in [4usize, 5] {
            store.begin_frame();
            terminal
                .draw(|frame| {
                    for (index, key) in keys.iter().take(shown).enumerate() {
                        store.render(frame, key, Rect::new(index as u16 * 3, 0, 2, 2));
                    }
                })
                .unwrap();
            store.end_frame();
            assert_eq!(store.next_wake().is_some(), shown <= MAX_ANIMATED);
        }
    }
}
