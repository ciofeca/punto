// --- modules ------------------------------------------------------------------------------------

mod gpsd;                      // gps collector via port 2947
mod imu;                       // imu collector via serial port
mod obd;                       // obd collector via serial port
mod troublecodes;              // obd strings supplement
mod buffer;                    // buffered events saving to disk
mod vcsa;                      // virtual console (text)
pub mod fb;                    // framebuffer (graphics)


// --- shared stuff --------------------------------------------------------------------------------

extern crate time;
extern crate rand;
extern crate serial;

use std::sync::mpsc;
use std::thread;


// Info packets:
//   implicit byte "enum variant" supplied by the compiler
//   unused 3 bytes for "alignment"
//   20 bytes payload, normally u32 clock_t + 16 bytes data
//
pub enum Info {

    Usr {                      // user events that only affect the display (not to save to disk)
        synced: bool           // true if buffers were just synced on disk (show "Sync" text)
    },

    Gps {                      // GPS timestamp and extra info
        t:      usize,         // timestamp in sysclock units
        ts:     usize,         // GPS timestamp
        alt:    isize,         // altitude in meters (expected -1000...10000)
        track:  isize,         // heading in degrees (expected 0..360;  <0: unknown)
        speed:  isize,         // speed in km*10/hr  (expected 0..2500; <0: unknown)
    },

    Pos {                      // GPS position
        t:      usize,         // timestamp in sysclock units
        lat:    f64,           // latitude
        lon:    f64,           // longitude
    },

    Obd {                      // OBD-II record
        t:      usize,         // timestamp in sysclock units
        pid:    usize,         // obd-ii pid
        val:    isize,         // raw binary value (or value*10), depending on pid
        extra:  isize,         // unused, zero cost
        extra2: isize          // unused, zero cost
    },

    Imu {
        t:      usize,         // timestamp in sysclock units
        mag:    [i16; 3],      // magnetometer xyz
        acc:    [i16; 3],      // accelerometer xyz (2g)
        rot:    [i16; 2]       // gyroscopic xy
    }
}


pub fn wait(msec: u64) {       // pause in milliseconds
    if msec == 0 {
        thread::yield_now()
    } else {
        thread::sleep(std::time::Duration::from_millis(msec))
    }
}


// --- shameful unsafe stuff ----------------------------------------------------------------------

extern crate libc;

mod ffi {
    extern { pub fn clock() -> ::libc::clock_t; }
    extern { pub fn sync(); }
}

pub fn sync() {
    unsafe { ffi::sync() }
}

pub fn clock() -> usize {
    unsafe { ffi::clock() as usize }
}


// --- convenience macros -------------------------------------------------------------------------

macro_rules! normalize(
    ($var: expr, $min: expr, $max: expr) => (
        if $var < $min { $min } else { if $var > $max { $max } else { $var } }
    )
);

macro_rules! start(
    ($name:expr, $code:expr) => (
        thread::Builder::new().name($name.to_string()).spawn(move || { $code }).expect("spawn thread");
    )
);


// --- main thread --------------------------------------------------------------------------------

