mod chart;
mod dwi_parser;
mod game;
mod sm_parser;
mod song_select_services;

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;
use std::time::Duration;

use game::{Game, GameExitAction, build_preview_source_for_path, window_conf};
use macroquad::prelude::*;
use rodio::{OutputStream, OutputStreamHandle, Sink, Source};
use song_select_services::CoverTextureLoader;
const UI_FONT_CANDIDATES: [&str; 2] = [
    "mini_stepmania_rust/assets/fonts/JingNanBoBoHei-Bold.ttf",
    "assets/fonts/JingNanBoBoHei-Bold.ttf",
];
const PREVIEW_BASE_VOLUME: f32 = 0.72;
const PREVIEW_DEBOUNCE_SEC: f64 = 0.08;
const PREVIEW_FADE_SEC: f64 = 0.20;
const LAST_SELECTION_FILE: &str = "last_selection.txt";
const LAST_DIFFICULTY_FILE: &str = "last_difficulty.txt";

#[derive(Clone)]
struct SongEntry {
    chart_path: PathBuf,
    jacket_path: Option<PathBuf>,
    preview_music_path: Option<PathBuf>,
    title: String,
    artist: String,
    meter: i32,
    difficulties: Vec<(String, i32)>,
}

fn set_workdir_to_project_root() {
    #[cfg(target_os = "android")]
    {
        return;
    }
    #[cfg(not(target_os = "android"))]
    if let Ok(exe) = std::env::current_exe() {
        let mut dir = exe.parent().map(|p| p.to_path_buf());
        while let Some(d) = dir {
            if d.join("Songs").is_dir() {
                let _ = std::env::set_current_dir(&d);
                return;
            }
            dir = d.parent().map(|p| p.to_path_buf());
        }
    }
}

fn app_storage_root() -> PathBuf {
    #[cfg(target_os = "android")]
    {
        let p1 = PathBuf::from("/storage/emulated/0/stepmania");
        if p1.exists() {
            return p1;
        }
        let p2 = PathBuf::from("/sdcard/stepmania");
        if p2.exists() {
            return p2;
        }
        p1
    }
    #[cfg(not(target_os = "android"))]
    {
        PathBuf::from("mini_stepmania_rust")
    }
}

fn songs_dir_candidates(root: &Path) -> Vec<PathBuf> {
    let mut out = vec![root.join("Songs"), root.join("songs"), root.join("sonngs")];
    out.retain(|p| p.is_dir());
    if out.is_empty() {
        out.push(root.join("Songs"));
    }
    out
}

fn data_file_path(root: &Path, name: &str) -> PathBuf {
    root.join(name)
}

fn find_jacket_path(song_dir: &Path) -> Option<PathBuf> {
    let dir_name = song_dir.file_name()?.to_str()?;
    let exact = [
        song_dir.join(format!("{dir_name}-jacket.png")),
        song_dir.join(format!("{dir_name}-jacket.jpg")),
        song_dir.join(format!("{dir_name}-jacket.jpeg")),
        song_dir.join(format!("{dir_name}-bg.png")),
        song_dir.join(format!("{dir_name}-bg.jpg")),
        song_dir.join(format!("{dir_name}-bg.jpeg")),
    ];
    for p in exact {
        if p.is_file() {
            return Some(p);
        }
    }
    if let Ok(rd) = fs::read_dir(song_dir) {
        let mut bg_fallback: Option<PathBuf> = None;
        let mut any_image: Option<PathBuf> = None;
        for e in rd.flatten() {
            let p = e.path();
            let ext = p
                .extension()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_ascii_lowercase();
            if !matches!(ext.as_str(), "png" | "jpg" | "jpeg") {
                continue;
            }
            if any_image.is_none() {
                any_image = Some(p.clone());
            }
            let stem = p
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_ascii_lowercase();
            if stem.ends_with("-jacket") {
                return Some(p);
            }
            if stem.ends_with("-bg") {
                bg_fallback = Some(p);
            }
        }
        if let Some(p) = bg_fallback {
            return Some(p);
        }
        if let Some(p) = any_image {
            return Some(p);
        }
    }
    None
}

