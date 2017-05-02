/// interface to obd-ii dongle
/// - via serial port; don't assume it is always connected
/// - sends an Info message with "field, value" info, both integer and floating point


// ---- configuration ------------------------------------------------------------------------------

// timeouts in milliseconds
//
const TIMEOUT:   u64 = 3100;  // read/write timeout on serial port
const RETRYWAIT: u64 = 2777;  // time to wait before another attempt to open the serial port
const FAILWAIT:  u64 = 1777;  // time to wait before


// ---- obd constants are also used as message identifiers -----------------------------------------
//
pub const RPM:     usize = 0x10c;
pub const THROT:   usize = 0x111;
pub const ELOAD:   usize = 0x104;
pub const SPEED:   usize = 0x10d;
pub const AIRTEMP: usize = 0x10f;
pub const ECTEMP:  usize = 0x105;
pub const EGR:     usize = 0x12c;
pub const EEGR:    usize = 0x12d;
pub const BPRESS:  usize = 0x133;
pub const FUEL:    usize = 0x12f;
pub const FSTATUS: usize = 0x103;
pub const SFTRIM1: usize = 0x106;
pub const LFTRIM1: usize = 0x107;
pub const SFTRIM2: usize = 0x108;
pub const LFTRIM2: usize = 0x109;
pub const TIMING:  usize = 0x10e;
pub const INTAKE:  usize = 0x10f;
pub const MAFLOW:  usize = 0x110;
pub const FPRESSD: usize = 0x123;
pub const FPRESSM: usize = 0x122;
pub const EVAP:    usize = 0x12e;
pub const CATA1S1: usize = 0x13c;
pub const CATA2S1: usize = 0x13d;
pub const CATA1S2: usize = 0x13e;
pub const CATA2S2: usize = 0x13f;
pub const RUNTIME: usize = 0x11f;
pub const MIL:     usize = 0x121;
pub const WARMUPS: usize = 0x130;
pub const MILSTAT: usize = 0x101;
pub const MAXPIDS: usize = 0x180;      // pids theoretically available as of common OBD-II dongles

pub const BATTERY: usize = 0x181;      // battery voltage info (volts * 10)
pub const TROUBLE: usize = 0x182;      // trouble code message (0 for "no more trouble codes")
pub const CAPA:    usize = 0x183;      // information packet regarding OBD dongle capabilities


// --- convenience macros -------------------------------------------------------------------------

macro_rules! normalize(
    ($var: expr, $min: expr, $max: expr) => (
        if $var < $min { $min } else { if $var > $max { $max } else { $var } }
    )
);

macro_rules! start(
    ($name:expr, $code:expr) => (
        ::std::thread::Builder::new().name($name.to_string()).spawn(move || { $code }).expect("thread spawn");
    )
);


// ---- low-level stuff ----------------------------------------------------------------------------

use ::serial::prelude::*;
use ::serial::posix::TTYPort;
use rand::random;

use std::str;
use std::time::Duration;
use std::io::prelude::*;
use std::sync::mpsc;

use Info;


// obd dongle internal configuration (the one loaded with "atz" command) shall be prepared with:
//    atpp0dsv0a\r   # set "endline" to '\n'
//    atpp0don\r     # save it to NVRAM
//    atpp0csv23\r   # set "baudrate" to 115200 (08:500000, 23:115200, 68:38400)
//    atpp0con\r     # save it to NVRAM
// then cross your fingers and unplug and plug again the dongle
//
// typical serial port setup - serial crate does not allow speeds higher than 115200,
// obd dongle must be configured accordingly:
//
const OBD_SETUP:  ::serial::PortSettings = ::serial::PortSettings {
    baud_rate:    ::serial::Baud115200,
    char_size:    ::serial::Bits8,
    parity:       ::serial::ParityNone,
    stop_bits:    ::serial::Stop1,
    flow_control: ::serial::FlowNone
};


// obd environment, to avoid to propagate everything down to leaf functions
//
struct Obd<'a>{
    port:  &'a mut TTYPort,            // serial port
    capa:  [bool; MAXPIDS],            // capabilities: true if pid is available
    tx:    &'a mpsc::Sender<Info>,     // message queue
    rpm:   usize,                      // last known "engine rotations per minute" value
    crash: bool                        // if true, dongle must be initialized again
}


// ---- main thread - requires a serial port and a channel -----------------------------------------

