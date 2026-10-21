//! Команда `rumorph`: морфологический анализ текста из файлов и стандартного ввода

use std::process::ExitCode;

fn main() -> ExitCode {
    let args = std::env::args_os().skip(1).collect();
    ExitCode::from(rumorph::cli::run(args, env!("CARGO_PKG_VERSION"), None))
}
