#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod clock;
mod feed;
mod pomodoro;

use std::sync::mpsc::{Receiver, Sender};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime};

use chrono::{Datelike, Timelike};

use clock::{beijing_now, beijing_offset, format_quote_time, gaokao, market_status, weekday_zh, Gaokao};
use eframe::egui::{
    self, Align, Align2, Button, CentralPanel, Color32, CornerRadius, CursorIcon, FontData,
    FontDefinitions,
    FontFamily, FontId, Frame, Key, Layout, Margin, Mesh, Pos2, Rect, RichText, UiBuilder,
    ScrollArea, Sense, Shape, Stroke, TextStyle, Vec2, ViewportCommand, WindowLevel,
};
use feed::{spawn_feed, Bar, Feed, Quote};
use pomodoro::{format_remaining, NoteStatus, Pomodoro};

const BG: Color32 = Color32::from_rgb(18, 16, 14);
const CARD: Color32 = Color32::from_rgb(34, 29, 24);
/// 指针离开后的整窗不透明度。悬停时渐变到完全不透明。
const WINDOW_ALPHA: u8 = 176;
/// 透明度渐变的时间常数，大约三分之一秒走到大部分路程。
const ALPHA_FADE_TAU: f32 = 0.16;
/// 指针坐标偶发丢一帧时，不要立刻开始变透，否则悬停会来回闪。
const ALPHA_LEAVE_DELAY: Duration = Duration::from_millis(140);
const LINE: Color32 = Color32::from_rgb(62, 52, 42);
const CREAM: Color32 = Color32::from_rgb(243, 234, 216);
const MUTED: Color32 = Color32::from_rgb(168, 152, 128);
const GOLD: Color32 = Color32::from_rgb(228, 177, 90);
const BTC_COLOR: Color32 = Color32::from_rgb(242, 163, 58);
const UP: Color32 = Color32::from_rgb(226, 91, 74);
const DOWN: Color32 = Color32::from_rgb(60, 186, 139);
const WARN: Color32 = Color32::from_rgb(214, 148, 90);
const COLLAPSED_H: f32 = 78.0;

#[derive(Clone, Copy, PartialEq, Eq)]
enum ChartMode {
    Day,
    Intra,
}

struct DeskApp {
    rx: Receiver<Feed>,
    refresh_tx: Sender<()>,
    feed: Feed,
    pinned: bool,
    level_applied: bool,
    chart: ChartMode,
    chart_chosen: bool,
    chart_inited: bool,
    collapsed: bool,
    /// 当前分层窗口不透明度，在悬停的 255 和离开后的 WINDOW_ALPHA 之间渐变。
    alpha: f32,
    /// 已经写进分层窗口的不透明度。没变就不要再调系统接口，否则悬停重画会闪。
    applied_alpha: Option<u8>,
    /// 指针第一次看起来离开窗口的时刻。
    outside_at: Option<Instant>,
    expanded_size: Vec2,
    pending_height: Option<f32>,
    size_ready: bool,
    pomo: Pomodoro,
    pomo_log_open: bool,
    new_focus_open: bool,
    new_focus_draft: String,
    new_focus_need_focus: bool,
    edit_task_open: bool,
    edit_task_draft: String,
    edit_task_need_focus: bool,
    pomo_log_more: bool,
    review_open: bool,
    review_index: usize,
    review_started_ms: u64,
    review_score: u8,
    review_text: String,
    review_need_focus: bool,
}

