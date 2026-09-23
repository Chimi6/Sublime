//! Startup measurement, shared by every pair. Timed in-process with
//! `Instant` so no shell forks are counted.

const STARTUP_SAMPLES: usize = 25;

/// Median milliseconds from spawning `binary convert <input> --to json` to
/// its first byte of stdout. Timed in-process, so no shell forks are counted.
pub fn measure_startup(binary: &str, input: &str) -> Result<(), String> {
    use std::io::Read;
    use std::process::{Command, Stdio};
    use std::time::Instant;
    let mut samples: Vec<f64> = Vec::with_capacity(STARTUP_SAMPLES);
    for _ in 0..STARTUP_SAMPLES {
        let started = Instant::now();
        let mut child = Command::new(binary)
            .args(["-q", "convert", input, "--to", "json"])
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|error| error.to_string())?;
        let mut first_byte = [0u8; 1];
        let mut stdout = child.stdout.take().ok_or("no stdout")?;
        stdout
            .read_exact(&mut first_byte)
            .map_err(|error| error.to_string())?;
        let elapsed_ms = started.elapsed().as_secs_f64() * 1000.0;
        samples.push(elapsed_ms);
        drop(stdout);
        child.wait().map_err(|error| error.to_string())?;
    }
    println!("{:.3}", median(&mut samples));
    Ok(())
}

/// Median milliseconds to spawn `binary` and wait for it to exit. The
/// process-creation floor on this machine, for context next to `startup`.
pub fn measure_spawn_baseline(binary: &str) -> Result<(), String> {
    use std::process::{Command, Stdio};
    use std::time::Instant;
    let mut samples: Vec<f64> = Vec::with_capacity(STARTUP_SAMPLES);
    for _ in 0..STARTUP_SAMPLES {
        let started = Instant::now();
        Command::new(binary)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map_err(|error| error.to_string())?;
        let elapsed_ms = started.elapsed().as_secs_f64() * 1000.0;
        samples.push(elapsed_ms);
    }
    println!("{:.3}", median(&mut samples));
    Ok(())
}

fn median(samples: &mut [f64]) -> f64 {
    samples.sort_by(|left, right| left.total_cmp(right));
    samples[samples.len() / 2]
}
