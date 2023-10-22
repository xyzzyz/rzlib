use std::{cmp, io};
use std::io::{BufRead, ErrorKind, Read};

pub trait BitRead {
    fn read_bits(&mut self, buf: &mut u64, n: usize) -> io::Result<usize>;
    fn read_bits_exact(&mut self, n: usize) -> io::Result<u64> {
        let mut out = 0;
        let mut buf = 0;
        let mut total_read = 0;
        while total_read < n {
            let read = self.read_bits(&mut buf, n-total_read)?;
            if read == 0 {
                return Err(io::Error::new(ErrorKind::UnexpectedEof, "got eof when reading bits"));
            }
            out |= buf << total_read;
            total_read += read;
        }

        Ok(out)
    }
}

pub struct BitReader<R: BufRead> {
    reader: R,
    bits_count: usize,
    bits: u64,
}

impl<R: BufRead> BitReader<R> {
    pub fn new(reader: R) -> BitReader<R> {
        BitReader {
            reader: reader,
            bits_count: 0,
            bits: 0,
        }
    }

    // drops remaining unread bits in the currently processed byte
    pub fn drop_remaining_bits(&mut self) {
        self.bits = 0;
        self.bits_count = 0;
    }

    pub fn read_u8(&mut self) -> io::Result<u8> {
        let mut buf: [u8; 1] = [0; 1];
        self.reader.read_exact(&mut buf)?;
        Ok(u8::from_le_bytes(buf))
    }

    pub fn read_u16(&mut self) -> io::Result<u16> {
        let mut buf: [u8; 2] = [0; 2];
        self.reader.read_exact(&mut buf)?;
        Ok(u16::from_le_bytes(buf))
    }

    pub fn read_u32(&mut self) -> io::Result<u32> {
        let mut buf: [u8; 4] = [0; 4];
        self.reader.read_exact(&mut buf)?;
        Ok(u32::from_le_bytes(buf))
    }
}

fn bitmask(n: u64) -> u64 {
    if n >= 64 {
        u64::MAX
    } else {
        (1 << n) - 1
    }
}

impl<R: BufRead> BitRead for BitReader<R> {
    fn read_bits(&mut self, buf: &mut u64, n: usize) -> io::Result<usize> {
        if n == 0 {
            return Ok(0);
        }
        if self.bits_count == 0 {
            // try to fill partial, and bail early if EOF
            let byte_buf = self.fill_buf()?;
            if byte_buf.is_empty() {
                return Ok(0);
            }
            self.bits = byte_buf[0] as u64;
            self.bits_count = 8;
            self.consume(1);
        }

        // at this point, n > 0 and self.bits_count > 0
        let bits_from_partial = cmp::min(n, self.bits_count);
        *buf = (self.bits as u64) & bitmask(bits_from_partial as u64);
        self.bits >>= bits_from_partial;
        self.bits_count -= bits_from_partial;
        return Ok(bits_from_partial)
    }
}

impl<R: BufRead> Read for BitReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        assert_eq!(self.bits_count, 0);
        self.reader.read(buf)
    }
}

impl<R: BufRead> BufRead for BitReader<R> {
    fn fill_buf(&mut self) -> io::Result<&[u8]> {
        assert_eq!(self.bits_count, 0);
        self.reader.fill_buf()
    }
    fn consume(&mut self, amt: usize) {
        self.reader.consume(amt)
    }
}

#[cfg(test)]
mod tests {
    use crate::bitreader::{BitRead, BitReader};
    use std::io::Cursor;

    #[test]
    fn test_read_bits() {
        let bytes_in: Vec<u8> = vec![0b00001111, 0b00110011, 0b00000000, 0b11111111];
        let cursor = Cursor::new(bytes_in);
        let mut reader = BitReader::new(cursor);
        assert_eq!(reader.read_bits_exact(1).unwrap(), 1);
        assert_eq!(reader.read_bits_exact(1).unwrap(), 1);
        assert_eq!(reader.read_bits_exact(1).unwrap(), 1);

        assert_eq!(reader.read_bits_exact(2).unwrap(), 0b01);


        assert_eq!(reader.read_bits_exact(1).unwrap(), 0);
        assert_eq!(reader.read_bits_exact(1).unwrap(), 0);

        assert_eq!(reader.read_bits_exact(5).unwrap(), 0b00110);

        assert_eq!(reader.read_bits_exact(4+8+8).unwrap(), 0b11111111000000000011);
    }
}