fn discover_song_entries(root: &Path) -> Vec<SongEntry> {
    let mut out = Vec::new();
    let Ok(rd) = fs::read_dir(root) else {
        return out;
    };
    let mut dirs: Vec<PathBuf> = rd.flatten().map(|e| e.path()).filter(|p| p.is_dir()).collect();
    dirs.sort();
    for dir in dirs {
        let Ok(fr) = fs::read_dir(&dir) else {
            continue;
        };
        let mut chart_files: Vec<PathBuf> = fr
            .flatten()
            .map(|e| e.path())
            .filter(|p| {
                let ext = p
                    .extension()
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_default();
                ext.eq_ignore_ascii_case("sm")
                    || ext.eq_ignore_ascii_case("ssc")
                    || ext.eq_ignore_ascii_case("dwi")
            })
            .collect();
        chart_files.sort();
        if chart_files.is_empty() {
            continue;
        }
        for chart_path in chart_files {
            let ext = chart_path
                .extension()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_default()
                .to_ascii_lowercase();
            let chart_res = if ext == "dwi" {
                dwi_parser::parse_dwi(&chart_path, None)
            } else {
                sm_parser::parse_sm(&chart_path, None)
            };
            let Ok(chart) = chart_res else {
                println!("[scan] skip parse failed: {}", chart_path.display());
                continue;
            };
            let difficulties = list_difficulties_by_probe(&chart_path, &ext, &chart);
            out.push(SongEntry {
                jacket_path: find_jacket_path(&dir),
                preview_music_path: chart.music.clone(),
                chart_path,
                title: chart.title,
                artist: chart.artist,
                meter: chart.meter,
                difficulties,
            });
        }
    }
    out
}

fn discover_song_entries_multi(roots: &[PathBuf]) -> Vec<SongEntry> {
    let mut out = Vec::new();
    for r in roots {
        out.extend(discover_song_entries(r));
    }
    out
}

fn load_last_selection(path: &Path) -> String {
    fs::read_to_string(path)
        .ok()
        .map(|s| s.trim().to_string())
        .unwrap_or_default()
}

fn save_last_selection(path: &Path, chart_path: &Path) {
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let _ = fs::write(path, chart_path.to_string_lossy().to_string());
}

fn load_last_difficulty(path: &Path) -> String {
    fs::read_to_string(path)
        .ok()
        .map(|s| s.trim().to_string())
        .unwrap_or_default()
}

fn save_last_difficulty(path: &Path, difficulty: &str) {
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let _ = fs::write(path, difficulty);
}

