use std::path::{Path, PathBuf};
use std::{ffi::OsStr, fs};

#[derive(Debug, Clone)]
pub struct BpmSegment {
    pub beat: f32,
    pub bpm: f32,
}

#[derive(Debug, Clone)]
pub struct StopSegment {
    pub beat: f32,
    pub seconds: f32,
}

#[derive(Debug, Clone)]
pub enum NoteKind {
    Tap,
    Hold { end_beat: f32, end_time: f32 },
}

#[derive(Debug, Clone)]
pub struct Note {
    pub lane: usize,
    pub beat: f32,
    pub time: f32,
    pub kind: NoteKind,
}

#[derive(Debug, Clone)]
pub struct Chart {
    pub title: String,
    pub artist: String,
    pub music: Option<PathBuf>,
    pub background: Option<PathBuf>,
    pub difficulty: String,
    pub meter: i32,
    pub offset: f32,
    pub bpms: Vec<BpmSegment>,
    pub stops: Vec<StopSegment>,
    pub notes: Vec<Note>,
}

impl Chart {
    pub fn finalize_times(&mut self) {
        self.bpms
            .sort_by(|a, b| a.beat.partial_cmp(&b.beat).unwrap_or(std::cmp::Ordering::Equal));
        self.stops
            .sort_by(|a, b| a.beat.partial_cmp(&b.beat).unwrap_or(std::cmp::Ordering::Equal));
        if self.bpms.is_empty() {
            self.bpms.push(BpmSegment { beat: 0.0, bpm: 120.0 });
        }
        for note in &mut self.notes {
            note.time = beat_to_seconds(note.beat, &self.bpms, &self.stops) - self.offset;
            if let NoteKind::Hold { end_beat, end_time } = &mut note.kind {
                *end_time = beat_to_seconds(*end_beat, &self.bpms, &self.stops) - self.offset;
            }
        }
        self.notes
            .sort_by(|a, b| a.time.partial_cmp(&b.time).unwrap_or(std::cmp::Ordering::Equal));
    }

    pub fn resolve_music_path(&mut self, chart_path: &Path) {
        let folder = chart_path.parent().unwrap_or_else(|| Path::new("."));
        if let Some(music) = self.music.clone() {
            let raw = music.to_string_lossy().trim().trim_matches('"').to_string();
            let p = folder.join(&raw);
            if p.exists() {
                let ext = p
                    .extension()
                    .and_then(OsStr::to_str)
                    .unwrap_or("")
                    .to_ascii_lowercase();
                if matches!(ext.as_str(), "ogg" | "wav" | "mp3" | "aac" | "m4a") {
                    self.music = Some(p);
                    return;
                }

                let stem = p.file_stem().and_then(OsStr::to_str).unwrap_or("");
                for try_ext in ["wav", "mp3", "ogg", "aac", "m4a"] {
                    let sibling = folder.join(format!("{stem}.{try_ext}"));
                    if sibling.exists() {
                        self.music = Some(sibling);
                        return;
                    }
                }

                self.music = Some(p);
                return;
            }
        }

        let stem = chart_path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
        let exts = ["wav", "mp3", "ogg", "aac", "m4a"];
        for ext in exts {
            let p = folder.join(format!("{stem}.{ext}"));
            if p.exists() {
                self.music = Some(p);
                return;
            }
        }

        if let Ok(entries) = fs::read_dir(folder) {
            for e in entries.flatten() {
                let p = e.path();
                let ext = p.extension().and_then(OsStr::to_str).unwrap_or("").to_ascii_lowercase();
                if matches!(ext.as_str(), "ogg" | "mp3" | "wav" | "aac" | "m4a") {
                    self.music = Some(p);
                    return;
                }
            }
        }

        self.music = None;
    }

