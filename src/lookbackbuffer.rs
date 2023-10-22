use std::{cmp, io};
use std::io::ErrorKind::InvalidInput;
use std::io::Write;

pub struct LookbackBuffer {
    data: Vec<u8>,
    pos: usize,
}

impl LookbackBuffer {
    pub fn new(lookback_size: usize) -> LookbackBuffer {
        if lookback_size == 0 {
            panic!("lookback_size must be nonzero")
        }
        LookbackBuffer { data: vec![0; lookback_size], pos: 0 }
    }

    pub fn write_data(&mut self, buf: &[u8]) -> io::Result<()> {
        if buf.len() > self.data.len() {
            return Err(io::Error::new(InvalidInput,  format!("trying to write {} bytes to lookback buffer of size {}", buf.len(), self.data.len())));
        }

        let space_left_before_wraparound = self.data.len() - self.pos;
        if buf.len() <= space_left_before_wraparound {
            self.data[self.pos..(self.pos+ buf.len())].copy_from_slice(buf);
        } else {
            // we need to chunk
            let first_chunk_len = space_left_before_wraparound;
            self.data[self.pos..].copy_from_slice(&buf[..first_chunk_len]);
            self.data[..(buf.len()-first_chunk_len)].copy_from_slice(&buf[first_chunk_len..])
        }
        self.pos = (self.pos + buf.len()) % self.data.len();
        Ok(())
    }
    pub fn write_byte(&mut self, b: u8) -> io::Result<()> {
        self.data[self.pos] = b;
        self.pos = (self.pos+1) % self.data.len();
        Ok(())
    }

    pub fn read_lookback_exact(&self, buf: &mut [u8], distance: usize) -> io::Result<()> {
        if buf.len() > distance {
            return Err(io::Error::new(InvalidInput,  format!("lookback length {} greater than lookback distance {}", buf.len(), distance)));
        }
        if distance > self.data.len() {
            return Err(io::Error::new(InvalidInput,  format!("lookback distance {} greater than lookback window size {}", distance, self.data.len())));
        }

        if self.pos > distance {
            // we can lookback without wrapping around
            let read_pos = self.pos-distance;
            buf.copy_from_slice(&self.data[read_pos..(read_pos+buf.len())]);
            return Ok(())
        } else {
            // we must wrap around
            // TODO: merge both cases here
            let wraparound_distance = distance-self.pos;
            let wraparound_pos = self.data.len() - wraparound_distance;
            let wraparound_length = cmp::min(buf.len(), self.data.len() - wraparound_pos);
            let remaining_length = buf.len() - wraparound_length;
            buf[..wraparound_length].copy_from_slice(&self.data[wraparound_pos..(wraparound_pos+wraparound_length)]);
            buf[wraparound_length..].copy_from_slice(&self.data[..remaining_length]);
        }
        Ok(())

    }
}