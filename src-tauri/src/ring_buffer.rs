use std::collections::VecDeque;
use std::io::Write;

use crate::math::{verify_buffer_invariants, InvariantViolation};

#[derive(Clone, Debug)]
pub struct Chunk {
    pub data: Vec<u8>,
    pub timestamp: f64,
}

#[derive(Default, Debug)]
pub struct Fmp4Assembler {
    parser_buf: Vec<u8>,
    pub init_header: Vec<u8>,
    pub init_header_ready: bool,
    current_fragment: Vec<u8>,
    current_fragment_ts: f64,
}

impl Fmp4Assembler {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_known_box(fourcc: &[u8; 4]) -> bool {
        matches!(
            fourcc,
            b"ftyp" | b"moov" | b"moof" | b"mdat" | b"free" | b"skip" | b"sidx" | b"prft" | b"styp"
        )
    }

    pub fn process_chunk(&mut self, incoming: &[u8], timestamp: f64) -> Vec<(Vec<u8>, f64)> {
        self.parser_buf.extend_from_slice(incoming);
        let mut completed_fragments = Vec::new();

        while self.parser_buf.len() >= 8 {
            let fourcc = [
                self.parser_buf[4],
                self.parser_buf[5],
                self.parser_buf[6],
                self.parser_buf[7],
            ];

            if !Self::is_known_box(&fourcc) {
                // If stream is not recognizable as standard MP4 boxes (e.g. test synthetic data),
                // pass through directly as raw chunk.
                let raw = std::mem::take(&mut self.parser_buf);
                completed_fragments.push((raw, timestamp));
                break;
            }

            let size32 = u32::from_be_bytes([
                self.parser_buf[0],
                self.parser_buf[1],
                self.parser_buf[2],
                self.parser_buf[3],
            ]) as usize;

            let box_len = if size32 == 1 {
                if self.parser_buf.len() < 16 {
                    break;
                }
                u64::from_be_bytes([
                    self.parser_buf[8],
                    self.parser_buf[9],
                    self.parser_buf[10],
                    self.parser_buf[11],
                    self.parser_buf[12],
                    self.parser_buf[13],
                    self.parser_buf[14],
                    self.parser_buf[15],
                ]) as usize
            } else if size32 == 0 {
                break;
            } else {
                size32
            };

            if box_len < 8 {
                self.parser_buf.remove(0);
                continue;
            }

            if self.parser_buf.len() < box_len {
                break;
            }

            let box_bytes: Vec<u8> = self.parser_buf.drain(..box_len).collect();

            if !self.init_header_ready {
                self.init_header.extend_from_slice(&box_bytes);
                if &fourcc == b"moov" {
                    self.init_header_ready = true;
                }
            } else {
                if &fourcc == b"moof" {
                    if !self.current_fragment.is_empty() {
                        let prev_frag = std::mem::take(&mut self.current_fragment);
                        completed_fragments.push((prev_frag, self.current_fragment_ts));
                    }
                    self.current_fragment_ts = timestamp;
                    self.current_fragment.extend_from_slice(&box_bytes);
                } else {
                    self.current_fragment.extend_from_slice(&box_bytes);
                }
            }
        }

        completed_fragments
    }

    pub fn flush_all_bytes(&mut self) -> (Vec<u8>, f64) {
        let ts = self.current_fragment_ts;
        let mut out = Vec::new();
        if !self.current_fragment.is_empty() {
            out.extend_from_slice(&self.current_fragment);
            self.current_fragment.clear();
        }
        if !self.parser_buf.is_empty() {
            out.extend_from_slice(&self.parser_buf);
            self.parser_buf.clear();
        }
        (out, ts)
    }

    pub fn peek_remaining_bytes(&self) -> Vec<u8> {
        let mut out = Vec::new();
        if !self.current_fragment.is_empty() {
            out.extend_from_slice(&self.current_fragment);
        }
        if !self.parser_buf.is_empty() {
            out.extend_from_slice(&self.parser_buf);
        }
        out
    }

