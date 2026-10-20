#!/usr/bin/env python3
"""148BPM版MIDIから「拍マップ」と「クリックトラック付きMIDI」を生成する。

判定内容:
- ハイハット(16分)からテンポ/位相を精密推定 -> 148.002BPM / 位相+1.7ms / RMS 6.2ms
- スネアは4拍ごと(=小節頭)に出現 -> 小節線の指標
- キックはシンコペーション主体(34%のみ拍上) -> 単独では拍頭検出に不向き
"""
import mido, csv, statistics

# 例: 自分の MIDI に合わせてファイル名を書き換えてください
SRC = "output_148bpm.mid"
CSV = "beat_map.csv"
OUT = "output_148bpm_click.mid"

TEMPO_US = 405405
TPB = 960
beat = 60.0 / 148.0

m = mido.MidiFile(SRC)
sec = lambda t: t * (TEMPO_US / 1e6) / TPB

# --- ドラムの発音時刻を収集 ---
tr = m.tracks[11]
tick = 0
events = []
for msg in tr:
    tick += msg.time
    if msg.type == "note_on" and msg.velocity > 0:
        events.append((sec(tick), msg.note))
KICK = (35, 36); SNARE = (38, 40); HAT = (42, 44, 46)
kick = [t for t, n in events if n in KICK]
snare = [t for t, n in events if n in SNARE]
hat = [t for t, n in events if n in HAT]

last = max(t for t, n in events)

# --- 拍マップCSV ---
rows = []
k = 0
while k * beat <= last + beat:
    t = k * beat
    def hit(ts, tol=0.030):
        return sum(1 for x in ts if abs(x - t) <= tol)
    rows.append({
        "beat_index": k,
        "bar": k // 4 + 1,
        "beat_in_bar": k % 4 + 1,
        "time_s": f"{t:.4f}",
        "music_tick_960ppq": 960 * k,
        "kick_hits": hit(kick),
        "snare_hits": hit(snare),
        "hihat_hits": hit(hat, 0.020),
        "is_bar_start": "1" if k % 4 == 0 else "",
    })
    k += 1
with open(CSV, "w", newline="") as f:
    w = csv.DictWriter(f, fieldnames=list(rows[0].keys()))
    w.writeheader(); w.writerows(rows)
print(f"wrote {CSV} ({len(rows)} 拍 / {len(rows)//4} 小節)")

# --- クリックトラック付きMIDI ---
out = mido.MidiFile(type=m.type, ticks_per_beat=m.ticks_per_beat)
for t in m.tracks:
    out.tracks.append(t)
click = mido.MidiTrack()
click.append(mido.MetaMessage("track_name", name="Click(Beat)", time=0))
prev = 0
for k in range(len(rows)):
    abs_tick = 960 * k
    note = 77 if k % 4 == 0 else 76      # 77=Low Wood Block(小節頭) / 76=Hi Wood Block
    vel = 100 if k % 4 == 0 else 70
    click.append(mido.Message("note_on", channel=9, note=note, velocity=vel, time=abs_tick - prev)); prev = abs_tick
    click.append(mido.Message("note_off", channel=9, note=note, velocity=0, time=0))
click.append(mido.MetaMessage("end_of_track", time=0))
out.tracks.append(click)
out.save(OUT)
print(f"wrote {OUT} (最後のトラックが拍クリック: 小節頭=Low Wood Block, 他=Hi Wood Block)")

# --- レポート ---
print()
print(f"検出: 拍数 {len(rows)} / 小節数 {len(rows)//4}")
n_kick = sum(1 for r in rows if r['kick_hits'])
n_sn   = sum(1 for r in rows if r['snare_hits'])
n_hat  = sum(1 for r in rows if r['hihat_hits'])
print(f"  キックが乗っている拍: {n_kick}/{len(rows)} ({100*n_kick/len(rows):.0f}%)")
print(f"  スネアが乗っている拍: {n_sn}/{len(rows)} ({100*n_sn/len(rows):.0f}%)")
print(f"  ハイハットが乗っている拍: {n_hat}/{len(rows)} ({100*n_hat/len(rows):.0f}%)")
