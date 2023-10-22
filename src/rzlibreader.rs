use std::error::Error;
use std::fs::{read, File};
use std::io::ErrorKind::{InvalidData, InvalidInput};
use std::io::{BufRead, Read, Write};
use std::{cmp, fmt, io, mem, result};
use log::debug;

use crate::bitreader;
use crate::bitreader::BitRead;
use crate::huffman;
use crate::huffman::HuffmanTree;
use crate::lookbackbuffer::LookbackBuffer;
use crate::rzlibreader::State::{BlockHeader, BrokenStream, EndOfFile, HuffmanBlock, HuffmanBlockMatch, MemberHeader, MemberTrailer, NoCompressionBlock};

fn invalid_data_error(s: &str) -> io::Error {
    return io::Error::new(InvalidData, s);
}

#[derive(Debug)]
enum State {
    BrokenStream,
    MemberHeader,
    BlockHeader,
    NoCompressionBlock {
        len: usize,
        is_final: bool,
    },
    HuffmanBlock {
        litlen_tree: HuffmanTree<usize>,
        distance_tree: HuffmanTree<usize>,
        is_final: bool,
    },
    HuffmanBlockMatch {
        litlen_tree: HuffmanTree<usize>,
        distance_tree: HuffmanTree<usize>,
        length: usize,
        distance: usize,
        is_final: bool,
    },
    MemberTrailer,
    EndOfFile,
}
#[derive(Debug)]
enum Item {
    Literal { byte: u8 },
    Match { length: usize, distance: usize },
}

const LOOKBACK_WINDOW_SIZE: usize = 2_usize.pow(15);
pub struct RZLibReader<R: Read + BufRead> {
    state: State,
    reader: bitreader::BitReader<R>,
    lookback: LookbackBuffer,
    total_bytes_read: usize,

}
impl<R: Read + BufRead> RZLibReader<R> {
    pub fn new(reader: R) -> RZLibReader<R> {
        RZLibReader {
            state: MemberHeader,
            reader: bitreader::BitReader::new(reader),
            lookback: LookbackBuffer::new(LOOKBACK_WINDOW_SIZE),
            total_bytes_read: 0,
        }
    }

    fn read_cstring(&mut self) -> io::Result<String> {
        let mut buf = vec![];
        let bytes_read = self.reader.read_until(0, &mut buf)?;
        match buf.pop() {
            Some(0) => (),
            None | Some(_) => return Err(invalid_data_error("expect null-terminated string")),
        }

        String::from_utf8(buf).map_err(|e| io::Error::new(io::ErrorKind::Other, e.to_string()))
    }
    fn read_member_header(&mut self) -> io::Result<()> {
        if self.reader.fill_buf()?.is_empty() {
            self.state = EndOfFile;
            return Ok(());
        }
        let id1 = self.reader.read_u8()?;
        let id2 = self.reader.read_u8()?;

        if id1 != 0x1f || id2 != 0x8b {
            return Err(invalid_data_error(&format!(
                "wrong id1, id2 (0x{:x}, 0x{:x})",
                id1, id2
            )));
        }

        let cm = self.reader.read_u8()?;

        if cm != 0x08 {
            return Err(invalid_data_error(&format!("wrong cm (0x{:x})", cm)));
        }

        let mut flg = self.reader.read_u8()?;
        let ftext = flg & 1 == 1;
        // eprintln!("FTEXT: {}", ftext);
        flg >>= 1;
        let fhcrc = flg & 1 == 1;
        // eprintln!("FHCRC: {}", fhcrc);
        flg >>= 1;
        let fextra = flg & 1 == 1;
        // eprintln!("FEXTRA: {}", fextra);
        flg >>= 1;
        let fname = flg & 1 == 1;
        // eprintln!("FNAME: {}", fname);
        flg >>= 1;
        let fcomment = flg & 1 == 1;
        // eprintln!("FCOMMENT: {}", fcomment);

        let mtime = self.reader.read_u32()?;
        // eprintln!("MTIME: {}", mtime);

        let xfl = self.reader.read_u8()?;
        // eprintln!("XFL: {}", xfl);

        let os = self.reader.read_u8()?;
        // eprintln!("OS: {}", os);

        if fextra {
            let xlen = self.reader.read_u16()?;
            // eprintln!("XLEN: {}", xlen);

            let mut fextra_buf = vec![0; xlen as usize];
            self.reader.read_exact(&mut fextra_buf)?;
            let extra = match String::from_utf8(fextra_buf) {
                Ok(s) => s,
                Err(e) => return Err(invalid_data_error(&format!("error decoding extra: {}", e))),
            };
            // eprintln!("EXTRA: {}", extra);
        }

        if fname {
            let file_name = self.read_cstring()?;
            // eprintln!("FILE NAME: {}", file_name);
        }

        if fcomment {
            let comment = self.read_cstring()?;
            // eprintln!("COMMENT: {}", comment);
        }

        if fhcrc {
            let mut crc16_buf: [u8; 2] = [0; 2];
            self.reader.read_exact(&mut crc16_buf)?;
            let crc16 = u16::from_le_bytes(crc16_buf);
            // eprintln!("CRC16: {}", crc16);
        }
        self.state = BlockHeader;
        Ok(())
    }

