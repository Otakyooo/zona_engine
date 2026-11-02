//! Дешёвый замер кадра: сколько времени уходит на пересборку геометрии
//! относительно длительности самого кадра.
//!
//! GPU-таймстемпы (`TIMESTAMP_QUERY`) здесь не используются намеренно: они
//! vendor-специфичны, а вопрос сейчас не «сколько стоит растеризация», а
//! «не пересобираем ли мы меши по двести раз в секунду».
//!
//! Отчёт печатается не чаще раза в секунду, чтобы сам замер не стал нагрузкой.

use std::time::{Duration, Instant};

const REPORT_INTERVAL: Duration = Duration::from_secs(1);

#[derive(Default)]
pub struct PerfStats {
    window_start: Option<Instant>,
    last_frame: Option<Instant>,
    frames: u32,
    frame_secs_total: f64,
    frame_secs_worst: f64,
    draw_items: usize,
    rebuilds: u32,
    rebuild_secs_total: f64,
}

impl PerfStats {
    pub fn new() -> Self {
        Self::default()
    }

    /// Начало кадра. Первый вызов только запоминает время: `dt` считать не от чего.
    pub fn begin_frame(&mut self, now: Instant) {
        if self.window_start.is_none() {
            self.window_start = Some(now);
        }
        if let Some(prev) = self.last_frame {
            let dt = now.saturating_duration_since(prev).as_secs_f64();
            self.frames += 1;
            self.frame_secs_total += dt;
            if dt > self.frame_secs_worst {
                self.frame_secs_worst = dt;
            }
        }
        self.last_frame = Some(now);
    }

    /// Один пересозданный меш (новые вершинный и индексный буферы на GPU).
    pub fn mesh_rebuild(&mut self, elapsed: Duration) {
        self.rebuilds += 1;
        self.rebuild_secs_total += elapsed.as_secs_f64();
    }

    /// Конец кадра: сколько элементов реально ушло в отрисовку.
    pub fn end_frame(&mut self, draw_items: usize) {
        self.draw_items = draw_items;
    }

    /// Раз в секунду отдаёт готовую строку и сбрасывает счётчики.
    /// `rebuild_share` — доля времени окна, ушедшая на пересборку геометрии.
    pub fn take_report(&mut self, now: Instant) -> Option<String> {
        let start = self.window_start?;
        let window = now.saturating_duration_since(start);
        if window < REPORT_INTERVAL || self.frames == 0 {
            return None;
        }

        let secs = window.as_secs_f64();
        let frames = self.frames as f64;
        let fps = frames / secs;
        let frame_avg_ms = self.frame_secs_total / frames * 1000.0;
        let frame_worst_ms = self.frame_secs_worst * 1000.0;
        let rebuild_avg_ms = if self.rebuilds == 0 {
            0.0
        } else {
            self.rebuild_secs_total / self.rebuilds as f64 * 1000.0
        };
        let rebuilds_per_sec = self.rebuilds as f64 / secs;
        let rebuild_share = self.rebuild_secs_total / secs * 100.0;

        let line = format!(
            "fps={fps:.1} frame_avg={frame_avg_ms:.2}ms frame_worst={frame_worst_ms:.2}ms \
             draw={} rebuilds/s={rebuilds_per_sec:.0} rebuild_avg={rebuild_avg_ms:.3}ms \
             rebuild_share={rebuild_share:.1}%",
            self.draw_items
        );

        self.window_start = Some(now);
        self.frames = 0;
        self.frame_secs_total = 0.0;
        self.frame_secs_worst = 0.0;
        self.rebuilds = 0;
        self.rebuild_secs_total = 0.0;

        Some(line)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn report_aggregates_and_resets() {
        let mut perf = PerfStats::new();
        let t0 = Instant::now();
        perf.begin_frame(t0);

        assert!(
            perf.take_report(t0 + Duration::from_millis(500)).is_none(),
            "до истечения окна отчёта нет"
        );

        // Три кадра по 10 мс и две пересборки геометрии.
        for i in 1..=3u32 {
            perf.begin_frame(t0 + Duration::from_millis(10) * i);
            perf.end_frame(7);
        }
        perf.mesh_rebuild(Duration::from_millis(1));
        perf.mesh_rebuild(Duration::from_millis(2));

        let now = t0 + Duration::from_secs(1);
        let line = perf.take_report(now).expect("через секунду отчёт есть");

        assert!(line.contains("fps=3.0"), "{line}");
        assert!(line.contains("draw=7"), "{line}");
        assert!(line.contains("rebuilds/s=2"), "{line}");
        // 3 мс пересборки за окно в 1 с.
        assert!(line.contains("rebuild_share=0.3%"), "{line}");

        assert!(
            perf.take_report(now + Duration::from_millis(1)).is_none(),
            "после сброса счётчики пусты и отчёт не повторяется"
        );
    }
}
