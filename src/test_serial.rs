#![allow(unused_imports)]

use std::{
    sync::{
        atomic::{AtomicU16, Ordering},
        Arc, Condvar, Mutex, RwLock,
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

const MSG_TYPE_RES: u8 = 0x20;
// request 0x00 - 0x1F
// responses 0x20 - 0x3f = (0x00 - 0x1F | MSG_TYPE_RES)
const MSG_TYPE_REQ_CONFIG: u8 = 0x00;
const MSG_TYPE_RES_CONFIG: u8 = MSG_TYPE_REQ_CONFIG | MSG_TYPE_RES;

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
pub fn test(
    connect_args: ConnectArgs,
    no_send: bool,
    load_send: bool,
    at_cmd: bool,
    fix_send: Option<String>,
    send_time: u16,
) {
    let mut serial = serialport::new(connect_args.port, connect_args.baud)
        .open()
        .expect("Failed to open port");
    let write_data = Arc::new(RwLock::new(WriteData {
        seq_no: AtomicU16::new(0),
        wbuf: Vec::with_capacity(MAX_BUFFER_SIZE),
    }));
    let answer_data = Arc::new(Mutex::new(UartVec::with_capacity(MAX_BUFFER_SIZE)));
    let pair = Arc::new((Mutex::new(false), Condvar::new()));
    let pair2 = Arc::clone(&pair);

    if !no_send {
        let wlock_data = write_data.clone();
        let alock_data = answer_data.clone();

        let normal = Normal::new(
            if load_send { 70.0 } else { 500.0 },
            if load_send { 40.0 } else { 100.0 },
        )
        .unwrap();
        let mut wserial = serial
            .try_clone()
            .expect("Failed to clone port for writing");
        let hex_fix_send = fix_send.map(|s| hex::decode(s).unwrap());

        thread::spawn(move || {
            let (lock, cvar) = &*pair;

            let mut seq_no = 0;
            let mut total_sent: usize = 0;
            let mut total_sent_bytes: usize = 0;
            loop {
                if !load_send {
                    let started = lock.lock().unwrap();
                    cvar.wait_timeout(started, Duration::from_secs(send_time as u64))
                        .ok();
                }
                let mut wdata = wlock_data.write().unwrap();
                if !load_send && wdata.seq_no.load(Ordering::Relaxed) > 0 {
                    warn!("last send was NG");
                }

                seq_no += 1;

                wdata.wbuf.clear();
                let b = (seq_no) as u8;
                wdata.wbuf.push_escaped(b);
                let b = (seq_no >> 8) as u8;
                wdata.wbuf.push_escaped(b);
                let mut adata = alock_data.lock().unwrap();
                if !adata.is_empty() {
                    for &b in adata.iter() {
                        wdata.wbuf.push(b);
                    }
                    adata.clear();
                } else if let Some(ref hex) = hex_fix_send {
                    hex.iter().for_each(|b| wdata.wbuf.push_escaped(*b));
                } else {
                    wdata.wbuf.push_escaped(0xFF); // dummy message type
                    let len = normal.sample(&mut rand::thread_rng()) as usize;
                    for i in 0..len {
                        wdata.wbuf.push_escaped(i as u8);
                    }
                }
                let mut csum: u16 = 0;
                for b in &wdata.wbuf {
                    csum += *b as u16;
                    csum &= 0xFF;
                }
                wdata.wbuf.push_escaped(csum as u8);
                wdata.seq_no.store(seq_no, Ordering::SeqCst);

                if !load_send {
                    if wdata.wbuf.len() < 20 {
                        debug!(
                            "send SEQ:{} {} bytes CKSUM:{} {:02X?}",
                            seq_no,
                            wdata.wbuf.len(),
                            csum,
                            &wdata.wbuf,
                        );
                    } else {
                        debug!(
                            "send SEQ:{} {} bytes CKSUM:{} {:02X?} ... {:02X?}",
                            seq_no,
                            wdata.wbuf.len(),
                            csum,
                            &wdata.wbuf[..10],
                            &wdata.wbuf[(wdata.wbuf.len() - 8)..]
                        );
                    }
                    trace!("send txt\n{}", &wdata.wbuf.escape_ascii().to_string());
                    trace!("send bin\n{}", hex::encode_upper(&wdata.wbuf));
                }
                wdata.wbuf.push(AT_CMD);
                total_sent_bytes += wdata.wbuf.len();

                wserial.write_all(&wdata.wbuf).ok();
                wserial.flush().ok();

                #[cfg(feature = "async")]
                if at_cmd {
                    sleep(Duration::from_millis(50));
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
                    if !load_send {
                        debug!("sent at_cmd {repeat_at_cmd} bytes");
                    }
                }

                total_sent += 1;
                if seq_no % 1024 == 0 {
                    info!("sent {:05} bytes: {:07}", total_sent, total_sent_bytes);
                }
            }
        });
    }

    let mut start = 0;
    let mut offset = 0;
    let mut rbuf = vec![0; MAX_BUFFER_SIZE];
    let (lock, cvar) = &*pair2;
    loop {
        if let Ok(n) = serial.read(&mut rbuf[start..]) {
            trace!("received {n} bytes");
            trace!(
                "received txt\n{}",
                &rbuf[start..(start + n)].escape_ascii().to_string()
            );
            trace!(
                "received bin\n{}",
                hex::encode_upper(&rbuf[start..(start + n)])
            );
            for i in start..(start + n) {
                if rbuf[i] == AT_CMD && (i == 0 || rbuf[i - 1] != AT_ESC) {
                    if i > offset {
                        let recv_size = i - offset - 1;
                        if recv_size >= 3 {
                            let mut recv_end = i - 1;
                            let mut recv_csum = rbuf[recv_end] as u32;
                            let mut csum: u32 = 0;
                            for b in &rbuf[offset..recv_end] {
                                csum += *b as u32;
                            }
                            if rbuf[recv_end - 1] == AT_ESC {
                                //un-escape checksum
                                recv_end -= 1;
                                csum -= AT_ESC as u32;
                                recv_csum &= !AT_ESC_MASK as u32;
                            }
                            if csum & 0xFF == recv_csum {
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
                                    let msg_type =
                                        pop_escaped(&rbuf[offset..recv_end], &mut offset).unwrap();

                                    if msg_type & 0x80 != 0
                                        && wdata
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
                                        info!("recv ACK for {seq_no}");
                                    } else {
                                        if i - offset < 20 {
                                            debug!(
                                                "recv-new SEQ:{} {} bytes {:02X?}",
                                                seq_no,
                                                recv_size,
                                                &rbuf[offset..i]
                                            );
                                        } else {
                                            debug!(
                                                "recv-new SEQ:{} {} bytes {:02X?} ... {:02X?}",
                                                seq_no,
                                                recv_size,
                                                &rbuf[offset..(offset + 3)],
                                                &rbuf[(recv_end - 3)..i]
                                            );
                                        }
                                        // trace!("recv-new bin\n{:02X?}", &rbuf[offset..recv_end]);
                                        if msg_type == MSG_TYPE_REQ_CONFIG {
                                            info!("recv Req Config");
                                            let mut adata = answer_data.lock().unwrap();
                                            if adata.is_empty() {
                                                {
                                                    adata.push_escaped(MSG_TYPE_RES_CONFIG);
                                                    adata.push_escaped(0xFF);
                                                    for _ in 0..12 {
                                                        adata.push_escaped(0x00);
                                                    }
                                                }
                                                let mut started = lock.lock().unwrap();
                                                *started = true;
                                                // We notify the condvar that the value has changed.
                                                info!("notify Req Config");
                                                cvar.notify_one();
                                            } else {
                                                warn!("Cannot send res Config because data queue not empty!");
                                            }
                                        }
                                    }
                                } else if recv_size > 5 {
                                    debug!(
                                        "recv {} bytes {:02X?} ... {:02X?}",
                                        recv_size,
                                        &rbuf[offset..(offset + 5)],
                                        &rbuf[(recv_end - 5)..i]
                                    );
                                    // trace!("recv bin\n{:02X?}", &rbuf[offset..recv_end]);
                                }
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

pub(crate) fn generate(length: usize) {
    let mut csum: u32 = 0;
    for i in 1..length {
        print!("{}", i % 10);
        csum += 0x30 + (i % 10) as u32;
        csum &= 0xff;
    }
    while !(csum as u8).is_ascii_alphanumeric() {
        print!("0");
        csum += 0x30;
        csum &= 0xff;
    }
    println!("{}", (csum as u8) as char);
}

pub(crate) fn generate_bin(length: usize, checksum: Option<u8>) {
    let mut wbuf = UartVec::with_capacity(length * 2);
    let seq_no = 1;
    let b = (seq_no >> 8) as u8;
    wbuf.push_escaped(b);
    let b = (seq_no) as u8;
    wbuf.push_escaped(b);
    wbuf.push_escaped(0xFF); // dummy message type
    for i in 0..length {
        wbuf.push_escaped(i as u8);
    }
    let mut csum: u32 = 0;
    for b in &wbuf {
        csum += *b as u32;
        csum &= 0xFF;
        debug!("{:02X} -> {:02X}", *b, csum);
    }
    if let Some(cs) = checksum {
        while csum != cs as u32 {
            let b = 1;
            wbuf.push_escaped(b);
            csum += b as u32;
            csum &= 0xFF;
            debug!("{:02X} -> {:02X}", b, csum);
        }
    }
    wbuf.push_escaped(csum as u8);
    wbuf.push(AT_CMD);
    println!("{}", hex::encode_upper(wbuf));
}