impl DeskApp {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        install_style(&cc.egui_ctx);
        let (pinned, chart, chart_chosen, collapsed, expanded_size) = cc
            .storage
            .map(|storage| {
                let pinned = storage.get_string("pinned").map(|v| v == "1").unwrap_or(true);
                let chart = match storage.get_string("chart").as_deref() {
                    Some("intra") => ChartMode::Intra,
                    _ => ChartMode::Day,
                };
                let chart_chosen = storage.get_string("chart_chosen").as_deref() == Some("1");
                let collapsed = storage.get_string("collapsed").as_deref() == Some("1");
                let width = storage
                    .get_string("expanded_w")
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(392.0);
                let height = storage
                    .get_string("expanded_h")
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(900.0);
                (pinned, chart, chart_chosen, collapsed, Vec2::new(width, height))
            })
            .unwrap_or((true, ChartMode::Day, false, false, Vec2::new(392.0, 900.0)));
        let (rx, refresh_tx) = spawn_feed();
        let mut pomo = cc
            .storage
            .and_then(|storage| storage.get_string("pomo"))
            .and_then(|raw| Pomodoro::load(&raw))
            .unwrap_or_default();
        if let Some(storage) = cc.storage {
            if let Some(raw) = storage.get_string("pomo_task") {
                pomo.load_task(&raw);
            }
            if let Some(raw) = storage.get_string("pomo_log") {
                pomo.load_log(&raw);
            }
        }
        Self {
            rx,
            refresh_tx,
            feed: Feed::default(),
            pinned,
            level_applied: false,
            chart,
            chart_chosen,
            chart_inited: chart_chosen,
            collapsed,
            alpha: WINDOW_ALPHA as f32,
            applied_alpha: None,
            outside_at: None,
            expanded_size,
            pending_height: None,
            size_ready: false,
            pomo,
            pomo_log_open: false,
            new_focus_open: false,
            new_focus_draft: String::new(),
            new_focus_need_focus: false,
            edit_task_open: false,
            edit_task_draft: String::new(),
            edit_task_need_focus: false,
            pomo_log_more: false,
            review_open: false,
            review_index: 0,
            review_started_ms: 0,
            review_score: 0,
            review_text: String::new(),
            review_need_focus: false,
        }
    }

    fn apply_level(&self, ctx: &egui::Context) {
        let level = if self.pinned {
            WindowLevel::AlwaysOnTop
        } else {
            WindowLevel::Normal
        };
        ctx.send_viewport_cmd(ViewportCommand::WindowLevel(level));
    }

    fn take_feed(&mut self) {
        let mut latest = None;
        while let Ok(feed) = self.rx.try_recv() {
            latest = Some(feed);
        }
        if let Some(feed) = latest {
            self.merge_feed(feed);
        }
    }

    fn merge_feed(&mut self, mut feed: Feed) {
        if feed.sh.is_none() {
            feed.sh = self.feed.sh.clone();
        }
        if feed.sz.is_none() {
            feed.sz = self.feed.sz.clone();
        }
        if feed.sh_daily.is_empty() {
            feed.sh_daily = self.feed.sh_daily.clone();
        }
        if feed.sh_intra.is_empty() {
            feed.sh_intra = self.feed.sh_intra.clone();
        }
        if feed.btc_usd.is_none() {
            feed.btc_usd = self.feed.btc_usd;
            feed.btc_open = self.feed.btc_open;
            feed.btc_high = self.feed.btc_high;
            feed.btc_low = self.feed.btc_low;
            feed.btc_spark = self.feed.btc_spark.clone();
        }
        if feed.usdcny.is_none() {
            feed.usdcny = self.feed.usdcny;
        }
        if !self.chart_inited {
            if let Some(quote) = feed.sh.as_ref() {
                self.chart_inited = true;
                if market_status(&quote.time, beijing_now()) == "交易中" && !feed.sh_intra.is_empty() {
                    self.chart = ChartMode::Intra;
                }
            }
        }
        self.feed = feed;
    }

    fn refresh(&self) {
        let _ = self.refresh_tx.send(());
    }

    fn showing_full(&self) -> bool {
        !self.collapsed
    }

    fn note_size(&mut self, ctx: &egui::Context) {
        if let Some(rect) = ctx.input(|input| input.viewport().inner_rect) {
            if rect.height() > COLLAPSED_H + 40.0 {
                self.expanded_size = rect.size();
            }
        }
    }

    fn collapse(&mut self, ctx: &egui::Context) {
        self.note_size(ctx);
        self.collapsed = true;
        self.pending_height = Some(COLLAPSED_H);
    }

    fn expand_pinned(&mut self) {
        self.collapsed = false;
        self.pending_height = Some(self.expanded_size.y);
    }

    fn sync_fold(&mut self, ctx: &egui::Context) {
        if let Some(rect) = ctx.input(|input| input.viewport().inner_rect) {
            if !self.size_ready {
                self.size_ready = true;
                if !self.collapsed && rect.height() > COLLAPSED_H + 40.0 {
                    self.expanded_size = rect.size();
                }
                if self.collapsed {
                    self.pending_height = Some(COLLAPSED_H);
                }
            } else if !self.collapsed && rect.height() > COLLAPSED_H + 40.0 {
                self.expanded_size = rect.size();
            }
        }

        if let Some(height) = self.pending_height.take() {
            let width = ctx
                .input(|input| input.viewport().inner_rect.map(|rect| rect.width()))
                .unwrap_or(self.expanded_size.x)
                .max(320.0);
            ctx.send_viewport_cmd(ViewportCommand::MinInnerSize(Vec2::new(320.0, 64.0)));
            ctx.send_viewport_cmd(ViewportCommand::InnerSize(Vec2::new(width, height)));
        }
    }

    fn header(&mut self, ui: &mut egui::Ui, now: chrono::DateTime<chrono::FixedOffset>) {
        let clock = format!(
            "{}月{}日 {}  {:02}:{:02}:{:02}  北京时间",
            now.month(),
            now.day(),
            weekday_zh(now.weekday()),
            now.hour(),
            now.minute(),
            now.second()
        );
        let (rect, response) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 44.0), Sense::click_and_drag());
        if response.drag_started() {
            ui.ctx().send_viewport_cmd(ViewportCommand::StartDrag);
        }
        response.on_hover_cursor(CursorIcon::Grab);
        let painter = ui.painter().clone();
        painter.text(
            rect.left_top(),
            Align2::LEFT_TOP,
            "案头",
            FontId::proportional(18.0),
            CREAM,
        );
        painter.text(
            rect.left_bottom(),
            Align2::LEFT_BOTTOM,
            clock,
            FontId::proportional(12.0),
            MUTED,
        );

        let tools_rect = Rect::from_min_max(rect.left_top(), Pos2::new(rect.right(), rect.top() + 26.0));
        let mut tools = ui.new_child(
            UiBuilder::new()
                .max_rect(tools_rect)
                .layout(Layout::right_to_left(Align::Center)),
        );
        tools.spacing_mut().item_spacing.x = 2.0;
        if icon_button(&mut tools, BarIcon::Close, false, "关闭").clicked() {
            tools.ctx().send_viewport_cmd(ViewportCommand::Close);
        }
        if icon_button(&mut tools, BarIcon::Fold, false, "收起").clicked() {
            self.collapse(tools.ctx());
        }
        if icon_button(&mut tools, BarIcon::Refresh, false, "刷新").clicked() {
            self.refresh();
        }
        let pin_tip = if self.pinned { "取消置顶" } else { "置顶" };
        if icon_button(&mut tools, BarIcon::Pin, self.pinned, pin_tip).clicked() {
            self.pinned = !self.pinned;
            self.apply_level(tools.ctx());
        }
    }

    fn collapsed_bar(&mut self, ui: &mut egui::Ui, now: chrono::DateTime<chrono::FixedOffset>) {
        let days = match gaokao(now) {
            Gaokao::Countdown { days, .. } => format!("{days} 天"),
            Gaokao::InProgress { .. } => "开考中".to_string(),
        };
        let (rect, response) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 28.0), Sense::click_and_drag());
        if response.drag_started() {
            ui.ctx().send_viewport_cmd(ViewportCommand::StartDrag);
        }
        response.on_hover_cursor(CursorIcon::Grab);
        let painter = ui.painter().clone();
        painter.text(
            rect.left_center(),
            Align2::LEFT_CENTER,
            "案头",
            FontId::proportional(16.0),
            CREAM,
        );
        painter.text(
            rect.left_center() + Vec2::new(44.0, 0.0),
            Align2::LEFT_CENTER,
            &days,
            FontId::proportional(16.0),
            GOLD,
        );
        if self.pomo.running() {
            let now = SystemTime::now();
            let clock = format!("{} {}", self.pomo.phase().label(), format_remaining(self.pomo.remaining(now)));
            painter.text(
                rect.left_center() + Vec2::new(118.0, 0.0),
                Align2::LEFT_CENTER,
                clock,
                FontId::proportional(14.0),
                phase_color(self.pomo.phase()),
            );
        }
        let mut tools = ui.new_child(
            UiBuilder::new()
                .max_rect(rect)
                .layout(Layout::right_to_left(Align::Center)),
        );
        tools.spacing_mut().item_spacing.x = 2.0;
        if icon_button(&mut tools, BarIcon::Close, false, "关闭").clicked() {
            tools.ctx().send_viewport_cmd(ViewportCommand::Close);
        }
        if icon_button(&mut tools, BarIcon::Unfold, true, "展开").clicked() {
            self.expand_pinned();
        }
        if alarm_ringing() && ghost_button(&mut tools, "停止", true).clicked() {
            stop_alarm();
        }
        let btc = self.feed.btc_usd.map(|price| {
            let pct = self
                .feed
                .btc_open
                .filter(|open| *open > 0.0)
                .map(|open| (price - open) / open * 100.0);
            (format!("比特币 {}", fmt_grouped(price, 0)), pct)
        });
        let index = self.feed.sh.as_ref().map(|quote| {
            (
                format!("上证 {}", fmt_grouped(quote.price, 2)),
                quote.change_pct,
            )
        });
        let summary = ui.horizontal(|ui| {
            let task = self.pomo.task();
            if !task.is_empty() {
                ui.scope(|ui| {
                    ui.set_max_width(148.0);
                    ui.add(
                        egui::Label::new(RichText::new(task).size(13.0).strong().color(GOLD))
                            .truncate(),
                    );
                });
                ui.add_space(8.0);
            }
            if let Some((text, pct)) = btc {
                ui.label(RichText::new(text).size(12.0).color(CREAM));
                if let Some(pct) = pct {
                    ui.label(RichText::new(fmt_pct(pct)).size(12.0).color(dir_color(pct)));
                }
            } else {
                ui.label(RichText::new("比特币 …").size(12.0).color(MUTED));
            }
            ui.add_space(8.0);
            if let Some((text, pct)) = index {
                ui.label(RichText::new(text).size(12.0).color(CREAM));
                ui.label(RichText::new(fmt_pct(pct)).size(12.0).color(dir_color(pct)));
            } else {
                ui.label(RichText::new("上证 …").size(12.0).color(MUTED));
            }
        });
        let summary = ui.interact(summary.response.rect, ui.id().with("fold-drag"), Sense::click_and_drag());
        if summary.drag_started() {
            ui.ctx().send_viewport_cmd(ViewportCommand::StartDrag);
        }
        if summary.hovered() {
            ui.ctx().set_cursor_icon(CursorIcon::Grab);
        }
    }

    fn countdown(&self, ui: &mut egui::Ui, now: chrono::DateTime<chrono::FixedOffset>) {
        card(ui, |ui| {
            ui.label(RichText::new("高考倒计时").size(12.0).color(MUTED));
            ui.add_space(4.0);
            match gaokao(now) {
                Gaokao::InProgress { year } => {
                    ui.label(
                        RichText::new(format!("{year} 年高考进行中"))
                            .size(28.0)
                            .strong()
                            .color(GOLD),
                    );
                    ui.label(RichText::new("6 月 7 日 09:00 开考，8 日结束").size(13.0).color(CREAM));
                }
                Gaokao::Countdown {
                    target,
                    days,
                    hours,
                    minutes,
                    seconds,
                    progress,
                } => {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(days.to_string()).size(52.0).strong().color(GOLD));
                        ui.vertical(|ui| {
                            ui.add_space(26.0);
                            ui.label(RichText::new("天").size(16.0).color(GOLD));
                        });
                    });
                    ui.label(
                        RichText::new(format!(
                            "{}年{}月{}日 09:00 开考",
                            target.year(),
                            target.month(),
                            target.day()
                        ))
                        .size(13.0)
                        .color(CREAM),
                    );
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 8.0;
                        time_cell(ui, &format!("{hours:02}"), "时");
                        time_cell(ui, &format!("{minutes:02}"), "分");
                        time_cell(ui, &format!("{seconds:02}"), "秒");
                    });
                    ui.add_space(10.0);
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("备考进度").size(11.0).color(MUTED));
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            ui.label(
                                RichText::new(format!("{:.0}%", progress * 100.0))
                                    .size(11.0)
                                    .color(GOLD),
                            );
                        });
                    });
                    progress_bar(ui, progress);
                }
            }
        });
    }

    fn pomodoro_card(&mut self, ui: &mut egui::Ui) {
        let now = SystemTime::now();
        let phase = self.pomo.phase();
        let color = phase_color(phase);
        let remaining = format_remaining(self.pomo.remaining(now));
        let running = self.pomo.running();
        let progress = self.pomo.progress(now);
        let round = self.pomo.round_index();
        let done = self.pomo.focus_done();
        card(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new("番茄钟").size(12.0).color(MUTED));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.label(
                        RichText::new(format!("{} · {}/{}", phase.label(), round, pomodoro::LONG_EVERY))
                            .size(12.0)
                            .color(color),
                    );
                });
            });
            ui.add_space(6.0);
            let task_owned = self.pomo.task().to_string();
            let mut edit_task = false;
            Frame::new()
                .fill(BG)
                .stroke(Stroke::new(
                    1.0,
                    if task_owned.is_empty() { LINE } else { GOLD },
                ))
                .inner_margin(Margin::symmetric(8, 2))
                .corner_radius(8)
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 2.0;
                        let text_width = (ui.available_width() - 28.0).max(40.0);
                        let (text, color) = if task_owned.is_empty() {
                            ("填写任务描述", MUTED)
                        } else {
                            (task_owned.as_str(), GOLD)
                        };
                        ui.scope(|ui| {
                            ui.set_min_width(text_width);
                            ui.set_max_width(text_width);
                            if ui
                                .add(
                                    egui::Label::new(RichText::new(text).size(16.0).strong().color(color))
                                        .truncate()
                                        .selectable(false)
                                        .sense(Sense::click()),
                                )
                                .on_hover_cursor(CursorIcon::PointingHand)
                                .clicked()
                            {
                                edit_task = true;
                            }
                        });
                        if icon_button(ui, BarIcon::Edit, false, "修改任务").clicked() {
                            edit_task = true;
                        }
                    });
                });
            if edit_task {
                self.edit_task_draft = task_owned;
                self.edit_task_need_focus = true;
                self.edit_task_open = true;
                self.new_focus_open = false;
            }
            ui.add_space(8.0);
            ui.label(RichText::new(remaining).size(40.0).strong().color(CREAM));
            ui.add_space(4.0);
            progress_bar(ui, progress);
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                if alarm_ringing() && ghost_button(ui, "停止", true).clicked() {
                    stop_alarm();
                }
                if ghost_button(ui, "新建", false).clicked() {
                    self.new_focus_draft.clear();
                    self.new_focus_need_focus = true;
                    self.new_focus_open = true;
                }
                let primary = if running { "暂停" } else { "开始" };
                if ghost_button(ui, primary, running).clicked() {
                    let starting = !self.pomo.running();
                    self.pomo.toggle(SystemTime::now());
                    if starting {
                        stop_alarm();
                    }
                }
                if ghost_button(ui, "跳过", false).clicked() {
                    self.pomo.skip(SystemTime::now());
                    alarm();
                }
                if ghost_button(ui, "重置", false).clicked() {
                    self.pomo.reset(SystemTime::now());
                    stop_alarm();
                }
            });
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                ui.label(RichText::new(format!("已完成 {done} 个番茄")).size(11.0).color(MUTED));
                let label = if self.pomo_log_open { "收起记录" } else { "记录" };
                let reserve = if self.pomo_log_open { 84.0 } else { 52.0 };
                ui.add_space((ui.available_width() - reserve).max(0.0));
                if ghost_button(ui, label, self.pomo_log_open).clicked() {
                    self.pomo_log_open = !self.pomo_log_open;
                }
            });
            if self.pomo_log_open {
                ui.add_space(4.0);
                // 进行中的也列出来。时间走完就变成已完成，可以直接点开评价。
                let history = self.pomo.history(now);
                let mut rows: Vec<_> = history
                    .into_iter()
                    .enumerate()
                    .map(|(index, row)| {
                        let stored = row.ended_ms.and_then(|_| self.pomo.notes().get(index));
                        let score = stored.and_then(|note| note.score);
                        let review = stored.map(|note| note.review.clone()).unwrap_or_default();
                        let log_index = stored.map(|_| index);
                        (log_index, score, review, row)
                    })
                    .collect();
                rows.reverse();
                let total = rows.len();
                if total == 0 {
                    ui.label(RichText::new("还没有番茄记录").size(12.0).color(MUTED));
                }
                let visible = if self.pomo_log_more { total } else { total.min(3) };
                let mut open_review = None;
                for (log_index, score, review, row) in rows.into_iter().take(visible) {
                    let task = if row.task.is_empty() {
                        "未填写".to_string()
                    } else {
                        row.task.clone()
                    };
                    let status = row.status;
                    let block = ui.scope(|ui| {
                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new(note_span(row.started_ms, row.ended_ms))
                                    .size(12.0)
                                    .color(MUTED),
                            );
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                if log_index.is_some() {
                                    if let Some(score) = score {
                                        ui.label(RichText::new(format!("{score} 分")).size(12.0).color(GOLD));
                                    } else {
                                        ui.label(RichText::new("评价").size(12.0).color(MUTED));
                                    }
                                }
                                ui.label(RichText::new(status.label()).size(12.0).color(status_color(status)));
                            });
                        });
                        ui.label(
                            RichText::new(format!("{task}  ·  {}", note_spent(row.secs)))
                                .size(12.0)
                                .color(CREAM),
                        );
                    });
                    if let Some(index) = log_index {
                        let hit = ui
                            .interact(
                                block.response.rect,
                                block.response.id.with("hit"),
                                Sense::click(),
                            )
                            .on_hover_cursor(CursorIcon::PointingHand);
                        if hit.clicked() {
                            open_review = Some((index, row.started_ms, score.unwrap_or(0), review));
                        }
                    }
                    ui.add_space(6.0);
                }
                if let Some((index, started_ms, score, review)) = open_review {
                    self.review_index = index;
                    self.review_started_ms = started_ms;
                    self.review_score = score;
                    self.review_text = review;
                    self.review_need_focus = true;
                    self.review_open = true;
                    self.new_focus_open = false;
                    self.edit_task_open = false;
                }
                let hidden = total.saturating_sub(3);
                if hidden > 0 {
                    let more_label = if self.pomo_log_more {
                        "收起".to_string()
                    } else {
                        format!("更多 {hidden}")
                    };
                    if ghost_button(ui, &more_label, self.pomo_log_more).clicked() {
                        self.pomo_log_more = !self.pomo_log_more;
                    }
                }
            }
        });
    }

    fn bitcoin(&self, ui: &mut egui::Ui) {
        card(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new("比特币").size(12.0).color(MUTED));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.label(RichText::new("BTC / USDT").size(12.0).color(MUTED));
                });
            });
            ui.add_space(4.0);
            let Some(price) = self.feed.btc_usd else {
                ui.label(RichText::new("正在获取行情…").color(MUTED));
                return;
            };
            let pct = self
                .feed
                .btc_open
                .filter(|open| *open > 0.0)
                .map(|open| (price - open) / open * 100.0);
            let color = pct.map(dir_color).unwrap_or(BTC_COLOR);
            ui.horizontal(|ui| {
                ui.label(RichText::new(fmt_grouped(price, 1)).size(28.0).strong().color(CREAM));
                ui.vertical(|ui| {
                    ui.add_space(8.0);
                    ui.label(RichText::new("美元").size(12.0).color(MUTED));
                });
                if let Some(pct) = pct {
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.label(RichText::new(fmt_pct(pct)).size(16.0).strong().color(color));
                    });
                }
            });
            if let (Some(cny), Some(fx)) = (
                self.feed.usdcny.map(|fx| price * fx),
                self.feed.usdcny,
            ) {
                ui.label(
                    RichText::new(format!(
                        "约 {} 人民币    汇率 {}",
                        fmt_grouped(cny, 0),
                        fmt_grouped(fx, 4)
                    ))
                    .size(12.0)
                    .color(MUTED),
                );
            }
            if let (Some(low), Some(high)) = (self.feed.btc_low, self.feed.btc_high) {
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    ui.label(RichText::new(format!("低 {}", fmt_grouped(low, 0))).size(11.0).color(MUTED));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.label(RichText::new(format!("高 {}", fmt_grouped(high, 0))).size(11.0).color(MUTED));
                    });
                });
                range_bar(ui, low, high, price, color);
            }
            if self.feed.btc_spark.len() >= 2 {
                ui.add_space(6.0);
                area_chart(ui, 52.0, &self.feed.btc_spark, color);
                chart_ends(ui, &self.feed.btc_spark);
            }
        });
    }

    fn index_panel(&mut self, ui: &mut egui::Ui) {
        card(ui, |ui| {
            let quote = self.feed.sh.clone();
            ui.horizontal(|ui| {
                let title = quote.as_ref().map(|quote| quote.name).unwrap_or("上证指数");
                ui.label(RichText::new(title).size(12.0).color(MUTED));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if let Some(quote) = &quote {
                        let status = market_status(&quote.time, beijing_now());
                        let status_color = if status == "交易中" { GOLD } else { MUTED };
                        ui.label(RichText::new(status).size(12.0).color(status_color));
                        ui.label(RichText::new(&quote.code).size(12.0).color(MUTED));
                    }
                });
            });
            ui.add_space(4.0);
            let Some(quote) = quote else {
                ui.label(RichText::new("正在获取行情…").color(MUTED));
                return;
            };
            self.quote_head(ui, &quote);
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.label(RichText::new("走势").size(12.0).color(MUTED));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ghost_button(ui, "分时", self.chart == ChartMode::Intra).clicked() {
                        self.chart = ChartMode::Intra;
                        self.chart_chosen = true;
                        self.chart_inited = true;
                    }
                    if ghost_button(ui, "日K", self.chart == ChartMode::Day).clicked() {
                        self.chart = ChartMode::Day;
                        self.chart_chosen = true;
                        self.chart_inited = true;
                    }
                });
            });
            let series = match self.chart {
                ChartMode::Day => &self.feed.sh_daily,
                ChartMode::Intra => &self.feed.sh_intra,
            };
            ui.add_space(4.0);
            if series.len() >= 2 {
                let color = dir_color(quote.change);
                area_chart(ui, 108.0, series, color);
                chart_ends(ui, series);
            } else if self.feed.fetched_at.is_empty() {
                ui.label(RichText::new("正在获取走势…").color(MUTED));
            } else {
                ui.label(RichText::new("暂无走势").color(MUTED));
            }
            if let Some(sz) = &self.feed.sz {
                ui.add_space(8.0);
                quote_row(ui, sz.name, sz.price, sz.change_pct, 2);
            }
            if let Some(fx) = self.feed.usdcny {
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.label(RichText::new("美元/人民币").size(13.0).color(MUTED));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.label(RichText::new(fmt_grouped(fx, 4)).size(13.0).color(CREAM));
                    });
                });
            }
        });
    }

    fn quote_head(&self, ui: &mut egui::Ui, quote: &Quote) {
        let color = dir_color(quote.change);
        ui.horizontal(|ui| {
            ui.label(RichText::new(fmt_grouped(quote.price, 2)).size(28.0).strong().color(CREAM));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.vertical(|ui| {
                    ui.label(RichText::new(fmt_pct(quote.change_pct)).size(14.0).strong().color(color));
                    ui.label(
                        RichText::new(fmt_signed(quote.change, 2))
                            .size(13.0)
                            .color(color),
                    );
                });
            });
        });
        ui.add_space(2.0);
        ui.horizontal_wrapped(|ui| {
            stat(ui, "开", quote.open);
            stat(ui, "高", quote.high);
            stat(ui, "低", quote.low);
            if let Some(amount) = quote.amount_yi {
                ui.label(
                    RichText::new(format!("额 {}亿", fmt_grouped(amount, 0)))
                        .size(12.0)
                        .color(MUTED),
                );
            }
        });
        ui.label(
            RichText::new(format!("行情 {}", format_quote_time(&quote.time)))
                .size(11.0)
                .color(MUTED),
        );
    }

    fn status(&self, ui: &mut egui::Ui) {
        if self.feed.fetched_at.is_empty() {
            ui.label(RichText::new("正在连接行情…").size(11.0).color(MUTED));
        } else {
            ui.label(
                RichText::new(format!("更新于 {}    上证 · 腾讯    比特币 · 芝麻开门", self.feed.fetched_at))
                    .size(11.0)
                    .color(MUTED),
            );
        }
        for note in &self.feed.notes {
            ui.label(RichText::new(note).size(11.0).color(WARN));
        }
    }

    fn menu(&mut self, ui: &mut egui::Ui) {
        if ui.button("立即刷新").clicked() {
            self.refresh();
            ui.close();
        }
        if self.collapsed {
            if ui.button("展开").clicked() {
                self.expand_pinned();
                ui.close();
            }
        } else if ui.button("收起").clicked() {
            self.collapse(ui.ctx());
            ui.close();
        }
        let label = if self.pinned { "取消置顶" } else { "置顶窗口" };
        if ui.button(label).clicked() {
            self.pinned = !self.pinned;
            self.apply_level(ui.ctx());
            ui.close();
        }
        if ui.button("退出").clicked() {
            ui.ctx().send_viewport_cmd(ViewportCommand::Close);
        }
    }

    fn sync_opacity(&mut self, ctx: &egui::Context) {
        // 不用 egui 的 hover：改透明度时系统会丢一帧悬停，目标来回跳就会闪。
        let hovering = cursor_over_window();
        let now = Instant::now();
        if hovering {
            self.outside_at = None;
        } else if self.outside_at.is_none() {
            self.outside_at = Some(now);
        }
        let gone = self.outside_at.is_some_and(|at| now.saturating_duration_since(at) >= ALPHA_LEAVE_DELAY);
        if !hovering && !gone {
            let waited = self.outside_at.map(|at| now.saturating_duration_since(at)).unwrap_or_default();
            ctx.request_repaint_after(ALPHA_LEAVE_DELAY.saturating_sub(waited));
        }
        let target = if gone { WINDOW_ALPHA as f32 } else { 255.0 };
        let dt = ctx.input(|input| input.stable_dt).clamp(0.0, 0.05);
        let delta = target - self.alpha;
        if delta.abs() > 0.6 {
            let blend = 1.0 - (-dt / ALPHA_FADE_TAU).exp();
            self.alpha += delta * blend;
            ctx.request_repaint();
        } else {
            self.alpha = target;
        }
        let alpha = self.alpha.round() as u8;
        if apply_window_alpha(alpha, self.applied_alpha) {
            self.applied_alpha = Some(alpha);
        }
    }

    fn show_new_focus(&mut self, ctx: &egui::Context) {
        let mut start = false;
        let mut close = false;
        let interrupting = self.pomo.has_open_focus();
        let response = egui::Modal::new(egui::Id::new("new-focus"))
            .backdrop_color(Color32::from_black_alpha(160))
            .frame(
                Frame::new()
                    .fill(CARD)
                    .stroke(Stroke::new(1.0, LINE))
                    .inner_margin(Margin::same(16))
                    .corner_radius(12),
            )
            .show(ctx, |ui| {
                ui.set_min_width(240.0);
                ui.label(RichText::new("新建番茄钟").size(16.0).strong().color(CREAM));
                ui.add_space(4.0);
                ui.label(RichText::new("任务描述").size(12.0).color(MUTED));
                ui.add_space(2.0);
                let editor = ui.scope(|ui| {
                    ui.visuals_mut().weak_text_color = Some(MUTED);
                    ui.add(
                        egui::TextEdit::singleline(&mut self.new_focus_draft)
                            .hint_text("这一轮要做什么")
                            .desired_width(f32::INFINITY)
                            .text_color(CREAM)
                            .background_color(BG)
                            .frame(
                                Frame::new()
                                    .fill(BG)
                                    .stroke(Stroke::new(1.0, GOLD))
                                    .inner_margin(Margin::symmetric(8, 6))
                                    .corner_radius(8),
                            ),
                    )
                });
                if self.new_focus_need_focus {
                    editor.inner.request_focus();
                    self.new_focus_need_focus = false;
                }
                if editor.inner.lost_focus() && ui.input(|input| input.key_pressed(Key::Enter)) {
                    start = true;
                }
                if interrupting {
                    ui.add_space(4.0);
                    ui.label(RichText::new("当前专注会记为已中断").size(11.0).color(MUTED));
                }
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ghost_button(ui, "取消", false).clicked() {
                        close = true;
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ghost_button(ui, "开始", true).clicked() {
                            start = true;
                        }
                    });
                });
            });
        if response.should_close() {
            close = true;
        }
        if start {
            self.pomo
                .start_new_focus(SystemTime::now(), &self.new_focus_draft);
            stop_alarm();
            self.new_focus_draft.clear();
            self.new_focus_open = false;
        } else if close {
            self.new_focus_draft.clear();
            self.new_focus_open = false;
        }
    }

    fn show_edit_task(&mut self, ctx: &egui::Context) {
        let mut save = false;
        let mut close = false;
        let response = egui::Modal::new(egui::Id::new("edit-task"))
            .backdrop_color(Color32::from_black_alpha(160))
            .frame(
                Frame::new()
                    .fill(CARD)
                    .stroke(Stroke::new(1.0, LINE))
                    .inner_margin(Margin::same(16))
                    .corner_radius(12),
            )
            .show(ctx, |ui| {
                ui.set_min_width(240.0);
                ui.label(RichText::new("修改任务").size(16.0).strong().color(CREAM));
                ui.add_space(4.0);
                ui.label(RichText::new("任务描述").size(12.0).color(MUTED));
                ui.add_space(2.0);
                let editor = ui.scope(|ui| {
                    ui.visuals_mut().weak_text_color = Some(MUTED);
                    ui.add(
                        egui::TextEdit::singleline(&mut self.edit_task_draft)
                            .hint_text("这一轮要做什么")
                            .desired_width(f32::INFINITY)
                            .text_color(CREAM)
                            .background_color(BG)
                            .frame(
                                Frame::new()
                                    .fill(BG)
                                    .stroke(Stroke::new(1.0, GOLD))
                                    .inner_margin(Margin::symmetric(8, 6))
                                    .corner_radius(8),
                            ),
                    )
                });
                if self.edit_task_need_focus {
                    editor.inner.request_focus();
                    self.edit_task_need_focus = false;
                }
                if editor.inner.lost_focus() && ui.input(|input| input.key_pressed(Key::Enter)) {
                    save = true;
                }
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ghost_button(ui, "取消", false).clicked() {
                        close = true;
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ghost_button(ui, "保存", true).clicked() {
                            save = true;
                        }
                    });
                });
            });
        if response.should_close() {
            close = true;
        }
        if save {
            self.pomo.set_task(&self.edit_task_draft);
            self.edit_task_draft.clear();
            self.edit_task_open = false;
        } else if close {
            self.edit_task_draft.clear();
            self.edit_task_open = false;
        }
    }

    fn show_review(&mut self, ctx: &egui::Context) {
        let summary = self.pomo.notes().get(self.review_index).and_then(|note| {
            if note.started_ms != self.review_started_ms {
                return None;
            }
            let task = if note.task.is_empty() {
                "未填写".to_string()
            } else {
                note.task.clone()
            };
            Some((
                task,
                note_span(note.started_ms, Some(note.ended_ms)),
                note_spent(note.secs),
            ))
        });
        let Some((task, span, spent)) = summary else {
            self.review_open = false;
            return;
        };
        let mut save = false;
        let mut close = false;
        let response = egui::Modal::new(egui::Id::new("pomo-review"))
            .backdrop_color(Color32::from_black_alpha(160))
            .frame(
                Frame::new()
                    .fill(CARD)
                    .stroke(Stroke::new(1.0, LINE))
                    .inner_margin(Margin::same(16))
                    .corner_radius(12),
            )
            .show(ctx, |ui| {
                ui.set_min_width(240.0);
                ui.label(RichText::new("自我评价").size(16.0).strong().color(CREAM));
                ui.add_space(4.0);
                ui.label(RichText::new(task).size(14.0).color(CREAM));
                ui.label(RichText::new(format!("{span}    {spent}")).size(12.0).color(MUTED));
                ui.add_space(8.0);
                ui.label(RichText::new("打分").size(12.0).color(MUTED));
                ui.add_space(2.0);
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 4.0;
                    for score in 1..=5 {
                        if score_button(ui, score, self.review_score >= score).clicked() {
                            self.review_score = score;
                        }
                    }
                });
                ui.add_space(8.0);
                ui.label(RichText::new("一句评价").size(12.0).color(MUTED));
                ui.add_space(2.0);
                let editor = ui.scope(|ui| {
                    ui.visuals_mut().weak_text_color = Some(MUTED);
                    ui.add(
                        egui::TextEdit::singleline(&mut self.review_text)
                            .hint_text("这一轮怎么样")
                            .desired_width(f32::INFINITY)
                            .text_color(CREAM)
                            .background_color(BG)
                            .frame(
                                Frame::new()
                                    .fill(BG)
                                    .stroke(Stroke::new(1.0, GOLD))
                                    .inner_margin(Margin::symmetric(8, 6))
                                    .corner_radius(8),
                            ),
                    )
                });
                if self.review_need_focus {
                    editor.inner.request_focus();
                    self.review_need_focus = false;
                }
                if editor.inner.lost_focus()
                    && ui.input(|input| input.key_pressed(Key::Enter))
                    && (1..=5).contains(&self.review_score)
                {
                    save = true;
                }
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ghost_button(ui, "取消", false).clicked() {
                        close = true;
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        let ready = (1..=5).contains(&self.review_score);
                        if ghost_button(ui, "保存", ready).clicked() && ready {
                            save = true;
                        }
                    });
                });
            });
        if response.should_close() {
            close = true;
        }
        if save {
            self.pomo
                .set_review(self.review_index, self.review_started_ms, self.review_score, &self.review_text);
            self.review_text.clear();
            self.review_open = false;
        } else if close {
            self.review_text.clear();
            self.review_open = false;
        }
    }
}