pub fn main(device: &str, tx: mpsc::Sender<Info>) {
    if device.len() == 0 {
        // no troubles in simulation/demo mode:
        //
        tx.send(Info::Obd { t: ::clock(), pid: TROUBLE, val: 0, extra: 0, extra2: 0 }).expect("obd sim");
        tx.send(Info::Obd { t: ::clock(), pid: MILSTAT, val: 0, extra: 0, extra2: 0 }).expect("sim obd");

        let txc = tx.clone();  start!("battery", simula(BATTERY,   11.8, 14.6,   txc));
        let txc = tx.clone();  start!("rpm",     simula(RPM,      800.0, 3000.0, txc));
        let txc = tx.clone();  start!("eload",   simula(ELOAD,      0.0, 100.0,  txc));
        let txc = tx.clone();  start!("speed",   simula(SPEED,      0.0, 80.0,   txc));
        let txc = tx.clone();  start!("throt",   simula(THROT,      0.0, 100.0,  txc));
        let txc = tx.clone();  start!("ectemp",  simula(ECTEMP,    20.0, 120.0,  txc));
        let txc = tx.clone();  start!("airtemp", simula(AIRTEMP,   -5.0, 50.0,   txc));
        let txc = tx.clone();  start!("sftrim1", simula(SFTRIM1, -100.0, 100.0,  txc));
        let txc = tx.clone();  start!("lftrim1", simula(LFTRIM1, -100.0, 100.0,  txc));
        let txc = tx.clone();  start!("egr",     simula(EGR,        0.0, 100.0,  txc));
        return
    }

    const MAXTRIES: usize = 10;        // obd open: a few tries before giving up

    for try in 1..MAXTRIES {
        match ::serial::open(&device) {  // is it available and ready?
            Err(e) => {
                if try==MAXTRIES-1 {
                    println!("!--obd serial port {} not available, giving up - {}", &device, e);
                    return
                } else {
                    println!("!--obd serial port not available, retrying ({}): {}", try, e);
                    ::wait(RETRYWAIT);
                    continue
                }
            },

            Ok(mut port) => {
                port.configure(&OBD_SETUP).expect("config port");
                port.set_timeout(Duration::from_millis(TIMEOUT)).expect("timeout set");
                let mut obd = Obd { port:  &mut port,
                                    capa:  [ false; MAXPIDS ],
                                    tx:    &tx,
                                    rpm:   0,
                                    crash: false };

                // kick in a carriage return and wait the dongle to digest it, then initialize
                //
                obd.cmd("\n");
                ::wait(300);
                obd.cmd("atz\n");
                ::wait(700);

                if obd.cmd_ok("atl1\n")   ||  // linefeeds: on (debugging only)
                    obd.cmd_ok("ate0\n")  ||  // echo: off
                    obd.cmd_ok("atsp0\n") ||  // protocol: auto
                    obd.cmd_ok("ats0\n")  ||  // spaces between values: off
                    obd.cmd_ok("atal\n")  ||  // long messages: allow
                    obd.cmd_ok("ath0\n")  ||  // display headers: off
                    obd.cmd_ok("atd0\n") {    // display DLC: off
                    println!("!--obd dongle configuration failure, retrying ({})", try);
                    continue
                }

                // get VIN (vehicle identification number), 17 to 20 digits, if supported
                //
                let _ = obd.cmd_multi("0902\n");

                // get calibration string, up to 16 digits
                //
                let _ = obd.cmd_multi("0904\n");

                // fetch obd interface capabilities at least for the 0x100-0x17f groups:
                //
                let mut cap: usize = 0;

                for c in 0..MAXPIDS-1 {
                    let idx = c & 0x1f;
                    if idx == 0 && c >= 0x100 {
                        match obd.pid(c, 4) {  // expecting 4 bytes (32 flag bits)
                            Err(_) => { },     // ignore errors
                            Ok(pkt) => {
                                cap = pkt;
                                obd.tx.send(Info::Obd {
                                    t: ::clock(),
                                    pid: CAPA,
                                    val: cap as isize,
                                    extra: 0,
                                    extra2: 0 }).expect("obd send capa");
                            }
                        }
                    }

                    obd.capa[c + 1] = ((cap >> (0x1f - idx)) & 1) == 1
                }
                obd.capa[RPM] = true;  // patch needed to catch "unconnected port" errors

                // fetch trouble codes
                //
                let s = obd.cmd("03\n");
                if s.len() >= 14 {
                    let tc1 = u32::from_str_radix(&s[2..6], 10).unwrap_or(9999);
                    let tc2 = u32::from_str_radix(&s[6..10], 10).unwrap_or(9999);
                    let tc3 = u32::from_str_radix(&s[10..14], 10).unwrap_or(9999);

                    if tc1 > 0 {
                        obd.tx.send(Info::Obd {
                            t: ::clock(),
                            pid: TROUBLE,
                            val: tc1 as isize,
                            extra: 0,
                            extra2: 0 }).expect("obd send pkt trouble");
                    }
                    if tc2 > 0 {
                        obd.tx.send(Info::Obd {
                            t: ::clock(),
                            pid: TROUBLE,
                            val: tc2 as isize,
                            extra: 0,
                            extra2: 0 }).expect("obd send pkt trouble");
                    }
                    if tc3 > 0 {
                        obd.tx.send(Info::Obd {
                            t: ::clock(),
                            pid: TROUBLE,
                            val: tc3 as isize,
                            extra: 0,
                            extra2: 0 }).expect("obd send pkt trouble");
                    }

                    obd.tx.send(Info::Obd {
                            t: ::clock(),   // end of troubles
                            pid: TROUBLE,
                            val: 0,
                            extra: 0,
                            extra2: 0 }).expect("obd send trouble codes");
                }

                // dongle initialization complete, start the main obd read loop:
                //
                obd.mainloop();
                println!("!--obd loop crashed");   // now init again serial dongle and obd structure
            }
        }
        ::wait(FAILWAIT)               // a little wait after a failed serial open
    }

    println!("!--obd task gave up");
}


