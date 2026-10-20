#!/bin/bash
# ダブルクリックで起動（未ビルドならビルドします）
cd "$(dirname "$0")" || exit 1
if [ ! -x target/release/midi_head_align ]; then
  echo "初回ビルド中です（数分かかります）..."
  cargo build --release || { echo "ビルド失敗"; read -r -p "Enterで閉じます"; exit 1; }
fi
exec ./target/release/midi_head_align
