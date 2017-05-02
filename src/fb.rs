/// framebuffer graphics:
/// - uses /dev/fb0, expects u32 pixels
/// - struct Video


// --- configuration -------------------------------------------------------------------------------

pub const XSIZE:     usize = 1024;  // constants enable easy optimizations
pub const YSIZE:     usize = 600;


// ---- low-level stuff ----------------------------------------------------------------------------

// map a file to a mutable slice -- thanking the bloated Posix standards
//
fn mmap<T>(fname: &str, len: usize) -> Result<&mut[T], String> {
    unsafe {
        let fd = ::libc::open(::std::ffi::CString::new(fname).expect("cstring").as_ptr(),
            ::libc::O_RDWR);
        if fd < 0 {
            let errno = *::libc::__errno_location();
            let errstr = ::std::str::from_utf8_unchecked(::std::ffi::CStr::from_ptr(
                                                         ::libc::strerror(errno)).to_bytes());
            let e = format!("!--mmap/open: E{}: {}", errno, errstr);
            return Err(e)
        }

        let addr = ::libc::mmap(0 as *mut ::libc::c_void,
                                (len * ::std::mem::size_of::<T>()) as ::libc::size_t,
                                ::libc::PROT_READ | ::libc::PROT_WRITE,
                                ::libc::MAP_SHARED,
                                fd,
                                0 as ::libc::off_t);
        if addr == ::libc::MAP_FAILED {
            ::libc::close(fd);
            let errno = *::libc::__errno_location();
            let errstr = ::std::str::from_utf8_unchecked(::std::ffi::CStr::from_ptr(
                    ::libc::strerror(errno)).to_bytes());
            let e = format!("!--mmap/open: E{}: {}", errno, errstr);
            Err(e)
        } else {
            Ok(::std::slice::from_raw_parts_mut(addr as *mut T, len))
        }
    }
}


// ---- graphics library ---------------------------------------------------------------------------
//
// x (column) and y (row) are expressed as non-negative (usize) to get maximum speed on indexing
// colors are expressed as u32 (RGBA format is target-dependent)

#[inline]
fn ispix(x: usize, y: usize) -> bool {
   x < XSIZE && y < YSIZE
}

// bizarre width and height are not allowed
#[inline]
fn isarea(x: usize, y: usize, w: usize, h: usize) -> bool {
   ispix(x, y)  &&  w > 0 && h > 0  && ispix(w - 1, h - 1)  && ispix(x + w - 1, y + h - 1)
}


pub struct Video<'a> {
    buf: &'a mut[u32]
}


impl<'a> Video<'a> {