impl eframe::App for DeskApp {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.take_feed();
        let logged = self.pomo.notes().len();
        if self.pomo.settle(SystemTime::now()) > 0 {
            alarm();
            if self.pomo.notes().len() > logged {
                self.pomo_log_open = true;
            }
        }
        ctx.request_repaint_after(Duration::from_millis(250));
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        if !self.level_applied {
            self.apply_level(ui.ctx());
            self.level_applied = true;
        }
        if ui.ctx().input(|input| input.key_pressed(Key::F5)) {
            self.refresh();
        }
        self.sync_fold(ui.ctx());
        let now = beijing_now();
        let show_full = self.showing_full();
        let margin = if show_full {
            Margin::same(14)
        } else {
            Margin::symmetric(12, 8)
        };
        let panel = CentralPanel::default()
            .frame(Frame::new().fill(BG).inner_margin(margin))
            .show(ui, |ui| {
                if show_full {
                    self.header(ui, now);
                    ui.add_space(8.0);
                    let scroll_h = (ui.available_height() - 16.0).max(80.0);
                    ScrollArea::vertical()
                        .max_height(scroll_h)
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            self.countdown(ui, now);
                            self.pomodoro_card(ui);
                            self.bitcoin(ui);
                            self.index_panel(ui);
                            self.status(ui);
                        });
                    resize_grip(ui);
                } else {
                    self.collapsed_bar(ui, now);
                }
            });
        panel.response.context_menu(|ui| self.menu(ui));
        if self.new_focus_open {
            self.show_new_focus(ui.ctx());
        }
        if self.edit_task_open {
            self.show_edit_task(ui.ctx());
        }
        if self.review_open {
            self.show_review(ui.ctx());
        }
        self.sync_opacity(ui.ctx());
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        storage.set_string("pinned", if self.pinned { "1" } else { "0" }.into());
        storage.set_string(
            "chart",
            match self.chart {
                ChartMode::Day => "day",
                ChartMode::Intra => "intra",
            }
            .into(),
        );
        storage.set_string("chart_chosen", if self.chart_chosen { "1" } else { "0" }.into());
        storage.set_string("collapsed", if self.collapsed { "1" } else { "0" }.into());
        storage.set_string("expanded_w", format!("{:.0}", self.expanded_size.x));
        storage.set_string("expanded_h", format!("{:.0}", self.expanded_size.y));
        storage.set_string("pomo", self.pomo.save(SystemTime::now()));
        storage.set_string("pomo_task", self.pomo.save_task());
        storage.set_string("pomo_log", self.pomo.save_log());
    }

    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        BG.to_normalized_gamma_f32()
    }
}

