//! Ordered profile upgrades inside 409315. Each writeback publishes the whole
//! current bank before the next missing-entry check. The binding's rollback
//! snapshot is taken only after this function returns, retaining these upgrades.
use super::generator::{Bank, Generator, child, fixed, sex};

pub trait Host: Bank {
    fn generate(&mut self, generator: Generator) -> Result<(), String>;
    /// Publish to the runtime resource bank, never to an original game file.
    fn publish(&mut self) -> Result<(), String>;
}

fn empty(host: &mut impl Host, entry: usize) -> Result<bool, String> {
    let value = host
        .read(entry)?
        .ok_or("native clothing migration entry absent")?;
    Ok(fixed(&value, usize::MAX).is_empty())
}

pub fn upgrade(host: &mut impl Host) -> Result<(), String> {
    if empty(host, 2)? || !matches!(sex(host)?, 0 | 1) {
        return Ok(());
    }
    if !child(host)? {
        if empty(host, 31)? {
            host.generate(Generator::Adult)?;
            host.publish()?;
        }
        if empty(host, 35)? {
            host.generate(Generator::HighFashion)?;
            host.publish()?;
        }
    } else {
        if empty(host, 31)? {
            host.set(31, b"ADDED".to_vec())?;
            host.generate(Generator::Child)?;
            host.publish()?;
        }
        if empty(host, 35)? {
            host.set(35, b"ADDED".to_vec())?;
            host.publish()?;
        }
    }
    if empty(host, 34)? {
        host.generate(Generator::Winter)?;
        host.publish()?;
    }
    Ok(())
}
