#!/usr/bin/env python3
"""AI採譜MIDI(120BPM表記)を実テンポ148BPMへ変換する。

- 音の実時間(=秒)を維持しつつ、tick位置を148BPMグリッドへスケールする
  (DAWで148BPMの小節/拍グリッドに音符が乗る)
- 拍グリッドの位相ズレ(解析値 16.2ms)を除去して拍頭を揃える
- notes/CC/メタイベントすべてのデルタタイムを同じ変換で処理
"""
import mido
import sys

# 例: 自分の MIDI に合わせてファイル名を書き換えてください
SRC = "input.mid"
DST = "output_148bpm.mid"

SRC_BPM = 120.0
DST_BPM = 148.0
PHASE_SHIFT_S = 0.0162  # 拍グリッドに対するズレ(音符が遅れている分) → 前へ詰める

mid = mido.MidiFile(SRC)
ppq = mid.ticks_per_beat  # 960

# 元: tempo=120BPM, ppq → 1tick = 60/(120*ppq) 秒
src_tick_s = 60.0 / (SRC_BPM * ppq)
# 新: tempo=148BPM, ppq → 1tick = 60/(148*ppq) 秒
dst_tick_s = 60.0 / (DST_BPM * ppq)

def convert_tick(t_old: int) -> int:
    """元tick → 実時間(位相補正込み) → 新tick"""
    t_sec = t_old * src_tick_s - PHASE_SHIFT_S
    return int(round(t_sec / dst_tick_s))

out = mido.MidiFile(type=mid.type, ticks_per_beat=ppq)

for track in mid.tracks:
    nt = mido.MidiTrack()
    abs_old = 0
    prev_new = 0
    for msg in track:
        abs_old += msg.time
        if msg.type == "set_tempo":
            # テンポは新規に設定し直す
            msg = mido.MetaMessage("set_tempo", tempo=int(round(60_000_000 / DST_BPM)), time=0)
            new_abs = 0
        else:
            new_abs = max(0, convert_tick(abs_old))
        new_abs = max(new_abs, prev_new)  # デルタが負にならないよう保険
        nt.append(msg.copy(time=new_abs - prev_new))
        prev_new = new_abs
    out.tracks.append(nt)

out.save(DST)
print(f"wrote {DST}")
print(f"  src: {SRC_BPM}BPM ppq={ppq}  ->  dst: {DST_BPM}BPM ppq={ppq}")
print(f"  tempo meta = {int(round(60_000_000/DST_BPM))} us/beat")
print(f"  tick scale = {dst_tick_s and (src_tick_s/dst_tick_s):.6f} (=148/120)")
print(f"  phase shift = -{PHASE_SHIFT_S*1000:.1f} ms (拍グリッドの頭をtick0へ)")