// obd simulator task - development only, not for sale
//
fn simula(p: usize, min: f64, max: f64, tx: mpsc::Sender<Info>) {
    let mut v = min + (max - min).abs() / 2.0;
    loop {
        tx.send(Info::Obd { t: ::clock(),
                            pid: p,
                            val: (v * 10.0) as isize,
                            extra: 0,
                            extra2: 0 }).expect("obd sim pkt");

        let diff = (((random::<u16>() % 2001) as i16 - 1000) as f64) / 1000.0;
        v += (max - min).abs() / 20.0 * diff;
        v = normalize!(v, min, max);

        ::wait((random::<u16>() % 300) as u64 + 300 );
    }
}


// utilities: reduce a 0..255 value to 0..1000
//
fn perc(val: usize) -> usize {
    ((val as f64 / 2.55) * 10.0) as usize
}


// reduce a 0..255 value to -640..635
//
fn halfdeg(val: usize) -> usize {
    ((val as isize) * 128 - 64*256) as usize
}


// reduce a 16 bit value to a 0..5178 kPa*10
//
fn kpa10(val: usize) -> usize {
    (val as f64 * 0.079 * 10.0) as usize
}


// reduce a 16 bit value to a catalyst temperature -40..6513.5 C * 10
//
fn cata(val: usize) -> usize {
    val - 40 * 10
}


impl<'a> Obd<'a> {

    // send a numeric 4-digits pid command, fetch and decode the hex string reply
    //
    fn pid(&mut self, pid: usize, expected: usize) -> Result<usize, String> {
        let command = format!("{:04x}\n", pid);        // always use 4 hex digits in commands
        let digits = expected * 2;                     // expected hex digits in replies

        match self.get_pid_val(&command) {
            Err(e) => Err(e),
            Ok(s) => {
                if s.len() != digits {
                    println!("!--obd pid {:03x} returned {}/{} chars [{}]", pid, s.len(), digits, s);
                    Err(String::from("invalid pid reply size"))
                } else {
                    match u32::from_str_radix(&s[..], 16) {    // convert hex digits to u32
                        Err(e) => {
                            println!("!--obd pid {:03x} returned invalid value {} ({})", pid, s, e);
                            Err(String::from("invalid pid reply format"))
                        },
                        Ok(x) => {
                            if (expected == 1 && x >= 0x100) || (expected == 2 && x >= 0x10000) {
                                println!("!--obd pid {:03x} overflow return value {:08x}", pid, x);
                                Err(String::from("invalid pid reply value"))
                            } else {
                                Ok(x as usize)
                            }
                        }
                    }
                }
            }
        }
    }