    fn read_member_trailer(&mut self) -> io::Result<()>{
        self.reader.drop_remaining_bits();
        let crc32 = self.reader.read_u32()?;
        // eprintln!("CRC32: {}", crc32);
        let isize = self.reader.read_u32()?;
        // eprintln!("isize: {}", isize);
        self.state = MemberHeader;
        Ok(())
    }

    fn read_no_compression_block_header(&mut self, is_final: bool) -> io::Result<()> {
        self.reader.drop_remaining_bits();
        let len = self.reader.read_u16()?;
        let nlen = self.reader.read_u16()?;
        if !len != nlen {
            return Err(invalid_data_error(&format!(
                "len ({}) is not one-complement of nlen ({})",
                len, nlen
            )));
        }

        self.state = NoCompressionBlock {
            len: len as usize,
            is_final,
        };
        Ok(())
    }
    fn read_no_compression_block(
        &mut self,
        buf: &mut [u8],
        block_len: usize,
        is_final: bool,
    ) -> io::Result<usize> {
        let can_read = cmp::min(block_len, buf.len());

        let read_len = self.reader.read(&mut buf[..can_read])?;
        self.lookback.write_data(&buf[..read_len])?;

        let remaining_len = block_len - read_len;
        self.state = if remaining_len == 0 {
            if is_final {
                MemberTrailer
            } else {
                BlockHeader
            }
        } else {
            NoCompressionBlock {
                len: remaining_len,
                is_final,
            }
        };
        return Ok(read_len);
    }

    fn read_fixed_huffman_block_header(&mut self) -> io::Result<()> {
        todo!()
    }

