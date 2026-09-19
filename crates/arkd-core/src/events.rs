use std::{collections::VecDeque, time::SystemTime};

use serde_json::{Value, json};

use crate::messages::{BATTLE_WHAT, is_error, is_significant, message_name};

pub struct Event {
    pub seq: u64,
    pub at: SystemTime,
    pub message_id: i32,
    pub message: String,
    pub payload: Value,
}

pub fn iso(t: SystemTime) -> String {
    let secs = t
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;
    let days = secs.div_euclid(86400);
    let tod = secs.rem_euclid(86400);
    let (y, m, d) = civil_from_days(days);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}",
        tod / 3600,
        (tod % 3600) / 60,
        tod % 60
    )
}

fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719468;
    let era = z.div_euclid(146097);
    let doe = z.rem_euclid(146097);
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m, d)
}

impl Event {
    pub fn is_error(&self) -> bool {
        is_error(self.message_id)
    }

    pub fn describe(&self, include_payload: bool) -> Value {
        let mut out = json!({
            "seq": self.seq,
            "at": iso(self.at),
            "message": self.message,
            "message_id": self.message_id,
        });
        let map = out.as_object_mut().unwrap();
        if let Some(v) = self.payload.get("taskchain").filter(|v| !v.is_null()) {
            map.insert("taskchain".into(), v.clone());
        }
        if let Some(v) = self.payload.get("taskid").filter(|v| !v.is_null()) {
            map.insert("task_id".into(), v.clone());
        }
        if let Some(v) = self.payload.get("what").filter(|v| !v.is_null()) {
            map.insert("what".into(), v.clone());
        }
        if let Some(v) = self.payload.get("why").filter(|v| !v.is_null()) {
            map.insert("why".into(), v.clone());
        }
        let is_battle = self
            .payload
            .get("what")
            .and_then(|v| v.as_str())
            .map(|w| BATTLE_WHAT.contains(&w))
            .unwrap_or(false);
        if is_battle && let Some(v) = self.payload.get("details").filter(|v| !v.is_null()) {
            map.insert("details".into(), v.clone());
        }
        if include_payload {
            map.insert("payload".into(), self.payload.clone());
        }
        out
    }
}

pub struct EventLog {
    buf: VecDeque<Event>,
    cap: usize,
    seq: u64,
}

pub struct EventsPage {
    pub events: Vec<Value>,
    pub count: usize,
    pub next_seq: u64,
    pub latest_seq: u64,
    pub has_more: bool,
    pub dropped: u64,
}

impl EventLog {
    pub fn new(cap: usize) -> Self {
        Self {
            buf: VecDeque::with_capacity(cap),
            cap,
            seq: 0,
        }
    }

    pub fn push(&mut self, message_id: i32, payload: Value) -> &Event {
        self.seq += 1;
        if self.buf.len() >= self.cap {
            self.buf.pop_front();
        }
        self.buf.push_back(Event {
            seq: self.seq,
            at: SystemTime::now(),
            message_id,
            message: message_name(message_id),
            payload,
        });
        self.buf.back().unwrap()
    }

    pub fn iter(&self) -> impl Iterator<Item = &Event> {
        self.buf.iter()
    }

    pub fn latest_seq(&self) -> u64 {
        self.seq
    }

    pub fn page(
        &self,
        after_seq: u64,
        limit: usize,
        significant_only: bool,
        include_payload: bool,
    ) -> EventsPage {
        let candidates: Vec<&Event> = self
            .buf
            .iter()
            .filter(|e| e.seq > after_seq)
            .filter(|e| !significant_only || is_significant(e.message_id, &e.payload))
            .collect();
        let oldest_held = self.buf.front().map(|e| e.seq).unwrap_or(0);
        let page: Vec<&Event> = candidates.iter().take(limit).copied().collect();
        EventsPage {
            count: page.len(),
            next_seq: page.last().map(|e| e.seq).unwrap_or(after_seq),
            latest_seq: self.seq,
            has_more: candidates.len() > page.len(),
            dropped: if after_seq == 0 {
                0
            } else {
                oldest_held.saturating_sub(after_seq + 1)
            },
            events: page.iter().map(|e| e.describe(include_payload)).collect(),
        }
    }
}
