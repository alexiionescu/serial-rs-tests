#![feature(vec_push_within_capacity)]

use std::error::Error;

use clap::{Args, Parser, Subcommand};
use flexi_logger::Logger;

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
struct Cli {
    /// Turn debugging information on
    #[arg(short, long, action = clap::ArgAction::Count)]
    verbose: u8,

    #[command(subcommand)]
    command: Option<Commands>,
}

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
    ///
    Generate {
        #[arg(short, long, default_value_t = 250)]
        length: u16,
    },
    /// show all serial ports
    Devs {},
    /// Test serial port (read/write)
    Test {
        #[clap(flatten)]
        connect_args: ConnectArgs,
        #[arg(long)]
        no_send: bool,
    },
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
        .adaptive_format_for_stderr(flexi_logger::AdaptiveFormat::Detailed)
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
        }) => test_serial::test(connect_args, no_send),
        Some(Commands::Generate { length }) => test_serial::generate(length),
        None => {}
    }
    Ok(())
}