    // initialization: mmap /dev/fb0
    //
    pub fn new() -> Result<Video<'a>, String>  {
        match mmap::<u32>("/dev/fb0", XSIZE * YSIZE) {
            Err(s) => Err(s),
            Ok(fb) => Ok(Video { buf: fb })
        }
    }

    // clear entire screen area
    //
    pub fn cls(&mut self, col: u32) {
        for n in 0..XSIZE*YSIZE {
            self.buf[n] = col;
        }
    }

    // draw a single pixel
    //
    pub fn plot(&mut self, col: u32, x: usize, y: usize) {
        if ispix(x, y) {
            self.buf[y * XSIZE + x] = col
        }
    }

    // scroll window n pixels to left, caller shall update the n colums on the right
    //
    pub fn leftscroll(&mut self, x: usize, y: usize, w: usize, h: usize, n: usize) {
        if isarea(x, y, w, h) && n > 0 && n < w {
            for row in y..y+h {
                let idx = row * XSIZE + x;
                unsafe { // but faster
                    ::std::ptr::copy(&self.buf[idx+n] , &mut self.buf[idx], w-n)
                }
            }
        }
    }

    // scroll window n pixels to right, caller shall update the n columns on the left
    //
    pub fn rightscroll(&mut self, x: usize, y: usize, w: usize, h: usize, n: usize) {
        if isarea(x, y, w, h) && n > 0 && n < w {
            for row in y..y+h {
                let idx = row * XSIZE + x;
                unsafe { // but faster
                    ::std::ptr::copy(&self.buf[idx], &mut self.buf[idx+n], w-n)
                }
            }
        }
    }

    // draw a full-width horizontal line
    //
    pub fn scanline(&mut self, col: u32, y: usize) {
        if y < YSIZE {
            for i in (y*XSIZE)..((y+1)*XSIZE) {
                self.buf[i] = col
            }
        }
    }

    // draw an horizontal line
    //
    pub fn horizline(&mut self, col: u32, x: usize, y: usize, w: usize) {
        if ispix(x, y) && x+w <= XSIZE {
            let start = y * XSIZE + x;
            for i in start..start+w {
                self.buf[i] = col
            }
        }
    }

    // draw a vertical line - slower than horizontal
    //
    pub fn vertline(&mut self, col: u32, x: usize, y: usize, h: usize) {
        if ispix(x, y) && y+h <= YSIZE {
            for i in 0..h {
                self.buf[(y+i) * XSIZE + x] = col
            }
        }
    }

    // draw a vertical line from bottom up - as anti-flicker in massive draw sessions
    //
    pub fn upvertline(&mut self, col: u32, x: usize, y: usize, h: usize) {
        if ispix(x, y) && y+h <= YSIZE {
            for i in (0..h).rev() {
                self.buf[(y+i) * XSIZE + x] = col
            }
        }
    }

    // draw an empty rectangle
    //
    pub fn rectangle(&mut self, col: u32, x: usize, y: usize, w: usize, h: usize) {
        if isarea(x, y, w, h) {
            for xi in x..x+w {
                self.buf[xi + y * XSIZE] = col
            }

            if h == 1 { return }

            for yi in y+1..y+h-1 {
                self.buf[x     + yi * XSIZE] = col;
                self.buf[x+w-1 + yi * XSIZE] = col
            }

            for xi in x..x+w {
                self.buf[xi + (y+h-1) * XSIZE] = col
            }
        }
    }

    // draw a line between two arbitrary coordinates
    //
    pub fn line(&mut self, col: u32, mut x: isize, mut y: isize, x1: isize, y1: isize) {
        if ispix(x as usize, y as usize) && ispix(x1 as usize, y1 as usize) {
            let dx: isize = if x < x1 { x1-x } else { x-x1 };
            let dy: isize = if y < y1 { y1-y } else { y-y1 };
            let sx: isize = if x < x1 { 1 } else { -1 };
            let sy: isize = if y < y1 { 1 } else { -1 };
            let mut r: isize = if dx > dy { dx/2 } else { -dy/2 };

            loop {
                self.buf[y as usize * XSIZE + x as usize] = col;
                if x == x1 || y == y1 { break }
                let e = r;
                if e > -dx { r -= dy;  x += sx; }
                if e < dy  { r += dx;  y += sy; }
            }
        }
    }

    // draw a filled rectangle
    //
    pub fn fillbox(&mut self, col: u32, x: usize, y: usize, w: usize, h: usize) {
        if isarea(x, y, w, h) {
            for row in y..y+h {
                let p = row * XSIZE + x;
                for i in p..p+w {
                    self.buf[i] = col
                }
            }
        }
    }

    // draw a filled rectangle from bottom up
    //
    pub fn vertfillbox(&mut self, col: u32, x: usize, y: usize, w: usize, h: usize) {
        if isarea(x, y, w, h) {
            for column in x..x+w {
                let p = y * XSIZE + column;
                for i in 0..h {
                    self.buf[p + i * XSIZE] = col
                }
            }
        }
    }

    // draw a circle
    //
    pub fn circle(&mut self, col: u32, xc: isize, yc: isize, mut r: isize) {
        if r > 0 && ispix(xc as usize, yc as usize) &&
            ispix((xc-r) as usize, (yc-r) as usize) && ispix((xc+r) as usize, (yc+r) as usize) {
            let mut x = -r;
            let mut y = 0;
            let mut e = 2 - 2 * r;
            loop {
                self.buf[(yc+y) as usize *XSIZE + (xc-x) as usize] = col;
                self.buf[(yc-x) as usize *XSIZE + (xc-y) as usize] = col;
                self.buf[(yc-y) as usize *XSIZE + (xc+x) as usize] = col;
                self.buf[(yc+x) as usize *XSIZE + (xc+y) as usize] = col;
                r = e;
                if r <= y {
                    y += 1;
                    e += y*2 + 1
                }
                if r > x || e > y {
                    x += 1;
                    e += x * 2 + 1
                }
                if x >= 0 {
                    break
                }
            }
        }
    }
}