fn main() -> eframe::Result {
    let viewport = egui::ViewportBuilder::default()
        .with_title("案头")
        .with_app_id("desk-widget")
        .with_inner_size([392.0, 900.0])
        .with_min_inner_size([320.0, 64.0])
        .with_decorations(false)
        .with_resizable(true)
        .with_transparent(true)
        .with_always_on_top()
        .with_icon(eframe::icon_data::from_png_bytes(include_bytes!("../assets/desk.png")).expect("程序图标"));
    eframe::run_native(
        "案头",
        eframe::NativeOptions {
            viewport,
            wgpu_options: wgpu_options(),
            ..Default::default()
        },
        Box::new(|cc| Ok(Box::new(DeskApp::new(cc)))),
    )
}

const GWL_EXSTYLE: i32 = -20;
const WS_EX_LAYERED: i32 = 0x0008_0000;
const LWA_ALPHA: u32 = 0x2;

#[link(name = "user32")]
unsafe extern "system" {
    fn FindWindowW(class: *const u16, name: *const u16) -> isize;
    fn GetWindowLongW(hwnd: isize, index: i32) -> i32;
    fn SetWindowLongW(hwnd: isize, index: i32, value: i32) -> i32;
    fn SetLayeredWindowAttributes(hwnd: isize, key: u32, alpha: u8, flags: u32) -> i32;
    fn GetCursorPos(point: *mut Point) -> i32;
    fn GetWindowRect(hwnd: isize, rect: *mut WinRect) -> i32;
}

