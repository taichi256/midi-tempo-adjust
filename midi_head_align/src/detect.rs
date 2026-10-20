//! ドラムのオンセットからテンポ(BPM)と拍グリッド位置を推定する。
//!
//! 方式:
//!  1. BPM候補ごとに「16分音符の櫛型テンプレート」を当てはめ、オンセットが
//!     どれだけ格子に乗るかを評価（拍>8分>16分 の重み + 90〜180BPMの弱い事前分布）
//!  2. 上位候補を細かく探索
//!  3. 勝ち残ったテンポで、位相を平均シフト法により ±数ms まで詰める

pub const BPM_MIN: f64 = 45.0;
pub const BPM_MAX: f64 = 220.0;

#[derive(Clone, Debug)]
pub struct Detection {
    pub bpm: f64,
    /// 拍グリッドの位置(ms)。ビートは phase_ms + k*拍 の位置にある。
    pub phase_ms: f64,
    pub score: f64,
    /// 上位候補 (bpm, score)
    pub candidates: Vec<(f64, f64)>,
    /// ドラムだけで解析したか
    pub used_drums: bool,
    pub onset_count: usize,
    /// 小節頭の候補（0..3 拍目）
    pub bar_phase_hint: usize,
}

const NB: usize = 128; // 拍内の位相ビン数
const PEAK_W: f64 = 0.10; // テンプレートの裾(拍単位)。±0.10拍まで許容
const COARSE_STEP: f64 = 0.10;
const FINE_STEP: f64 = 0.002;

/// 16分音符グリッドのテンプレート。delta は拍頭からの距離(拍, 0..1)
fn template(delta: f64) -> f64 {
    let d = if delta > 0.5 { delta - 1.0 } else { delta };
    let peaks: [(f64, f64); 4] = [(0.0, 1.0), (0.25, 0.45), (-0.25, 0.45), (0.5, 0.7)];
    let mut best = 0.0f64;
    for (p, w) in peaks {
        let dist = (d - p).abs();
        let k = (1.0 - dist / PEAK_W).max(0.0);
        if w * k > best {
            best = w * k;
        }
    }
    best
}

fn prior(bpm: f64) -> f64 {
    let x = (bpm / 125.0).ln() / 0.30;
    (-0.5 * x * x).exp()
}

/// 指定テンポ・位相での格子への平均適合度
fn score_at(bpm: f64, phase: f64, onsets: &[(f64, f64)], total_w: f64) -> f64 {
    let beat = 60.0 / bpm;
    let mut s = 0.0;
    for &(t, w) in onsets {
        let d = ((t - phase) / beat).rem_euclid(1.0);
        s += w * template(d);
    }
    s / total_w
}

/// 位相ヒストグラムから最良の位相とスコアを求める（粗い位相決め）
fn best_phase(bpm: f64, onsets: &[(f64, f64)], total_w: f64) -> (f64, f64) {
    let beat = 60.0 / bpm;
    let mut hist = [0.0f64; NB];
    for &(t, w) in onsets {
        let x = (t / beat).rem_euclid(1.0);
        let b = ((x * NB as f64) as usize).min(NB - 1);
        hist[b] += w;
    }
    let tpl: Vec<f64> = (0..NB).map(|d| template(d as f64 / NB as f64)).collect();
    let mut best = (-1.0f64, 0.0f64);
    for s in 0..NB {
        let mut sc = 0.0;
        for j in 0..NB {
            if hist[j] > 0.0 {
                let mut d = j as isize - s as isize;
                if d < 0 {
                    d += NB as isize;
                }
                sc += hist[j] * tpl[d as usize];
            }
        }
        if sc > best.0 {
            best = (sc, s as f64 / NB as f64 * beat);
        }
    }
    (best.1, best.0 / total_w)
}

