// MIDI 頭出しツール / MIDI Head Align / MIDI 节拍对齐工具
//
//  - MIDIをドラッグ&ドロップ（または「開く」）
//  - ドラムトラックから本当のBPMと拍グリッド位置を解析（手入力も可）
//  - ピアノロールで頭出し位置を目視＋手動微調整
//  - 正しいBPMのMIDIとして書き出し
//  - UIは ja / en / zh（言語ファイルを追加・選択可能）
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
use std::path::{Path, PathBuf};

use detect::Detection;
use eframe::egui;
use egui::{Align2, Color32, FontId, Rect, Stroke, Vec2};
use i18n::Lang;
use midi::{ExportOptions, Loaded};

mod detect;
mod i18n;
mod midi;

const RULER_H: f32 = 20.0;
const ROW_H: f32 = 6.0;

fn main() -> eframe::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();

    // --- 言語の決定（引数 > 保存済み設定 > 日本語）---
    let arg_lang = take_arg(&args, "--lang").or_else(|| {
        // `--lang` が無ければ保存済み設定を使う
        i18n::load_saved_lang().map(|(c, _)| c)
    });
    let langs = i18n::discover();
    let lang = pick_lang(&langs, arg_lang.as_deref()).unwrap_or_default();

    // 引数が1つだけで、フラグでなければ「GUIでそのファイルを開く」
    let initial = if args.len() == 1 && !args[0].starts_with('-') {
        Some(PathBuf::from(&args[0]))
    } else {
        None
    };
    if !args.is_empty() && initial.is_none() {
        cli(&args, &lang);
        return Ok(());
    }

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1280.0, 820.0])
            .with_min_inner_size([900.0, 560.0])
            .with_title(lang.t("app.window_title")),
        ..Default::default()
    };
    eframe::run_native(
        "midi_head_align",
        options,
        Box::new(move |cc| {
            let mut app = App::new(cc);
            app.langs = langs;
            app.lang = lang;
            if let Some(p) = &initial {
                app.load_file(p);
            }
            Ok(Box::new(app))
        }),
    )
}

fn take_arg(args: &[String], name: &str) -> Option<String> {
    let i = args.iter().position(|a| a == name)?;
    args.get(i + 1).cloned()
}

/// コード(ja/en/zh)または言語ファイルのパスから言語を選ぶ
fn pick_lang(langs: &[Lang], spec: Option<&str>) -> Option<Lang> {
    let spec = spec?;
    let p = Path::new(spec);
    if p.extension().map(|x| x == "json").unwrap_or(false) && p.exists() {
        return Lang::load_file(p).ok();
    }
    if let Some(l) = langs.iter().find(|l| l.code == spec) {
        return Some(l.clone());
    }
    Lang::embedded(spec)
}

// ------------------------------------------------------------------
// アプリ本体
// ------------------------------------------------------------------
struct App {
    loaded: Option<Loaded>,
    detection: Option<Detection>,
    /// 解析に使うオンセット列(読み込み時に作る)
    onsets: Vec<(f64, f64)>,
    used_drums: bool,
    bpm: f64,
    bpm_text: String,
    /// 拍グリッドの位置(ms)。ビートは phase_ms + k*拍 にある。
    phase_ms: f64,
    /// 小節頭はどの拍か (0..3)
    bar_phase: usize,
    /// true: tick位置そのまま（再生速度が変わる）
    keep_ticks: bool,
    out_path: String,
    status: String,
    error: bool,
    px_per_beat: f32,
    show_16th: bool,
    visible: Vec<bool>,
    show_drums: bool,
    fit_zoom: bool,
    // --- i18n ---
    langs: Vec<Lang>,
    lang: Lang,
    title_set: String,
}