fn main() {
    // fetch the three command-line arguments
    //
    let arg: Vec<String> = std::env::args().collect();
    if arg.len() != 4 {
        panic!("expecting three arguments (serialobd, serialimu, datadirectory)")
    }
    let ser = arg[1].to_string();
    let imu = arg[2].to_string();
    let dir = arg[3].to_string();

    // initialize graphics subsystems
    //
    let mut fb = fb::Video::new().expect("!--cannot init graphics framebuffer");
    let mut vc = vcsa::Video::new().expect("!--cannot open virtual console");

    // activate channels and start threads
    //
    let (txbuf, rxbuf) = mpsc::channel();      // messaging from main to buffer
    let (tx,    rx)    = mpsc::channel();      // messaging from threads to main

    let txc = tx.clone();  start!("buffer", buffer::main(&dir[..], rxbuf, txc));
    let txc = tx.clone();  start!("gpsd",   gpsd::main(txc));
    let txc = tx.clone();  start!("obd",    obd::main(&ser[..], txc));
    let txc = tx.clone();  start!("imu",    imu::main(&imu[..], txc));

    // preliminary loop, waiting for trouble codes
    //
    let mut tc = 3;
    loop {
        match rx.recv().expect("main recv") {
            Info::Obd { t, pid, val, .. }  if pid == obd::TROUBLE => {
                if val == 0 {
                    break          // no more trouble codes
                }

                if tc == 3 {       // troubles? prepare screen
                    fb.cls(0);
                    vc.paper(4);
                    vc.ink(7);
                    vc.puts(3, 3, " ATTENZIONE: ");
                    vc.paper(0);
                    vc.ink(14);
                }

                let s = format!("{}", troublecodes::msg(val));
                vc.puts(3, tc + 3, &s);
                tc += 3;
                txbuf.send(Info::Obd { t: t, pid: pid, val: val, extra: 0, extra2: 0 }).expect("early send")
            },
            any => {
                txbuf.send(any).expect("early msg")      // forward any info
            }
        }
    }
    if tc > 3 {                // wait some time when showing the trouble codes
        wait(7000)
    }

    // draw main screen
    //
    fb.cls(0);
    vc.ink(15);
    vc.puts(0,0, "V M    acceleratore:           giri/minuto:                 G A");
    vc.puts(6,8,       "carico motore:           velocit|:");
    vc.puts(7,16, "aria:");
    vc.puts(6,17, "acqua:");
    vc.ink(14);

    let mut accel  = Widget { min:  -512.0,   max:  512.0,    last: 0,
                              wid:  205,      hgt:  204,      xpos: 60,   ypos: 34,
                              ink:  0xf3f399, ink2: 0x000033, pap:  0,    bord: 0x000077 };

    let mut gyro   = Widget { min:  -512.0,   max:  512.0,    last: 0,
                              wid:  205,      hgt:  204,      xpos: 60,   ypos: 290,
                              ink:  0xf5f577, ink2: 0x000033, pap:  0,    bord: 0x000077 };

    let mut throt  = Widget { min:  0.0,      max:  100.0,    last: 0,
                              wid:  695,      hgt:  102,      xpos: 265,  ypos: 34,
                              ink:  0xff2222, ink2: 0,        pap:  0,    bord: 0x000077 };

    let mut throtv = Widget { min:  0.0,      max:  100.0,    last: 0,
                              wid:  32,       hgt:  566,      xpos: 992,  ypos: 34,
                              ink:  0xff2222, ink2: 0x000033, pap:  0,    bord: 0x000077 };

    let mut rpm    = Widget { min:  700.0,    max:  3800.0,   last: 0,
                              wid:  695,      hgt:  102,      xpos: 265,  ypos: 136,
                              ink:  0x00ff00, ink2: 0,        pap:  0,    bord: 0x000077 };

    let mut rpmv   = Widget { min:  700.0,    max:  3800.0,   last: 0,
                              wid:  32,       hgt:  566,      xpos: 960,  ypos: 34,
                              ink:  0x00ff00, ink2: 0x000033, pap:  0,    bord: 0x000077 };

    let mut eload  = Widget { min:  0.0,      max:  100.0,    last: 0,
                              wid:  695,      hgt:  102,      xpos: 265,  ypos: 290,
                              ink:  0x1111ff, ink2: 0,        pap:  0,    bord: 0x000077 };

    let mut eloadv = Widget { min:  0.0,      max:  100.0,    last: 0,
                              wid:  30,       hgt:  566,      xpos: 30,   ypos: 34,
                              ink:  0x1111ff, ink2: 0x000033, pap:  0,    bord: 0x000077 };

    let mut kmh    = Widget { min:  0.0,      max:  100.0,    last: 0,
                              wid:  695,      hgt:  102,      xpos: 265,  ypos: 392,
                              ink:  0x00ee44, ink2: 0,        pap:  0,    bord: 0x000077 };

    let mut kmhv   = Widget { min:  0.0,      max:  100.0,    last: 0,
                              wid:  30,       hgt:  566,      xpos: 0,    ypos: 34,
                              ink:  0x00ee44, ink2: 0x000033, pap:  0,    bord: 0x000077 };

    let mut sftrim = Widget { min: -100.0,    max:  100.0,    last: 0,
                              wid:  500,      hgt:  48,       xpos: 290,  ypos: 504,
                              ink:  0x4444ff, ink2: 0xff4444, pap:  0,    bord: 0x000077 };

    let mut lftrim = Widget { min: -100.0,    max:  100.0,    last: 0,
                              wid:  500,      hgt:  48,       xpos: 290,  ypos: 552,
                              ink:  0x4444ff, ink2: 0xff4444, pap:  0,    bord: 0x000077 };
    throt.setup_hist(&mut fb);
    throtv.setup_level(&mut fb);
    rpm.setup_hist(&mut fb);
    rpmv.setup_level(&mut fb);
    eload.setup_hist(&mut fb);
    eloadv.setup_level(&mut fb);
    kmh.setup_hist(&mut fb);
    kmhv.setup_level(&mut fb);
    sftrim.setup_diff(&mut fb);
    lftrim.setup_diff(&mut fb);
    gyro.setup_curs(&mut fb);
    accel.setup_curs(&mut fb);

    // main polling loop
    //
    loop {
        let rcv = rx.recv().expect("main mpsc recv");

        match rcv {
            Info::Imu { rot, acc, .. } => {
                gyro.update_curs(&mut fb, rot[0], rot[1]);
                accel.update_curs(&mut fb, acc[1], acc[0]);
            },

            Info::Usr { synced } => {
                if synced {
                    vc.puts(49, 0, "Sync")
                } else {
                    vc.puts(49, 0, "    ")
                }
            },

            Info::Pos { lat, .. } => {
                if lat == std::f64::NAN {
                    vc.puts(55, 0, "   ")
                } else {
                    vc.puts(55, 0, "GPS")
                }
            },

            Info::Gps { speed, .. } => {
                if speed < 10 {
                    vc.puts(52, 8, "       ")
                } else {
                    let mut spd = format!("({})  ", speed / 10);
                    spd.truncate(7);
                    vc.puts(52, 8, &spd)
                }
            },

            Info::Obd { pid, val, .. } => {
                let infoz = val as f64 / 10.0;
                let infoy = val as usize;

                match pid {
                    obd::RPM => {
                        rpm.update_hist(&mut fb, infoz);
                        rpmv.update_level(&mut fb, infoz);
                        vc.puts(44, 0, &format!("{}  ", infoy / 10));
                    },
                    obd::SPEED => {
                        kmh.update_hist(&mut fb, infoz);
                        kmhv.update_level(&mut fb, infoz);
                        if infoy == 0 {
                            vc.puts(41, 8, "fermo   ");
                        } else {
                            let mut spd = format!("{} km/h  ", infoy / 10);
                            spd.truncate(8);
                            vc.puts(41, 8, &spd);
                        }
                    },
                    obd::THROT => {
                        throt.update_hist(&mut fb, infoz);
                        throtv.update_level(&mut fb, infoz);
                        vc.puts(21, 0, &format!("{}.{}%   ", infoy / 10, infoy % 10));
                    },
                    obd::ELOAD => {
                        eload.update_hist(&mut fb, infoz);
                        eloadv.update_level(&mut fb, infoz);
                        vc.puts(21, 8, &format!("{}.{}%   ", infoy / 10, infoy % 10));
                    },
                    obd::AIRTEMP => {
                        let mut temp = format!("{}^  ", infoy as isize / 10);
                        temp.truncate(4);
                        vc.puts(13,16, &temp)
                    },
                    obd::ECTEMP => {
                        let mut temp = format!("{}^  ", infoy as isize / 10);
                        temp.truncate(4);
                        vc.puts(13,17, &temp)
                    },
                    obd::FSTATUS => {
                        match infoy / 10 {
                            0x0100 => { vc.puts(6, 8, "+ aria") },
                            0x0400 => { vc.puts(6, 8, "freno ") },
                            _      => { vc.puts(6, 8, "carico") }
                        }
                    },
                    obd::SFTRIM1 => {
                        sftrim.update_diff(&mut fb, infoz)
                    },
                    obd::LFTRIM1 => {
                        lftrim.update_diff(&mut fb, infoz)
                    },
                    obd::EGR => {
                        let mut egr = format!("{}.{}%  ", infoy / 10, infoy % 10);
                        egr.truncate(6);
                        vc.puts(52, 17, &egr)
                    },
                    obd::BATTERY => {
                        let mut bat = format!("{}.{} V   ", infoy / 10, infoy % 10);
                        bat.truncate(6);
                        vc.puts(52, 16, &bat)
                    },
                    _ => { }
                }
            }
        }

        txbuf.send(rcv).expect("forward")        // forward any info
    }
}