#[link(name = "winmm")]
unsafe extern "system" {
    fn PlaySoundW(name: *const u16, module: isize, flags: u32) -> i32;
}

#[link(name = "kernel32")]
unsafe extern "system" {
    fn Beep(frequency: u32, duration_ms: u32) -> i32;
}

const SND_ASYNC: u32 = 0x0001;
const SND_NODEFAULT: u32 = 0x0002;
const SND_LOOP: u32 = 0x0008;
const SND_PURGE: u32 = 0x0040;
const SND_FILENAME: u32 = 0x0002_0000;
const ALARM_FOR: Duration = Duration::from_secs(60);
static ALARM_GEN: AtomicU64 = AtomicU64::new(0);
/// 正在响的那一次。0 表示没在响。
static ALARM_LIVE: AtomicU64 = AtomicU64::new(0);

#[repr(C)]
struct Point {
    x: i32,
    y: i32,
}

#[repr(C)]
struct WinRect {
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
}

fn window_hwnd() -> isize {
    let title: Vec<u16> = "案头".encode_utf16().chain(std::iter::once(0)).collect();
    unsafe { FindWindowW(std::ptr::null(), title.as_ptr()) }
}

fn cursor_over_window() -> bool {
    unsafe {
        let hwnd = window_hwnd();
        if hwnd == 0 {
            return false;
        }
        let mut point = Point { x: 0, y: 0 };
        let mut rect = WinRect {
            left: 0,
            top: 0,
            right: 0,
            bottom: 0,
        };
        if GetCursorPos(&mut point) == 0 || GetWindowRect(hwnd, &mut rect) == 0 {
            return false;
        }
        point.x >= rect.left && point.x < rect.right && point.y >= rect.top && point.y < rect.bottom
    }
}

