//! MIDI の読み込み・解析・書き出し
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use midly::{
    num::{u24, u28},
    Format, Header, MetaMessage, MidiMessage, Smf, Timing, TrackEvent, TrackEventKind,
};

pub const DRUM_CHANNEL: u8 = 9;

#[derive(Clone, Debug)]
pub struct Note {
    pub track: usize,
    #[allow(dead_code)]
    pub channel: u8,
    pub key: u8,
    pub start: u64, // tick(絶対)
    pub end: u64,
    pub velocity: u8,
    pub is_drum: bool,
}

#[derive(Clone, Debug)]
pub struct TrackInfo {
    pub name: String,
    pub is_drum: bool,
    pub notes: usize,
    #[allow(dead_code)]
    pub channels: Vec<u8>,
}

pub struct Loaded {
    pub path: PathBuf,
    pub smf: Smf<'static>,
    pub ppq: u16,
    /// (絶対tick, us/beat) 昇順。先頭は必ず tick 0。
    pub tempos: Vec<(u64, f64)>,
    pub notes: Vec<Note>,
    pub tracks: Vec<TrackInfo>,
    #[allow(dead_code)]
    pub end_tick: u64,
}

impl Loaded {
    pub fn load(path: &Path) -> Result<Self, String> {
        let bytes = std::fs::read(path).map_err(|e| format!("ファイルを読めません: {e}"))?;
        // Smf は入力バッファを借用するので 'static にする
        let bytes: &'static [u8] = Box::leak(bytes.into_boxed_slice());
        let smf = Smf::parse(bytes).map_err(|e| format!("MIDIの解析に失敗: {e}"))?;

        let ppq = match smf.header.timing {
            Timing::Metrical(t) => t.as_int(),
            Timing::Timecode(_, _) => {
                return Err("SMPTE タイムコード形式のMIDIには未対応です".to_string())
            }
        };
        if ppq == 0 {
            return Err("PPQ が 0 のMIDIは扱えません".to_string());
        }

        // --- テンポマップ ---
        let mut tempos: Vec<(u64, f64)> = Vec::new();
        for track in &smf.tracks {
            let mut abs = 0u64;
            for ev in track {
                abs += ev.delta.as_int() as u64;
                if let TrackEventKind::Meta(MetaMessage::Tempo(t)) = ev.kind {
                    tempos.push((abs, t.as_int() as f64));
                }
            }
        }
        tempos.sort_by_key(|x| x.0);
        if tempos.is_empty() {
            tempos.push((0, 500_000.0));
        } else if tempos[0].0 != 0 {
            let first = tempos[0].1;
            tempos.insert(0, (0, first));
        }

        // --- ノートとトラック情報 ---
        let mut notes: Vec<Note> = Vec::new();
        let mut tracks: Vec<TrackInfo> = Vec::new();
        let mut end_tick = 0u64;

        for (ti, track) in smf.tracks.iter().enumerate() {
            let mut abs = 0u64;
            let mut name = String::new();
            let mut open: HashMap<(u8, u8), (u64, u8)> = HashMap::new();
            let mut t_notes = 0usize;
            let mut channels: Vec<u8> = Vec::new();
            let mut is_drum = false;

            for ev in track {
                abs += ev.delta.as_int() as u64;
                match ev.kind {
                    TrackEventKind::Meta(MetaMessage::TrackName(n)) => {
                        if name.is_empty() {
                            name = String::from_utf8_lossy(n).to_string();
                        }
                    }
                    TrackEventKind::Midi { channel, message } => {
                        let ch = channel.as_int();
                        if !channels.contains(&ch) {
                            channels.push(ch);
                        }
                        if ch == DRUM_CHANNEL {
                            is_drum = true;
                        }
                        match message {
                            MidiMessage::NoteOn { key, vel } if vel.as_int() > 0 => {
                                open.insert((ch, key.as_int()), (abs, vel.as_int()));
                            }
                            MidiMessage::NoteOn { key, .. } | MidiMessage::NoteOff { key, .. } => {
                                if let Some((start, velocity)) = open.remove(&(ch, key.as_int())) {
                                    notes.push(Note {
                                        track: ti,
                                        channel: ch,
                                        key: key.as_int(),
                                        start,
                                        end: abs.max(start + 1),
                                        velocity,
                                        is_drum: ch == DRUM_CHANNEL,
                                    });
                                    t_notes += 1;
                                }
                            }
                            _ => {}
                        }
                    }
                    _ => {}
                }
            }
            // 閉じられていないノートは最後まで伸ばす
            for ((ch, key), (start, velocity)) in open {
                notes.push(Note {
                    track: ti,
                    channel: ch,
                    key,
                    start,
                    end: abs.max(start + 1),
                    velocity,
                    is_drum: ch == DRUM_CHANNEL,
                });
                t_notes += 1;
            }
            end_tick = end_tick.max(abs);
            channels.sort_unstable();
            tracks.push(TrackInfo { name, is_drum, notes: t_notes, channels });
        }
        notes.sort_by_key(|n| (n.start, n.key));

        Ok(Loaded { path: path.to_path_buf(), smf, ppq, tempos, notes, tracks, end_tick })
    }

    /// ファイルに書かれているテンポ(BPM)
    pub fn stated_bpm(&self) -> f64 {
        60_000_000.0 / self.tempos.first().map(|x| x.1).unwrap_or(500_000.0)
    }

    /// ファイル内に複数のテンポがあるか
    pub fn tempo_changes(&self) -> usize {
        self.tempos.len().saturating_sub(1)
    }