    fn read_dynamic_huffman_block_header(&mut self, is_final: bool) -> io::Result<()> {
        let nlit = self.reader.read_bits_exact(5)? as usize + 257;
        let ndist = self.reader.read_bits_exact(5)? as usize + 1;
        let ncode = self.reader.read_bits_exact(4)? as usize + 4;

        // eprintln!("nlit: {}, ndist: {}, ncode: {}", nlit, ndist, ncode);

        // See RFC 3.2.7
        let clen_order: [usize; 19] = [
            16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15,
        ];

        let mut clen_lengths: Vec<usize> = vec![0; 19];
        for i in 0..ncode {
            clen_lengths[clen_order[i]] = self.reader.read_bits_exact(3)? as usize;
        }

        // eprintln!("clengths:");
        for (i, l) in clen_lengths.iter().enumerate() {
            // eprintln!("{}: {}", i, l);
        }

        let lengths_tree: HuffmanTree<usize> =
            huffman::HuffmanTree::<usize>::new_from_lengths(&clen_lengths);
        let mut all_lengths: Vec<usize> = vec![0; nlit + ndist];
        let mut next_length_i = 0;
        let mut previous_length = 0;
        while next_length_i < nlit + ndist {
            let clc = lengths_tree
                .decode(&mut self.reader)?
                .ok_or(invalid_data_error("failed to decode clc"))?;
            if clc <= 15 {
                // see 3.2.7 in RFC
                all_lengths[next_length_i] = clc;
                // eprintln!("length {:?}: {:?}", next_length_i, clc);
                next_length_i += 1;
                previous_length = clc;
            } else {
                // code repeats
                let repeat_count;
                let repeat_length;
                if clc == 16 {
                    repeat_count = self.reader.read_bits_exact(2)? + 3;
                    repeat_length = previous_length;
                } else if clc == 17 {
                    repeat_count = self.reader.read_bits_exact(3)? + 3;
                    repeat_length = 0;
                } else if clc == 18 {
                    repeat_count = self.reader.read_bits_exact(7)? + 11;
                    repeat_length = 0;
                } else {
                    return Err(invalid_data_error(&format!(
                        "unexpected length code: {:?}",
                        clc
                    )));
                }
                for _ in 0..repeat_count {
                    all_lengths[next_length_i] = repeat_length;
                    // eprintln!("length {:?}: {:?}", next_length_i, repeat_length);
                    next_length_i += 1;
                }
                previous_length = repeat_length;
            }
        }
        // eprintln!("read {:?} lengths:", next_length_i);
        for (i, l) in all_lengths.iter().enumerate() {
            // eprintln!("length {:?}: {:?}", i, l);
        }

        let litlen_tree = huffman::HuffmanTree::<usize>::new_from_lengths(&all_lengths[..nlit]);
        let distance_tree =
            huffman::HuffmanTree::<usize>::new_from_lengths(&all_lengths[nlit..(nlit + ndist)]);

        self.state = HuffmanBlock {
            litlen_tree,
            distance_tree,
            is_final,
        };
        return Ok(());
    }

    fn read_huffman_block(
        &mut self,
        buf: &mut [u8],
        litlen_tree: HuffmanTree<usize>,
        distance_tree: HuffmanTree<usize>,
        is_final: bool,
    ) -> io::Result<usize> {
        // size base for length codes 257..285
        let length_offsets = [
            3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83, 99,
            115, 131, 163, 195, 227, 258,
        ];
        // extra bits for length codes 257..285
        let length_extra_bits = [
            0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0,
        ];

        // offset base for distance codes 0..29
        let distance_offsets = [
            1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513, 769, 1025,
            1537, 2049, 3073, 4097, 6145, 8193, 12289, 16385, 24577,
        ];
        // extra bits for distance codes 0..29
        let distance_extra_bits = [
            0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12,
            12, 13, 13,
        ];
        let mut pos = 0;
        // actual decode loop
        while pos < buf.len() {
            let litlen = litlen_tree
                .decode(&mut self.reader)?
                .ok_or(io::Error::new(InvalidData, "failed to decode litlen"))?;
            if litlen < 256 {
                // add to buffer and to lookback
                let b = litlen as u8;
                buf[pos] = b;
                pos += 1;
                self.lookback.write_byte(b)?;
                continue;
            } else if litlen == 256 {
                // eprintln!("end of block, final = {:?}", is_final);
                // end of block
                if is_final {
                    self.state = MemberTrailer;
                } else {
                    self.state = BlockHeader;
                }
                return Ok(pos);
            } else if litlen <= 285 {
                // found a match
                let match_length = self
                    .reader
                    .read_bits_exact(length_extra_bits[litlen - 257])?
                    + length_offsets[litlen - 257];
                let dist_code = distance_tree
                    .decode(&mut self.reader)?
                    .ok_or(invalid_data_error("failed to decode distance code"))?;
                let match_distance = self
                    .reader
                    .read_bits_exact(distance_extra_bits[dist_code])?
                    + distance_offsets[dist_code];
                // eprintln!("match {:?} {:?}", match_length, match_distance);
                self.state = HuffmanBlockMatch {
                    litlen_tree,
                    distance_tree,
                    length: match_length as usize,
                    distance: match_distance as usize,
                    is_final,
                };
                return Ok(pos);
            }
        }
        // we filled the entire buffer
        self.state = HuffmanBlock {
            litlen_tree,
            distance_tree,
            is_final,
        };
        return Ok(pos);
    }