impl App {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        setup_fonts(&cc.egui_ctx);
        Self {
            loaded: None,
            detection: None,
            onsets: Vec::new(),
            used_drums: false,
            bpm: 120.0,
            bpm_text: "120".into(),
            phase_ms: 0.0,
            bar_phase: 0,
            keep_ticks: false,
            out_path: String::new(),
            status: String::new(),
            error: false,
            px_per_beat: 26.0,
            show_16th: true,
            visible: Vec::new(),
            show_drums: true,
            fit_zoom: true,
            langs: Vec::new(),
            lang: Lang::default(),
            title_set: String::new(),
        }
    }

    fn t(&self, key: &str) -> String {
        self.lang.t(key)
    }
    fn tf(&self, key: &str, args: &[&str]) -> String {
        self.lang.tf(key, args)
    }
    fn set_status(&mut self, key: &str, args: &[&str]) {
        self.status = self.lang.tf(key, args);
        self.error = false;
    }
    fn set_error(&mut self, key: &str, args: &[&str]) {
        self.status = self.lang.tf(key, args);
        self.error = true;
    }

    fn set_lang(&mut self, l: Lang) {
        self.lang = l;
        i18n::save_lang(&self.lang.code, self.lang.path.as_deref());
    }

    fn beat_sec(&self) -> f64 {
        60.0 / self.bpm.max(1.0)
    }

    /// 書き出し時のずらし量(秒)
    fn shift_sec(&self) -> f64 {
        if self.keep_ticks {
            0.0
        } else {
            self.phase_ms / 1000.0 + self.bar_phase as f64 * self.beat_sec()
        }
    }

    /// tick -> 書き出し後の「拍」位置
    fn tick_to_beats(&self, tick: u64, loaded: &Loaded) -> f64 {
        if self.keep_ticks {
            tick as f64 / loaded.ppq as f64
        } else {
            (loaded.time_at(tick) - self.shift_sec()).max(0.0) / self.beat_sec()
        }
    }

    fn load_file(&mut self, path: &Path) {
        match Loaded::load(path) {
            Ok(l) => {
                self.bpm = l.stated_bpm();
                self.bpm_text = trim_num(self.bpm, 4);
                self.out_path = default_out_path(path, self.bpm);
                self.visible = vec![true; l.tracks.len()];
                self.fit_zoom = true;
                self.phase_ms = 0.0;
                self.bar_phase = 0;
                let msg = self.tf(
                    "status.loaded",
                    &[
                        &path.file_name().unwrap_or_default().to_string_lossy(),
                        &format!(
                            "{:.1}",
                            std::fs::metadata(path).map(|m| m.len() as f64 / 1024.0).unwrap_or(0.0)
                        ),
                        &format!("{:.2}", l.stated_bpm()),
                        &l.ppq.to_string(),
                        &l.notes.len().to_string(),
                        &l.drum_notes().to_string(),
                    ],
                );
                self.error = false;
                self.status = msg;
                self.loaded = Some(l);
                let drums = self.loaded.as_ref().map(|l| l.has_drums()).unwrap_or(false);
                self.used_drums = drums;
                self.onsets = self
                    .loaded
                    .as_ref()
                    .map(|l| midi::collect_onsets(l, drums))
                    .unwrap_or_default();
                self.auto_detect();
            }
            Err(e) => self.set_error("status.load_error", &[&e]),
        }
    }

    fn auto_detect(&mut self) {
        let drums = self.used_drums;
        match detect::detect(&self.onsets, drums) {
            Some(d) => {
                self.bpm = d.bpm;
                self.bpm_text = format!("{:.3}", d.bpm);
                self.phase_ms = d.phase_ms;
                self.bar_phase = 0;
                let src = if d.used_drums { self.t("status.src_drums") } else { self.t("status.src_all") };
                let msg = self.tf(
                    "status.detected",
                    &[
                        &src,
                        &format!("{:.3}", d.bpm),
                        &format!("{:+.1}", d.phase_ms),
                        &format!("{:.2}", d.score),
                        &d.onset_count.to_string(),
                    ],
                );
                self.status = msg;
                self.error = false;
                if let Some(p) = &self.loaded {
                    self.out_path = default_out_path(&p.path, d.bpm);
                }
                self.detection = Some(d);
            }
            None => {
                self.set_error("status.detect_failed", &[]);
                self.detection = None;
            }
        }
    }

    /// 先頭のオンセットを小節頭に合わせる
    fn align_first_onset(&mut self) {
        let Some(first) = self.onsets.first().map(|x| x.0) else { return };
        let beat = self.beat_sec();
        // 先頭の発音がちょうど小節頭(beat 0)に来るように格子位置を決める
        self.phase_ms = (first.rem_euclid(beat)) * 1000.0;
        self.set_status("status.align_first_onset", &[&format!("{first:.3}s")]);
    }

    /// 最初のドラム発音を拍頭に合わせる
    fn align_first_drum(&mut self) {
        let Some(first) = self.onsets.first().map(|x| x.0) else { return };
        let beat = self.beat_sec();
        self.phase_ms = first.rem_euclid(beat) * 1000.0;
        self.set_status("status.align_first_drum", &[&format!("{first:.3}s")]);
    }

    /// 現在のBPMで拍グリッド位置を求め直す（手入力テンポ用）
    fn detect_phase_for_bpm(&mut self) {
        match detect::phase_for_bpm(&self.onsets, self.bpm) {
            Some(ms) => {
                self.phase_ms = ms;
                self.set_status(
                    "status.phase_for_bpm",
                    &[&format!("{:.3}", self.bpm), &format!("{ms:+.1}")],
                );
            }
            None => self.set_error("status.phase_failed", &[]),
        }
    }

    fn do_export(&mut self) {
        let Some(l) = &self.loaded else {
            self.set_error("status.need_file", &[]);
            return;
        };
        if self.bpm <= 0.0 {
            self.set_error("status.bad_bpm", &[]);
            return;
        }
        let out = PathBuf::from(self.out_path.trim());
        let opts =
            ExportOptions { bpm: self.bpm, shift_sec: self.shift_sec(), keep_ticks: self.keep_ticks };
        let res = midi::export(l, &opts, &out);
        match res {
            Ok(()) => self.set_status("status.exported", &[&out.display().to_string()]),
            Err(e) => self.set_error("status.export_failed", &[&e]),
        }
    }
}

