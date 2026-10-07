use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub const FOCUS: Duration = Duration::from_secs(25 * 60);
pub const SHORT_BREAK: Duration = Duration::from_secs(5 * 60);
pub const LONG_BREAK: Duration = Duration::from_secs(15 * 60);
pub const LONG_EVERY: u32 = 4;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Phase {
    Focus,
    ShortBreak,
    LongBreak,
}

impl Phase {
    pub fn label(self) -> &'static str {
        match self {
            Phase::Focus => "专注",
            Phase::ShortBreak => "短休",
            Phase::LongBreak => "长休",
        }
    }

    pub fn duration(self) -> Duration {
        match self {
            Phase::Focus => FOCUS,
            Phase::ShortBreak => SHORT_BREAK,
            Phase::LongBreak => LONG_BREAK,
        }
    }

    fn as_key(self) -> &'static str {
        match self {
            Phase::Focus => "focus",
            Phase::ShortBreak => "short",
            Phase::LongBreak => "long",
        }
    }

    fn from_key(key: &str) -> Option<Self> {
        match key {
            "focus" => Some(Phase::Focus),
            "short" => Some(Phase::ShortBreak),
            "long" => Some(Phase::LongBreak),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum NoteStatus {
    /// 这一轮正在走时。
    Running,
    /// 已经按下开始，当前停着。
    Paused,
    /// 二十五分钟走完。
    Done,
    /// 跳过、重置，或被新的一轮打断。
    Interrupted,
}

impl NoteStatus {
    pub fn label(self) -> &'static str {
        match self {
            NoteStatus::Running => "进行中",
            NoteStatus::Paused => "已暂停",
            NoteStatus::Done => "已完成",
            NoteStatus::Interrupted => "已中断",
        }
    }

    fn as_key(self) -> &'static str {
        match self {
            NoteStatus::Running => "running",
            NoteStatus::Paused => "paused",
            NoteStatus::Done => "done",
            NoteStatus::Interrupted => "interrupted",
        }
    }

    fn from_key(key: &str) -> Option<Self> {
        match key {
            "running" => Some(NoteStatus::Running),
            "paused" => Some(NoteStatus::Paused),
            "done" => Some(NoteStatus::Done),
            "interrupted" => Some(NoteStatus::Interrupted),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FocusNote {
    pub task: String,
    pub started_ms: u64,
    pub ended_ms: u64,
    pub secs: u64,
    /// 计时走完为 true，中途停下为 false。
    pub finished: bool,
}

impl FocusNote {
    pub fn status(&self) -> NoteStatus {
        if self.finished {
            NoteStatus::Done
        } else {
            NoteStatus::Interrupted
        }
    }
}

/// 记录列表里的一行。进行中和已暂停还没写入历史，查的时候补在最后。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HistoryRow {
    pub task: String,
    pub started_ms: u64,
    pub ended_ms: Option<u64>,
    pub secs: u64,
    pub status: NoteStatus,
}

#[derive(Clone, Debug)]
pub struct Pomodoro {
    phase: Phase,
    /// 正在走时。到点后切到下一阶段并停下，等手动开始。
    deadline: Option<SystemTime>,
    remaining: Duration,
    /// 已经完成的专注次数。
    focus_done: u32,
    /// 这一轮专注要做的事。完成或跳过时按框里当时的文字记录。
    task: String,
    /// 本轮专注第一次按下开始的时刻。暂停后续上不改。
    focus_begun: Option<u64>,
    log: Vec<FocusNote>,
}

impl Default for Pomodoro {
    fn default() -> Self {
        Self::new()
    }
}

impl Pomodoro {
    pub fn new() -> Self {
        Self {
            phase: Phase::Focus,
            deadline: None,
            remaining: FOCUS,
            focus_done: 0,
            task: String::new(),
            focus_begun: None,
            log: Vec::new(),
        }
    }

    #[allow(dead_code)]
    pub fn task(&self) -> &str {
        &self.task
    }

    pub fn task_mut(&mut self) -> &mut String {
        &mut self.task
    }

    pub fn notes(&self) -> &[FocusNote] {
        &self.log
    }

    /// 已结束的记录，再加上当前这一轮（进行中或已暂停）。
    pub fn history(&self, now: SystemTime) -> Vec<HistoryRow> {
        let mut rows: Vec<_> = self
            .log
            .iter()
            .map(|note| HistoryRow {
                task: note.task.clone(),
                started_ms: note.started_ms,
                ended_ms: Some(note.ended_ms),
                secs: note.secs,
                status: note.status(),
            })
            .collect();
        if self.phase == Phase::Focus {
            if let Some(started) = self.focus_begun {
                let secs = self.phase.duration().saturating_sub(self.remaining(now)).as_secs();
                rows.push(HistoryRow {
                    task: self.task.trim().to_string(),
                    started_ms: started,
                    ended_ms: None,
                    secs,
                    status: if self.running() {
                        NoteStatus::Running
                    } else {
                        NoteStatus::Paused
                    },
                });
            }
        }
        rows
    }

    pub fn phase(&self) -> Phase {
        self.phase
    }

    pub fn running(&self) -> bool {
        self.deadline.is_some()
    }

    pub fn focus_done(&self) -> u32 {
        self.focus_done
    }

    /// 当前这一轮里的第几个专注，1 到 4。
    pub fn round_index(&self) -> u32 {
        match self.phase {
            Phase::Focus => self.focus_done % LONG_EVERY + 1,
            Phase::ShortBreak | Phase::LongBreak => {
                let done = self.focus_done % LONG_EVERY;
                if done == 0 { LONG_EVERY } else { done }
            }
        }
    }

    pub fn remaining(&self, now: SystemTime) -> Duration {
        if let Some(end) = self.deadline {
            end.duration_since(now).unwrap_or(Duration::ZERO)
        } else {
            self.remaining
        }
    }

    pub fn progress(&self, now: SystemTime) -> f32 {
        let total = self.phase.duration().as_secs_f32().max(1.0);
        let left = self.remaining(now).as_secs_f32();
        (1.0 - left / total).clamp(0.0, 1.0)
    }

    pub fn toggle(&mut self, now: SystemTime) {
        if let Some(end) = self.deadline.take() {
            self.remaining = end.duration_since(now).unwrap_or(Duration::ZERO);
            if self.remaining.is_zero() {
                self.note_focus(end, true);
                self.complete_current();
            }
        } else {
            if self.phase == Phase::Focus && self.focus_begun.is_none() {
                self.focus_begun = Some(millis(now));
            }
            if self.remaining.is_zero() {
                self.remaining = self.phase.duration();
            }
            self.deadline = now.checked_add(self.remaining);
        }
    }

    /// 结束当前阶段，停在下一阶段，等手动开始。
    pub fn skip(&mut self, now: SystemTime) {
        self.note_focus(now, false);
        self.deadline = None;
        self.complete_current();
    }

    pub fn reset(&mut self, now: SystemTime) {
        if self.phase == Phase::Focus && self.focus_begun.is_some() {
            let task = self.task.clone();
            self.note_focus(now, false);
            self.task = task;
        }
        self.deadline = None;
        self.remaining = self.phase.duration();
        if self.phase == Phase::Focus {
            self.focus_begun = None;
        }
    }

    pub fn has_open_focus(&self) -> bool {
        self.phase == Phase::Focus && self.focus_begun.is_some()
    }

    /// 立刻开始一轮新的 25 分钟专注。已开始的专注按原描述记成已中断。
    pub fn start_new_focus(&mut self, now: SystemTime, task: &str) {
        let task = task.trim().to_string();
        if self.has_open_focus() {
            self.note_focus(now, false);
        }
        self.deadline = None;
        self.phase = Phase::Focus;
        self.remaining = FOCUS;
        self.task = task;
        self.focus_begun = Some(millis(now));
        self.deadline = now.checked_add(FOCUS);
    }

    /// 当前阶段到点则切到下一阶段并停下。返回 1 表示刚完成一段。
    pub fn settle(&mut self, now: SystemTime) -> u32 {
        let Some(end) = self.deadline else {
            return 0;
        };
        if now < end {
            return 0;
        }
        self.note_focus(end, true);
        self.deadline = None;
        self.complete_current();
        1
    }

    pub fn save(&self, now: SystemTime) -> String {
        let left = self.remaining(now).as_millis();
        let deadline = self
            .deadline
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map(|d| d.as_millis())
            .unwrap_or(0);
        format!("{},{left},{deadline},{}", self.phase.as_key(), self.focus_done)
    }

    pub fn load(raw: &str) -> Option<Self> {
        let mut parts = raw.split(',');
        let phase = Phase::from_key(parts.next()?)?;
        let left: u64 = parts.next()?.parse().ok()?;
        let deadline_ms: u64 = parts.next()?.parse().ok()?;
        let focus_done: u32 = parts.next()?.parse().ok()?;
        let deadline = (deadline_ms > 0).then(|| UNIX_EPOCH + Duration::from_millis(deadline_ms));
        Some(Self {
            phase,
            deadline,
            remaining: Duration::from_millis(left),
            focus_done,
            task: String::new(),
            focus_begun: None,
            log: Vec::new(),
        })
    }

    pub fn save_task(&self) -> String {
        serde_json::json!({
            "task": self.task,
            "begun": self.focus_begun,
        })
        .to_string()
    }

    pub fn load_task(&mut self, raw: &str) {
        let Ok(value) = serde_json::from_str::<serde_json::Value>(raw) else {
            return;
        };
        if let Some(task) = value.get("task").and_then(|v| v.as_str()) {
            self.task = task.to_string();
        }
        self.focus_begun = value.get("begun").and_then(|v| v.as_u64());
    }

    pub fn save_log(&self) -> String {
        let notes: Vec<_> = self
            .log
            .iter()
            .map(|note| {
                serde_json::json!({
                    "task": note.task,
                    "started": note.started_ms,
                    "ended": note.ended_ms,
                    "secs": note.secs,
                    "done": note.finished,
                    "status": note.status().as_key(),
                })
            })
            .collect();
        serde_json::Value::Array(notes).to_string()
    }

    pub fn load_log(&mut self, raw: &str) {
        let Ok(value) = serde_json::from_str::<serde_json::Value>(raw) else {
            return;
        };
        let Some(items) = value.as_array() else {
            return;
        };
        self.log = items
            .iter()
            .filter_map(|item| {
                Some(FocusNote {
                    task: item.get("task")?.as_str()?.to_string(),
                    started_ms: item.get("started")?.as_u64()?,
                    ended_ms: item.get("ended")?.as_u64()?,
                    secs: item.get("secs")?.as_u64()?,
                    finished: stored_finished(item)?,
                })
            })
            .collect();
    }

    fn complete_current(&mut self) {
        if self.phase == Phase::Focus {
            self.focus_done = self.focus_done.saturating_add(1);
        }
        self.phase = match self.phase {
            Phase::Focus if self.focus_done.is_multiple_of(LONG_EVERY) => Phase::LongBreak,
            Phase::Focus => Phase::ShortBreak,
            Phase::ShortBreak | Phase::LongBreak => Phase::Focus,
        };
        self.remaining = self.phase.duration();
    }

    fn note_focus(&mut self, ended_at: SystemTime, finished: bool) {
        if self.phase != Phase::Focus {
            return;
        }
        if self.focus_begun.is_none() && !finished {
            return;
        }
        let ended = millis(ended_at);
        let started = self.focus_begun.unwrap_or(ended);
        let secs = if finished {
            self.phase.duration().as_secs()
        } else {
            self.phase.duration().saturating_sub(self.remaining(ended_at)).as_secs()
        };
        self.log.push(FocusNote {
            task: self.task.trim().to_string(),
            started_ms: started,
            ended_ms: ended,
            secs,
            finished,
        });
        if self.log.len() > 200 {
            let extra = self.log.len() - 200;
            self.log.drain(0..extra);
        }
        self.task.clear();
        self.focus_begun = None;
    }
}

fn stored_finished(item: &serde_json::Value) -> Option<bool> {
    match item.get("status").and_then(|v| v.as_str()).and_then(NoteStatus::from_key) {
        Some(NoteStatus::Done) => Some(true),
        Some(NoteStatus::Interrupted) => Some(false),
        Some(NoteStatus::Running | NoteStatus::Paused) => None,
        None => item.get("done").and_then(|v| v.as_bool()),
    }
}

fn millis(time: SystemTime) -> u64 {
    time.duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

pub fn format_remaining(duration: Duration) -> String {
    let total = duration.as_secs();
    format!("{:02}:{:02}", total / 60, total % 60)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(secs: u64) -> SystemTime {
        UNIX_EPOCH + Duration::from_secs(secs)
    }

    #[test]
    fn starts_on_a_fresh_focus() {
        let pomo = Pomodoro::new();
        assert_eq!(pomo.phase(), Phase::Focus);
        assert!(!pomo.running());
        assert_eq!(pomo.remaining(at(0)), FOCUS);
        assert_eq!(format_remaining(FOCUS), "25:00");
    }

    #[test]
    fn finish_waits_for_a_manual_start() {
        let mut pomo = Pomodoro::new();
        let start = at(1_000_000);
        pomo.toggle(start);
        let later = start + FOCUS + Duration::from_secs(30);
        assert_eq!(pomo.settle(later), 1);
        assert_eq!(pomo.phase(), Phase::ShortBreak);
        assert!(!pomo.running());
        assert_eq!(pomo.focus_done(), 1);
        assert_eq!(pomo.remaining(later), SHORT_BREAK);
        assert_eq!(pomo.settle(later + Duration::from_secs(3600)), 0);
        pomo.toggle(later);
        assert!(pomo.running());
        assert_eq!(pomo.phase(), Phase::ShortBreak);
    }

    #[test]
    fn fourth_focus_waits_on_a_long_break() {
        let mut pomo = Pomodoro::new();
        let mut now = at(1_000_000);
        let next = [
            Phase::ShortBreak,
            Phase::Focus,
            Phase::ShortBreak,
            Phase::Focus,
            Phase::ShortBreak,
            Phase::Focus,
            Phase::LongBreak,
        ];
        for expected in next {
            pomo.toggle(now);
            now += pomo.phase().duration() + Duration::from_secs(1);
            assert_eq!(pomo.settle(now), 1);
            assert_eq!(pomo.phase(), expected);
            assert!(!pomo.running());
        }
        assert_eq!(pomo.focus_done(), 4);
        assert_eq!(pomo.remaining(now), LONG_BREAK);
    }

    #[test]
    fn skip_while_paused_does_not_start() {
        let mut pomo = Pomodoro::new();
        pomo.skip(at(50));
        assert_eq!(pomo.phase(), Phase::ShortBreak);
        assert!(!pomo.running());
        assert_eq!(pomo.remaining(at(50)), SHORT_BREAK);
        assert_eq!(pomo.focus_done(), 1);
    }

    #[test]
    fn pause_keeps_the_time_left() {
        let mut pomo = Pomodoro::new();
        let start = at(10);
        pomo.toggle(start);
        let mid = start + Duration::from_secs(65);
        pomo.toggle(mid);
        assert!(!pomo.running());
        assert_eq!(pomo.remaining(mid), FOCUS - Duration::from_secs(65));
    }

    #[test]
    fn save_roundtrip_keeps_a_running_deadline() {
        let mut pomo = Pomodoro::new();
        let start = at(5_000);
        pomo.toggle(start);
        let raw = pomo.save(start + Duration::from_secs(10));
        let loaded = Pomodoro::load(&raw).expect("load");
        assert!(loaded.running());
        assert_eq!(loaded.phase(), Phase::Focus);
        assert_eq!(loaded.remaining(start + Duration::from_secs(10)), FOCUS - Duration::from_secs(10));
    }

    #[test]
    fn finished_focus_keeps_the_task() {
        let mut pomo = Pomodoro::new();
        let start = at(1_700_000_000);
        pomo.task_mut().push_str("  写周报  ");
        pomo.toggle(start);
        let end = start + FOCUS + Duration::from_secs(3);
        assert_eq!(pomo.settle(end), 1);
        let notes = pomo.notes();
        assert_eq!(notes.len(), 1);
        assert_eq!(notes[0].task, "写周报");
        assert!(notes[0].finished);
        assert_eq!(notes[0].secs, FOCUS.as_secs());
        assert_eq!(notes[0].started_ms, millis(start));
        assert!(pomo.task().is_empty());
        let raw = pomo.save_log();
        let mut loaded = Pomodoro::new();
        loaded.load_log(&raw);
        assert_eq!(loaded.notes(), notes);
    }

    #[test]
    fn skip_records_the_time_already_spent() {
        let mut pomo = Pomodoro::new();
        let start = at(50_000);
        pomo.task_mut().push_str("读论文");
        pomo.toggle(start);
        let mid = start + Duration::from_secs(65);
        pomo.skip(mid);
        let note = &pomo.notes()[0];
        assert_eq!(note.task, "读论文");
        assert!(!note.finished);
        assert_eq!(note.secs, 65);
        assert_eq!(pomo.phase(), Phase::ShortBreak);
    }

    #[test]
    fn skip_before_start_does_not_record() {
        let mut pomo = Pomodoro::new();
        pomo.task_mut().push_str("还没开始");
        pomo.skip(at(10));
        assert!(pomo.notes().is_empty());
        assert_eq!(pomo.task(), "还没开始");
    }

    #[test]
    fn new_focus_starts_from_a_break_and_keeps_the_task() {
        let mut pomo = Pomodoro::new();
        let start = at(1_000);
        pomo.toggle(start);
        pomo.settle(start + FOCUS);
        assert_eq!(pomo.phase(), Phase::ShortBreak);
        let now = start + FOCUS + Duration::from_secs(20);
        pomo.start_new_focus(now, "写方案");
        assert_eq!(pomo.phase(), Phase::Focus);
        assert!(pomo.running());
        assert_eq!(pomo.task(), "写方案");
        assert_eq!(pomo.remaining(now), FOCUS);
        assert_eq!(pomo.notes().len(), 1);
    }

    #[test]
    fn new_focus_records_the_one_already_running() {
        let mut pomo = Pomodoro::new();
        let start = at(5_000);
        pomo.task_mut().push_str("旧任务");
        pomo.toggle(start);
        let now = start + Duration::from_secs(90);
        pomo.start_new_focus(now, "  新任务  ");
        assert_eq!(pomo.notes().len(), 1);
        assert_eq!(pomo.notes()[0].task, "旧任务");
        assert!(!pomo.notes()[0].finished);
        assert_eq!(pomo.notes()[0].secs, 90);
        assert!(pomo.running());
        assert_eq!(pomo.task(), "新任务");
        assert_eq!(pomo.remaining(now), FOCUS);
    }

    #[test]
    fn history_shows_a_running_focus_then_a_pause() {
        let mut pomo = Pomodoro::new();
        let start = at(1_700_000_000);
        pomo.task_mut().push_str("写方案");
        pomo.toggle(start);
        let mid = start + Duration::from_secs(90);
        let live = pomo.history(mid);
        assert_eq!(live.len(), 1);
        assert_eq!(live[0].status, NoteStatus::Running);
        assert_eq!(live[0].task, "写方案");
        assert_eq!(live[0].started_ms, millis(start));
        assert_eq!(live[0].ended_ms, None);
        assert_eq!(live[0].secs, 90);
        pomo.toggle(mid);
        let paused = pomo.history(mid);
        assert_eq!(paused[0].status, NoteStatus::Paused);
        assert_eq!(paused[0].ended_ms, None);
        assert_eq!(paused[0].secs, 90);
    }

    #[test]
    fn finished_history_ends_at_the_deadline() {
        let mut pomo = Pomodoro::new();
        let start = at(1_700_000_000);
        pomo.task_mut().push_str("写周报");
        pomo.toggle(start);
        let late = start + FOCUS + Duration::from_secs(40);
        assert_eq!(pomo.settle(late), 1);
        let row = &pomo.history(late)[0];
        assert_eq!(row.status, NoteStatus::Done);
        assert_eq!(row.started_ms, millis(start));
        assert_eq!(row.ended_ms, Some(millis(start + FOCUS)));
        assert_eq!(row.secs, FOCUS.as_secs());
        assert_eq!(row.task, "写周报");
    }

    #[test]
    fn skip_history_is_interrupted_between_two_clocks() {
        let mut pomo = Pomodoro::new();
        let start = at(90_000);
        pomo.task_mut().push_str("读论文");
        pomo.toggle(start);
        let mid = start + Duration::from_secs(8 * 60);
        pomo.skip(mid);
        let row = &pomo.history(mid)[0];
        assert_eq!(row.status, NoteStatus::Interrupted);
        assert_eq!(row.started_ms, millis(start));
        assert_eq!(row.ended_ms, Some(millis(mid)));
        assert_eq!(row.secs, 8 * 60);
        assert_eq!(row.task, "读论文");
    }

    #[test]
    fn reset_records_an_interruption_and_keeps_the_task() {
        let mut pomo = Pomodoro::new();
        let start = at(80_000);
        pomo.task_mut().push_str("草稿");
        pomo.toggle(start);
        let mid = start + Duration::from_secs(30);
        pomo.reset(mid);
        assert_eq!(pomo.phase(), Phase::Focus);
        assert!(!pomo.running());
        assert_eq!(pomo.task(), "草稿");
        assert_eq!(pomo.history(mid).len(), 1);
        assert_eq!(pomo.history(mid)[0].status, NoteStatus::Interrupted);
        assert_eq!(pomo.history(mid)[0].secs, 30);
        assert_eq!(pomo.history(mid)[0].ended_ms, Some(millis(mid)));
    }

    #[test]
    fn old_log_without_status_still_loads() {
        let mut pomo = Pomodoro::new();
        pomo.load_log(
            r#"[{"task":"旧记录","started":10,"ended":20,"secs":25,"done":true},{"task":"半途","started":30,"ended":40,"secs":5,"done":false}]"#,
        );
        assert_eq!(pomo.notes()[0].status(), NoteStatus::Done);
        assert_eq!(pomo.notes()[1].status(), NoteStatus::Interrupted);
    }

    #[test]
    fn task_text_roundtrips() {
        let mut pomo = Pomodoro::new();
        pomo.task_mut().push_str("带，逗号");
        pomo.toggle(at(80));
        let raw = pomo.save_task();
        let mut loaded = Pomodoro::load(&pomo.save(at(90))).expect("timer");
        loaded.load_task(&raw);
        assert_eq!(loaded.task(), "带，逗号");
        assert_eq!(loaded.notes().len(), 0);
        assert!(loaded.running());
    }
}