fn beijing_at(ms: u64) -> Option<chrono::DateTime<chrono::FixedOffset>> {
    chrono::DateTime::from_timestamp_millis(ms as i64).map(|time| time.with_timezone(&beijing_offset()))
}

fn note_span(started_ms: u64, ended_ms: Option<u64>) -> String {
    let Some(start) = beijing_at(started_ms) else {
        return "--".to_string();
    };
    let start_hm = start.format("%H:%M");
    let start_day = start.format("%m-%d");
    match ended_ms.and_then(beijing_at) {
        Some(end) => {
            let end_hm = end.format("%H:%M");
            if start.date_naive() == end.date_naive() {
                format!("{start_day} {start_hm}–{end_hm}")
            } else {
                format!("{start_day} {start_hm}–{} {end_hm}", end.format("%m-%d"))
            }
        }
        None => format!("{start_day} {start_hm} 起"),
    }
}

fn status_color(status: NoteStatus) -> Color32 {
    match status {
        NoteStatus::Running => GOLD,
        NoteStatus::Paused => MUTED,
        NoteStatus::Done => DOWN,
        NoteStatus::Interrupted => UP,
    }
}

fn note_spent(secs: u64) -> String {
    if secs >= 60 {
        format!("{} 分钟", secs / 60)
    } else {
        format!("{secs} 秒")
    }
}

fn phase_color(phase: pomodoro::Phase) -> Color32 {
    match phase {
        pomodoro::Phase::Focus => UP,
        pomodoro::Phase::ShortBreak => DOWN,
        pomodoro::Phase::LongBreak => GOLD,
    }
}

fn alarm_ringing() -> bool {
    ALARM_LIVE.load(Ordering::SeqCst) != 0
}

fn silence_alarm() {
    unsafe {
        PlaySoundW(std::ptr::null(), 0, SND_PURGE);
    }
}

fn stop_alarm() {
    ALARM_GEN.fetch_add(1, Ordering::SeqCst);
    ALARM_LIVE.store(0, Ordering::SeqCst);
    silence_alarm();
}

