# midi_tempo_adjust

表記テンポが実際と違う MIDI（例: 表記120BPMだが本当は148BPM）を、
**音の位置（実時間）は変えずに**正しいBPMのMIDIへ直すためのプロジェクトです。
解析・変換のGUIアプリ（Rust）、検証用スクリプト（Python）、変換済みMIDIをまとめています。

## 中身

| パス | 内容 |
|---|---|
| `midi_head_align/` | **GUIアプリ本体（Rust）**。ドラッグ&ドロップ→BPM自動解析→ピアノロールで頭出し調整→書き出し。日本語/English/中文対応 |
| `convert_to_148bpm.py` | 変換スクリプト（Python/mido・アプリと同じ処理の検証用） |
| `make_beat_map.py` | 拍マップとクリックMIDIを作るスクリプト |
| `samples/`（`midi_head_align/samples/`） | 動作確認用の合成MIDI |

> 注: 元楽曲の MIDI および `beat_map.csv` は著作権の関係でリポジトリに含めていません。
> 自分の MIDI で試す場合は `convert_to_148bpm.py` の `SRC` を書き換えてください。

## 解析結果（参考例・元楽曲の解析値）

- 表記テンポ: **120.000 BPM**（PPQ 960 / 4774ノート / ドラム 2137音）
- 実際のテンポ: **148.002 BPM**（ドラムのオンセットから自動検出。他の候補 118.4 / 129.5 BPM とはっきり差）
- 拍頭オフセット: **+18.1 ms**（拍グリッドを前へずらす量）
- 音符の実時間は変換前後で **最大 0.21ms（0.5tick）** の誤差 → 演奏位置はそのまま
- 16分グリッドへの乗り: 平均 -0.5ms / **RMS 7.2ms**（元が10ms刻みで採譜されているためのジッタ）

### ドラムトラックの内訳（参考）

- ハイハット: 16分音符で全編安定（RMS 6ms）→ テンポ検出の主軸
- スネア: 4拍ごと（＝小節頭）に出現 → 小節線の指標
- キック: シンコペーション主体（拍上は34%のみ）→ 単独では拍検出に不向き

## 使い方（アプリ）

```sh
cd midi_head_align
cargo build --release
./target/release/midi_head_align            # GUI
open "MIDI頭出しツール.app"                  # Finderから起動する場合
```

詳しくは [`midi_head_align/README.md`](midi_head_align/README.md) を参照。

## ライセンス

MIT License（詳細は `LICENSE` を参照）。