struct Widget {
    min:  f64,         // displayable range
    max:  f64,

    wid:  usize,       // widget area width, height and coordinates
    hgt:  usize,
    xpos: usize,
    ypos: usize,

    ink:  u32,         // main RGB color (0xRRGGBB)
    ink2: u32,         // secondary RGB
    pap:  u32,         // "paper" color
    bord: u32,         // border color

    last: usize,       // last pixel width
}

impl Widget {

    // scale value to widget height following screen physical coordinate
    // (low input values have high y coordinate)
    //
    fn to_hgt(&self, v: f64) -> usize {
        let resol = (self.max - self.min).abs() / self.hgt as f64;
        let r = ((v - self.min) / resol) as usize;
        if r > self.hgt {
            0                  // target coordinate: highest (100%)
        } else {
            self.hgt - r       // target coordinate: medium to low widget area
        }
    }


    // draw a border (n rectangles) shrinking widget available area
    //
    fn border(&mut self, g: &mut fb::Video, c: u32, mut n: usize) {
        while n > 0 {
            n -= 1;
            g.rectangle(c, self.xpos, self.ypos, self.wid, self.hgt);
            self.xpos += 1;
            self.ypos += 1;
            self.wid -= 2;
            self.hgt -= 2
        }
    }


    // clear widget drawable area
    //
    fn cls(&mut self, g: &mut fb::Video) {
         g.fillbox(self.pap, self.xpos, self.ypos, self.wid, self.hgt)
    }