fn default_out_path(input: &Path, bpm: f64) -> String {
    let stem = input.file_stem().unwrap_or_default().to_string_lossy().to_string();
    let name = format!("{}_{}BPM.mid", stem, trim_num(bpm, 3));
    input.with_file_name(name).to_string_lossy().to_string()
}

fn trim_num(v: f64, digits: usize) -> String {
    let s = format!("{v:.digits$}");
    let s = s.trim_end_matches('0').trim_end_matches('.').to_string();
    if s.is_empty() { "0".into() } else { s }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        // --- ウィンドウタイトルを言語に合わせる ---
        let want = self.t("app.window_title");
        if want != self.title_set {
            self.title_set = want.clone();
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Title(want));
        }

        // --- ドラッグ&ドロップ ---
        let dropped: Vec<PathBuf> = ui
            .ctx()
            .input(|i| i.raw.dropped_files.iter().map(|f| f.path().to_path_buf()).collect());
        if let Some(p) = dropped.first() {
            self.load_file(p);
        }

        // --- 上部バー ---
        egui::Panel::top("top").show(ui, |ui| {
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.heading(self.t("app.title"));
                ui.separator();
                if ui.button(self.t("btn.open")).clicked() {
                    if let Some(p) =
                        rfd::FileDialog::new().add_filter("MIDI", &["mid", "midi"]).pick_file()
                    {
                        self.load_file(&p);
                    }
                }
                if ui.add_enabled(self.loaded.is_some(), egui::Button::new(self.t("btn.reanalyze"))).clicked() {
                    self.auto_detect();
                }
                ui.separator();
                match &self.loaded {
                    Some(l) => {
                        let info = self.tf(
                            "top.file_info",
                            &[
                                &l.path.file_name().unwrap_or_default().to_string_lossy(),
                                &format!("{:.2}", l.stated_bpm()),
                                &l.ppq.to_string(),
                                &l.notes.len().to_string(),
                                &l.tracks.len().to_string(),
                            ],
                        );
                        ui.label(info);
                        if l.tempo_changes() > 0 {
                            let w = self.tf("top.tempo_warn", &[&l.tempo_changes().to_string()]);
                            ui.colored_label(Color32::from_rgb(220, 160, 0), w);
                        }
                    }
                    None => {
                        ui.label(self.t("top.drop_hint"));
                    }
                }

                // --- 言語選択（右寄せ）---
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    egui::ComboBox::from_id_salt("lang_select")
                        .selected_text(self.lang.label.clone())
                        .width(150.0)
                        .show_ui(ui, |ui| {
                            let langs = self.langs.clone();
                            for l in &langs {
                                if ui.selectable_label(l.code == self.lang.code, &l.label).clicked() {
                                    self.set_lang(l.clone());
                                }
                            }
                        });
                    if ui.small_button("…").on_hover_text(self.t("lang.load_file")).clicked() {
                        if let Some(p) = rfd::FileDialog::new()
                            .add_filter("Language", &["json"])
                            .pick_file()
                        {
                            match Lang::load_file(&p) {
                                Ok(l) => {
                                    let label = l.label.clone();
                                    if !self.langs.iter().any(|x| x.code == l.code) {
                                        self.langs.push(l.clone());
                                    }
                                    self.set_lang(l);
                                    self.set_status("lang.loaded", &[&label]);
                                }
                                Err(e) => self.set_error("lang.load_error", &[&e]),
                            }
                        }
                    }
                    ui.label(self.t("lang.label"));
                });
            });
            ui.add_space(4.0);
        });

        // --- 下部ステータス ---
        egui::Panel::bottom("status").show(ui, |ui| {
            ui.add_space(3.0);
            let color = if self.error { Color32::from_rgb(230, 90, 90) } else { Color32::GRAY };
            ui.colored_label(color, self.status.clone());
            ui.add_space(3.0);
        });

        // --- 左パネル ---
        egui::Panel::left("side").resizable(true).default_size(360.0).show(ui, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                self.side_panel(ui);
            });
        });

        // --- 中央: ピアノロール ---
        egui::CentralPanel::default().show(ui, |ui| {
            if self.loaded.is_none() {
                ui.centered_and_justified(|ui| {
                    ui.label(self.t("view.drop_hint_center"));
                });
                return;
            }
            ui.horizontal(|ui| {
                let bfit = self.t("btn.fit");
                if ui.button(bfit).clicked() {
                    self.fit_zoom = true;
                }
                let zl = self.t("view.zoom_label");
                ui.add(egui::Slider::new(&mut self.px_per_beat, 2.0..=200.0).text(zl));
                ui.separator();
                ui.label(self.t("view.help"));
            });
            egui::ScrollArea::both().auto_shrink([false, false]).show(ui, |ui| {
                self.roll(ui);
            });
        });
    }
}

