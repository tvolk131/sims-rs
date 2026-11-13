//! Foreground capability exerciser. Every connection requires the expected
//! host UID and the exact admitted broker epoch; no discovery/reconnection.
use std::{
    error::Error,
    io::{self, BufRead, Write},
    path::PathBuf,
};

fn epoch(raw: &str) -> Result<[u8; 16], Box<dyn Error>> {
    if raw.len() != 32 || !raw.is_ascii() {
        return Err("epoch must be32 hex characters".into());
    }
    let mut bytes = [0; 16];
    for (i, byte) in bytes.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&raw[i * 2..i * 2 + 2], 16)?;
    }
    Ok(bytes)
}

fn run() -> Result<(), Box<dyn Error>> {
    let mut arguments = std::env::args_os().skip(1);
    let (mut socket, mut uid, mut expected_epoch, mut name) = (None, None, None, None);
    while let Some(argument) = arguments.next() {
        let value = arguments.next().ok_or("option value missing")?;
        match argument.to_str() {
            Some("--socket") if socket.is_none() => socket = Some(PathBuf::from(value)),
            Some("--expected-uid") if uid.is_none() => {
                uid = Some(value.to_str().ok_or("invalid UID")?.parse::<u32>()?)
            }
            Some("--expected-epoch") if expected_epoch.is_none() => {
                expected_epoch = Some(epoch(value.to_str().ok_or("invalid epoch")?)?)
            }
            Some("--name") if name.is_none() => {
                name = Some(value.into_string().map_err(|_| "name must be ASCII")?)
            }
            _ => {
                return Err(
                    "expected --socket PATH --expected-uid UID --expected-epoch HEX --name NAME"
                        .into(),
                );
            }
        }
    }
    let name = name.ok_or("name required")?;
    if !name.is_ascii() {
        return Err("this exerciser accepts ASCII names".into());
    }
    let client = sims_kernel::Client::connect(
        &socket.ok_or("socket required")?,
        uid.ok_or("expected UID required")?,
    )?;
    if client.epoch() != expected_epoch.ok_or("expected epoch required")? {
        return Err("admitted authority epoch differs".into());
    }
    let operation = client.begin_create(name.as_bytes())?;
    let lease = match operation.resolve()? {
        sims_kernel::CreateOutcome::Created(lease) => lease,
        sims_kernel::CreateOutcome::Rejected(failure) => {
            return Err(format!("authority API rejected create: {}", failure.code()).into());
        }
    };
    let epoch: String = lease.epoch().iter().map(|b| format!("{b:02x}")).collect();
    println!(
        "CREATED epoch={epoch} connection={} handle={} object={} session={}",
        client.connection(),
        lease.handle(),
        lease.object(),
        client
            .peer_session()
            .map_or_else(|| "none".into(), |value| value.to_string())
    );
    io::stdout().flush()?;
    for line in io::stdin().lock().lines() {
        match line?.as_str() {
            "check" => {
                lease.check()?;
                println!("LIVE object={}", lease.object());
            }
            "close" => {
                lease.close()?;
                println!("CLOSED acknowledged={}", lease.is_closed());
            }
            "history" => println!(
                "HISTORY epoch={epoch} handle={} object={} closed={}",
                lease.handle(),
                lease.object(),
                lease.is_closed()
            ),
            "exit-without-close" => {
                // Explicit diagnostic: actual client process termination skips
                // Rust cleanup, so the broker must retire references via EOF.
                io::stdout().flush()?;
                std::process::exit(0);
            }
            "quit" => break,
            _ => return Err("expected check, close, history, quit or exit-without-close".into()),
        }
        io::stdout().flush()?;
    }
    lease.close()?;
    println!("FINISHED acknowledged={}", lease.is_closed());
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
