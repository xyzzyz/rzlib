mod rzlibreader;
mod bitreader;
mod huffman;
mod lookbackbuffer;


use std::io;
use std::io::{Read, Write};
use crate::rzlibreader::RZLibReader;


fn main_r() -> io::Result<()> {
    let stdin = io::stdin().lock();
    let mut reader = RZLibReader::new(stdin);

    let mut stdout = io::stdout().lock();

    io::copy(&mut reader, &mut stdout)?;

    return Ok(());
}

fn main() {
    main_r().expect("main");

    eprintln!("finished!")
}