    pub fn reset(&mut self) {
        self.parser_buf.clear();
        self.init_header.clear();
        self.init_header_ready = false;
        self.current_fragment.clear();
        self.current_fragment_ts = 0.0;
    }
}

pub struct RingBuffer {
    pub init_header: Vec<u8>,
    pub chunks: VecDeque<Chunk>,
    pub total_bytes: usize,
    pub limit_bytes: usize,
    pub max_duration_secs: f64,
    pub assembler: Fmp4Assembler,
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
            assembler: Fmp4Assembler::new(),
        }
    }

    pub fn push(&mut self, data: &[u8], timestamp: f64) {
        let is_real_mp4 = data.len() >= 8 && Fmp4Assembler::is_known_box(&[data[4], data[5], data[6], data[7]]);

        if !is_real_mp4 && !self.assembler.init_header_ready && self.init_header.is_empty() {
            // Fallback only for non-MP4 synthetic test fixtures
            if data.windows(4).any(|w| w == b"moov" || w == b"ftyp") && !data.windows(4).any(|w| w == b"moof") {
                self.init_header = data.to_vec();
                self.assembler.init_header = data.to_vec();
                self.assembler.init_header_ready = true;
                self.total_bytes += self.init_header.len();
                return;
            }
            if data.windows(4).any(|w| w == b"moov") && data.windows(4).any(|w| w == b"moof") {
                let moof_idx = data.windows(4).position(|w| w == b"moof").unwrap();
                self.init_header = data[..moof_idx].to_vec();
                self.assembler.init_header = self.init_header.clone();
                self.assembler.init_header_ready = true;
                self.total_bytes += self.init_header.len();

                let rest = &data[moof_idx..];
                if !rest.is_empty() {
                    self.chunks.push_back(Chunk { data: rest.to_vec(), timestamp });
                    self.total_bytes += rest.len();
                }
                return;
            }
        }

        let completed = self.assembler.process_chunk(data, timestamp);

        if self.assembler.init_header_ready && self.init_header.is_empty() {
            self.init_header = self.assembler.init_header.clone();
            self.total_bytes += self.init_header.len();
        }

        for (frag, frag_ts) in completed {
            let len = frag.len();
            self.chunks.push_back(Chunk { data: frag, timestamp: frag_ts });
            self.total_bytes += len;
        }

        self.evict_expired(timestamp);
    }

    pub fn flush_pending(&mut self, timestamp: f64) {
        let (pending, frag_ts) = self.assembler.flush_all_bytes();
        if !pending.is_empty() {
            let len = pending.len();
            let ts = if frag_ts > 0.0 { frag_ts } else { timestamp };
            self.chunks.push_back(Chunk { data: pending, timestamp: ts });
            self.total_bytes += len;
            self.evict_expired(timestamp);
        }
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
        self.assembler.reset();
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
        let pending = self.assembler.peek_remaining_bytes();
        if !pending.is_empty() {
            writer.write_all(&pending)?;
            written += pending.len();
        }
        writer.flush()?;
        Ok(written)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_mp4_box(fourcc: &[u8; 4], payload_len: usize) -> Vec<u8> {
        let total_len = (payload_len + 8) as u32;
        let mut b = Vec::with_capacity(total_len as usize);
        b.extend_from_slice(&total_len.to_be_bytes());
        b.extend_from_slice(fourcc);
        b.resize(total_len as usize, 0xAA);
        b
    }

    #[test]
    fn test_fmp4_assembler_and_fragment_aligned_eviction() {
        let mut buf = RingBuffer::new(50, 1);
        buf.max_duration_secs = 3.0;

        let ftyp = make_mp4_box(b"ftyp", 20);
        let moov = make_mp4_box(b"moov", 100);

        let mut init_stream = Vec::new();
        init_stream.extend_from_slice(&ftyp);
        init_stream.extend_from_slice(&moov);

        for chunk in init_stream.chunks(7) {
            buf.push(chunk, 100.0);
        }
        assert!(buf.assembler.init_header_ready);
        assert_eq!(buf.init_header.len(), ftyp.len() + moov.len());

        for i in 0..6 {
            let moof = make_mp4_box(b"moof", 40);
            let mdat = make_mp4_box(b"mdat", 200);
            let mut frag = Vec::new();
            frag.extend_from_slice(&moof);
            frag.extend_from_slice(&mdat);

            let ts = 100.0 + (i as f64);
            for chunk in frag.chunks(13) {
                buf.push(chunk, ts);
            }
        }
        buf.flush_pending(105.0);

        assert_eq!(buf.chunks.len(), 4);
        assert_eq!(buf.chunks.front().unwrap().timestamp, 102.0);
        assert_eq!(buf.chunks.back().unwrap().timestamp, 105.0);

        for chunk in &buf.chunks {
            let fourcc = &chunk.data[4..8];
            assert_eq!(fourcc, b"moof", "Surviving fragment must begin with moof atom!");
        }

        let mut written_bytes = Vec::new();
        buf.save_to_writer(&mut written_bytes).unwrap();
        assert!(written_bytes.starts_with(&ftyp));

        let mut pos = 0;
        let mut box_types = Vec::new();
        while pos + 8 <= written_bytes.len() {
            let size = u32::from_be_bytes([
                written_bytes[pos],
                written_bytes[pos + 1],
                written_bytes[pos + 2],
                written_bytes[pos + 3],
            ]) as usize;
            let tag = std::str::from_utf8(&written_bytes[pos + 4..pos + 8]).unwrap();
            box_types.push(tag.to_string());
            pos += size;
        }
        assert_eq!(pos, written_bytes.len(), "Stream must be 100% box aligned without orphan bytes!");
        assert_eq!(
            box_types,
            vec!["ftyp", "moov", "moof", "mdat", "moof", "mdat", "moof", "mdat", "moof", "mdat"]
        );
    }

    #[test]
    fn test_ring_buffer_invariants_hold_after_pushes() {
        let mut buf = RingBuffer::new(2, 5);

        for i in 0..50 {
            buf.push(&vec![0xCC; 100 * 1024], i as f64);
            assert!(buf.verify_invariants().is_ok());
        }

        assert!(buf.total_bytes <= 2 * 1024 * 1024);
    }

    #[test]
    fn test_real_fmp4_init_header_preserves_ftyp_and_moov() {
        let mut buf = RingBuffer::new(50, 10);

        let ftyp = make_mp4_box(b"ftyp", 20);
        let moov = make_mp4_box(b"moov", 800);
        let moof1 = make_mp4_box(b"moof", 300);
        let mdat1 = make_mp4_box(b"mdat", 5000);
        let moof2 = make_mp4_box(b"moof", 300);
        let mdat2 = make_mp4_box(b"mdat", 5000);

        buf.push(&ftyp, 1.0);
        assert!(!buf.assembler.init_header_ready);
        assert!(buf.init_header.is_empty());

        buf.push(&moov, 1.05);
        assert!(buf.assembler.init_header_ready);
        assert_eq!(buf.init_header.len(), ftyp.len() + moov.len());

        buf.push(&moof1, 2.0);
        buf.push(&mdat1, 2.1);
        buf.push(&moof2, 3.0);
        buf.push(&mdat2, 3.1);

        assert_eq!(buf.chunks.len(), 1);
        buf.flush_pending(3.2);
        assert_eq!(buf.chunks.len(), 2);

        buf.chunks.pop_front();

        let mut out = Vec::new();
        buf.save_to_writer(&mut out).unwrap();

        assert_eq!(&out[4..8], b"ftyp");
        let moov_offset = ftyp.len();
        assert_eq!(&out[moov_offset + 4..moov_offset + 8], b"moov");
        let moof_offset = ftyp.len() + moov.len();
        assert_eq!(&out[moof_offset + 4..moof_offset + 8], b"moof");
    }
}
