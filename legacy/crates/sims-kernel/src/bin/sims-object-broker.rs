//! Foreground authority with an explicit endpoint/mode and semantic stdin stop.
//! It installs no service and adopts no existing socket or process.
use std::{
    error::Error,
    io::{self, BufRead, Write},
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

fn run() -> Result<(), Box<dyn Error>> {
    let mut arguments = std::env::args_os().skip(1);
    let mut socket = None;
    let mut mode = None;
    while let Some(argument) = arguments.next() {
        match argument.to_str() {
            Some("--socket") if socket.is_none() => {
                socket = Some(PathBuf::from(arguments.next().ok_or("socket missing")?))
            }
            Some("--socket-mode") if mode.is_none() => {
                let raw = arguments.next().ok_or("socket mode missing")?;
                mode = Some(u32::from_str_radix(
                    raw.to_str().ok_or("invalid socket mode")?,
                    8,
                )?);
            }
            _ => return Err("expected --socket PATH --socket-mode 0600|0666".into()),
        }
    }
    let server = sims_kernel::Server::bind(
        &socket.ok_or("socket required")?,
        mode.ok_or("explicit socket mode required")?,
    )?;
    let stop = Arc::new(AtomicBool::new(false));
    let signal = Arc::clone(&stop);
    let input = std::thread::spawn(move || {
        for line in io::stdin().lock().lines() {
            match line {
                Ok(line) if line == "quit" => break,
                Ok(_) => continue,
                Err(_) => break,
            }
        }
        signal.store(true, Ordering::Release);
    });
    let epoch: String = server.epoch().iter().map(|b| format!("{b:02x}")).collect();
    println!("READY epoch={epoch}");
    io::stdout().flush()?;
    let result = server.run(stop);
    if input.is_finished() {
        input.join().map_err(|_| "stdin controller panicked")?;
    }
    let summary = result?;
    println!(
        "STOPPED objects={} handles={} connections={}",
        summary.objects_remaining, summary.handles_remaining, summary.connections_remaining
    );
    io::stdout().flush()?;
    Ok(())
}

fn main() -> std::process::ExitCode {
    match run() {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            std::process::ExitCode::FAILURE
        }
    }
}