/// 阶段到点后循环播放闹钟，满一分钟停下。
fn alarm() {
    let ticket = ALARM_GEN.fetch_add(1, Ordering::SeqCst) + 1;
    ALARM_LIVE.store(ticket, Ordering::SeqCst);
    std::thread::spawn(move || {
        if ALARM_GEN.load(Ordering::SeqCst) != ticket {
            let _ = ALARM_LIVE.compare_exchange(ticket, 0, Ordering::SeqCst, Ordering::SeqCst);
            return;
        }
        let started = Instant::now();
        if start_alarm_loop() {
            while started.elapsed() < ALARM_FOR {
                if ALARM_GEN.load(Ordering::SeqCst) != ticket {
                    return;
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            if ALARM_GEN
                .compare_exchange(ticket, ticket + 1, Ordering::SeqCst, Ordering::SeqCst)
                .is_ok()
            {
                let _ = ALARM_LIVE.compare_exchange(ticket, 0, Ordering::SeqCst, Ordering::SeqCst);
                silence_alarm();
            }
        } else {
            synth_alarm(ticket, started);
            if ALARM_GEN
                .compare_exchange(ticket, ticket + 1, Ordering::SeqCst, Ordering::SeqCst)
                .is_ok()
            {
                let _ = ALARM_LIVE.compare_exchange(ticket, 0, Ordering::SeqCst, Ordering::SeqCst);
            }
        }
    });
}

fn alarm_wide() -> Option<&'static [u16]> {
    static PATH: OnceLock<Option<Vec<u16>>> = OnceLock::new();
    PATH.get_or_init(|| {
        let path = r"C:\Windows\Media\Alarm01.wav";
        if !std::path::Path::new(path).exists() {
            return None;
        }
        Some(path.encode_utf16().chain(std::iter::once(0)).collect())
    })
    .as_deref()
}

fn start_alarm_loop() -> bool {
    let Some(wide) = alarm_wide() else {
        return false;
    };
    unsafe { PlaySoundW(wide.as_ptr(), 0, SND_ASYNC | SND_LOOP | SND_FILENAME | SND_NODEFAULT) != 0 }
}

fn synth_alarm(ticket: u64, started: Instant) {
    while started.elapsed() < ALARM_FOR && ALARM_GEN.load(Ordering::SeqCst) == ticket {
        unsafe {
            Beep(880, 180);
        }
        if ALARM_GEN.load(Ordering::SeqCst) != ticket || started.elapsed() >= ALARM_FOR {
            break;
        }
        unsafe {
            Beep(660, 180);
        }
    }
}

/// 只在不透明度变化，或系统把分层样式清掉时才写窗口。重复设置会让悬停重画闪一下。
fn apply_window_alpha(alpha: u8, applied: Option<u8>) -> bool {
    unsafe {
        let hwnd = window_hwnd();
        if hwnd == 0 {
            return false;
        }
        let ex = GetWindowLongW(hwnd, GWL_EXSTYLE);
        let missing = ex & WS_EX_LAYERED == 0;
        if !missing && applied == Some(alpha) {
            return true;
        }
        if missing {
            SetWindowLongW(hwnd, GWL_EXSTYLE, ex | WS_EX_LAYERED);
        }
        SetLayeredWindowAttributes(hwnd, 0, alpha, LWA_ALPHA) != 0
    }
}

fn wgpu_options() -> eframe::egui_wgpu::WgpuConfiguration {
    let mut options = eframe::egui_wgpu::WgpuConfiguration::default();
    if let eframe::egui_wgpu::WgpuSetup::CreateNew(setup) = &mut options.wgpu_setup {
        // 这台机器上 Vulkan 设备会立刻丢失。Windows 用 DX12，不行再退回 OpenGL。
        setup.instance_descriptor.backends = eframe::wgpu::Backends::from_env()
            .unwrap_or(eframe::wgpu::Backends::DX12 | eframe::wgpu::Backends::GL);
    }
    options
}

fn install_style(ctx: &egui::Context) {
    load_cjk(ctx);
    ctx.all_styles_mut(|style| {
        style.spacing.item_spacing = Vec2::new(8.0, 6.0);
        style.spacing.button_padding = Vec2::new(8.0, 3.0);
        style.visuals.panel_fill = BG;
        style.visuals.window_fill = BG;
        style.visuals.extreme_bg_color = CARD;
        style.visuals.override_text_color = Some(CREAM);
        style.text_styles.insert(TextStyle::Body, FontId::proportional(14.0));
        style.text_styles.insert(TextStyle::Button, FontId::proportional(13.0));
        style.text_styles.insert(TextStyle::Small, FontId::proportional(12.0));
    });
    ctx.set_visuals(egui::Visuals {
        panel_fill: BG,
        window_fill: BG,
        extreme_bg_color: CARD,
        override_text_color: Some(CREAM),
        ..egui::Visuals::dark()
    });
}

fn load_cjk(ctx: &egui::Context) {
    let candidates = [
        (r"C:\Windows\Fonts\msyh.ttc", 0),
        (r"C:\Windows\Fonts\simhei.ttf", 0),
        (r"C:\Windows\Fonts\simsun.ttc", 0),
    ];
    for (path, index) in candidates {
        let Ok(bytes) = std::fs::read(path) else {
            continue;
        };
        let mut font = FontData::from_owned(bytes);
        font.index = index;
        let mut fonts = FontDefinitions::default();
        fonts.font_data.insert("cjk".to_owned(), std::sync::Arc::new(font));
        fonts
            .families
            .entry(FontFamily::Proportional)
            .or_default()
            .insert(0, "cjk".to_owned());
        fonts
            .families
            .entry(FontFamily::Monospace)
            .or_default()
            .insert(0, "cjk".to_owned());
        ctx.set_fonts(fonts);
        return;
    }
}

fn card(ui: &mut egui::Ui, body: impl FnOnce(&mut egui::Ui)) {
    Frame::new()
        .fill(CARD)
        .corner_radius(12u8)
        .inner_margin(Margin::same(12))
        .show(ui, body);
    ui.add_space(10.0);
}

#[derive(Clone, Copy)]
enum BarIcon {
    Pin,
    Refresh,
    Fold,
    Unfold,
    Close,
    Edit,
}

fn icon_button(ui: &mut egui::Ui, icon: BarIcon, on: bool, tip: &str) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::new(26.0, 26.0), Sense::click());
    let fill = if on {
        Color32::from_rgb(72, 54, 28)
    } else if response.hovered() {
        Color32::from_rgb(46, 39, 32)
    } else {
        Color32::TRANSPARENT
    };
    if fill != Color32::TRANSPARENT {
        ui.painter().rect_filled(rect, CornerRadius::same(6), fill);
    }
    let color = if on || response.hovered() { GOLD } else { MUTED };
    paint_icon(ui.painter(), rect, icon, color, on);
    response.on_hover_cursor(CursorIcon::PointingHand).on_hover_text(tip)
}

fn paint_icon(painter: &egui::Painter, rect: Rect, icon: BarIcon, color: Color32, filled: bool) {
    let c = rect.center();
    let stroke = Stroke::new(1.5, color);
    match icon {
        BarIcon::Close => {
            let d = 4.2;
            painter.line_segment([c + Vec2::new(-d, -d), c + Vec2::new(d, d)], stroke);
            painter.line_segment([c + Vec2::new(-d, d), c + Vec2::new(d, -d)], stroke);
        }
        BarIcon::Fold => {
            painter.line_segment([c + Vec2::new(-4.6, -1.6), c + Vec2::new(0.0, 3.0)], stroke);
            painter.line_segment([c + Vec2::new(0.0, 3.0), c + Vec2::new(4.6, -1.6)], stroke);
        }
        BarIcon::Unfold => {
            painter.line_segment([c + Vec2::new(-4.6, 1.6), c + Vec2::new(0.0, -3.0)], stroke);
            painter.line_segment([c + Vec2::new(0.0, -3.0), c + Vec2::new(4.6, 1.6)], stroke);
        }
        BarIcon::Refresh => {
            let radius = 4.3;
            let mut points = Vec::with_capacity(20);
            for step in 0..=18 {
                let degrees = 28.0 + step as f32 * 15.5;
                points.push(c + Vec2::angled(degrees.to_radians()) * radius);
            }
            painter.add(Shape::line(points, Stroke::new(1.6, color)));
            let end = 307.0_f32.to_radians();
            let radial = Vec2::angled(end);
            let tangent = Vec2::new(-radial.y, radial.x);
            let tip = c + radial * radius + tangent * 3.4;
            let base = c + radial * radius - tangent * 1.0;
            let wing = Vec2::new(-tangent.y, tangent.x) * 2.5;
            let mut mesh = Mesh::default();
            mesh.colored_vertex(tip, color);
            mesh.colored_vertex(base + wing, color);
            mesh.colored_vertex(base - wing, color);
            mesh.add_triangle(0, 1, 2);
            painter.add(Shape::from(mesh));
        }
        BarIcon::Pin => {
            let head = c + Vec2::new(0.0, -2.6);
            if filled {
                painter.circle_filled(head, 3.0, color);
            } else {
                painter.circle_stroke(head, 3.0, Stroke::new(1.4, color));
            }
            painter.line_segment([head + Vec2::new(0.0, 2.8), c + Vec2::new(0.0, 6.2)], Stroke::new(1.4, color));
        }
        BarIcon::Edit => {
            let tail = c + Vec2::new(-4.6, 3.4);
            let neck = c + Vec2::new(2.2, -3.4);
            let tip = c + Vec2::new(4.8, -5.0);
            painter.line_segment([tail, neck], stroke);
            let side = Vec2::new(-1.15, -1.15);
            painter.line_segment([neck + side, tip], stroke);
            painter.line_segment([neck - side, tip], stroke);
            painter.line_segment([tail + side, tail - side], stroke);
        }
    }
}

