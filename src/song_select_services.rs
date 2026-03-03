use std::{
    fs,
    path::{Path, PathBuf},
    sync::mpsc,
    time::SystemTime,
};

use image::imageops::FilterType;
use macroquad::prelude::{FilterMode, Texture2D};

const JACKET_CACHE_EDGE: u32 = 512;

fn local_cache_path(src: &Path) -> Option<PathBuf> {
    let parent = src.parent()?;
    let stem = src.file_stem()?.to_str()?;
    Some(parent.join(format!(".{stem}.msm_jacket_{JACKET_CACHE_EDGE}.png")))
}

fn ensure_jacket_cache(src: &Path) -> Option<PathBuf> {
    let cache_path = local_cache_path(src)?;
    if cache_path.is_file() {
        let src_m = fs::metadata(src)
            .ok()
            .and_then(|m| m.modified().ok())
            .unwrap_or(SystemTime::UNIX_EPOCH);
        let cache_m = fs::metadata(&cache_path)
            .ok()
            .and_then(|m| m.modified().ok())
            .unwrap_or(SystemTime::UNIX_EPOCH);
        if cache_m >= src_m {
            return Some(cache_path);
        }
    }
    let img = image::open(src).ok()?;
    let sq = img.width().min(img.height());
    if sq == 0 {
        return None;
    }
    let x = (img.width() - sq) / 2;
    let y = (img.height() - sq) / 2;
    let cropped = img.crop_imm(x, y, sq, sq);
    let resized = cropped.resize_exact(JACKET_CACHE_EDGE, JACKET_CACHE_EDGE, FilterType::Triangle);
    if resized.save(&cache_path).is_ok() {
        Some(cache_path)
    } else {
        None
    }
}

pub struct CoverTextureLoader {
    pending: Option<(usize, mpsc::Receiver<Option<PathBuf>>)>,
}

impl CoverTextureLoader {
    pub fn new() -> Self {
        Self { pending: None }
    }

    pub fn request(&mut self, idx: usize, src: Option<&Path>, textures: &[Option<Option<Texture2D>>]) {
        if self.pending.is_some() || idx >= textures.len() || textures[idx].is_some() {
            return;
        }
        let Some(src) = src else { return; };
        let src = src.to_path_buf();
        let (tx, rx) = mpsc::channel::<Option<PathBuf>>();
        std::thread::spawn(move || {
            let out = ensure_jacket_cache(&src).or_else(|| if src.is_file() { Some(src) } else { None });
            let _ = tx.send(out);
        });
        self.pending = Some((idx, rx));
    }

    pub async fn poll_upload(&mut self, textures: &mut [Option<Option<Texture2D>>]) {
        let Some((idx, rx)) = self.pending.take() else { return; };
        match rx.try_recv() {
            Ok(Some(path)) => {
                if let Ok(t) = macroquad::texture::load_texture(path.to_string_lossy().as_ref()).await {
                    t.set_filter(FilterMode::Linear);
                    if idx < textures.len() {
                        textures[idx] = Some(Some(t));
                    }
                } else if idx < textures.len() {
                    textures[idx] = Some(None);
                }
            }
            Ok(None) => {
                if idx < textures.len() {
                    textures[idx] = Some(None);
                }
            }
            Err(mpsc::TryRecvError::Empty) => self.pending = Some((idx, rx)),
            Err(mpsc::TryRecvError::Disconnected) => {
                if idx < textures.len() {
                    textures[idx] = Some(None);
                }
            }
        }
    }
}
