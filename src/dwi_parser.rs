use std::{collections::HashMap, fs, path::Path};

use anyhow::{Result, anyhow};

use crate::chart::{BpmSegment, Chart, Note, NoteKind, StopSegment};

// DWI FREEZE 在不同老谱包里存在实现差异，这里留一个兼容缩放参数。
// 1.0 = StepMania 标准换算；2.0 可匹配部分 DDR 老包的停顿体感。
const DWI_FREEZE_SCALE: f32 = 2.0;

#[derive(Clone)]
struct Candidate {
    difficulty: String,
    meter: i32,
    stream: String,
}

pub fn parse_dwi(path: &Path, difficulty_filter: Option<&str>) -> Result<Chart> {
    let raw = fs::read_to_string(path)?;
    let tags = parse_tags(&raw);

    let mut title = String::from("Untitled");
    let mut artist = String::new();
    let mut music = None;
    let mut background = None;
    let mut gap_ms = 0.0_f32;
    let mut bpms = vec![BpmSegment { beat: 0.0, bpm: 120.0 }];
    let mut stops = Vec::<StopSegment>::new();

    let mut charts: Vec<Candidate> = Vec::new();

    for (k, v) in tags {
        match k.as_str() {
            "TITLE" => title = v,
            "ARTIST" => artist = v,
            "FILE" | "MUSIC" => music = Some(v.into()),
            "BACKGROUND" | "BG" => background = Some(v.into()),
            "GAP" => gap_ms = v.parse::<f32>().unwrap_or(0.0),
            "BPM" => {
                let bpm = v.parse::<f32>().unwrap_or(120.0);
                bpms = vec![BpmSegment { beat: 0.0, bpm }];
            }
            "CHANGEBPM" | "CHANGEBPMS" | "BPMCHANGE" | "BPMCHANGES" => {
                for pair in v.split(',') {
                    let mut it = pair.split('=');
                    let beat = it.next().unwrap_or("0").trim().parse::<f32>().unwrap_or(0.0) / 4.0;
                    let bpm = it
                        .next()
                        .unwrap_or("120")
                        .trim()
                        .parse::<f32>()
                        .unwrap_or(120.0);
                    if bpm > 0.0 {
                        bpms.push(BpmSegment { beat, bpm });
                    }
                }
            }
            "FREEZE" | "FREEZES" => {
                for pair in v.split(',') {
                    let mut it = pair.split('=');
                    let beat = it.next().unwrap_or("0").trim().parse::<f32>().unwrap_or(0.0) / 4.0;
                    let seconds = (it.next().unwrap_or("0").trim().parse::<f32>().unwrap_or(0.0)
                        / 1000.0)
                        * DWI_FREEZE_SCALE;
                    if seconds > 0.0 {
                        stops.push(StopSegment { beat, seconds });
                    }
                }
            }
            "SINGLE" => {
                let parts: Vec<&str> = v.split(':').collect();
                if parts.len() >= 3 {
                    charts.push(Candidate {
                        difficulty: normalize_difficulty(parts[0]),
                        meter: parts[1].trim().parse::<i32>().unwrap_or(1),
                        stream: parts[2..].join(":"),
                    });
                }
            }
            _ => {}
        }
    }

    let selected = select_chart(&charts, difficulty_filter)
        .ok_or_else(|| anyhow!("No SINGLE chart found in .dwi"))?;

    let mut chart = Chart {
        title,
        artist,
        music,
        background,
        difficulty: selected.difficulty,
        meter: selected.meter,
        offset: -(gap_ms / 1000.0),
        bpms,
        stops,
        notes: parse_dwi_stream(&selected.stream),
    };

    chart.finalize_times();
    chart.resolve_music_path(path);
    chart.resolve_background_path(path);
    Ok(chart)
}

fn normalize_difficulty(s: &str) -> String {
    let key = s.trim().to_uppercase();
    let mut map = HashMap::new();
    map.insert("BEGINNER", "Beginner");
    map.insert("EASY", "Easy");
    map.insert("BASIC", "Easy");
    map.insert("LIGHT", "Easy");
    map.insert("ANOTHER", "Medium");
    map.insert("STANDARD", "Medium");
    map.insert("MANIAC", "Hard");
    map.insert("HEAVY", "Hard");
    map.insert("HARD", "Hard");
    map.insert("CHALLENGE", "Challenge");
    map.insert("SMANIAC", "Challenge");

    map.get(key.as_str()).unwrap_or(&"Edit").to_string()
}

fn select_chart(charts: &[Candidate], difficulty_filter: Option<&str>) -> Option<Candidate> {
    if charts.is_empty() {
        return None;
    }

    if let Some(filter) = difficulty_filter {
        let f = filter.to_lowercase();
        if let Some(c) = charts.iter().find(|c| c.difficulty.to_lowercase().contains(&f)) {
            return Some(c.clone());
        }
    }

    charts.iter().max_by_key(|c| c.meter).cloned()
}

