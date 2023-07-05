#![allow(unused_imports)]

use std::{
    sync::{
        atomic::{AtomicU16, Ordering},
        Arc, RwLock,
    },
    thread::{self, sleep},
    time::Duration,
};

use log::{debug, error, info, trace, warn};
use rand_distr::{Distribution, Normal};

use crate::ConnectArgs;

// rx_fifo_full_threshold
const READ_BUF_SIZE: usize = 128;
// EOT (CTRL-D)
const AT_CMD: u8 = 0x04;
const AT_ESC: u8 = 0x1b;
const AT_ESC_MASK: u8 = 0x30;

// max message size to receive
// leave some extra space for AT-CMD characters
const MAX_BUFFER_SIZE: usize = 5 * READ_BUF_SIZE + 20;
const RESET_BUFFER_SIZE: usize = MAX_BUFFER_SIZE - READ_BUF_SIZE;

type UartVec = Vec<u8>;

#[inline]
fn pop_escaped(buf: &[u8], offset: &mut usize) -> Option<u8> {
    if buf.is_empty() {
        return None;
    }
    if buf[0] == AT_ESC {
        if buf.len() == 1 {
            None
        } else {
            *offset += 2;
            match buf[1] {
                AT_ESC => Some(AT_ESC),
                b => Some(b & !AT_ESC_MASK),
            }
        }
    } else {
        *offset += 1;
        Some(buf[0])
    }
}
trait PushEscape {
    fn push_escaped(&mut self, b: u8);
    fn pop_escaped(&mut self) -> Option<u8>;
}

impl PushEscape for UartVec {
    fn push_escaped(&mut self, b: u8) {
        match b {
            AT_CMD => {
                self.push(AT_ESC);
                self.push(b | AT_ESC_MASK);
            }
            AT_ESC => {
                self.push(AT_ESC);
                self.push(b);
            }
            b => self.push(b),
        }
    }

    fn pop_escaped(&mut self) -> Option<u8> {
        let b = self.pop()?;
        if b == AT_ESC {
            match self.pop() {
                Some(b) if b == AT_ESC => Some(AT_ESC),
                Some(b) => Some(b & !AT_ESC_MASK),
                _ => None,
            }
        } else {
            Some(b)
        }
    }
}

