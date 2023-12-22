#![feature(vec_push_within_capacity)]

use std::error::Error;

use clap::{Args, Parser, Subcommand};
use flexi_logger::{Age, Cleanup, Criterion, DeferredNow, Duplicate, FileSpec, Logger, Naming};
use log::Record;

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
struct Cli {
    /// Turn debugging information on
    #[arg(short, long, action = clap::ArgAction::Count)]
    verbose: u8,

    #[command(subcommand)]
    command: Option<Commands>,
}

mod test_esp;
mod test_serial;

#[derive(Args)]
pub struct ConnectArgs {
    #[arg(short, long)]
    port: String,
    #[arg(short, long, default_value_t = 115_200)]
    baud: u32,
}

#[derive(Subcommand)]
enum Commands {
    /// Generators
    Generate {
        #[arg(short, long, default_value_t = 250)]
        length: usize,
        #[arg(short, long)]
        bin: bool,
        #[arg(short, long)]
        checksum: Option<u8>,
    },
    /// show all serial ports
    Devs {},
    /// Test serial port (read/write)
    Test {
        #[clap(flatten)]
        connect_args: ConnectArgs,
        #[arg(long)]
        no_send: bool,
        #[arg(long)]
        load_send: bool,
        #[arg(long)]
        at_cmd: bool,
        #[arg(long, value_parser, num_args = 0.., value_delimiter = ' ')]
        send: Vec<String>,
        #[arg(long, value_parser, num_args = 0.., value_delimiter = ' ')]
        send_time: Vec<u64>,
        #[arg(long)]
        esp_test: bool,
    },
}

pub fn logging_format(
    w: &mut dyn std::io::Write,
    now: &mut DeferredNow,
    record: &Record,
) -> Result<(), std::io::Error> {
    let level = record.level();
    write!(
        w,
        "{} {} {}",
        now.format("%Y-%m-%d %H:%M:%S%.6f"),
        level,
        &record.args()
    )
}
fn main() -> Result<(), Box<dyn Error>> {
    let cli = Cli::parse();

    let logger_str = match cli.verbose {
        1 => "info",
        2 => "debug",
        3 => "trace",
        _ => "error",
    };
    Logger::try_with_str(logger_str)?
        .format_for_stderr(logging_format)
        .log_to_file(FileSpec::default().directory("log_files"))
        .format_for_files(logging_format)
        .rotate(
            // If the program runs long enough,
            Criterion::Age(Age::Day), // - create a new file every day
            Naming::Timestamps,       // - let the rotated files have a timestamp in their name
            Cleanup::KeepLogFiles(7), // - keep at most 7 log files
        )
        .duplicate_to_stderr(Duplicate::Warn)
        .start()?;

    match cli.command {
        Some(Commands::Devs {}) => {
            let ports = serialport::available_ports().expect("No ports found!");
            for p in ports {
                println!("{}", p.port_name);
            }
        }
        Some(Commands::Test {
            connect_args,
            no_send,
            load_send,
            at_cmd,
            send,
            send_time,
            esp_test,
        }) => test_serial::test(
            connect_args,
            no_send,
            load_send,
            at_cmd,
            send,
            send_time,
            esp_test,
        ),
        Some(Commands::Generate {
            length,
            bin,
            checksum,
        }) => {
            if bin {
                test_serial::generate_bin(length, checksum);
            } else {
                test_serial::generate(length);
            }
        }
        None => {}
    }
    Ok(())
}