    // send a PID command (including the line terminator) and fetch a decent reply
    //
    fn get_pid_val(&mut self, command: &str) -> Result<String, String> {
        if cfg!(feature = "obdecho") {
            print!("WRITE: {}", command);
        }

        match self.port.write(command.as_bytes()) {
            Err(e) => {
                println!("!--obd send command: {}", e);
                Err("obd write error".to_string())
            },
            Ok(_) => {
                match self.get_reply() {
                    Err(e) => Err(e),
    
                    Ok(mut s) => {
                        // special case: AT commands with numeric-only answers
                        // -- (currently only used to read battery voltage)
                        if command.starts_with("a") {
                            s = s.trim_matches(|c| (c < '0' && c != '.') || (c > '9')).to_string();
                            if s.len() > 0 {
                                Ok(s)
                            } else {
                                println!("!--obd reply contains invalid characters: {}", s);
                                Err(s)
                            }
                        } else {                       // else: normal pids hex replies
                            if s.starts_with("4") {
                                Ok(s.split_at(4).1.to_string())  // "4xyz" = pid xyz OK - skip header
                            } else {
                                Err(s)
                            }
                        }
                    }
                }
            }
        }
    }


    // fetch serial data until prompt, exclude non-significant characters and useless messages
    //
    fn get_reply(&mut self) -> Result<String, String> {
        let mut rcvd = String::with_capacity(32);
        let mut buf = [ 0u8; 2048 ];

        loop {
            match self.port.read(&mut buf[..]) {
                Err(e) => {
                    return Err(format!("{}", e))
                },

                Ok(bytes) => {
                    for n in 0..bytes {
                        let b = buf[n];
                        if b > 32 && b < 127 {          // wipe away spaces and non-ascii
                            if b != b'>' {              // if not an ending prompt:
                                rcvd.push(b as char);   // add to the reply string
                            } else {
                                if cfg!(feature = "obdecho") {
                                    println!("READ: {}", rcvd);
                                }

                                // wipe away the "searching" message:
                                if rcvd.starts_with("SEARCHING...") {
                                    let wiped = (rcvd.split_at(12).1).to_string();
                                    return Ok(wiped)
                                } else {
                                    return Ok(rcvd)
                                }
                            }
                        }
                    }
                }
            }
        }
    }


    // send an AT command and return the result String
    //
    fn cmd(&mut self, cmdstr: &str) -> String {
        if cfg!(feature = "obdecho") {
            print!("WRITE: {}", cmdstr);
        }

        match self.port.write(cmdstr.as_bytes()) {
            Err(e) => {
                println!("!--obd send command: {}", e);
                String::new()
            },

            Ok(_) => {
                match self.get_reply() {
                    Err(s) => {
                        println!("!--obd: read error: {}", s);
                        String::new()
                    },

                    Ok(s) => s
                }
            }
        }
    }


    // send an AT command and expect an OK; return true if something went wrong
    //
    fn cmd_ok(&mut self, cmdstr: &str) -> bool {
        self.cmd(cmdstr) != "OK"
    }


    // send a command expecting a multiline reply (for example VIN PID 0902)
    // reply is like:
    // 0....5....0....5....0....5....0....5....0....5....0....5
    // 49040131394243490402573439204904032020200049040400000...
    //       ^^^^^^^^      ^^^^^^^^      ^^^^^^^^      ^^^^^...
    //
    fn cmd_multi(&mut self, multi: &str) -> String {
        let mut reply = self.cmd(multi);
        let mut ret = String::with_capacity(40);
        while reply.len() >= 7 && reply.starts_with("4") {
            if reply.len() > 14 {
                ret.push_str(&reply[6..14]);
                reply = (reply[14..reply.len()]).to_string()
            } else {
                ret.push_str(&reply[6..reply.len()]);
                break
            }
        }
        ret
    }


    // read a pid if there is corresponding capability and emit message with the fetched value
    //
    fn emit<F>(&mut self, method: usize, bytes: usize, apply: F) where F: Fn(usize) -> usize {
        if ! self.capa[method] {
            return
        }

        match self.pid(method, bytes) {
            Err(_) => {
                // capability list says we were authorized to use this pid:
                // then ignore any errors, except if it was the frequently-requested RPM
                //
                self.crash = method == RPM;
            },
            Ok(value) => {
                let result = apply(value);              // up to 16 bit value

                if method == RPM {                      // always keep engine running status
                    self.rpm = result
                }

                self.tx.send(Info::Obd { t: ::clock(),
                                         pid: method,
                                         val: result as isize,
                                         extra: 0,
                                         extra2: 0 }).expect("obd emit pkt");
            }
        }
    }