    pub fn resolve_background_path(&mut self, chart_path: &Path) {
        let folder = chart_path.parent().unwrap_or_else(|| Path::new("."));
        if let Some(bg) = self.background.clone() {
            let raw = bg.to_string_lossy().trim().trim_matches('"').to_string();
            if !raw.is_empty() {
                let p = folder.join(&raw);
                if p.is_file() {
                    self.background = Some(p);
                    return;
                }
            }
        }

        let stem = chart_path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
        let p = folder.join(format!("{stem}-bg.png"));
        if p.is_file() {
            self.background = Some(p);
            return;
        }

        if let Ok(entries) = fs::read_dir(folder) {
            for e in entries.flatten() {
                let p = e.path();
                if !p.is_file() {
                    continue;
                }
                let name = p
                    .file_name()
                    .and_then(OsStr::to_str)
                    .unwrap_or("")
                    .to_ascii_lowercase();
                if name.ends_with("-bg.png") {
                    self.background = Some(p);
                    return;
                }
            }
        }

        self.background = None;
    }
}

pub fn beat_to_seconds(beat: f32, segments: &[BpmSegment], stops: &[StopSegment]) -> f32 {
    if segments.is_empty() {
        return beat * 0.5;
    }

    let mut time = 0.0;
    let mut i = 0;
    while i < segments.len() {
        let seg = &segments[i];
        let next_beat = segments.get(i + 1).map(|s| s.beat).unwrap_or(f32::INFINITY);

        if beat <= seg.beat {
            return time;
        }

        let start = seg.beat;
        let end = beat.min(next_beat);
        if end > start {
            let bpm = if seg.bpm > 0.0 { seg.bpm } else { 0.0001 };
            time += (end - start) * (60.0 / bpm);
        }

        if beat <= next_beat {
            break;
        }
        i += 1;
    }
    for stop in stops {
        if stop.beat <= beat {
            time += stop.seconds.max(0.0);
        } else {
            break;
        }
    }

    time
}

pub fn seconds_to_beat(seconds: f32, segments: &[BpmSegment], stops: &[StopSegment]) -> f32 {
    if segments.is_empty() {
        return seconds * 2.0;
    }

    let target = seconds.max(0.0);
    let mut time = 0.0_f32;
    let mut beat = segments[0].beat.max(0.0);
    let mut bpm_idx = 0usize;
    let mut stop_idx = 0usize;

    while bpm_idx + 1 < segments.len() && segments[bpm_idx + 1].beat <= beat {
        bpm_idx += 1;
    }

    loop {
        let bpm = if segments[bpm_idx].bpm > 0.0 {
            segments[bpm_idx].bpm
        } else {
            0.0001
        };
        let sec_per_beat = 60.0 / bpm;

        let next_bpm_beat = segments
            .iter()
            .skip(bpm_idx + 1)
            .map(|s| s.beat)
            .find(|b| *b > beat)
            .unwrap_or(f32::INFINITY);
        let next_stop_beat = stops
            .iter()
            .skip(stop_idx)
            .map(|s| s.beat)
            .find(|b| *b >= beat)
            .unwrap_or(f32::INFINITY);
        let next_event_beat = next_bpm_beat.min(next_stop_beat);

        if next_event_beat.is_finite() {
            let beat_span = (next_event_beat - beat).max(0.0);
            let move_secs = beat_span * sec_per_beat;
            if target < time + move_secs {
                return beat + (target - time) / sec_per_beat;
            }
            time += move_secs;
            beat = next_event_beat;
        } else {
            return beat + (target - time) / sec_per_beat;
        }

        while stop_idx < stops.len() && (stops[stop_idx].beat - beat).abs() < 0.0001 {
            let stop_secs = stops[stop_idx].seconds.max(0.0);
            if target < time + stop_secs {
                return beat;
            }
            time += stop_secs;
            stop_idx += 1;
        }

        while bpm_idx + 1 < segments.len() && (segments[bpm_idx + 1].beat - beat).abs() < 0.0001 {
            bpm_idx += 1;
        }
    }
}