struct WriteData {
    seq_no: AtomicU16,
    wbuf: UartVec,
}
pub fn test(connect_args: ConnectArgs, no_send: bool) {
    let mut serial = serialport::new(connect_args.port, connect_args.baud)
        .open()
        .expect("Failed to open port");
    let write_data = Arc::new(RwLock::new(WriteData {
        seq_no: AtomicU16::new(0),
        wbuf: Vec::with_capacity(MAX_BUFFER_SIZE),
    }));

    if !no_send {
        let wlock_data = write_data.clone();
        let normal = Normal::new(500.0, 100.0).unwrap();
        let mut wserial = serial
            .try_clone()
            .expect("Failed to clone port for writing");

        thread::spawn(move || {
            let mut seq_no = 0;
            loop {
                sleep(Duration::from_secs(10));
                let mut wdata = wlock_data.write().unwrap();
                if wdata.seq_no.load(Ordering::Relaxed) > 0 {
                    warn!("last send was NG");
                }
                let len = normal.sample(&mut rand::thread_rng()) as usize;
                seq_no += 1;

                wdata.wbuf.clear();
                let b = (seq_no) as u8;
                wdata.wbuf.push_escaped(b);
                let b = (seq_no >> 8) as u8;
                wdata.wbuf.push_escaped(b);
                wdata.wbuf.push_escaped(0xFF); // dummy message type
                for i in 0..len {
                    wdata.wbuf.push_escaped(i as u8);
                }
                let mut csum: u16 = 0;
                for b in &wdata.wbuf {
                    csum += *b as u16;
                    csum &= 0xFF;
                }
                wdata.wbuf.push_escaped(csum as u8);

                debug!(
                    "send SEQ:{} {} bytes CKSUM:{} {:02X?} ... {:02X?}",
                    seq_no,
                    len,
                    csum,
                    &wdata.wbuf[..10],
                    &wdata.wbuf[(wdata.wbuf.len() - 8)..]
                );
                trace!("send txt\n{}", &wdata.wbuf.escape_ascii().to_string());
                // trace!("send bin\n{:02X?}", &wdata.wbuf);
                wdata.seq_no.store(seq_no, Ordering::SeqCst);

                #[cfg(not(feature = "async"))]
                wdata.wbuf.push(AT_CMD);

                wserial.write_all(&wdata.wbuf).ok();
                wserial.flush().ok();

                #[cfg(feature = "async")]
                {
                    sleep(Duration::from_millis(10));
                    wserial.write_all(&[AT_CMD]).ok();
                    wserial.flush().ok();
                    sleep(Duration::from_millis(200));
                    let mut repeat_at_cmd = 1;
                    while wserial.bytes_to_read().unwrap() == 0
                        && repeat_at_cmd < wdata.wbuf.len() / 100 + 3
                    {
                        repeat_at_cmd += 1;
                        sleep(Duration::from_millis(200));

                        wserial.write_all(&[AT_CMD]).ok();
                        wserial.flush().ok();
                    }
                    debug!("sent at_cmd {repeat_at_cmd} bytes");
                }
            }
        });
    }

    let mut start = 0;
    let mut offset = 0;
    let mut rbuf = vec![0; MAX_BUFFER_SIZE];
    loop {
        if let Ok(n) = serial.read(&mut rbuf[start..]) {
            trace!("received {n} bytes");
            trace!(
                "received txt\n{}",
                &rbuf[start..(start + n)].escape_ascii().to_string()
            );
            trace!("received bin\n{:02X?}", &rbuf[start..(start + n)]);
            for i in start..(start + n) {
                if rbuf[i] == AT_CMD && (i == 0 || rbuf[i - 1] != AT_ESC) {
                    if i > offset {
                        let recv_size = i - offset - 1;
                        if recv_size >= 3 {
                            let recv_end = i - 1;
                            let mut csum: u16 = 0;
                            for b in &rbuf[offset..recv_end] {
                                csum += *b as u16;
                                csum &= 0xFF;
                            }
                            if csum == rbuf[recv_end] as u16 {
                                if rbuf[offset] == 0
                                    && rbuf[offset + 1] == 0
                                    && rbuf[offset + 2] == 0x7E
                                {
                                    //debug print
                                    println!(
                                        "{}",
                                        &rbuf[(offset + 3)..recv_end].escape_ascii().to_string()
                                    );
                                } else if !no_send {
                                    let wdata = write_data.read().unwrap();
                                    let mut seq_no: u16 =
                                        pop_escaped(&rbuf[offset..recv_end], &mut offset).unwrap()
                                            as u16;
                                    seq_no += (pop_escaped(&rbuf[offset..recv_end], &mut offset)
                                        .unwrap()
                                        as u16)
                                        << 8;

                                    if wdata
                                        .seq_no
                                        .compare_exchange(
                                            seq_no,
                                            0,
                                            Ordering::Acquire,
                                            Ordering::Relaxed,
                                        )
                                        .is_ok()
                                    {
                                        debug!(
                                            "recv-ack {} bytes {:02X?} ... {:02X?}",
                                            recv_size,
                                            &rbuf[offset..(offset + 2)],
                                            &rbuf[(recv_end - 2)..i]
                                        );
                                        // trace!("recv-ack bin\n{:02X?}", &rbuf[offset..recv_end]);
                                        info!("recv ACK");
                                    } else {
                                        debug!(
                                            "recv-new SEQ:{} {} bytes {:02X?} ... {:02X?}",
                                            seq_no,
                                            recv_size,
                                            &rbuf[offset..(offset + 3)],
                                            &rbuf[(recv_end - 3)..i]
                                        );
                                        // trace!("recv-new bin\n{:02X?}", &rbuf[offset..recv_end]);
                                    }
                                } else if recv_size > 5 {
                                    debug!(
                                        "recv {} bytes {:02X?} ... {:02X?}",
                                        recv_size,
                                        &rbuf[offset..(offset + 5)],
                                        &rbuf[(recv_end - 5)..i]
                                    );
                                    trace!("recv bin\n{:02X?}", &rbuf[offset..recv_end]);
                                }
                            } else {
                                error!(
                                    "Invalid Check Sum: calculated {} != {} received",
                                    csum, rbuf[recv_end]
                                );
                                trace!("recv bin\n{:02X?}", &rbuf[offset..recv_end]);
                            }
                        }
                    }
                    offset = i + 1;
                }
            }
            start += n;
            if offset == start || start >= RESET_BUFFER_SIZE {
                start = 0;
                offset = 0;
            }
        } else {
            sleep(Duration::from_millis(500));
        }
    }
}

pub(crate) fn generate(length: u16) {
    let mut csum: u16 = 0;
    for i in 1..length {
        print!("{}", i % 10);
        csum += 0x30 + (i % 10);
        csum &= 0xff;
    }
    while !(csum as u8).is_ascii_alphanumeric() {
        print!("0");
        csum += 0x30;
        csum &= 0xff;
    }
    println!("{}", (csum as u8) as char);
}
