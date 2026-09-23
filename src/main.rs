fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let env_log = std::env::var("SUBLIME_LOG").ok();
    let exit_code = sublime::cli::run(args, env_log);
    std::process::exit(exit_code.as_i32());
}
