use std::{fs, path::Path};

use anyhow::{Result, anyhow};

use crate::chart::{BpmSegment, Chart, Note, NoteKind, StopSegment};

// 与 DWI 保持一致的停顿体感缩放。
const SM_STOP_SCALE: f32 = 2.0;

#[derive(Clone)]
struct Candidate {
    difficulty: String,
    meter: i32,
    notes_blob: String,
}

pub fn parse_sm(path: &Path, difficulty_filter: Option<&str>) -> Result<Chart> {
    let raw = fs::read_to_string(path)?;
    let tags = parse_tags(&raw);

    let mut title = String::from("Untitled");
    let mut artist = String::new();
    let mut music = None;
    let mut background = None;
    let mut offset = 0.0;
    let mut bpms = vec![BpmSegment { beat: 0.0, bpm: 120.0 }];
    let mut stops = Vec::<StopSegment>::new();

    let mut charts = Vec::<Candidate>::new();

    for (k, v) in tags {
        match k.as_str() {
            "TITLE" => title = v,
            "ARTIST" => artist = v,
            "MUSIC" => music = Some(v.into()),
            "BACKGROUND" => background = Some(v.into()),
            "OFFSET" => offset = v.parse::<f32>().unwrap_or(0.0),
            "BPMS" => bpms = parse_bpms(&v),
            "STOPS" | "FREEZES" | "DELAYS" => {
                stops.extend(parse_stops(&v, SM_STOP_SCALE));
            }
            "NOTES" => {
                if let Some(c) = parse_notes_block(&v) {
                    charts.push(c);
                }
            }
            _ => {}
        }
    }

    let selected = select_chart(&charts, difficulty_filter)
        .ok_or_else(|| anyhow!("No dance-single chart found in .sm"))?;

    let mut chart = Chart {
        title,
        artist,
        music,
        background,
        difficulty: selected.difficulty,
        meter: selected.meter,
        offset,
        bpms,
        stops,
        notes: parse_sm_notes(&selected.notes_blob),
    };

    chart.finalize_times();
    chart.resolve_music_path(path);
    chart.resolve_background_path(path);
    Ok(chart)
}

fn parse_notes_block(raw: &str) -> Option<Candidate> {
    let parts: Vec<&str> = raw.split(':').collect();
    if parts.len() < 6 {
        return None;
    }

    let style = parts[0].trim().to_lowercase();
    if style != "dance-single" {
        return None;
    }

    let difficulty = parts[2].trim().to_string();
    let meter = parts[3].trim().parse::<i32>().unwrap_or(1);
    let notes_blob = parts[5..].join(":").trim().to_string();

    Some(Candidate {
        difficulty,
        meter,
        notes_blob,
    })
}

fn select_chart(charts: &[Candidate], difficulty_filter: Option<&str>) -> Option<Candidate> {
    if charts.is_empty() {
        return None;
    }

    if let Some(filter) = difficulty_filter {
        let target = filter.to_lowercase();
        if let Some(c) = charts
            .iter()
            .find(|c| c.difficulty.to_lowercase().contains(&target))
        {
            return Some(c.clone());
        }
    }

    charts.iter().max_by_key(|c| c.meter).cloned()
}

fn parse_sm_notes(blob: &str) -> Vec<Note> {
    let mut out = Vec::new();
    let mut hold_starts: [Option<f32>; 4] = [None, None, None, None];
    let measures: Vec<&str> = blob.split(',').collect();

    for (m_idx, measure_raw) in measures.iter().enumerate() {
        let rows: Vec<&str> = measure_raw
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .collect();
        if rows.is_empty() {
            continue;
        }

        let rows_len = rows.len() as f32;
        for (r_idx, row) in rows.iter().enumerate() {
            let beat = m_idx as f32 * 4.0 + (r_idx as f32 / rows_len) * 4.0;

            for (lane, ch) in row.chars().take(4).enumerate() {
                match ch {
                    '1' => out.push(Note {
                        lane,
                        beat,
                        time: 0.0,
                        kind: NoteKind::Tap,
                    }),
                    '2' | '4' => {
                        hold_starts[lane] = Some(beat);
                    }
                    '3' => {
                        if let Some(start_beat) = hold_starts[lane].take() {
                            let end_beat = beat.max(start_beat + 0.01);
                            out.push(Note {
                                lane,
                                beat: start_beat,
                                time: 0.0,
                                kind: NoteKind::Hold {
                                    end_beat,
                                    end_time: 0.0,
                                },
                            });
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    out
}

fn parse_bpms(s: &str) -> Vec<BpmSegment> {
    let mut out = Vec::new();
    for pair in s.split(',') {
        let mut it = pair.split('=');
        let beat = it.next().unwrap_or("0").trim().parse::<f32>().unwrap_or(0.0);
        let bpm = it
            .next()
            .unwrap_or("120")
            .trim()
            .parse::<f32>()
            .unwrap_or(120.0);
        out.push(BpmSegment { beat, bpm });
    }
    if out.is_empty() {
        out.push(BpmSegment { beat: 0.0, bpm: 120.0 });
    }
    out
}

fn parse_stops(s: &str, scale: f32) -> Vec<StopSegment> {
    let mut out = Vec::new();
    for pair in s.split(',') {
        let mut it = pair.split('=');
        let beat = it.next().unwrap_or("0").trim().parse::<f32>().unwrap_or(0.0);
        let seconds = it
            .next()
            .unwrap_or("0")
            .trim()
            .parse::<f32>()
            .unwrap_or(0.0)
            * scale;
        if seconds > 0.0 {
            out.push(StopSegment { beat, seconds });
        }
    }
    out
}

fn parse_tags(raw: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut buf = String::new();
    let mut in_tag = false;

    for line in raw.lines() {
        let line = line.split("//").next().unwrap_or("");
        for ch in line.chars() {
            if ch == '#' {
                in_tag = true;
                buf.clear();
            }
            if in_tag {
                buf.push(ch);
            }
            if in_tag && ch == ';' {
                if let Some(colon) = buf.find(':') {
                    let key = buf[1..colon].trim().to_uppercase();
                    let val = buf[colon + 1..buf.len() - 1].trim().to_string();
                    out.push((key, val));
                }
                in_tag = false;
                buf.clear();
            }
        }
        if in_tag {
            buf.push('\n');
        }
    }

    out
}
