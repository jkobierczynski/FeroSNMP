//! What the Log tab shows: every operation started, every request sent with
//! its request-id, retries and timing, notices such as a fallback from
//! GETBULK to GETNEXT, and how each operation ended.

use std::collections::VecDeque;
use std::time::Duration;

use snmp_core::{EngineInfo, LogEntry, Summary, hex, uptime_text as uptime};

use crate::results::count_text;

/// How many lines the log keeps; older ones are dropped. A walk of 100,000
/// rows with 10 per request writes about 10,000.
pub const LIMIT: usize = 50_000;

/// A column of the Log list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogColumn {
    Time,
    Agent,
    Event,
}

impl LogColumn {
    /// Every column, in display order.
    pub const ALL: [LogColumn; 3] = [LogColumn::Time, LogColumn::Agent, LogColumn::Event];

    /// The column heading.
    pub fn title(self) -> &'static str {
        match self {
            LogColumn::Time => "Time",
            LogColumn::Agent => "Agent",
            LogColumn::Event => "Event",
        }
    }

    /// Width at 96 DPI.
    pub fn width(self) -> i32 {
        match self {
            LogColumn::Time => 95,
            LogColumn::Agent => 160,
            LogColumn::Event => 640,
        }
    }
}

/// One line of the log.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogLine {
    /// Local time, such as `14:03:22.120`.
    pub time: String,
    /// The agent, as `host:port`.
    pub agent: String,
    /// What happened.
    pub text: String,
}

/// The log, oldest line first.
#[derive(Debug, Default)]
pub struct OpLog {
    lines: VecDeque<LogLine>,
}

impl OpLog {
    /// Add a line, dropping the oldest when the log is full.
    pub fn push(&mut self, time: String, agent: &str, text: String) {
        if self.lines.len() == LIMIT {
            self.lines.pop_front();
        }
        self.lines.push_back(LogLine {
            time,
            agent: agent.to_string(),
            text,
        });
    }

    /// Number of lines.
    pub fn len(&self) -> usize {
        self.lines.len()
    }

    /// The text of one cell.
    pub fn cell(&self, row: usize, column: LogColumn) -> &str {
        match self.lines.get(row) {
            Some(line) => match column {
                LogColumn::Time => &line.time,
                LogColumn::Agent => &line.agent,
                LogColumn::Event => &line.text,
            },
            None => "",
        }
    }

    /// Some lines as tab-separated text, for Ctrl+C.
    pub fn tab_separated(&self, rows: impl IntoIterator<Item = usize>) -> String {
        let mut text = String::new();
        for line in rows.into_iter().filter_map(|row| self.lines.get(row)) {
            text.push_str(&format!("{}\t{}\t{}\r\n", line.time, line.agent, line.text));
        }
        text
    }
}

/// A request as the log shows it: `GETBULK 1.3.6.1.2.1.2 (...), request-id
/// 1234, retry 1: 10 varbinds in 1.2 ms`. The agent has a column of its own.
pub fn request_text(entry: &LogEntry) -> String {
    let mut text = format!("{}, request-id {}", entry.request, entry.request_id);
    if entry.attempt > 1 {
        text.push_str(&format!(", retry {}", entry.attempt - 1));
    }
    text.push_str(&format!(": {} in {}", entry.result, millis(entry.elapsed)));
    text
}

/// How an operation finished: `Done: 1,234 values, 124 requests, 0.8 s`.
pub fn summary_text(summary: &Summary) -> String {
    format!(
        "Done: {} value{}, {} request{}, {}",
        count_text(summary.varbinds),
        if summary.varbinds == 1 { "" } else { "s" },
        count_text(summary.requests as usize),
        if summary.requests == 1 { "" } else { "s" },
        seconds(summary.elapsed)
    )
}

/// An SNMPv3 agent's engine:
/// `SNMPv3 engine ID 80001f88..., boots 3, time 1 d 2 h (discovered)`.
pub fn engine_text(info: &EngineInfo) -> String {
    format!(
        "SNMPv3 engine ID {}, boots {}, up {} ({})",
        hex(&info.engine_id),
        info.boots,
        uptime(info.time),
        if info.fresh {
            "discovered"
        } else {
            "remembered"
        }
    )
}

/// A duration as `1.2 ms`.
pub fn millis(duration: Duration) -> String {
    format!("{:.1} ms", duration.as_secs_f64() * 1000.0)
}

/// A duration as `0.8 s`, or in milliseconds when shorter than 0.1 s.
pub fn seconds(duration: Duration) -> String {
    if duration < Duration::from_millis(100) {
        millis(duration)
    } else {
        format!("{:.1} s", duration.as_secs_f64())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_and_limit() {
        let mut log = OpLog::default();
        for i in 0..LIMIT + 5 {
            log.push(format!("t{i}"), "h:161", format!("line {i}"));
        }
        assert_eq!(log.len(), LIMIT);
        assert_eq!(log.cell(0, LogColumn::Time), "t5");
        assert_eq!(log.cell(0, LogColumn::Event), "line 5");
        assert_eq!(log.cell(LIMIT, LogColumn::Agent), "");
        assert_eq!(log.tab_separated([0]), "t5\th:161\tline 5\r\n");
    }

    #[test]
    fn texts() {
        let entry = LogEntry {
            target: "h:161".into(),
            request: "GET 1.3.6.1.2.1.1.1.0".into(),
            request_id: 42,
            attempt: 2,
            elapsed: Duration::from_micros(1250),
            result: "1 varbind".into(),
        };
        assert_eq!(
            request_text(&entry),
            "GET 1.3.6.1.2.1.1.1.0, request-id 42, retry 1: 1 varbind in 1.2 ms"
        );
        let summary = Summary {
            varbinds: 100_000,
            requests: 1,
            elapsed: Duration::from_millis(2500),
        };
        assert_eq!(
            summary_text(&summary),
            "Done: 100,000 values, 1 request, 2.5 s"
        );
    }
}
