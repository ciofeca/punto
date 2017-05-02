/// memory buffer - collect info packets to save on disk and sync

// --- configuration ------------------------------------------------------------------------------

const SECONDS:  u64   = 60;       // interval between buffer flush, in seconds
const MAXELEMS: usize = 115;      // maximum elements that typically get in wait queue every second
const BUFSIZE:  usize = 512;      // kilobytes output buffer for stream write


// --- main ---------------------------------------------------------------------------------------

use Info;

use std::io::prelude::*;
use std::io::BufWriter;
use std::fs::OpenOptions;
use std::sync::mpsc;
use std::mem;
use std::slice;


pub fn main(dir: &str,                 // writeable directory
            rx: mpsc::Receiver<Info>,  // events collector
            tx: mpsc::Sender<Info>) {  // to send "synced" events

    let start = ::std::time::Instant::now();   // to save one file per minute and to get unique filenames
    let mut elaps = SECONDS;                   // how many seconds from the start

    // the buffer has capacity for about 3 minutes, just in case:
    let mut buf = Vec::with_capacity(MAXELEMS * 3 * SECONDS as usize);

    loop {
        let event = rx.recv().expect("buffer rx recv");

        let lastt;     // remember last useful sysclock value, used here for unique file names
        match event {
            Info::Gps { t, .. } => { lastt = t; buf.push(event) },
            Info::Pos { t, .. } => { lastt = t; buf.push(event) },
            Info::Obd { t, .. } => { lastt = t; buf.push(event) },
            Info::Imu { t, .. } => { lastt = t; buf.push(event) },
            Info::Usr { .. }    => { continue }        // won't save local events
        };

        if start.elapsed().as_secs() < elaps {
            continue
        }

        let lastc = ::std::time::SystemTime::now().duration_since(::std::time::UNIX_EPOCH).
            expect("buffer duration since").as_secs();

        // create an output stream, a few retries in case of open error:
        //
        let mut ftemp;         // temporary filename
        let mut fname;         // final filename
        {
            let mut stream;    // where to serialize data
            let mut tries = 0; // how many times tried to create a file
            loop {
                let uniq = lastt + tries;      // changing at every retry
                tries += 1;

                let tm = ::time::at(::time::Timespec { sec: lastc as i64, nsec: 0 });
                let now = ::time::strftime("%Y%m%d.%H%M%S", &tm).expect("strftime");

                ftemp = format!("{}/dat.{}.{:08x}.tmp", dir, now, uniq);
                fname = format!("{}/dat.{}.{:08x}",     dir, now, uniq);

                match OpenOptions::new().create(true).write(true).open(&ftemp) {
                    Err(e) => {
                        if tries <= 5 {    // a few retries before panicking
                            ::wait(100);
                            continue;
                        }
                        panic!("!--file: create {}: {}", ftemp, e);
                    },
                    Ok(fp) => {
                        stream = BufWriter::with_capacity(BUFSIZE * 1024, fp);
                        break
                    }
                }
            }

            // serialize every object, then zap the vec
            //
            for idx in 0..buf.len() {
                let siz = mem::size_of::<Info>();
                let p: *const Info = &buf[idx];
                let p = p as *const u8;

                // actual binary data starts with the enum_instance byte,
                // followed by aligned data
                //
                let sli: &[u8] = unsafe { slice::from_raw_parts(p, siz) };
                stream.write(sli).expect("disk write error");
            }
            buf.clear();
        } // stream now closed

        // update to next "minute timeout", making sure we'll have to wait
        loop {
            elaps += SECONDS;
            if elaps > start.elapsed().as_secs() {
                break
            }
        }

        match ::std::fs::rename(&ftemp, &fname) {
            Ok(_) => {
                // another process will ship it to server
            },
            Err(e) => {
                println!("!--file: rename {}: {}", fname, e);
                ::wait(1000)
            }
        }

        ::sync();   // arrgh, sync data on memorycard and hope no power loss happens while syncing

        tx.send(Info::Usr { synced: true }).expect("buffer sync");

        // a while later, tell the main process to stop showing the "Sync" message:
        //
        ::wait(2377);
        tx.send(Info::Usr { synced: false }).expect("sync buffer")
    }
}