fn parse_dwi_stream(stream: &str) -> Vec<Note> {
    let mut out = Vec::<Note>::new();
    let s: String = stream.chars().filter(|c| !c.is_whitespace()).collect();
    let chars: Vec<char> = s.chars().collect();

    let mut beat = 0.0_f64; // f64 与 SM5 源码 double 一致，消除累积误差
    let mut inc = 0.5_f64;
    let mut i = 0;

    while i < chars.len() {
        let c = chars[i].to_ascii_uppercase();
        i += 1;
        match c {
            '(' => {
                inc = 0.25;
                continue;
            }
            '[' => {
                inc = 1.0 / 6.0;
                continue;
            }
            '{' => {
                inc = 1.0 / 16.0;
                continue;
            }
            '`' => {
                inc = 1.0 / 48.0;
                continue;
            }
            ')' | ']' | '}' | '\'' | '>' => {
                inc = 0.5;
                continue;
            }
            _ => {
                if c == '!' {
                    continue;
                }

                let mut jump = false;
                if c == '<' {
                    // SM5 兼容：若 <...> 中出现 0，则将其视为 1/192 切分标记而非 jump。
                    if is_192_marker(&chars, i) {
                        inc = 1.0 / 48.0;
                        continue;
                    }
                    jump = true;
                }

                if jump {
                    while i < chars.len() {
                        let jc = chars[i].to_ascii_uppercase();
                        i += 1;
                        if jc == '>' {
                            break;
                        }
                        emit_tap_char(jc, beat as f32, &mut out);
                        if i < chars.len() && chars[i] == '!' {
                            i += 1;
                            if i < chars.len() {
                                let hold_char = chars[i].to_ascii_uppercase();
                                i += 1;
                                mark_hold_heads(hold_char, beat as f32, &mut out);
                            }
                        }
                    }
                } else {
                    emit_tap_char(c, beat as f32, &mut out);
                    if i < chars.len() && chars[i] == '!' {
                        i += 1;
                        if i < chars.len() {
                            let hold_char = chars[i].to_ascii_uppercase();
                            i += 1;
                            mark_hold_heads(hold_char, beat as f32, &mut out);
                        }
                    }
                }

                beat += inc;
            }
        }
    }

    finalize_dwi_holds(&mut out);
    out
}

fn is_192_marker(chars: &[char], mut pos: usize) -> bool {
    while pos < chars.len() {
        let c = chars[pos];
        if c == '>' {
            return false;
        }
        if c == '0' {
            return true;
        }
        pos += 1;
    }
    false
}

fn emit_tap_char(c: char, beat: f32, out: &mut Vec<Note>) {
    for lane in dwi_char_to_lanes(c) {
        out.push(Note {
            lane: *lane,
            beat,
            time: 0.0,
            kind: NoteKind::Tap,
        });
    }
}

fn mark_hold_heads(hold_char: char, beat: f32, out: &mut Vec<Note>) {
    const EPS: f32 = 0.0001;
    for lane in dwi_char_to_lanes(hold_char) {
        let mut found = false;
        for note in out.iter_mut().rev() {
            if note.lane == *lane && (note.beat - beat).abs() < EPS {
                note.kind = NoteKind::Hold {
                    end_beat: beat, // 占位，后续按 SM5 规则用同轨下一音符闭合
                    end_time: 0.0,
                };
                found = true;
                break;
            }
        }
        if !found {
            out.push(Note {
                lane: *lane,
                beat,
                time: 0.0,
                kind: NoteKind::Hold {
                    end_beat: beat, // 占位，后续按 SM5 规则用同轨下一音符闭合
                    end_time: 0.0,
                },
            });
        }
    }
}

fn finalize_dwi_holds(notes: &mut Vec<Note>) {
    const EPS: f32 = 0.0001;
    let mut remove = vec![false; notes.len()];
    for lane in 0..4usize {
        let lane_indices: Vec<usize> = notes
            .iter()
            .enumerate()
            .filter_map(|(idx, n)| if n.lane == lane { Some(idx) } else { None })
            .collect();

        let mut open_head: Option<usize> = None;
        for idx in lane_indices {
            if let Some(head_idx) = open_head {
                let end_beat = notes[idx].beat;
                if let NoteKind::Hold {
                    end_beat: ref mut hb,
                    ..
                } = notes[head_idx].kind
                {
                    *hb = end_beat;
                }
                remove[idx] = true; // SM5: 尾点音符被吞掉，仅用于闭合长条
                open_head = None;
                continue;
            }

            if let NoteKind::Hold { end_beat, .. } = notes[idx].kind {
                if (end_beat - notes[idx].beat).abs() < EPS {
                    open_head = Some(idx);
                }
            }
        }

        if let Some(unclosed) = open_head {
            remove[unclosed] = true; // SM5: 未闭合长条头会被移除
        }
    }

    let mut kept = Vec::with_capacity(notes.len());
    for (idx, n) in notes.iter().enumerate() {
        if !remove[idx] {
            kept.push(n.clone());
        }
    }
    *notes = kept;
}

fn dwi_char_to_lanes(c: char) -> &'static [usize] {
    match c.to_ascii_uppercase() {
        '1' => &[1, 0],
        '2' => &[1],
        '3' => &[1, 3],
        '4' => &[0],
        '6' => &[3],
        '7' => &[2, 0],
        '8' => &[2],
        '9' => &[2, 3],
        'A' => &[2, 1],
        'B' => &[0, 3],
        _ => &[],
    }
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