async fn song_select(
    entries: &[SongEntry],
    mut idx: usize,
    mut diff_idx: usize,
    ignore_escape_until: f64,
    ui_font: Option<&Font>,
    last_diff_name: &mut String,
) -> Option<(PathBuf, String, usize, usize)> {
    if entries.is_empty() {
        return None;
    }
    idx = idx.min(entries.len() - 1);
    const COLS: usize = 3;
    const PAGE_ROWS: usize = 4;
    const ITEMS_PER_PAGE: usize = COLS * PAGE_ROWS;
    const BANNER_H: f32 = 130.0;
    const PAD: f32 = 14.0;
    let mut page_pos: f32 = (idx / ITEMS_PER_PAGE) as f32;
    let mut cover_loader = CoverTextureLoader::new();
    let mut cover_textures: Vec<Option<Option<Texture2D>>> = vec![None; entries.len()];
    let mut _preview_stream_keepalive: Option<OutputStream> = None;
    let mut preview_handle: Option<OutputStreamHandle> = None;
    let mut preview_sink: Option<Sink> = None;
    let mut preview_idx: Option<usize> = None;
    let mut preview_fade_in_start = 0.0_f64;
    let mut fading_out_sinks: Vec<(Sink, f64)> = Vec::new();
    let mut pending_preview_idx: Option<usize> = Some(idx);
    let mut pending_preview_since = get_time() - PREVIEW_DEBOUNCE_SEC;

    loop {
        let now = get_time();
        if is_key_pressed(KeyCode::Left) {
            idx = idx.saturating_sub(1);
            if !entries[idx].difficulties.is_empty() {
                diff_idx = entries[idx]
                    .difficulties
                    .iter()
                    .position(|d| d.0.eq_ignore_ascii_case(last_diff_name))
                    .unwrap_or(0);
            }
            pending_preview_idx = Some(idx);
            pending_preview_since = now;
        }
        if is_key_pressed(KeyCode::Right) {
            idx = (idx + 1).min(entries.len() - 1);
            if !entries[idx].difficulties.is_empty() {
                diff_idx = entries[idx]
                    .difficulties
                    .iter()
                    .position(|d| d.0.eq_ignore_ascii_case(last_diff_name))
                    .unwrap_or(0);
            }
            pending_preview_idx = Some(idx);
            pending_preview_since = now;
        }
        let diff_len = entries[idx].difficulties.len().max(1);
        if is_key_pressed(KeyCode::Up) {
            diff_idx = (diff_idx + diff_len - 1) % diff_len;
            if let Some(d) = entries[idx].difficulties.get(diff_idx) {
                *last_diff_name = d.0.clone();
            }
        }
        if is_key_pressed(KeyCode::Down) {
            diff_idx = (diff_idx + 1) % diff_len;
            if let Some(d) = entries[idx].difficulties.get(diff_idx) {
                *last_diff_name = d.0.clone();
            }
        }
        if is_key_pressed(KeyCode::Enter) || is_key_pressed(KeyCode::Space) {
            let diff = entries[idx]
                .difficulties
                .get(diff_idx)
                .map(|d| d.0.clone())
                .unwrap_or_else(|| "Hard".to_string());
            *last_diff_name = diff.clone();
            if let Some(s) = preview_sink.take() {
                s.stop();
            }
            for (s, _) in fading_out_sinks.drain(..) {
                s.stop();
            }
            return Some((entries[idx].chart_path.clone(), diff, idx, diff_idx));
        }
        if (is_key_pressed(KeyCode::Escape) || is_key_pressed(KeyCode::Backspace))
            && now >= ignore_escape_until
        {
            if let Some(s) = preview_sink.take() {
                s.stop();
            }
            for (s, _) in fading_out_sinks.drain(..) {
                s.stop();
            }
            return None;
        }

        let sw = screen_width();
        let sh = screen_height();
        let total_pages = entries.len().div_ceil(ITEMS_PER_PAGE).max(1);
        let target_page = (idx / ITEMS_PER_PAGE) as f32;
        page_pos += (target_page - page_pos) * 0.2;
        page_pos = page_pos.clamp(0.0, (total_pages.saturating_sub(1)) as f32);

        let cell_size = (sw - PAD * (COLS as f32 + 1.0)) / COLS as f32;
        let stride = cell_size + PAD;
        let grid_top = BANNER_H + PAD;
        let grid_bottom = grid_top + PAGE_ROWS as f32 * stride;
        let base_page = page_pos.floor().max(0.0) as usize;
        let page_for_hit = page_pos.round().clamp(0.0, (total_pages.saturating_sub(1)) as f32) as usize;
        let hit_test = |mx: f32, my: f32| -> Option<usize> {
            if my < grid_top || mx < PAD || mx > sw - PAD {
                return None;
            }
            let colf = (mx - PAD) / stride;
            let rowf = (my - grid_top) / stride;
            if colf < 0.0 || rowf < 0.0 {
                return None;
            }
            let col = colf.floor() as usize;
            let row = rowf.floor() as usize;
            if col >= COLS || row >= PAGE_ROWS {
                return None;
            }
            let local_x = (mx - PAD) - col as f32 * stride;
            let local_y = (my - grid_top) - row as f32 * stride;
            if local_x < 0.0 || local_x > cell_size || local_y < 0.0 || local_y > cell_size {
                return None;
            }
            let i = page_for_hit * ITEMS_PER_PAGE + row * COLS + col;
            if i < entries.len() { Some(i) } else { None }
        };
        let (mx, my) = mouse_position();
        let dot_y = (grid_bottom + 18.0).min(sh - 72.0);
        let dot_r = 6.0;
        let dot_gap = 24.0;
        let dots_w = (total_pages.saturating_sub(1)) as f32 * dot_gap;
        let dot_start_x = sw * 0.5 - dots_w * 0.5;
        let hit_page_dot = |mx: f32, my: f32| -> Option<usize> {
            for p in 0..total_pages {
                let x = dot_start_x + p as f32 * dot_gap;
                let dx = mx - x;
                let dy = my - dot_y;
                if dx * dx + dy * dy <= (dot_r * 2.2) * (dot_r * 2.2) {
                    return Some(p);
                }
            }
            None
        };
        if is_mouse_button_pressed(MouseButton::Left) {
            if let Some(p) = hit_page_dot(mx, my) {
                page_pos = p as f32;
                let ni = (p * ITEMS_PER_PAGE).min(entries.len().saturating_sub(1));
                idx = ni;
                if !entries[idx].difficulties.is_empty() {
                    diff_idx = entries[idx]
                        .difficulties
                        .iter()
                        .position(|d| d.0.eq_ignore_ascii_case(last_diff_name))
                        .unwrap_or(0);
                }
                pending_preview_idx = Some(idx);
                pending_preview_since = now;
            } else 
            if let Some(click_idx) = hit_test(mx, my) {
                if click_idx == idx && preview_idx == Some(idx) {
                    let diff = entries[idx]
                        .difficulties
                        .get(diff_idx)
                        .map(|d| d.0.clone())
                        .unwrap_or_else(|| "Hard".to_string());
                    *last_diff_name = diff.clone();
                    if let Some(s) = preview_sink.take() {
                        s.stop();
                    }
                    for (s, _) in fading_out_sinks.drain(..) {
                        s.stop();
                    }
                    return Some((entries[idx].chart_path.clone(), diff, idx, diff_idx));
                }
                idx = click_idx;
                if !entries[idx].difficulties.is_empty() {
                    diff_idx = entries[idx]
                        .difficulties
                        .iter()
                        .position(|d| d.0.eq_ignore_ascii_case(last_diff_name))
                        .unwrap_or(0);
                }
                pending_preview_idx = Some(idx);
                pending_preview_since = now;
            }
        }

        if let Some(target_idx) = pending_preview_idx {
            if now - pending_preview_since >= PREVIEW_DEBOUNCE_SEC && preview_idx != Some(target_idx) {
                if let Some(s) = preview_sink.take() {
                    fading_out_sinks.push((s, now));
                }
                preview_idx = Some(target_idx);
                preview_fade_in_start = now;
                pending_preview_idx = None;
            }
        }

        if preview_idx.is_some() && preview_sink.is_none() {
            if preview_handle.is_none() {
                if let Ok((stream, handle)) = OutputStream::try_default() {
                    _preview_stream_keepalive = Some(stream);
                    preview_handle = Some(handle);
                }
            }
            if let Some(pi) = preview_idx {
                if let (Some(handle), Some(path)) = (preview_handle.as_ref(), entries[pi].preview_music_path.as_ref()) {
                    if let Some(sink) = create_preview_sink(handle, path, 0.0) {
                        sink.set_volume(0.0);
                        preview_sink = Some(sink);
                    }
                }
            }
        }

        if let Some(sink) = preview_sink.as_ref() {
            let t = ((now - preview_fade_in_start) / PREVIEW_FADE_SEC).clamp(0.0, 1.0) as f32;
            sink.set_volume(PREVIEW_BASE_VOLUME * t);
        }
        if preview_sink.as_ref().map(|s| s.empty()).unwrap_or(false) {
            preview_sink = None;
            preview_fade_in_start = now;
        }
        fading_out_sinks.retain_mut(|(sink, started)| {
            let t = ((now - *started) / PREVIEW_FADE_SEC).clamp(0.0, 1.0) as f32;
            sink.set_volume(PREVIEW_BASE_VOLUME * (1.0 - t).max(0.0));
            if t >= 1.0 || sink.empty() {
                sink.stop();
                false
            } else {
                true
            }
        });

        let mut request_queue = Vec::<usize>::new();
        for p in [
            base_page.saturating_sub(1),
            base_page,
            (base_page + 1).min(total_pages - 1),
        ] {
            let start = p * ITEMS_PER_PAGE;
            for row in 0..PAGE_ROWS {
                for col in 0..COLS {
                    let ei = start + row * COLS + col;
                    if ei < entries.len() && cover_textures[ei].is_none() {
                        request_queue.push(ei);
                    }
                }
            }
        }
        if cover_textures[idx].is_none() {
            request_queue.insert(0, idx);
        }
        if let Some(&ri) = request_queue.first() {
            cover_loader.request(ri, entries[ri].jacket_path.as_deref(), &cover_textures);
        }
        cover_loader.poll_upload(&mut cover_textures).await;

        clear_background(Color::from_rgba(12, 14, 26, 255));
        for p in [
            base_page.saturating_sub(1),
            base_page,
            (base_page + 1).min(total_pages - 1),
        ] {
            let page_dx = (p as f32 - page_pos) * sw;
            if page_dx.abs() > sw * 1.2 {
                continue;
            }
            let start = p * ITEMS_PER_PAGE;
            for row in 0..PAGE_ROWS {
                for col in 0..COLS {
                    let ei = start + row * COLS + col;
                    if ei >= entries.len() {
                        continue;
                    }
                    let x = PAD + col as f32 * stride + page_dx;
                    let y = grid_top + row as f32 * stride;
                    if x + cell_size < 0.0 || x > sw {
                        continue;
                    }
                    let sel = ei == idx;
                    draw_rectangle(x, y, cell_size, cell_size, Color::from_rgba(22, 30, 46, 230));
                    if let Some(Some(tex)) = &cover_textures[ei] {
                        draw_texture_ex(
                            tex,
                            x + 6.0,
                            y + 6.0,
                            WHITE,
                            DrawTextureParams {
                                dest_size: Some(vec2(cell_size - 12.0, cell_size - 12.0)),
                                ..Default::default()
                            },
                        );
                    }
                    draw_rectangle_lines(
                        x,
                        y,
                        cell_size,
                        cell_size,
                        if sel { 4.0 } else { 2.0 },
                        if sel {
                            Color::from_rgba(245, 90, 90, 255)
                        } else {
                            Color::from_rgba(70, 95, 130, 180)
                        },
                    );
                    let s = &entries[ei];
                    draw_rectangle(
                        x,
                        y + cell_size - 68.0,
                        cell_size,
                        68.0,
                        Color::from_rgba(10, 14, 24, 210),
                    );
                    draw_text_ui(ui_font, &s.title, x + 10.0, y + cell_size - 34.0, 24.0, WHITE);
                    draw_text_ui(
                        ui_font,
                        &format!("{}  Lv {}", s.artist, s.meter),
                        x + 10.0,
                        y + cell_size - 12.0,
                        18.0,
                        Color::from_rgba(170, 205, 255, 255),
                    );
                }
            }
        }

        let cur = &entries[idx];
        let cur_diff = cur
            .difficulties
            .get(diff_idx)
            .cloned()
            .unwrap_or_else(|| ("Hard".to_string(), cur.meter));
        draw_text_ui(ui_font, "SONG SELECT", 36.0, 62.0, 52.0, Color::from_rgba(245, 90, 90, 255));
        draw_text_ui(
            ui_font,
            &format!("Difficulty: {}  [{}]", cur_diff.0, cur_diff.1),
            36.0,
            106.0,
            36.0,
            WHITE,
        );
        draw_text_ui(
            ui_font,
            "LEFT/RIGHT Song  UP/DOWN Difficulty  Hover Preview  Click/Enter Start  ESC/BACK Quit",
            36.0,
            sh - 38.0,
            28.0,
            GRAY,
        );
        for p in 0..total_pages {
            let x = dot_start_x + p as f32 * dot_gap;
            let active = (p as f32 - page_pos).abs() < 0.35;
            draw_circle(
                x,
                dot_y,
                if active { dot_r + 1.0 } else { dot_r },
                if active {
                    Color::from_rgba(245, 90, 90, 255)
                } else {
                    Color::from_rgba(120, 145, 175, 190)
                },
            );
        }
        next_frame().await;
    }
}