    /// 絶対tick -> 秒（テンポマップ準拠）
    pub fn time_at(&self, tick: u64) -> f64 {
        let ppq = self.ppq as f64;
        let mut sec = 0.0f64;
        let mut prev = 0u64;
        let mut us = self.tempos.first().map(|x| x.1).unwrap_or(500_000.0);
        for &(tk, nus) in &self.tempos {
            if tk > tick {
                break;
            }
            sec += (tk - prev) as f64 * us / (ppq * 1_000_000.0);
            prev = tk;
            us = nus;
        }
        sec += (tick - prev) as f64 * us / (ppq * 1_000_000.0);
        sec
    }

    /// 最後の音が終わる時刻(秒)
    pub fn last_time(&self) -> f64 {
        self.notes.iter().map(|n| self.time_at(n.end)).fold(0.0, f64::max)
    }

    pub fn has_drums(&self) -> bool {
        self.notes.iter().any(|n| n.is_drum)
    }

    pub fn drum_notes(&self) -> usize {
        self.notes.iter().filter(|n| n.is_drum).count()
    }
}

/// テンポ解析に使うオンセット列 (秒, 重み)
pub fn collect_onsets(loaded: &Loaded, drums_only: bool) -> Vec<(f64, f64)> {
    let mut v: Vec<(f64, f64)> = Vec::new();
    for n in &loaded.notes {
        let (sec, w) = (loaded.time_at(n.start), onset_weight(n, drums_only));
        if w <= 0.0 {
            continue;
        }
        v.push((sec, w));
    }
    v.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
    // 10ms 以内の発音はひとつにまとめる（同時打点を重複評価しないため）
    let mut merged: Vec<(f64, f64)> = Vec::with_capacity(v.len());
    for (t, w) in v {
        if let Some(last) = merged.last_mut() {
            if t - last.0 < 0.010 {
                last.1 = last.1.max(w);
                continue;
            }
        }
        merged.push((t, w));
    }
    merged
}

fn onset_weight(n: &Note, drums_only: bool) -> f64 {
    if !n.is_drum {
        return if drums_only { 0.0 } else { 0.35 };
    }
    match n.key {
        35 | 36 => 1.0,       // キック
        38 | 40 | 39 => 1.0,  // スネア/手拍子
        42 | 44 | 46 => 0.6,  // ハイハット
        49 | 57 | 52 | 55 => 0.7, // クラッシュ
        _ => 0.45,
    }
}

#[derive(Clone, Debug)]
pub struct ExportOptions {
    /// 書き出すテンポ
    pub bpm: f64,
    /// 拍グリッドを前へずらす量(秒)。この秒数だけ音を前へ動かす。
    pub shift_sec: f64,
    /// true: tick位置はそのまま（再生速度が変わる）
    pub keep_ticks: bool,
}

pub fn export(loaded: &Loaded, opts: &ExportOptions, out: &Path) -> Result<(), String> {
    let ppq = loaded.ppq as f64;
    let us = (60_000_000.0 / opts.bpm).round().clamp(1.0, 16_777_215.0) as u32;
    let dst_tick_sec = (us as f64 / 1_000_000.0) / ppq; // 新テンポでの 1tick 秒数
    let tempo_msg = TrackEvent {
        delta: u28::new(0),
        kind: TrackEventKind::Meta(MetaMessage::Tempo(u24::new(us))),
    };

    let format = loaded.smf.header.format;
    let header = Header::new(format, Timing::Metrical(midly::num::u15::new(loaded.ppq)));
    let mut out_smf = Smf::new(header);

    for (ti, track) in loaded.smf.tracks.iter().enumerate() {
        let mut abs = 0u64;
        let mut prev_new = 0u64;
        let mut out_track: Vec<TrackEvent> = Vec::with_capacity(track.len() + 1);
        if ti == 0 {
            out_track.push(tempo_msg);
        }
        let mut has_eot = false;
        for ev in track {
            abs += ev.delta.as_int() as u64;
            // 元のテンポ情報は捨てる（書き出しは単一テンポ）
            if matches!(ev.kind, TrackEventKind::Meta(MetaMessage::Tempo(_))) {
                continue;
            }
            let new_abs = if opts.keep_ticks {
                abs
            } else {
                let sec = (loaded.time_at(abs) - opts.shift_sec).max(0.0);
                (sec / dst_tick_sec).round() as u64
            };
            let new_abs = new_abs.max(prev_new);
            if matches!(ev.kind, TrackEventKind::Meta(MetaMessage::EndOfTrack)) {
                has_eot = true;
            }
            out_track.push(TrackEvent {
                delta: u28::from_int_lossy((new_abs - prev_new) as u32),
                kind: ev.kind,
            });
            prev_new = new_abs;
        }
        if !has_eot {
            out_track.push(TrackEvent {
                delta: u28::new(0),
                kind: TrackEventKind::Meta(MetaMessage::EndOfTrack),
            });
        }
        out_smf.tracks.push(out_track);
    }

    if out_smf.header.format == Format::SingleTrack && out_smf.tracks.len() > 1 {
        out_smf.header.format = Format::Parallel;
    }

    let mut buf = Vec::with_capacity(64 * 1024);
    out_smf.write(&mut buf).map_err(|e| format!("MIDIの書き出しに失敗: {e}"))?;
    std::fs::write(out, buf).map_err(|e| format!("保存に失敗: {e}"))?;
    Ok(())
}