/// 平均シフト法で位相を詰める（16分格子に合わせ込む）
fn refine_phase(bpm: f64, phase0: f64, onsets: &[(f64, f64)]) -> f64 {
    let beat = 60.0 / bpm;
    let step = beat / 4.0;
    let mut phase = phase0;
    for _ in 0..10 {
        let (mut sum, mut wsum) = (0.0f64, 0.0f64);
        for &(t, w) in onsets {
            let x = (t - phase) / step;
            let d = x - x.round();
            if d.abs() < 0.35 {
                sum += w * d;
                wsum += w;
            }
        }
        if wsum <= 0.0 {
            break;
        }
        let corr = sum / wsum * step;
        phase += corr;
        if corr.abs() < 1e-6 {
            break;
        }
    }
    // 位相は 0..拍 の範囲に正規化
    phase.rem_euclid(beat)
}

/// 指定BPMでの拍グリッド位置(ms)を求める（手入力テンポ用）
pub fn phase_for_bpm(onsets: &[(f64, f64)], bpm: f64) -> Option<f64> {
    if onsets.len() < 4 || bpm <= 0.0 {
        return None;
    }
    let total_w: f64 = onsets.iter().map(|x| x.1).sum();
    let (ph0, _) = best_phase(bpm, onsets, total_w);
    Some(refine_phase(bpm, ph0, onsets) * 1000.0)
}

pub fn detect(onsets: &[(f64, f64)], used_drums: bool) -> Option<Detection> {    if onsets.len() < 8 {
        return None;
    }
    let total_w: f64 = onsets.iter().map(|x| x.1).sum();
    if total_w <= 0.0 {
        return None;
    }

    // --- 粗探索 ---
    let mut coarse: Vec<(f64, f64)> = Vec::new(); // score, bpm
    let mut bpm = BPM_MIN;
    while bpm <= BPM_MAX {
        let (_, sc) = best_phase(bpm, onsets, total_w);
        coarse.push((sc * prior(bpm), bpm));
        bpm += COARSE_STEP;
    }
    coarse.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));

    // --- 精密化 ---
    let mut fine: Vec<(f64, f64)> = Vec::new();
    for (_, seed) in coarse.iter().take(5) {
        let mut b = (seed - 0.35).max(BPM_MIN);
        while b <= seed + 0.35 && b <= BPM_MAX {
            let (_, sc) = best_phase(b, onsets, total_w);
            fine.push((sc * prior(b), b));
            b += FINE_STEP;
        }
    }
    fine.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    if fine.is_empty() {
        return None;
    }

    let (_, bpm_best) = fine[0];
    // 位相を平均シフトで精密化
    let (ph0, _) = best_phase(bpm_best, onsets, total_w);
    let phase = refine_phase(bpm_best, ph0, onsets);
    let score = score_at(bpm_best, phase, onsets, total_w) * prior(bpm_best);

    // 候補を間引いて上位5件
    let mut candidates: Vec<(f64, f64)> = Vec::new();
    for (sc, b) in &fine {
        if candidates.iter().all(|(cb, _)| (cb / b - 1.0).abs() > 0.03) {
            candidates.push((*b, *sc));
        }
        if candidates.len() >= 5 {
            break;
        }
    }

    // 小節頭の候補: 拍ごとのオンセット量が多い拍を小節頭とみなす
    let beat_sec = 60.0 / bpm_best;
    let mut per_beat = [0.0f64; 4];
    for &(t, w) in onsets {
        let b = (t - phase) / beat_sec;
        if (b - b.round()).abs() < 0.20 {
            let idx = (b.round() as i64).rem_euclid(4) as usize;
            per_beat[idx] += w;
        }
    }
    let bar_phase_hint = per_beat
        .iter()
        .enumerate()
        .max_by(|a, b| a.1.partial_cmp(b.1).unwrap_or(std::cmp::Ordering::Equal))
        .map(|(i, _)| i)
        .unwrap_or(0);

    Some(Detection {
        bpm: bpm_best,
        phase_ms: phase * 1000.0,
        score,
        candidates,
        used_drums,
        onset_count: onsets.len(),
        bar_phase_hint,
    })
}
