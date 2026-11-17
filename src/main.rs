fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let env_log = std::env::var("SUBLIME_LOG").ok();
    // The same stack on every platform: the readers' nesting limit is
    // sized for it.
    let worker = std::thread::Builder::new()
        .stack_size(sublime::limits::STACK)
        .spawn(move || sublime::cli::run(args, env_log))
        .expect("the conversion thread starts");
    let exit_code = worker
        .join()
        .unwrap_or_else(|panic| std::panic::resume_unwind(panic));
    std::process::exit(exit_code.as_i32());
}