fn create_preview_sink(
    handle: &OutputStreamHandle,
    path: &Path,
    start_sec: f32,
) -> Option<Sink> {
    let src = build_preview_source_for_path(path).ok()?;
    let sink = Sink::try_new(handle).ok()?;
    sink.set_volume(PREVIEW_BASE_VOLUME);
    sink.append(src.skip_duration(Duration::from_secs_f32(start_sec.max(0.0))));
    sink.play();
    Some(sink)
}

#[macroquad::main(window_conf)]
async fn main() {
    let boot_t0 = Instant::now();
    set_workdir_to_project_root();
    let app_root = app_storage_root();
    let _ = fs::create_dir_all(&app_root);
    let songs_roots = songs_dir_candidates(&app_root);
    println!("[boot] set_workdir: {} ms", boot_t0.elapsed().as_millis());

    let t_find = Instant::now();
    let entries = discover_song_entries_multi(&songs_roots);
    println!("[boot] scan songs: {} ms | count={}", t_find.elapsed().as_millis(), entries.len());
    if entries.is_empty() {
        return;
    }
    let ui_font = load_ui_font().await;

    let last_selection_path = data_file_path(&app_root, LAST_SELECTION_FILE);
    let last_difficulty_path = data_file_path(&app_root, LAST_DIFFICULTY_FILE);
    let last_chart = load_last_selection(&last_selection_path);
    let mut song_idx = entries
        .iter()
        .position(|e| e.chart_path.to_string_lossy() == last_chart)
        .unwrap_or(0);
    let mut last_diff_name = load_last_difficulty(&last_difficulty_path);
    if last_diff_name.is_empty() {
        last_diff_name = "Hard".to_string();
    }
    let mut diff_idx = entries[song_idx]
        .difficulties
        .iter()
        .position(|d| d.0.eq_ignore_ascii_case(&last_diff_name))
        .unwrap_or(0);
    let mut ignore_escape_until = 0.0_f64;
    loop {
        let Some((chart_path, diff, next_song_idx, next_diff_idx)) =
            song_select(
                &entries,
                song_idx,
                diff_idx,
                ignore_escape_until,
                ui_font.as_ref(),
                &mut last_diff_name,
            )
            .await
        else {
            break;
        };
        song_idx = next_song_idx;
        diff_idx = next_diff_idx;
        save_last_selection(&last_selection_path, &chart_path);
        save_last_difficulty(&last_difficulty_path, &diff);

        let t_parse = Instant::now();
        let ext = chart_path
            .extension()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default()
            .to_ascii_lowercase();
        let chart = match if ext == "dwi" {
            dwi_parser::parse_dwi(&chart_path, Some(&diff))
        } else {
            sm_parser::parse_sm(&chart_path, Some(&diff))
        } {
            Ok(c) => c,
            Err(e) => {
                println!("Parse failed {}: {e}", chart_path.display());
                continue;
            }
        };
        println!("[boot] parse selected: {} ms", t_parse.elapsed().as_millis());
        println!(
            "[chart] timing: offset={:.3} bpms={} stops={} notes={}",
            chart.offset,
            chart.bpms.len(),
            chart.stops.len(),
            chart.notes.len()
        );
        let bpm_preview = chart
            .bpms
            .iter()
            .take(24)
            .map(|b| format!("{:.3}={:.3}", b.beat, b.bpm))
            .collect::<Vec<_>>()
            .join(", ");
        println!("[chart] BPMS(first): {}", bpm_preview);
        let stop_preview = chart
            .stops
            .iter()
            .take(24)
            .map(|s| format!("{:.3}={:.3}", s.beat, s.seconds))
            .collect::<Vec<_>>()
            .join(", ");
        println!("[chart] STOPS(first): {}", stop_preview);

        'play_loop: loop {
            let t_game = Instant::now();
            let mut game = Game::new(chart.clone(), 1.0).await;
            println!("[boot] game new await: {} ms", t_game.elapsed().as_millis());
            match game.run().await {
                GameExitAction::BackToSongSelect => {
                    ignore_escape_until = get_time() + 0.25;
                    break 'play_loop;
                }
                GameExitAction::Restart => {
                    continue 'play_loop;
                }
            }
        }
    }
}