    // setup horizontal-scrolling histogram widget
    //
    fn setup_hist(&mut self, g: &mut fb::Video) {
        let border_ext_color = self.bord;
        let border_int_color = self.pap;
        self.border(g, border_ext_color, 1);
        self.border(g, border_int_color, 1);
        self.cls(g)
    }


    // scroll the histogram and draw a new chunk
    //
    fn update_hist(&mut self, g: &mut fb::Video, val: f64) {
        let value = self.to_hgt(normalize!(val, self.min, self.max));
        let pixels = 1;
        g.leftscroll(self.xpos, self.ypos, self.wid, self.hgt, pixels);

        // it's a few pixels only; don't bother drawing only the differences

        if value > 0 {                 // black pixels on top area?
            g.vertfillbox(self.pap,
                          self.xpos + self.wid - pixels,
                          self.ypos,
                          pixels,
                          value)
        }
        if value < self.hgt {          // colored pixels in lower widget area?
            g.vertfillbox(self.ink,
                          self.xpos + self.wid - pixels,
                          self.ypos + value,
                          pixels,
                          self.hgt - value)
        }

    }


    // prepare a vertical indicator (level widget):
    //
    fn setup_level(&mut self, g: &mut fb::Video) {
        let border_color = self.bord;
        self.border(g, border_color, 1);

        // having a slightly brighter "paper" area will highlight the maximum value
        //
        let (tmp_paper, x, y, w, h) = (self.ink2, self.xpos, self.ypos, self.wid, self.hgt);
        g.fillbox(tmp_paper, x, y, w, h);

        // "zero" is the top value (entire height)
        //
        self.last = self.hgt
    }


