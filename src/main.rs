use std::{env, process::ExitCode};

const HELP: &str =
    "TDS Factory\n\nA small software factory.\n\nUsage: tds-factory [--help | --version]";

fn main() -> ExitCode {
    let args: Vec<_> = env::args_os().skip(1).collect();
    match args.as_slice() {
        [] => println!("{HELP}"),
        [arg] if arg == "--help" || arg == "-h" => println!("{HELP}"),
        [arg] if arg == "--version" || arg == "-V" => {
            println!("tds-factory {}", env!("CARGO_PKG_VERSION"));
        }
        _ => {
            eprintln!("Unknown arguments. Run tds-factory --help for usage.");
            return ExitCode::FAILURE;
        }
    }
    ExitCode::SUCCESS
}