fn score_button(ui: &mut egui::Ui, score: u8, on: bool) -> egui::Response {
    let fill = if on {
        Color32::from_rgb(72, 54, 28)
    } else {
        Color32::from_rgb(46, 39, 32)
    };
    let color = if on { GOLD } else { MUTED };
    ui.add(
        Button::new(RichText::new(score.to_string()).size(13.0).color(color))
            .fill(fill)
            .stroke(Stroke::NONE)
            .corner_radius(8u8)
            .min_size(Vec2::new(28.0, 28.0)),
    )
}

fn ghost_button(ui: &mut egui::Ui, text: &str, on: bool) -> egui::Response {
    let fill = if on {
        Color32::from_rgb(72, 54, 28)
    } else {
        Color32::from_rgb(46, 39, 32)
    };
    let color = if on { GOLD } else { MUTED };
    ui.add(
        Button::new(RichText::new(text).size(12.0).color(color))
            .fill(fill)
            .stroke(Stroke::NONE)
            .corner_radius(8u8)
            .min_size(Vec2::new(46.0, 24.0)),
    )
}

fn time_cell(ui: &mut egui::Ui, value: &str, unit: &str) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(76.0, 54.0), Sense::hover());
    let painter = ui.painter();
    painter.rect_filled(rect, CornerRadius::same(8), BG);
    painter.text(
        Pos2::new(rect.center().x, rect.top() + 5.0),
        Align2::CENTER_TOP,
        value,
        FontId::proportional(22.0),
        CREAM,
    );
    painter.text(
        Pos2::new(rect.center().x, rect.bottom() - 4.0),
        Align2::CENTER_BOTTOM,
        unit,
        FontId::proportional(11.0),
        MUTED,
    );
}

fn progress_bar(ui: &mut egui::Ui, t: f32) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 4.0), Sense::hover());
    let painter = ui.painter();
    painter.rect_filled(rect, CornerRadius::same(2), LINE);
    let mut fill = rect;
    fill.max.x = rect.left() + rect.width() * t.clamp(0.0, 1.0);
    if fill.width() > 0.0 {
        painter.rect_filled(fill, CornerRadius::same(2), GOLD);
    }
}

fn range_bar(ui: &mut egui::Ui, low: f64, high: f64, last: f64, color: Color32) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 10.0), Sense::hover());
    let painter = ui.painter();
    let y = rect.center().y;
    painter.hline(rect.x_range(), y, Stroke::new(2.0, LINE));
    let t = if high > low {
        ((last - low) / (high - low)) as f32
    } else {
        0.5
    };
    let x = rect.left() + rect.width() * t.clamp(0.0, 1.0);
    painter.circle_filled(Pos2::new(x, y), 4.0, color);
}

fn area_chart(ui: &mut egui::Ui, height: f32, series: &[Bar], color: Color32) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::new(ui.available_width(), height), Sense::hover());
    if series.len() < 2 {
        return response;
    }
    let min = series.iter().map(|bar| bar.close).fold(f64::INFINITY, f64::min);
    let max = series.iter().map(|bar| bar.close).fold(f64::NEG_INFINITY, f64::max);
    let span = (max - min).max(1e-9);
    let n = series.len();
    let mut points = Vec::with_capacity(n);
    for (i, bar) in series.iter().enumerate() {
        let x = rect.left() + rect.width() * i as f32 / (n - 1) as f32;
        let y = rect.bottom() - 6.0 - (rect.height() - 12.0) * ((bar.close - min) / span) as f32;
        points.push(Pos2::new(x, y));
    }
    let painter = ui.painter().with_clip_rect(rect);
    painter.hline(rect.x_range(), rect.center().y, Stroke::new(1.0, LINE));

    let mut mesh = Mesh::default();
    let top = color.gamma_multiply(0.42);
    let bottom = color.gamma_multiply(0.02);
    for pair in points.windows(2) {
        let a = pair[0];
        let b = pair[1];
        let base = mesh.vertices.len() as u32;
        mesh.colored_vertex(a, top);
        mesh.colored_vertex(Pos2::new(a.x, rect.bottom()), bottom);
        mesh.colored_vertex(b, top);
        mesh.colored_vertex(Pos2::new(b.x, rect.bottom()), bottom);
        mesh.add_triangle(base, base + 1, base + 2);
        mesh.add_triangle(base + 2, base + 1, base + 3);
    }
    painter.add(Shape::from(mesh));
    painter.add(Shape::line(points.clone(), Stroke::new(1.6, color)));

    if response.hovered() {
        if let Some(pos) = response.hover_pos() {
            let t = ((pos.x - rect.left()) / rect.width()).clamp(0.0, 1.0);
            let index = ((t * (n - 1) as f32).round() as usize).min(n - 1);
            let point = points[index];
            painter.vline(point.x, rect.y_range(), Stroke::new(1.0, MUTED));
            painter.circle_filled(point, 3.2, color);
            response.clone().on_hover_text(format!(
                "{}\n{}",
                series[index].label,
                fmt_grouped(series[index].close, 2)
            ));
        }
    }
    response
}

fn chart_ends(ui: &mut egui::Ui, series: &[Bar]) {
    let Some(first) = series.first() else { return };
    let Some(last) = series.last() else { return };
    ui.horizontal(|ui| {
        ui.label(RichText::new(&first.label).size(11.0).color(MUTED));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.label(RichText::new(&last.label).size(11.0).color(MUTED));
        });
    });
}

fn quote_row(ui: &mut egui::Ui, name: &str, price: f64, pct: f64, digits: usize) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(name).size(13.0).color(MUTED));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.label(RichText::new(fmt_pct(pct)).size(13.0).color(dir_color(pct)));
            ui.label(RichText::new(fmt_grouped(price, digits)).size(13.0).color(CREAM));
        });
    });
}

fn stat(ui: &mut egui::Ui, name: &str, value: f64) {
    ui.label(RichText::new(format!("{name} {}", fmt_grouped(value, 2))).size(12.0).color(MUTED));
}

fn resize_grip(ui: &mut egui::Ui) {
    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
        let (rect, response) = ui.allocate_exact_size(Vec2::new(18.0, 14.0), Sense::drag());
        if response.drag_started() {
            ui.ctx()
                .send_viewport_cmd(ViewportCommand::BeginResize(egui::ResizeDirection::SouthEast));
        }
        if response.hovered() {
            ui.ctx().set_cursor_icon(CursorIcon::ResizeNwSe);
        }
        let painter = ui.painter();
        for i in 0..3 {
            let offset = i as f32 * 4.0;
            painter.line_segment(
                [
                    Pos2::new(rect.right() - 2.0 - offset, rect.bottom() - 2.0),
                    Pos2::new(rect.right() - 2.0, rect.bottom() - 2.0 - offset),
                ],
                Stroke::new(1.2, MUTED),
            );
        }
    });
}

fn dir_color(value: f64) -> Color32 {
    if value > 0.0 {
        UP
    } else if value < 0.0 {
        DOWN
    } else {
        MUTED
    }
}

fn fmt_grouped(value: f64, digits: usize) -> String {
    let negative = value < 0.0;
    let raw = format!("{:.*}", digits, value.abs());
    let (int, frac) = match raw.split_once('.') {
        Some((int, frac)) => (int.to_string(), format!(".{frac}")),
        None => (raw, String::new()),
    };
    let mut grouped = String::new();
    for (i, ch) in int.chars().rev().enumerate() {
        if i > 0 && i % 3 == 0 {
            grouped.push(',');
        }
        grouped.push(ch);
    }
    let int: String = grouped.chars().rev().collect();
    if negative {
        format!("-{int}{frac}")
    } else {
        format!("{int}{frac}")
    }
}

fn fmt_signed(value: f64, digits: usize) -> String {
    let body = fmt_grouped(value.abs(), digits);
    if value > 0.0 {
        format!("+{body}")
    } else if value < 0.0 {
        format!("-{body}")
    } else {
        body
    }
}

fn fmt_pct(value: f64) -> String {
    format!("{value:+.2}%")
}