    // update a vertical indicator (level widget)
    //
    fn update_level(&mut self, g: &mut fb::Video, val: f64) {
        let lev = self.to_hgt(normalize!(val, self.min, self.max));
        if lev != self.last {
            if lev == self.hgt {               // empty?
                self.cls(g)
            } else if lev == 0 {               // full?
                g.fillbox(self.ink, self.xpos, self.ypos, self.wid, self.hgt)
            } else if lev < self.last {        // need to add ink?
                g.fillbox(self.ink, self.xpos, self.ypos + lev, self.wid, self.last - lev)
            } else {                           // need to remove ink?
                g.fillbox(self.pap, self.xpos, self.ypos + self.last, self.wid, lev - self.last)
            }
            self.last = lev
        }
    }


    // setup a differential widget (horizontally "mirrored" positive/negative values)
    //
    fn setup_diff(&mut self, g: &mut fb::Video) {
        let border_color = self.bord;
        self.border(g, border_color, 1);
        self.border(g, 0, 1);
        self.cls(g)
    }


    // update the differential widget with a new value
    //
    fn update_diff(&mut self, g: &mut fb::Video, val: f64) {
        let value = normalize!(val, self.min, self.max);
        let curr = ((value - self.min) / ((self.max - self.min).abs()) * self.wid as f64) as usize;

        if curr == self.last {
            return
        }

        let half = self.wid / 2;

        if curr < half {               // new "blue" (right-side) value?

            if self.last >= half {     // if last drawn box was in the "red" area, clean it
                g.fillbox(self.pap, self.xpos + half, self.ypos, half, self.hgt);
                self.last = half;
            }

            if curr < self.last {  // yes: add pixels, no: erase pixels
                g.fillbox(self.ink, self.xpos + curr, self.ypos, self.last - curr, self.hgt)
            } else {
                g.fillbox(self.pap, self.xpos + self.last, self.ypos, curr - self.last, self.hgt)
            }

        } else {                       // zero or "red" (positive) value:

            if self.last < half {      // if last drawn box was in the negative half, clean it
                g.fillbox(self.pap, self.xpos, self.ypos, half, self.hgt);
                self.last = half
            }
            if curr > self.last {      // yes: add pixels, no: erase pixels
                g.fillbox(self.ink2, self.xpos + self.last, self.ypos, curr - self.last, self.hgt)
            } else {
                g.fillbox(self.pap, self.xpos + curr, self.ypos, self.last - curr, self.hgt)
            }
        }

        self.last = curr
    }


    // setup a generic rectangular area - quite like hist or diff
    //
    fn setup_curs(&mut self, g: &mut fb::Video) {
        self.setup_diff(g)
    }


    // update cursor thing
    //
    fn update_curs(&mut self, g: &mut fb::Video, xp: i16, yp: i16) {
        let x = self.xpos + (xp + 512) as usize * self.wid / 1024;
        let y = self.ypos + (512 - yp) as usize * self.wid / 1024;
        let curr = (x << 16) | y;
        if curr == self.last {
            return
        }

        let lastx = self.last >> 16;
        let lasty = self.last & 65535;
        self.last = curr;

        g.fillbox(self.ink2, lastx - 1, lasty - 1, 3, 3);
        g.fillbox(self.ink,  x - 1,     y - 1,     3, 3)
    }
}

