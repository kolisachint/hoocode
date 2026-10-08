//! `hoocode` binary entry point. Everything lives in `hoocode-code-cli`.

fn main() {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    std::process::exit(hoocode_code_cli::main(&argv));
}
