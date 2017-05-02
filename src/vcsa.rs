/// interface to the text display file /dev/vcsa1 (vt console)


// ---- configuration ------------------------------------------------------------------------------

const DEFAULT_ATTR: u8 = 0x1c; // paper 1, ink 12


// ---- low-level stuff ----------------------------------------------------------------------------

use std::io::prelude::*;
use std::io::SeekFrom;
use std::fs::OpenOptions;

pub struct Video {
    fp: ::std::fs::File,       // non mappable display file
    xsize: usize,              // characters per row
    tsize: usize,              // display file size in bytes
    pub attr: u8               // extfont bit, 4 bits ink, 3 bits paper
}


// ---- real meat comes here -----------------------------------------------------------------------

impl Video {
    pub fn new() -> Result<Video, ::std::io::Error> {
        let fname = "/dev/vcsa1";
        match OpenOptions::new().write(true).read(true).open(fname) {
            Err(e) => {
                println!("!--vcsa: open {}: {}", fname, e);
                Err(e)
            },
            Ok(mut fp) => {            // once open ok, always ok
                let mut buf = [ 0u8; 2 ];
                fp.read(&mut buf[..]).unwrap();

                let x = buf[1] as usize;
                let t = 2 * (x * buf[0] as usize + 2);
                Ok(Video { fp: fp, xsize: x, tsize: t, attr: DEFAULT_ATTR })
            }
        }
    }

    pub fn paper(&mut self, c: usize) {
        self.attr = ((self.attr & 0x1e) | (((c as u8) & 7) << 5)) as u8
    }

    pub fn ink(&mut self, c: usize) {
        self.attr = ((self.attr & 0xe0) | (((c as u8) & 15) << 1)) as u8
    }

    pub fn puts(&mut self, x: usize, y: usize, buf: &str) {
        let pos = 2 * ( y * self.xsize + x + 2 );
        match self.fp.seek(SeekFrom::Start(pos as u64)) {
            Ok(_) => {},
            Err(e) => {
                panic!("!--vcsa: coords({},{}): {}", x, y, e);
            }
        }

        // intersperse color attributes:
        //
        let buf = buf.as_bytes();
        let mut s: Vec<u8> = vec![0; 2 * buf.len()];
        for c in 0..buf.len() {
            if pos + 2 * c >= self.tsize {
                break
            }

            // non-UTF8 character substitutions:
            //
            s[c+c] = match buf[c] {
                b'|' => 0x85,          // à
                b'^' => 0xf8,          // °
                _    => buf[c]
            };

            s[c+c+1] = self.attr;
        }

        // send the binary data to screen text file:
        //
        self.fp.write_all(&s[..]).unwrap();
    }
}

