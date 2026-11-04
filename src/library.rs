//! "Your backgrounds": every image or video the user applies is kept, already converted for the
//! screen, in the data dir as `lib-<time>.png` / `lib-<time>.mp4` (plus a poster frame for
//! videos), so it can be applied again later even after the original file is gone. The list
//! lives in `library.json`, newest first.

use std::path::{Path, PathBuf};

use image::RgbaImage;
use serde::{Deserialize, Serialize};

use crate::config;
use crate::media::{self, Fit};

/// Prefix of library files; other media files in the data dir are temporary.
pub const PREFIX: &str = "lib-";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Item {
    /// File name in the data dir, as the config's `media_file` names it.
    pub file: String,
    /// The original file name, for display.
    pub name: String,
    pub video: bool,
}

impl Item {
    pub fn path(&self) -> PathBuf {
        config::data_dir().join(&self.file)
    }

    /// The still shown for this item: the image itself, or the video's poster frame.
    pub fn poster_path(&self) -> PathBuf {
        if self.video { poster_of(&self.file) } else { self.path() }
    }

    pub fn poster(&self) -> Option<RgbaImage> {
        image::open(self.poster_path()).ok().map(|i| i.to_rgba8())
    }
}

fn poster_of(file: &str) -> PathBuf {
    let stem = Path::new(file).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    config::data_dir().join(format!("{stem}-poster.png"))
}

fn index_path() -> PathBuf {
    config::data_dir().join("library.json")
}

/// The saved items whose files still exist.
pub fn load() -> Vec<Item> {
    let items: Vec<Item> = std::fs::read_to_string(index_path())
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();
    items.into_iter().filter(|i| i.path().exists()).collect()
}

pub fn save(items: &[Item]) -> Result<(), String> {
    config::write_atomic(index_path(), &serde_json::to_vec_pretty(items).map_err(|e| e.to_string())?)
}

/// Convert the user's file for the screen and store it as a new library file.
/// Returns the item (not yet in the index) and its poster.
pub fn prepare(src: &Path, fit: Fit) -> Result<(Item, RgbaImage), String> {
    let dir = config::data_dir();
    std::fs::create_dir_all(&dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    let stamp = media::now_ms();
    let name = src.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    if media::is_video(src) {
        let file = format!("{PREFIX}{stamp}.mp4");
        media::prepare_video(src, fit, &dir.join(&file))?;
        let poster = media::video_poster(&dir.join(&file), Fit::Cover)?;
        write_png(&poster_of(&file), &poster)?;
        Ok((Item { file, name, video: true }, poster))
    } else {
        let img = media::prepare_image(src, fit)?;
        let file = format!("{PREFIX}{stamp}.png");
        write_png(&dir.join(&file), &img)?;
        Ok((Item { file, name, video: false }, img))
    }
}

fn write_png(path: &Path, img: &RgbaImage) -> Result<(), String> {
    std::fs::write(path, media::encode_png(img)?).map_err(|e| format!("cannot save {}: {e}", path.display()))
}

/// Put `item` first in the list (new items, or ones applied again).
pub fn add_front(items: &mut Vec<Item>, item: Item) {
    items.retain(|i| i.file != item.file);
    items.insert(0, item);
}

/// Delete an item's files. The caller removes it from the list.
pub fn delete_files(item: &Item) {
    let _ = std::fs::remove_file(item.path());
    if item.video {
        let _ = std::fs::remove_file(poster_of(&item.file));
    }
}

/// Move a background applied before the library existed (a `media-…` file from the user's own
/// file) into the library, so it shows under "Your backgrounds". Returns its new item.
pub fn adopt(file: &str, name: &str, video: bool, poster: Option<&RgbaImage>) -> Result<Item, String> {
    let dir = config::data_dir();
    let ext = Path::new(file).extension().map(|e| e.to_string_lossy().into_owned()).unwrap_or_default();
    let new_file = format!("{PREFIX}{}.{ext}", media::now_ms());
    std::fs::rename(dir.join(file), dir.join(&new_file)).map_err(|e| format!("cannot move {file}: {e}"))?;
    if let (true, Some(p)) = (video, poster) {
        write_png(&poster_of(&new_file), p)?;
    }
    Ok(Item { file: new_file, name: name.to_string(), video })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn add_front_moves_existing_items_to_the_top() {
        let item = |f: &str| Item { file: f.into(), name: f.into(), video: false };
        let mut items = vec![item("a"), item("b"), item("c")];
        add_front(&mut items, item("c"));
        assert_eq!(items.iter().map(|i| i.file.as_str()).collect::<Vec<_>>(), ["c", "a", "b"]);
        add_front(&mut items, item("d"));
        assert_eq!(items.len(), 4);
        assert_eq!(items[0].file, "d");
    }

    #[test]
    fn video_posters_sit_next_to_the_video() {
        let v = Item { file: "lib-5.mp4".into(), name: "clip.mp4".into(), video: true };
        assert!(v.poster_path().ends_with("lib-5-poster.png"));
        let i = Item { file: "lib-6.png".into(), name: "pic.png".into(), video: false };
        assert_eq!(i.poster_path(), i.path());
    }
}