impl App {
    fn side_panel(&mut self, ui: &mut egui::Ui) {
        let has_file = self.loaded.is_some();
        let gap = ui.spacing().item_spacing.x;
        let row_h = ui.spacing().interact_size.y.max(22.0);

        // ================= 1. テンポ (BPM) =================
        let title = self.t("sec.bpm");
        let unit = self.t("bpm.unit");
        let manual = self.t("bpm.manual");
        section(ui, &title, |ui| {
            // --- BPM 入力行 ---
            ui.horizontal(|ui| {
                let r = ui.add(
                    egui::DragValue::new(&mut self.bpm)
                        .speed(0.01)
                        .range(20.0..=400.0)
                        .fixed_decimals(3),
                );
                if r.changed() {
                    self.bpm_text = format!("{:.3}", self.bpm);
                }
                ui.label(&unit);
                if ui.add_sized([34.0, row_h], egui::Button::new("×2")).clicked() {
                    self.bpm = (self.bpm * 2.0).min(400.0);
                    self.bpm_text = format!("{:.3}", self.bpm);
                }
                if ui.add_sized([34.0, row_h], egui::Button::new("÷2")).clicked() {
                    self.bpm = (self.bpm / 2.0).max(20.0);
                    self.bpm_text = format!("{:.3}", self.bpm);
                }
            });
            ui.add_space(4.0);

            // --- 手入力 ---
            ui.horizontal(|ui| {
                ui.label(&manual);
                let w = ui.available_width();
                let r = ui.add_sized([w, row_h], egui::TextEdit::singleline(&mut self.bpm_text));
                if r.lost_focus() || r.changed() {
                    if let Ok(v) = self.bpm_text.trim().parse::<f64>() {
                        if v > 0.0 {
                            self.bpm = v;
                        }
                    }
                }
            });

            // --- 解析結果 ---
            if let Some(d) = &self.detection {
                ui.add_space(6.0);
                let score = self.tf(
                    "bpm.score",
                    &[&format!("{:.2}", d.score), &d.onset_count.to_string()],
                );
                ui.label(egui::RichText::new(score).weak().small());
                let clabel = self.t("bpm.candidates");
                ui.label(egui::RichText::new(clabel).weak().small());
                let cands = d.candidates.clone();
                let current = self.bpm;
                egui::ScrollArea::vertical()
                    .max_height(120.0)
                    .id_salt("cands")
                    .show(ui, |ui| {
                        for (cb, sc) in cands {
                            let label = self
                                .tf("bpm.candidate_item", &[&format!("{cb:.3}"), &format!("{sc:.2}")]);
                            if ui.selectable_label((current - cb).abs() < 0.001, label).clicked() {
                                self.bpm = cb;
                                self.bpm_text = format!("{:.3}", cb);
                            }
                        }
                    });
            }
        });

        // ================= 2. 頭出し =================
        let title = self.t("sec.head");
        let desc = self.t("head.desc");
        let off_label = self.t("head.offset");
        let auto = self.t("head.auto");
        let recalc = self.t("head.recalc");
        let first_drum = self.t("head.first_drum");
        let first_note = self.t("head.first_note");
        let bar_beat = self.t("head.bar_beat");
        let beat_ms = 60_000.0 / self.bpm.max(1.0);
        let hint = self
            .detection
            .as_ref()
            .map(|d| self.tf("head.bar_hint", &[&(d.bar_phase_hint + 1).to_string()]));
        let shift_info =
            self.tf("head.shift_info", &[&format!("{:.1}", self.shift_sec() * 1000.0)]);
        let phase_now = self.phase_ms;
        section(ui, &title, |ui| {
            ui.add_enabled_ui(has_file, |ui| {
                ui.label(egui::RichText::new(&desc).weak().small());
                ui.add_space(4.0);

                // オフセット（現在値 + スライダー）
                ui.horizontal(|ui| {
                    ui.label(&off_label);
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.monospace(format!("{phase_now:+.1} ms"));
                    });
                });
                ui.add(
                    egui::Slider::new(&mut self.phase_ms, -beat_ms..=beat_ms)
                        .show_value(false)
                        .clamping(egui::SliderClamping::Always),
                );
                ui.add_space(3.0);

                // 微調整
                let w5 = ((ui.available_width() - gap * 4.0) / 5.0).max(20.0);
                ui.horizontal(|ui| {
                    for (label, d) in [("-10ms", -10.0), ("-1ms", -1.0), ("+1ms", 1.0), ("+10ms", 10.0), ("0", 0.0)] {
                        if ui.add_sized([w5, row_h], egui::Button::new(label)).clicked() {
                            if d == 0.0 {
                                self.phase_ms = 0.0;
                            } else {
                                self.phase_ms += d;
                            }
                        }
                    }
                });
                ui.add_space(4.0);

                // 自動合わせ（縦に並べて読みやすく）
                let wf = ui.available_width();
                if ui.add_sized([wf, row_h], egui::Button::new(&auto)).clicked() {
                    if let Some(d) = &self.detection {
                        self.phase_ms = d.phase_ms;
                    }
                }
                if ui.add_sized([wf, row_h], egui::Button::new(&recalc)).clicked() {
                    self.detect_phase_for_bpm();
                }
                if ui.add_sized([wf, row_h], egui::Button::new(&first_drum)).clicked() {
                    self.align_first_drum();
                }
                if ui.add_sized([wf, row_h], egui::Button::new(&first_note)).clicked() {
                    self.align_first_onset();
                }
                ui.add_space(4.0);

                // 小節頭の拍
                ui.horizontal(|ui| {
                    ui.label(&bar_beat);
                    for i in 0..4 {
                        if ui.add_sized([30.0, row_h], egui::Button::new(format!("{}", i + 1)))
                            .clicked()
                        {
                            self.bar_phase = i;
                        }
                    }
                });
                let mut hint_txt = String::new();
                if let Some(h) = &hint {
                    hint_txt.push_str(h);
                }
                if !hint_txt.is_empty() {
                    ui.label(egui::RichText::new(hint_txt).weak().small());
                }
                ui.add_space(3.0);
                ui.label(egui::RichText::new(shift_info).weak().small());
            });
        });

        // ================= 3. 書き出し =================
        let title = self.t("sec.export");
        let keep_ticks = self.t("exp.keep_ticks");
        let keep_note = self.t("exp.keep_ticks_note");
        let out_label = self.t("exp.out");
        let save_as = self.t("btn.save_as");
        let export = self.t("btn.export");
        let accent = ui.visuals().selection.bg_fill;
        section(ui, &title, |ui| {
            ui.add_enabled_ui(has_file, |ui| {
                ui.checkbox(&mut self.keep_ticks, &keep_ticks);
                if !self.keep_ticks {
                    ui.label(egui::RichText::new(&keep_note).weak().small());
                }
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.label(&out_label);
                    let w = ui.available_width();
                    ui.add_sized([w, row_h], egui::TextEdit::singleline(&mut self.out_path));
                });
                ui.add_space(4.0);
                let w2 = ((ui.available_width() - gap) / 2.0).max(40.0);
                ui.horizontal(|ui| {
                    if ui.add_sized([w2, row_h], egui::Button::new(&save_as)).clicked() {
                        if let Some(p) = rfd::FileDialog::new()
                            .add_filter("MIDI", &["mid", "midi"])
                            .set_file_name(
                                Path::new(&self.out_path)
                                    .file_name()
                                    .map(|s| s.to_string_lossy().to_string())
                                    .unwrap_or_else(|| "out.mid".into()),
                            )
                            .save_file()
                        {
                            self.out_path = p.to_string_lossy().to_string();
                        }
                    }
                    if ui
                        .add_sized(
                            [w2, row_h],
                            egui::Button::new(egui::RichText::new(&export).strong()).fill(accent),
                        )
                        .clicked()
                    {
                        self.do_export();
                    }
                });
            });
        });

        // ================= 4. 表示 =================
        let title = self.t("sec.view");
        let zoom = self.t("view.zoom_label");
        let g16 = self.t("view.grid16");
        let sd = self.t("view.drums");
        let tl = self.t("view.tracks");
        let tdef = self.t("view.track_default");
        section(ui, &title, |ui| {
            ui.horizontal(|ui| {
                ui.label(&zoom);
                let w = ui.available_width();
                ui.add_sized(
                    [w, row_h],
                    egui::Slider::new(&mut self.px_per_beat, 2.0..=200.0)
                        .show_value(true)
                        .integer(),
                );
            });
            ui.add_space(2.0);
            ui.horizontal(|ui| {
                ui.checkbox(&mut self.show_16th, &g16);
                ui.checkbox(&mut self.show_drums, &sd);
            });
            ui.add_space(4.0);
            ui.label(egui::RichText::new(&tl).weak().small());
            if let Some(l) = &self.loaded {
                let names: Vec<String> = l
                    .tracks
                    .iter()
                    .enumerate()
                    .map(|(i, t)| {
                        if t.name.is_empty() {
                            tdef.replace("{0}", &(i + 1).to_string())
                        } else {
                            t.name.clone()
                        }
                    })
                    .collect();
                let drum_flags: Vec<bool> = l.tracks.iter().map(|t| t.is_drum).collect();
                let counts: Vec<usize> = l.tracks.iter().map(|t| t.notes).collect();
                egui::ScrollArea::vertical()
                    .max_height(180.0)
                    .id_salt("tracks")
                    .show(ui, |ui| {
                        for (i, name) in names.iter().enumerate() {
                            ui.horizontal(|ui| {
                                if let Some(v) = self.visible.get_mut(i) {
                                    ui.checkbox(v, "");
                                }
                                ui.colored_label(track_color(i, drum_flags[i]), "■");
                                ui.label(name);
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        ui.label(egui::RichText::new(format!("{}", counts[i])).weak().small());
                                    },
                                );
                            });
                        }
                    });
            }
        });
        ui.add_space(10.0);
    }

    // --------------------------------------------------------------
    // ピアノロール
    // --------------------------------------------------------------
    fn roll(&mut self, ui: &mut egui::Ui) {
        let Some(l) = &self.loaded else { return };
        let beat_sec = self.beat_sec();
        let total_beats = ((l.last_time() / beat_sec) + 2.0).max(8.0);
        if self.fit_zoom {
            let avail = (ui.available_width() - 24.0).max(120.0);
            self.px_per_beat = (avail / total_beats as f32).clamp(2.0, 200.0);
            self.fit_zoom = false;
        }

        // 音域
        let mut lo = 127u8;
        let mut hi = 0u8;
        for n in &l.notes {
            if !self.visible.get(n.track).copied().unwrap_or(true) {
                continue;
            }
            if n.is_drum && !self.show_drums {
                continue;
            }
            lo = lo.min(n.key);
            hi = hi.max(n.key);
        }
        if lo > hi {
            lo = 60;
            hi = 72;
        }
        let pitch_lo = lo.saturating_sub(2).max(0);
        let pitch_hi = (hi + 2).min(127);

        let width = (total_beats as f32 * self.px_per_beat) + 60.0;
        let height = RULER_H + (pitch_hi - pitch_lo + 1) as f32 * ROW_H + 8.0;
        let (rect, resp) = ui.allocate_exact_size(Vec2::new(width, height), egui::Sense::hover());
        let painter = ui.painter_at(rect);
        let clip = ui.clip_rect();
        let px = self.px_per_beat;

        painter.rect_filled(rect, egui::CornerRadius::ZERO, Color32::from_rgb(24, 25, 30));

        let x0 = ((clip.left() - rect.left()) / px).floor().max(0.0) as i64;
        let x1 = (((clip.right() - rect.left()) / px).ceil() as i64).min(total_beats as i64 + 1);

        for bar in (x0 / 4)..=(x1 / 4) {
            if bar % 2 == 0 {
                let rx = rect.left() + bar as f32 * 4.0 * px;
                let r = Rect::from_min_size(
                    egui::pos2(rx, rect.top() + RULER_H),
                    Vec2::new(4.0 * px, height - RULER_H),
                );
                painter.rect_filled(r, egui::CornerRadius::ZERO, Color32::from_rgb(30, 31, 38));
            }
        }

        for b in x0..=x1 {
            let x = rect.left() + b as f32 * px;
            let (color, w) = if b % 4 == 0 {
                (Color32::from_rgb(120, 125, 140), 1.5)
            } else {
                (Color32::from_rgb(70, 72, 82), 1.0)
            };
            painter.line_segment(
                [egui::pos2(x, rect.top()), egui::pos2(x, rect.bottom())],
                Stroke::new(w, color),
            );
            if self.show_16th && (b % 4 != 0) {
                for s in 1..4 {
                    let xs = x + s as f32 * px / 4.0;
                    painter.line_segment(
                        [egui::pos2(xs, rect.top() + RULER_H), egui::pos2(xs, rect.bottom())],
                        Stroke::new(1.0, Color32::from_rgb(44, 45, 54)),
                    );
                }
            }
        }
        for bar in (x0 / 4)..=(x1 / 4) {
            let x = rect.left() + (bar * 4) as f32 * px;
            painter.text(
                egui::pos2(x + 3.0, rect.top() + 2.0),
                Align2::LEFT_TOP,
                format!("{}", bar + 1),
                FontId::proportional(11.0),
                Color32::from_rgb(170, 175, 190),
            );
        }

        for k in pitch_lo..=pitch_hi {
            if k % 12 == 0 {
                let y = rect.top() + RULER_H + (pitch_hi - k) as f32 * ROW_H;
                painter.line_segment(
                    [egui::pos2(rect.left(), y), egui::pos2(rect.right(), y)],
                    Stroke::new(1.0, Color32::from_rgb(50, 52, 62)),
                );
            }
        }

        let mut drawn = 0usize;
        for n in &l.notes {
            if !self.visible.get(n.track).copied().unwrap_or(true) {
                continue;
            }
            if n.is_drum && !self.show_drums {
                continue;
            }
            if n.key < pitch_lo || n.key > pitch_hi {
                continue;
            }
            let b0 = self.tick_to_beats(n.start, l);
            let b1 = self.tick_to_beats(n.end, l).max(b0 + 0.02);
            let xa = rect.left() + b0 as f32 * px;
            let xb = rect.left() + b1 as f32 * px;
            if xb < clip.left() - 4.0 || xa > clip.right() + 4.0 {
                continue;
            }
            let y = rect.top() + RULER_H + (pitch_hi - n.key) as f32 * ROW_H;
            let r = Rect::from_min_size(egui::pos2(xa, y), Vec2::new((xb - xa).max(1.5), ROW_H - 1.0));
            let color = if n.is_drum {
                Color32::from_rgb(235, 130, 60)
            } else {
                track_color(n.track, false)
            };
            let a = 90 + (n.velocity as u32 * 165 / 127) as u8;
            painter.rect_filled(r, egui::CornerRadius::same(1), color.gamma_multiply(a as f32 / 255.0));
            drawn += 1;
        }

        painter.line_segment(
            [egui::pos2(rect.left(), rect.top()), egui::pos2(rect.left(), rect.bottom())],
            Stroke::new(2.0, Color32::from_rgb(90, 200, 130)),
        );

        if resp.hovered() {
            if let Some(p) = ui.ctx().pointer_hover_pos() {
                let beat = ((p.x - rect.left()) / px) as f64;
                let bar = (beat / 4.0).floor() as i64 + 1;
                let b = beat % 4.0;
                let txt = self.tf(
                    "view.hover",
                    &[&bar.to_string(), &format!("{:.3}", b + 1.0), &drawn.to_string()],
                );
                resp.clone().on_hover_text(txt);
            }
        }
    }
}

