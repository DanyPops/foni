//! Playback buffer — ordered fill/drain queue for streaming TTS.
//!
//! Chunks arrive out of order from parallel TTS calls.
//! The buffer yields them in sequence for gapless playback.
//! Like a YouTube video buffer: play what you have, wait for what you don't.

use std::collections::HashMap;

/// Audio data for one chunk.
#[derive(Debug, Clone)]
pub struct AudioChunk {
    pub index: usize,
    pub samples: Vec<f32>,
    pub sample_rate: u32,
}

impl AudioChunk {
    pub fn duration_secs(&self) -> f32 {
        self.samples.len() as f32 / self.sample_rate as f32
    }
}

/// What the consumer should do.
#[derive(Debug, Clone, PartialEq)]
pub enum Yield {
    /// Chunk ready — play it.
    Chunk(usize),
    /// Next chunk hasn't arrived yet — wait or insert filler.
    Buffering { waiting_for: usize },
    /// All chunks played.
    Done,
}

/// Buffer status for monitoring.
#[derive(Debug, Clone, PartialEq)]
pub enum Status {
    /// Accepting chunks, not all submitted yet.
    Filling { received: usize, played: usize },
    /// All text submitted, draining remaining.
    Draining {
        received: usize,
        total: usize,
        played: usize,
    },
    /// Everything played.
    Done { total: usize },
}

pub struct PlaybackBuffer {
    chunks: HashMap<usize, AudioChunk>,
    next_play: usize,
    total: Option<usize>,
    received_count: usize,
}

impl PlaybackBuffer {
    pub fn new() -> Self {
        Self {
            chunks: HashMap::new(),
            next_play: 0,
            total: None,
            received_count: 0,
        }
    }

    /// Submit a chunk (may arrive out of order).
    pub fn submit(&mut self, chunk: AudioChunk) {
        self.received_count += 1;
        self.chunks.insert(chunk.index, chunk);
    }

    /// Signal that all text has been chunked — we know the total count.
    pub fn close(&mut self, total: usize) {
        self.total = Some(total);
    }

    /// Try to yield the next in-order chunk for playback.
    pub fn next(&mut self) -> Yield {
        if let Some(total) = self.total {
            if self.next_play >= total {
                return Yield::Done;
            }
        }

        if self.chunks.contains_key(&self.next_play) {
            let idx = self.next_play;
            self.next_play += 1;
            Yield::Chunk(idx)
        } else {
            Yield::Buffering {
                waiting_for: self.next_play,
            }
        }
    }

    /// Take the chunk data (moves it out of the buffer).
    pub fn take(&mut self, index: usize) -> Option<AudioChunk> {
        self.chunks.remove(&index)
    }

    /// Peek at a chunk without removing.
    pub fn peek(&self, index: usize) -> Option<&AudioChunk> {
        self.chunks.get(&index)
    }

    /// How many chunks are buffered ahead of the play cursor.
    pub fn buffered_ahead(&self) -> usize {
        let mut count = 0;
        let mut idx = self.next_play;
        while self.chunks.contains_key(&idx) {
            count += 1;
            idx += 1;
        }
        count
    }

    /// Total buffered audio duration ahead of the play cursor (seconds).
    pub fn buffered_secs(&self) -> f32 {
        let mut secs = 0.0;
        let mut idx = self.next_play;
        while let Some(chunk) = self.chunks.get(&idx) {
            secs += chunk.duration_secs();
            idx += 1;
        }
        secs
    }

    /// Current play position.
    pub fn play_cursor(&self) -> usize {
        self.next_play
    }

    /// Current status.
    pub fn status(&self) -> Status {
        match self.total {
            None => Status::Filling {
                received: self.received_count,
                played: self.next_play,
            },
            Some(total) if self.next_play >= total => Status::Done { total },
            Some(total) => Status::Draining {
                received: self.received_count,
                total,
                played: self.next_play,
            },
        }
    }