    fn read_huffman_block_match(
        &mut self,
        buf: &mut [u8],
        litlen_tree: HuffmanTree<usize>,
        distance_tree: HuffmanTree<usize>,
        length: usize,
        distance: usize,
        is_final: bool,
    ) -> io::Result<usize> {
        // we can only read at most LOOKBACK_WINDOW_SIZE at a time
        let read_length = cmp::min(
            LOOKBACK_WINDOW_SIZE,
            cmp::min(buf.len(), cmp::min(length, distance)),
        );
        self.lookback
            .read_lookback_exact(&mut buf[..read_length], distance)?;
        self.lookback.write_data(&mut buf[..read_length])?;
        self.state = if read_length == length {
            HuffmanBlock {
                litlen_tree,
                distance_tree,
                is_final,
            }
        } else {
            HuffmanBlockMatch {
                litlen_tree,
                distance_tree,
                length: length - read_length,
                distance,
                is_final,
            }
        };
        Ok(read_length)
    }
    fn read_block_header(&mut self) -> io::Result<()> {
        let bfinal = self.reader.read_bits_exact(1)?;
        let btype = self.reader.read_bits_exact(2)? as u8;

        // eprintln!("bfinal: {}, btype: {}", bfinal, btype);
        let is_final = bfinal == 1;

        const NO_COMPRESSION: u8 = 0;
        const FIXED_HUFFMAN: u8 = 1;
        const DYNAMIC_HUFFMAN: u8 = 2;

        match btype {
            NO_COMPRESSION => self.read_no_compression_block_header(is_final)?,
            FIXED_HUFFMAN => self.read_fixed_huffman_block_header()?,
            DYNAMIC_HUFFMAN => self.read_dynamic_huffman_block_header(is_final)?,
            _ => return Err(invalid_data_error(&format!("unknown btype: {}", btype))),
        }

        return Ok(());
    }

    fn read_impl(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let mut buf = buf;
        let mut total_read = 0;
        while total_read == 0 {
            let mut reader_state = BrokenStream;
            mem::swap(&mut self.state, &mut reader_state);
            // we expect the functions to restore state upon lack of errors
            match reader_state {
                BrokenStream => {
                    return Err(io::Error::new(
                        InvalidInput,
                        "trying to read from a broken stream",
                    ))
                }
                MemberHeader => self.read_member_header()?,
                MemberTrailer => self.read_member_trailer()?,
                BlockHeader => self.read_block_header()?,
                NoCompressionBlock { len, is_final } => {
                    let read = self.read_no_compression_block(buf, len, is_final)?;
                    buf = &mut buf[read..];
                    total_read += read;
                }
                HuffmanBlock {
                    litlen_tree,
                    distance_tree,
                    is_final,
                } => {
                    let read = self.read_huffman_block(buf, litlen_tree, distance_tree, is_final)?;
                    buf = &mut buf[read..];
                    total_read += read;
                },
                HuffmanBlockMatch {
                    litlen_tree,
                    distance_tree,
                    length,
                    distance,
                    is_final,
                } => {
                    let read =  self.read_huffman_block_match(
                        buf,
                        litlen_tree,
                        distance_tree,
                        length,
                        distance,
                        is_final,
                    )?;
                    buf = &mut buf[read..];
                    total_read += read;
                }
                EndOfFile => {
                    self.state = EndOfFile;
                    return Ok(0);

                }
            }
        }
        Ok(total_read)
    }
}

impl<R: Read + BufRead> Read for RZLibReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let bytes_read = self.read_impl(buf)?;
        self.total_bytes_read += bytes_read;

        return Ok(bytes_read);
    }
}
