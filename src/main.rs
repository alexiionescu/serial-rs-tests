#![feature(vec_push_within_capacity)]

use std::error::Error;

use clap::{Parser, Subcommand};
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

#[derive(Subcommand)]
enum Commands {
    /// show all serial ports
    Devs {},
    /// Test serial port (read/write)
    Test {
        #[arg(short, long)]
        port: String,
        #[arg(short, long, default_value_t = 115_200)]
        baud: u32,
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

    match &cli.command {
        Some(Commands::Devs {}) => {
            let ports = serialport::available_ports().expect("No ports found!");
            for p in ports {
                println!("{}", p.port_name);
            }
        }
        Some(Commands::Test { port, baud }) => test_serial::test(port, *baud),
        None => {}
    }
    Ok(())
}
