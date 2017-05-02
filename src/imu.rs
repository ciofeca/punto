/// interface to inertial measurement unit
/// - via serial port; don't assume it is always connected


// ---- configuration ------------------------------------------------------------------------------

// timeouts in milliseconds
//
const TIMEOUT:   u64 = 3100;  // read/write timeout on serial port
const RETRYWAIT: u64 = 2777;  // time to wait before another attempt to open the serial port
const FAILWAIT:  u64 = 1777;  // time to wait before


// --- convenience macros -------------------------------------------------------------------------

macro_rules! start(
    ($name:expr, $code:expr) => (
        ::std::thread::Builder::new().name($name.to_string()).spawn(move || { $code }).expect("spawn")
    )
);


// ---- low-level stuff ----------------------------------------------------------------------------

use ::serial::prelude::*;
use ::serial::posix::TTYPort;
use ::rand::random;

use std::str;
use std::time::Duration;
use std::io::prelude::*;
use std::sync::mpsc;

use Info;


// typical serial port setup - serial crate does not allow speeds higher than 115200,
// imu dongle must be configured accordingly:
//
const IMU_SETUP:  ::serial::PortSettings = ::serial::PortSettings {
    baud_rate:    ::serial::Baud115200,
    char_size:    ::serial::Bits8,
    parity:       ::serial::ParityNone,
    stop_bits:    ::serial::Stop1,
    flow_control: ::serial::FlowNone
};


// ---- main thread - requires a serial port and a channel -----------------------------------------

pub fn main(device: &str, tx: mpsc::Sender<Info>) {
    if device.len() == 0 {
        loop {
            // simulation mode loop:
            //
            let mag = [ random::<i16>() & 15 - 8, random::<i16>() & 15 - 8, random::<i16>() & 15 - 8 ];
            let acc = [ random::<i16>() & 15 - 8, random::<i16>() & 15 - 8, random::<i16>() & 15 - 8 ];
            let rot = [ random::<i16>() & 15 - 8, random::<i16>() & 15 - 8                           ];

            tx.send(Info::Imu { t: ::clock(), mag: mag, acc: acc, rot: rot }).expect("imu sim pkt");

            ::wait(10);        // 10msec because we send 100 packets per second
        }
    }

    const MAXTRIES: usize = 10;        // imu open: a few tries before giving up

    for try in 1..MAXTRIES {
        match ::serial::open(&device) {        // is it available and ready?
            Err(e) => {
                if try==MAXTRIES-1 {
                    println!("!--imu serial port {} not available, giving up - {}", &device, e);
                    return
                } else {
                    println!("!--imu serial port not available, retrying ({}): {}", try, e);
                    ::wait(RETRYWAIT);
                    continue
                }
            },

            Ok(mut port) => {
                port.configure(&IMU_SETUP).expect("port config");
                port.set_timeout(Duration::from_millis(TIMEOUT)).expect("set timeout");

                mainloop(&mut port, &tx);
                ::wait(RETRYWAIT)
            }
        }
        ::wait(FAILWAIT)               // a little wait after a failed serial open
    }

    println!("!--imu task gave up");
}


fn mainloop(port: &mut TTYPort, tx: &mpsc::Sender<Info>) {
    let mut rcvd = String::with_capacity(100);
    let mut buf = [ 0u8; 100 ];

    let mut wait_for_a = true; // default state: waiting for 'A' record start
    loop {
        loop {
            match port.read(&mut buf[..]) {
                Err(e) => {
                    println!("!--imu read: {}", e);
                    return
                },

                Ok(bytes) => {
                    for n in 0..bytes {
                        let b = buf[n];
                        if b < 127 {                    // wipe away non-ascii

                            if wait_for_a {             // if waiting for 'A', ignore bytes
                                wait_for_a = b != b'A';
                                continue
                            }

                            if b != b'Z' {              // if not an ending prompt:
                                rcvd.push(b as char);   // add to the reply string
                            } else {
                                wait_for_a = true;      // got sufficient bytes

// typical tab-separated records: 'A' [counter] magx magy magx accx accy accz rotx roty rotz 'Z' '\n'
//  A	11188	623	569	647	507	526	792	600	437	539	Z
//  A	11189	622	570	649	507	526	794	599	437	539	Z

                                let mut val: Vec<i16> = Vec::new();
                                for v in rcvd.trim().split_whitespace() {
                                    val.push(str::parse::<u16>(v).unwrap_or(512) as i16 - 512)
                                }
                                rcvd.clear();

                                // calibrate gyro values because bizarre hardware mount
                                //
                                let rotx = if val[7] - 90 <= -512 { -511 } else { val[7] - 90 };
                                let roty = if val[8] + 78 >= 512  { 511  } else { val[8] + 78 };

                                // ignore first field (the 15 bit counter)
                                // and last field (gyro "spinning top");
                                // only collect:
                                // - magnetometer data x/y/z
                                // - accelerometer data (up to 1.5g): forward, lateral, vertical "bump"
                                // - gyroscopic data: lateral and pitch only
                                //
                                tx.send(Info::Imu { t: ::clock(),
                                    mag: [ val[1], val[2], val[3] ],
                                    acc: [ val[4], val[5], val[6] ],
                                    rot: [ rotx, roty ] }).expect("imu send rec");
                            }
                        }
                    }
                }
            }
        }
    }
}

