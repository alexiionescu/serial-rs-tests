use std::{thread::sleep, time::Duration};

use log::{debug, trace};

// rx_fifo_full_threshold
const READ_BUF_SIZE: usize = 128;
// EOT (CTRL-D)
const AT_CMD: u8 = 0x04;

// max message size to receive
// leave some extra space for AT-CMD characters
const MAX_BUFFER_SIZE: usize = 2 * READ_BUF_SIZE + 20;

pub fn test(port: &String, baud: u32) {
    let mut serial = serialport::new(port, baud)
        .open()
        .expect("Failed to open port");

    let mut rbuf = vec![0; MAX_BUFFER_SIZE];
    let mut wbuf = Vec::with_capacity(MAX_BUFFER_SIZE);
    wbuf.resize(8, b'0'); // header
    'init: for i in 1..500 {
        let hex_str = format!("{}", i);
        for c in hex_str.as_bytes() {
            if wbuf.push_within_capacity(*c).is_err() {
                break 'init;
            }
        }
    }

    let mut seq_no: u16 = 0;
    loop {
        if let Ok(n) = serial.read(rbuf.as_mut_slice()) {
            if n > 0 && (n > 1 || rbuf[0] != AT_CMD) {
                debug!("received {n} bytes {:02X?}", &rbuf[..8]);
                trace!("received {:02X?}", &rbuf[..n]);
                let str = String::from_utf8(rbuf[..n].to_vec()).unwrap();
                println!("{}", &str);
            }
        } else {
            sleep(Duration::from_secs(5));
            debug!("write {} bytes {:02X?}", wbuf.len(), &wbuf[..4]);
            seq_no += 1;
            wbuf[..4].copy_from_slice(format!("{seq_no:04X}").as_bytes());
            serial.write_all(&wbuf).ok();
            serial.flush().ok();
            sleep(Duration::from_millis(200));
            debug!("write at_cmd bytes");
            serial.write_all(&[AT_CMD]).ok();
            serial.flush().ok();
            loop {
                sleep(Duration::from_millis(200));
                if serial.bytes_to_read().unwrap() > 0 {
                    break;
                } else {
                    serial.write_all(&[AT_CMD]).ok();
                    serial.flush().ok();
                }
            }
        }
    }
}
