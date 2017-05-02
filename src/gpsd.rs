/// interface to gpsd daemon
/// - via local port 2947


// ---- low-level stuff ----------------------------------------------------------------------------

use std::net::TcpStream;
use std::io::{Read, Write};
use std::f64;
use std::str;
use std::sync::mpsc;

use Info;


// ---- main thread --------------------------------------------------------------------------------

pub fn main(tx: mpsc::Sender<Info>) {
    const MAXTRIES: usize = 10;        // some tries, then give up
    for try in 1..MAXTRIES {
        match TcpStream::connect("127.0.0.1:2947") {
            Err(e) => {
                if try==MAXTRIES-1 {
                    println!("!--gpsd port 2947 not available, giving up - {}", e);
                    return
                } else {
                    println!("!--gpsd port 2947 not available, retrying ({}): {}", try, e);
                    ::wait(3000)
                }
            },

            // from now on we assume that gpsd socket won't crash/misbehave without a monumental reason
            //
            Ok(gpsd) => {
                flow(&tx, gpsd);
                ::wait(1000)
            }
        }
    }

    println!("!--gpsd thread premature end");
}


fn flow(tx: &mpsc::Sender<Info>, mut gpsd: ::std::net::TcpStream) {
    gpsd.write(b"?WATCH={\"enable\":true,\"json\":true}").expect("gpsd socket write");

    let mut buf: [u8; 1024] = [0; 1024];  // json packets from gpsd daemon
    let mut n = 0;                        // number of bytes currently in buffer

    loop {                                // loop reading chunks of data
        match gpsd.read(&mut buf[n..]) {
            Err(e) => {
                println!("!--gpsd port 2947 read error {}", e);
                return
            },

            Ok(0) => {
                continue
            },

            Ok(got) => {
                n += got;

                for idx in n-got..n {          // support partial chunks, just in case
                    if buf[idx] == b'\n'  {    // completed a line? (including \r\n)

                        // we only need Time/Position/Velocity packets
                        if buf.starts_with(b"{\"class\":\"TPV\",") {
                            let fields = &buf[1..idx-3];  // 1 to skip "{", -3 to skip "}\r\n"
                            emit(&fields, &tx)            // send available fields out
                        }

                        let len = n-idx-1;     // remaining bytes to parse on next iteration
                        for i in 0..len {
                            buf[i] = buf[idx + i + 1]
                        }
                        n = len;
                        break                  // exit the "wait for a complete line" loop
                    }
                }
            }
        }
    }
}


fn emit(buf: &[u8], tx: &mpsc::Sender<Info>) {
    let mut t = 0;                     // GPS timestamp if available, in Unix seconds
    let mut lat = ::std::f64::NAN;     // latitude or NaN
    let mut lon = ::std::f64::NAN;     // longitude or NaN
    let mut alt = -1;                  // altitude in meters (expected -1000...10000)
    let mut track = -1;                // heading in degrees (expected 0..360)
    let mut speed = -1;                // speed in decimeters/second (expected 0..25000)

    let fields = String::from_utf8_lossy(&buf[..]);
    for fld in fields.split(",\"") {
        if fld.starts_with("time\":\"") {          // timestamp field found?
            let tstr = &fld[7..31];                // "2016-02-19T00:16:14.000Z"
            let year = str::parse::<i32>(&tstr[0..4]).unwrap_or(0);
            if year >= 2016 {
                let month = str::parse::<i32>(&tstr[5..7]).unwrap_or(0);
                let day   = str::parse::<i32>(&tstr[8..10]).unwrap_or(1);
                let hour  = str::parse::<i32>(&tstr[11..13]).unwrap_or(0);
                let min   = str::parse::<i32>(&tstr[14..16]).unwrap_or(0);
                let sec   = str::parse::<i32>(&tstr[17..19]).unwrap_or(0);
                let mill  = str::parse::<i32>(&tstr[20..23]).unwrap_or(0);

                let tm = ::time::Tm { tm_year: year-1900, tm_mon: month-1, tm_mday: day,
                                      tm_hour: hour, tm_min: min, tm_sec: sec,
                                      tm_wday: 0, tm_utcoff: 0, tm_isdst: -1, tm_yday: 0,
                                      tm_nsec: mill*1_000_000 }.to_timespec();
                t = tm.sec;
            }
            continue
        }

        if fld.starts_with("lat\":") {             // latitude
            lat = str::parse::<f64>(str::from_utf8(fld[5..].as_bytes()).
                expect("gps lat: not utf8")).expect("gps lat: not parseable");
            continue
        }

        if fld.starts_with("lon\":") {             // longitude
            lon = str::parse::<f64>(str::from_utf8(fld[5..].as_bytes()).
                expect("gps lon: not utf8")).expect("gps lon: not parseable");
            continue
        }

        if fld.starts_with("alt\":") {             // altitude in meters (ignore fraction)
            alt = str::parse::<f64>(str::from_utf8(fld[5..].as_bytes()).
                expect("gps alt: not utf8")).expect("gps alt: not parseable") as isize;
            continue
        }

        if fld.starts_with("track\":") {           // estimated heading (ignore grade fraction)
            track = str::parse::<f64>(str::from_utf8(fld[7..].as_bytes()).
                expect("gps track: not utf8")).expect("gps track: not parseable") as isize;
            continue
        }

        if fld.starts_with("speed\":") {           // speed (convert from m/s to dm/hr and round)
            speed = ((str::parse::<f64>(str::from_utf8(fld[7..].as_bytes()).
                expect("gps speed: not utf8")).expect("gps speed: not parseable") * 36.0).
                    floor() / 10.0) as isize;
            continue
        }
    }

    tx.send(Info::Gps {
        t: ::clock(),
        ts: t as usize,
        alt: alt,
        track: track,
        speed: speed
    }).expect("gpsd send packet");

    // future modification: don't send a NaN packet after a NaN one
    //
    tx.send(Info::Pos {
        t: ::clock(),
        lat: lat,
        lon: lon,
    }).expect("gpsd packet send");
}

