use std::collections::VecDeque;
use std::io::Write;

use crate::math::{verify_buffer_invariants, InvariantViolation};

#[derive(Clone, Debug)]
pub struct Chunk {
    pub data: Vec<u8>,
    pub timestamp: f64,
}

pub struct RingBuffer {
    pub init_header: Vec<u8>,
    pub chunks: VecDeque<Chunk>,
    pub total_bytes: usize,
    pub limit_bytes: usize,
    pub max_duration_secs: f64,
    pub header_captured: bool,
}

impl RingBuffer {
    pub fn new(limit_mb: u32, max_minutes: u32) -> Self {
        let limit_bytes = (limit_mb as usize) * 1024 * 1024;
        let max_duration_secs = (max_minutes as f64) * 60.0;
        Self {
            init_header: Vec::new(),
            chunks: VecDeque::new(),
            total_bytes: 0,
            limit_bytes,
            max_duration_secs,
            header_captured: false,
        }
    }

    pub fn push(&mut self, data: Vec<u8>, timestamp: f64) {
        if !self.header_captured {
            let needed = 131072;
            self.init_header.extend_from_slice(&data);
            if self.init_header.len() >= needed || Self::has_first_moof(&self.init_header) {
                self.header_captured = true;
            }
            self.total_bytes = self.init_header.len();
            return;
        }

        let len = data.len();
        self.chunks.push_back(Chunk { data, timestamp });
        self.total_bytes += len;

        self.evict_expired(timestamp);
    }

    fn has_first_moof(buf: &[u8]) -> bool {
        buf.windows(4).any(|w| w == b"moof")
    }

    pub fn evict_expired(&mut self, current_time: f64) {
        while self.total_bytes > self.limit_bytes && !self.chunks.is_empty() {
            if let Some(evicted) = self.chunks.pop_front() {
                self.total_bytes -= evicted.data.len();
            }
        }

        if self.max_duration_secs > 0.0 {
            while let Some(front) = self.chunks.front() {
                if (current_time - front.timestamp) > self.max_duration_secs && self.chunks.len() > 1 {
                    if let Some(evicted) = self.chunks.pop_front() {
                        self.total_bytes -= evicted.data.len();
                    }
                } else {
                    break;
                }
            }
        }
    }

    pub fn clear(&mut self) {
        self.chunks.clear();
        self.chunks.shrink_to_fit();
        self.init_header.clear();
        self.init_header.shrink_to_fit();
        self.total_bytes = 0;
        self.header_captured = false;
    }

    pub fn verify_invariants(&self) -> Result<(), InvariantViolation> {
        let chunk_meta: Vec<(usize, f64)> = self.chunks.iter().map(|c| (c.data.len(), c.timestamp)).collect();
        verify_buffer_invariants(
            self.init_header.len(),
            &chunk_meta,
            self.total_bytes,
            self.limit_bytes,
            self.max_duration_secs,
        )
    }

    pub fn save_to_writer<W: Write>(&self, writer: &mut W) -> std::io::Result<usize> {
        let mut written = 0;
        if !self.init_header.is_empty() {
            writer.write_all(&self.init_header)?;
            written += self.init_header.len();
        }
        for chunk in &self.chunks {
            writer.write_all(&chunk.data)?;
            written += chunk.data.len();
        }
        writer.flush()?;
        Ok(written)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ring_buffer_invariants_hold_after_pushes() {
        let mut buf = RingBuffer::new(2, 5);
        buf.header_captured = true;

        for i in 0..50 {
            buf.push(vec![0xCC; 100 * 1024], i as f64);
            assert!(buf.verify_invariants().is_ok());
        }

        assert!(buf.total_bytes <= 2 * 1024 * 1024);
    }
}