    /// Is everything done?
    pub fn is_complete(&self) -> bool {
        matches!(self.status(), Status::Done { .. })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chunk(index: usize, duration_ms: u32) -> AudioChunk {
        let sr = 24_000u32;
        let n = sr * duration_ms / 1000;
        AudioChunk {
            index,
            samples: vec![0.1; n as usize],
            sample_rate: sr,
        }
    }

    // ── Construction ──

    #[test]
    fn new_buffer_is_empty() {
        let buf = PlaybackBuffer::new();
        assert_eq!(buf.play_cursor(), 0);
        assert_eq!(buf.buffered_ahead(), 0);
        assert!(!buf.is_complete());
    }

    // ── Submit ──

    #[test]
    fn submit_in_order() {
        let mut buf = PlaybackBuffer::new();
        buf.submit(chunk(0, 500));
        buf.submit(chunk(1, 500));
        assert_eq!(buf.buffered_ahead(), 2);
    }

    #[test]
    fn submit_out_of_order() {
        let mut buf = PlaybackBuffer::new();
        buf.submit(chunk(2, 500));
        buf.submit(chunk(0, 500));
        buf.submit(chunk(1, 500));
        assert_eq!(buf.buffered_ahead(), 3);
    }

    // ── Next / Yield ──

    #[test]
    fn next_yields_in_order() {
        let mut buf = PlaybackBuffer::new();
        buf.submit(chunk(0, 500));
        buf.submit(chunk(1, 500));

        assert_eq!(buf.next(), Yield::Chunk(0));
        assert_eq!(buf.next(), Yield::Chunk(1));
    }

    #[test]
    fn next_buffers_when_gap() {
        let mut buf = PlaybackBuffer::new();
        buf.submit(chunk(1, 500)); // chunk 0 missing

        assert_eq!(buf.next(), Yield::Buffering { waiting_for: 0 });
    }

    #[test]
    fn next_resumes_after_gap_filled() {
        let mut buf = PlaybackBuffer::new();
        buf.submit(chunk(1, 500));

        assert_eq!(buf.next(), Yield::Buffering { waiting_for: 0 });

        buf.submit(chunk(0, 500)); // fill the gap

        assert_eq!(buf.next(), Yield::Chunk(0));
        assert_eq!(buf.next(), Yield::Chunk(1));
    }

    #[test]
    fn next_done_after_close() {
        let mut buf = PlaybackBuffer::new();
        buf.submit(chunk(0, 500));
        buf.submit(chunk(1, 500));
        buf.close(2);

        assert_eq!(buf.next(), Yield::Chunk(0));
        assert_eq!(buf.next(), Yield::Chunk(1));
        assert_eq!(buf.next(), Yield::Done);
    }

    #[test]
    fn next_buffers_before_done() {
        let mut buf = PlaybackBuffer::new();
        buf.submit(chunk(0, 500));
        buf.close(3); // expecting 3, only have 1

        assert_eq!(buf.next(), Yield::Chunk(0));
        assert_eq!(buf.next(), Yield::Buffering { waiting_for: 1 });
    }

    // ── Take / Peek ──

    #[test]
    fn take_removes_chunk() {
        let mut buf = PlaybackBuffer::new();
        buf.submit(chunk(0, 500));
        buf.next(); // advance cursor

        let taken = buf.take(0);
        assert!(taken.is_some());
        assert!(buf.take(0).is_none()); // gone
    }

    #[test]
    fn peek_without_consuming() {
        let mut buf = PlaybackBuffer::new();
        buf.submit(chunk(0, 500));

        assert!(buf.peek(0).is_some());
        assert!(buf.peek(0).is_some()); // still there
    }

    // ── Buffered metrics ──

    #[test]
    fn buffered_secs_counts_ahead() {
        let mut buf = PlaybackBuffer::new();
        buf.submit(chunk(0, 500));
        buf.submit(chunk(1, 1000));

        let secs = buf.buffered_secs();
        assert!((secs - 1.5).abs() < 0.05, "expected ~1.5s, got {secs}");
    }

    #[test]
    fn buffered_secs_skips_played() {
        let mut buf = PlaybackBuffer::new();
        buf.submit(chunk(0, 500));
        buf.submit(chunk(1, 500));
        buf.next(); // play chunk 0

        let secs = buf.buffered_secs();
        assert!((secs - 0.5).abs() < 0.05, "only chunk 1 buffered");
    }

    #[test]
    fn buffered_ahead_stops_at_gap() {
        let mut buf = PlaybackBuffer::new();
        buf.submit(chunk(0, 500));
        buf.submit(chunk(2, 500)); // gap at 1

        assert_eq!(buf.buffered_ahead(), 1); // only chunk 0 contiguous
    }

    // ── Status ──

    #[test]
    fn status_filling() {
        let mut buf = PlaybackBuffer::new();
        buf.submit(chunk(0, 500));
        assert!(matches!(
            buf.status(),
            Status::Filling {
                received: 1,
                played: 0
            }
        ));
    }

    #[test]
    fn status_draining() {
        let mut buf = PlaybackBuffer::new();
        buf.submit(chunk(0, 500));
        buf.close(3);
        assert!(matches!(
            buf.status(),
            Status::Draining {
                received: 1,
                total: 3,
                played: 0
            }
        ));
    }

    #[test]
    fn status_done() {
        let mut buf = PlaybackBuffer::new();
        buf.submit(chunk(0, 500));
        buf.close(1);
        buf.next();
        assert!(matches!(buf.status(), Status::Done { total: 1 }));
        assert!(buf.is_complete());
    }

    // ── Duration ──

    #[test]
    fn chunk_duration() {
        let c = chunk(0, 500);
        assert!((c.duration_secs() - 0.5).abs() < 0.01);
    }

    // ── Edge cases ──

    #[test]
    fn empty_close_immediate_done() {
        let mut buf = PlaybackBuffer::new();
        buf.close(0);
        assert_eq!(buf.next(), Yield::Done);
        assert!(buf.is_complete());
    }

    #[test]
    fn duplicate_submit_overwrites() {
        let mut buf = PlaybackBuffer::new();
        buf.submit(chunk(0, 500));
        buf.submit(chunk(0, 1000)); // overwrite

        let c = buf.peek(0).unwrap();
        assert!((c.duration_secs() - 1.0).abs() < 0.05);
    }
}