    // loop reading obd parameters
    //
    fn mainloop(&mut self) {
        let mut batwait = 0;

        // emit the general "malfunction indicator lit" flags only once at beginning
        //
        self.emit(MILSTAT, 4, |x| x);

        loop {
            if batwait == 0 {                          // time to read battery voltage?
                match self.get_pid_val("atrv\n") {
                    Err(e) => {
                        println!("!--obd get battery status error: {}", e);
                        return
                    },

                    Ok(bat) => {
                        match str::parse::<f64>(bat.as_ref()) {
                            Err(e) => {
                                println!("!--obd parse error: battery[{}], error: {}", bat, e);
                                return;
                            },
                            Ok(v) => {
                                self.tx.send(Info::Obd {
                                    t: ::clock(),
                                    pid: BATTERY,
                                    val: (v * 10.0) as isize,
                                    extra: 0,
                                    extra2: 0 }).expect("obd send bat");
                            }
                        }
                    }
                }
            }
            batwait = (batwait + 1 ) % 5;  // wait 5 iterations before reading battery level again

            if self.crash {
                return
            }

            self.basicpids();
            self.temppids();

            self.basicpids();
            if self.rpm > 0 {
                self.fuelpids()
            }

            self.basicpids();
            if self.rpm > 0 {
                self.extrapids();
            }

            self.basicpids();
            if self.rpm > 0 {  
                self.catapids();
            }

            self.basicpids();
            if self.rpm > 0 {  
                self.infopids();
            }
        }
    }


    // read and edit a few basic pid values, calculating returnval*10
    //
    fn basicpids(&mut self) {
        self.emit(RPM,   2, |x| x + x + x/2);  // engine rpm
        self.emit(THROT, 1, |x| perc(x));      // % throttle
        self.emit(ELOAD, 1, |x| perc(x));      // % engine load
        self.emit(SPEED, 1, |x| x * 10);       // speed km/hr
    }


    // read and emit temperature and egr related pids
    //
    fn temppids(&mut self) {
        self.emit(AIRTEMP, 1, |x| (x - 40) * 10);      // air intake temperature
        self.emit(ECTEMP,  1, |x| (x - 40) * 10);      // engine coolant temperature
        self.emit(EGR,     1, |x| perc(x));            // commanded egr
        self.emit(EEGR,    1, |x| (x - 128) * 10);     // egr error
        self.emit(BPRESS,  1, |x| x * 10);             // absolute barometric pressure in kPa
    }


    // read and emit essential fuel-related pids
    //
    fn fuelpids(&mut self) {
        self.emit(FUEL,    1, |x| perc(x));    // available fuel
        self.emit(FSTATUS, 2, |x| x);          // fuel usage status (bank1*256+bank2)
        self.emit(SFTRIM1, 1, |x| perc(x));    // short term fuel trim (bank 1)
        self.emit(LFTRIM1, 1, |x| perc(x));    // long term fuel trim (bank 1)
        self.emit(SFTRIM2, 1, |x| perc(x));    // short term fuel trim (bank 2)
        self.emit(LFTRIM2, 1, |x| perc(x));    // long term fuel trim (bank 2)
    }


    // extra pids of interest when the engine is running
    //
    fn extrapids(&mut self) {
        self.emit(TIMING,  1, |x| halfdeg(x));         // timing advance, in half degrees -640..635
        self.emit(INTAKE,  1, |x| (x - 40) * 10);      // air intake temperature
        self.emit(MAFLOW,  2, |x| x);                  // MAF air flow rate, in decigrams/sec
        self.emit(FPRESSD, 2, |x| x * 10);             // fuel rail pressure (diesel) in kPa
        self.emit(FPRESSM, 2, |x| kpa10(x));           // fuel rail pressure relative to manifold * 10
        self.emit(EVAP,    1, |x| perc(x));            // commanded evaporative purge
    }


    // catalyst related pids, when the engine is running
    //
    fn catapids(&mut self) {
        self.emit(CATA1S1, 2, |x| cata(x));    // catalyst temperature bank 1 sensor 1
        self.emit(CATA2S1, 2, |x| cata(x));
        self.emit(CATA1S2, 2, |x| cata(x));
        self.emit(CATA2S2, 2, |x| cata(x));
    }


    // supplementary information pids
    //
    fn infopids(&mut self) {
        self.emit(RUNTIME, 2, |x| x * 10);     // time since engine start, in seconds
        self.emit(MIL,     2, |x| x * 10);     // kilometers traveled since malfunction indicator lit
        self.emit(WARMUPS, 1, |x| x * 10);     // warm-ups since codes cleared
    }
}