/// 左パネルのセクション枠（見出し付きのグループ）
fn section<R>(ui: &mut egui::Ui, title: &str, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    ui.add_space(6.0);
    egui::Frame::group(ui.style())
        .inner_margin(egui::Margin::symmetric(10, 8))
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.label(egui::RichText::new(title).strong().size(13.0));
            ui.add_space(6.0);
            add(ui)
        })
        .inner
}

fn track_color(i: usize, drum: bool) -> Color32 {
    if drum {
        return Color32::from_rgb(235, 130, 60);
    }
    const PALETTE: [(u8, u8, u8); 12] = [
        (110, 170, 255),
        (120, 220, 160),
        (255, 200, 90),
        (230, 120, 200),
        (140, 200, 255),
        (200, 160, 255),
        (255, 160, 140),
        (160, 255, 220),
        (240, 230, 120),
        (180, 180, 255),
        (120, 240, 200),
        (255, 140, 120),
    ];
    let (r, g, b) = PALETTE[i % PALETTE.len()];
    Color32::from_rgb(r, g, b)
}

fn setup_fonts(ctx: &egui::Context) {
    // 日本語/中国語が表示できるフォントを探して読み込む（無ければ既定フォントのまま）
    let candidates = [
        "/Library/Fonts/Arial Unicode.ttf",
        "/System/Library/Fonts/Supplemental/Arial Unicode.ttf",
        "/System/Library/Fonts/Hiragino Sans GB.ttc",
        "/System/Library/Fonts/PingFang.ttc",
        "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
        "/usr/share/fonts/truetype/wqy/wqy-zenhei.ttc",
        "C:/Windows/Fonts/meiryo.ttc",
        "C:/Windows/Fonts/msyh.ttc",
    ];
    for path in candidates {
        if let Ok(bytes) = std::fs::read(path) {
            let mut fonts = egui::FontDefinitions::default();
            fonts.font_data.insert("jp".to_owned(), egui::FontData::from_owned(bytes).into());
            fonts
                .families
                .entry(egui::FontFamily::Proportional)
                .or_default()
                .insert(0, "jp".to_owned());
            fonts
                .families
                .entry(egui::FontFamily::Monospace)
                .or_default()
                .push("jp".to_owned());
            ctx.set_fonts(fonts);
            return;
        }
    }
}

