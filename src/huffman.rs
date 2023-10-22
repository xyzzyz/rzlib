use std::fmt::Debug;
use std::{cmp, fmt, io};
use crate::bitreader::BitRead;

#[derive(PartialEq, Eq, Debug, Clone)]
pub struct Codeword {
    len: usize,
    code: u64,
}

fn bitmask(n: u64) -> u64 {
    if n >= 64 {
        u64::MAX
    } else {
        (1 << n) - 1
    }
}

fn reverse_bits(a: u64, len: usize) -> u64 {
    let mut b: u64 = 0;
    let mut a: u64 = a;
    for _ in 0..len {
        b = (b << 1) | (a & 1);
        a >>= 1;
    }
    b
}
impl BitRead for Codeword {
    fn read_bits(&mut self, buf: &mut u64, n: usize) -> io::Result<usize> {
        let n = cmp::min(n, self.len);
        *buf = *buf | self.code & bitmask(n as u64);
        self.code >>= n;
        Ok(n)
    }
}

impl Codeword {
    pub fn new(len: usize, code: u64) -> Self {
        Codeword{len, code}
    }

}

impl fmt::Display for Codeword {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut code = self.code;
        for _ in 0..self.len {
            write!(f, "{:01b}", code & 1 )?;
            code >>= 1;
        }
        Ok(())
    }
}

pub struct HuffmanTree<R: Debug> {
    value: Option<R>,
    zero: Box<Option<HuffmanTree<R>>>,
    one: Box<Option<HuffmanTree<R>>>
}

impl<R: Debug + Clone> fmt::Debug for HuffmanTree<R> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "HuffmanTree[size: {:?}]", self.size())
    }
}

impl<R: Debug + Clone> HuffmanTree<R> {
    pub fn new() -> HuffmanTree<R> {
        HuffmanTree { value: None, zero: Box::new(None), one: Box::new(None)}
    }

    pub fn size(&self) -> usize {
        let mut s = 0;
        if self.value.is_some() {
            s += 1;
        }
        if let Some(t) = self.zero.as_ref() {
            s += t.size();
        }
        if let Some(t) = self.one.as_ref() {
            s += t.size();
        }
        s
    }

    pub fn new_from_lengths(lengths: &[usize]) -> HuffmanTree<usize> {
        let n = lengths.len();
        let mut tree = HuffmanTree::new();

        let mut bl_count = vec![0; 32];
        for l in lengths.iter() {
            bl_count[*l as usize] += 1;
        }


        let mut next_code = vec![0_u64; 33];
        let mut code= 0;
        for bits in 1..=32 {
            code = (code + bl_count[bits-1]) << 1;
            next_code[bits] = code;
        }

        for b in 0..n {
            let l = lengths[b];
            if l != 0 {
                let rev_code = reverse_bits(next_code[l], lengths[b]);
                let hcode = Codeword::new(lengths[b], rev_code);
                tree.insert(&b, &hcode);
                next_code[l] += 1;
            }
        }
        tree
    }

    fn insert_impl(&mut self, val: &R, code: &mut Codeword, path: u64) {
        if let Some(val) = &self.value {
            panic!("found existing value {:?} at path {:b} while trying to insert {:?}", val, path, code)
        }

        if code.len == 0 {
            self.value = Some(val.clone());
            return;
        }

        let bit = code.code & 1;
        code.code >>= 1;
        code.len -= 1;

        match bit {
            0 => {
                    if self.zero.is_none() {
                        self.zero = Box::new(Some(HuffmanTree::new()));
                    }
                    (*self.zero).as_mut().unwrap().insert_impl(val, code, (path << 1) | 0);
                },
            1 => {
                    if self.one.is_none() {
                        self.one = Box::new(Some(HuffmanTree::new()));
                    }
                    (*self.one).as_mut().unwrap().insert_impl(val, code, (path << 1) | 1);
                }
            _ => panic!("found bit neither 0 nor 1")
        }
    }
    pub fn insert(&mut self, val: &R, code: &Codeword) {
        self.insert_impl(val, &mut code.clone(), 0);
    }
    pub fn decode<T: BitRead>(&self, bits: &mut T) -> io::Result<Option<R>> {
        if let Some(val) = self.value.as_ref() {
            return Ok(Some(val.clone()));
        }
        let subtree = match bits.read_bits_exact(1)? {
            0 => (*self.zero).as_ref(),
            1 => (*self.one).as_ref(),
            _ => panic!("found bit neither 0 nor 1")
        };
        match subtree {
            None => Ok(None),
            Some(t) => t.decode(bits),
        }
    }

    fn dump_impl(&self, path: &Codeword) {
        match &self.value {
            Some(val) => eprintln!("{}: {:?}", path, val),
            None => {
                let zero_path = Codeword { code: (path.code << 1) | 0, len: path.len+1};
                match (*self.zero).as_ref() {
                    None => eprintln!("incomplete tree at {}", zero_path),
                    Some(subtree) => subtree.dump_impl(&zero_path),
                }
                let one_path = Codeword { code: (path.code << 1) | 1, len: path.len+1};
                match (*self.one).as_ref() {
                    None => eprintln!("incomplete tree at {}", one_path),
                    Some(subtree) => subtree.dump_impl(&one_path),
                }
            }
        }
    }

    pub fn dump(&self) {
        self.dump_impl(&Codeword{ code: 0, len: 0});
    }
}
#[cfg(test)]
mod tests {
    use super::HuffmanTree;
    use super::Codeword;
    impl From<(usize, u64)> for Codeword {
        fn from(value: (usize, u64)) -> Self {
            Codeword::new(value.0, value.1)
        }
    }
    #[test]
    fn test_from_rfc_1() {
        let ls = vec![2, 1, 3, 3];
        let tree: HuffmanTree<usize> = HuffmanTree::<usize>::new_from_lengths(&ls);
        let expected: Vec<Codeword> = vec![
            (2, 0b01),
            (1, 0b0),
            (3, 0b011),
            (3, 0b111)
        ].into_iter().map(|p| p.into()).collect();
        for (a, code) in expected.iter().enumerate() {
            assert_eq!(Some(a), tree.decode(&mut code.clone()).unwrap())
        }
    }

    #[test]
    fn test_from_rfc2() {
        let ls = vec![3, 3, 3, 3, 3, 2, 4, 4];
        let tree: HuffmanTree<usize> = HuffmanTree::<usize>::new_from_lengths(&ls);
        let expected: Vec<Codeword> = vec![
            (3, 0b010),
            (3, 0b011),
            (3, 0b100),
            (3, 0b101),
            (3, 0b110),
            (2, 0b00),
            (4, 0b1110),
            (4, 0b1111),
        ].into_iter().map(|p| p.into()).collect();
        for (a, code) in expected.iter().enumerate() {
            assert_eq!(Some(a), tree.decode(&mut code.clone()).unwrap())
        }
    }

}