fn draw_text_ui(font: Option<&Font>, text: &str, x: f32, y: f32, size: f32, color: Color) {
    if let Some(font) = font {
        draw_text_ex(
            text,
            x,
            y,
            TextParams {
                font: Some(font),
                font_size: size as u16,
                font_scale: 1.0,
                color,
                ..Default::default()
            },
        );
    } else {
        draw_text(text, x, y, size, color);
    }
}

async fn load_ui_font() -> Option<Font> {
    for p in UI_FONT_CANDIDATES {
        if Path::new(p).is_file() {
            if let Ok(font) = load_ttf_font(p).await {
                return Some(font);
            }
        }
    }
    None
}

fn list_difficulties_by_probe(chart_path: &Path, ext: &str, fallback_chart: &chart::Chart) -> Vec<(String, i32)> {
    let probes = [
        "Beginner",
        "Easy",
        "Basic",
        "Light",
        "Medium",
        "Standard",
        "Hard",
        "Difficult",
        "Expert",
        "Challenge",
        "Edit",
    ];
    let mut out: Vec<(String, i32)> = Vec::new();
    for p in probes {
        let parsed = if ext == "dwi" {
            dwi_parser::parse_dwi(chart_path, Some(p))
        } else {
            sm_parser::parse_sm(chart_path, Some(p))
        };
        if let Ok(c) = parsed {
            let key = (c.difficulty.clone(), c.meter);
            if !out.iter().any(|x| x.0.eq_ignore_ascii_case(&key.0) && x.1 == key.1) {
                out.push(key);
            }
        }
    }
    if out.is_empty() {
        out.push((fallback_chart.difficulty.clone(), fallback_chart.meter));
    }
    out.sort_by(|a, b| a.1.cmp(&b.1).then_with(|| a.0.cmp(&b.0)));
    out
}