// ------------------------------------------------------------------
// CLI（解析/書き出しのみ。GUI無しで使える＆テスト用）
// ------------------------------------------------------------------
fn cli(args: &[String], lang: &Lang) {
    let t = |k: &str| lang.t(k).to_string();
    let tf = |k: &str, a: &[&str]| lang.tf(k, a);

    let mut detect_path: Option<String> = None;
    let mut input: Option<String> = None;
    let mut output: Option<String> = None;
    let mut bpm: Option<f64> = None;
    let mut offset_ms = 0.0f64;
    let mut bar_phase = 0usize;
    let mut keep_ticks = false;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--detect" => {
                i += 1;
                detect_path = args.get(i).cloned();
            }
            "--export" => {
                i += 1;
                input = args.get(i).cloned();
                i += 1;
                output = args.get(i).cloned();
            }
            "--bpm" => {
                i += 1;
                bpm = args.get(i).and_then(|s| s.parse().ok());
            }
            "--offset-ms" => {
                i += 1;
                offset_ms = args.get(i).and_then(|s| s.parse().ok()).unwrap_or(0.0);
            }
            "--bar-phase" => {
                i += 1;
                bar_phase = args.get(i).and_then(|s| s.parse().ok()).unwrap_or(0);
            }
            "--lang" => {
                i += 1; // 言語は main で解決済み
            }
            "--keep-ticks" => keep_ticks = true,
            "-h" | "--help" => {
                println!("{}", t("cli.help"));
                return;
            }
            other => {
                eprintln!("{}", tf("cli.unknown_arg", &[other]));
                println!("{}", t("cli.help"));
                return;
            }
        }
        i += 1;
    }

    if let Some(p) = detect_path {
        match Loaded::load(Path::new(&p)) {
            Ok(l) => {
                let drums = l.has_drums();
                let onsets = midi::collect_onsets(&l, drums);
                println!("{}", tf("cli.file", &[&l.path.file_name().unwrap_or_default().to_string_lossy()]));
                println!(
                    "{}",
                    tf(
                        "cli.stated",
                        &[&format!("{:.4}", l.stated_bpm()), &l.ppq.to_string(), &l.notes.len().to_string()]
                    )
                );
                println!(
                    "{}",
                    tf("cli.drums", &[&l.drum_notes().to_string(), &onsets.len().to_string()])
                );
                match detect::detect(&onsets, drums) {
                    Some(d) => {
                        let src = if d.used_drums { t("cli.source_drums") } else { t("cli.source_all") };
                        println!("{}", tf("cli.detected", &[&format!("{:.4}", d.bpm), &src]));
                        println!("{}", tf("cli.phase", &[&format!("{:+.2}", d.phase_ms)]));
                        println!("{}", tf("cli.score", &[&format!("{:.3}", d.score)]));
                        println!("{}", tf("cli.bar_hint", &[&(d.bar_phase_hint + 1).to_string()]));
                        println!("{}", t("cli.candidates"));
                        for (b, s) in &d.candidates {
                            println!("{}", tf("cli.cand_item", &[&format!("{b:.3}"), &format!("{s:.3}")]));
                        }
                    }
                    None => println!("{}", t("cli.not_detected")),
                }
            }
            Err(e) => eprintln!("{}", tf("cli.error", &[&e])),
        }
        return;
    }

    if let (Some(input), Some(output), Some(bpm)) = (input, output, bpm) {
        match Loaded::load(Path::new(&input)) {
            Ok(l) => {
                let shift = offset_ms / 1000.0 + bar_phase as f64 * (60.0 / bpm);
                let opts = ExportOptions { bpm, shift_sec: shift, keep_ticks };
                match midi::export(&l, &opts, Path::new(&output)) {
                    Ok(()) => println!(
                        "{}",
                        tf(
                            "cli.exported",
                            &[
                                &output,
                                &format!("{bpm:.4}"),
                                &format!("{:.2}", shift * 1000.0),
                                &if keep_ticks { t("cli.mode_keep") } else { t("cli.mode_time") },
                            ]
                        )
                    ),
                    Err(e) => eprintln!("{}", tf("cli.error", &[&e])),
                }
            }
            Err(e) => eprintln!("{}", tf("cli.error", &[&e])),
        }
        return;
    }

    println!("{}", t("cli.help"));
}
