//! Native neighborhood zoning text loop (0x424fb9), expansion tail and lookup
//! (0x42b9e3). Resource selection belongs to the content host.
use std::collections::BTreeMap;

/// 40745f / 42b9e3. Zero is always residential; missing rows return zero.
pub fn is_residential(table: &BTreeMap<i16, i32>, lot: i16) -> bool {
    lot == 0 || table.get(&lot) == Some(&1)
}

/// Native rows are ordered and duplicates are retained. This bounded parser
/// rejects unterminated/truncated/non-ASCII rows and overflowing atoi inputs;
/// it does not invent behavior for the original buffer's undefined contents.
pub fn parse(lines: &[String], expansions: u16) -> Result<Vec<[i32; 2]>, String> {
    let mut rows = vec![];
    let mut magic = false;
    for line in lines {
        if !line.is_ascii() || line.len() >= 99 || line.contains('\0') {
            return Err("zoning row exceeds validated native text profile".into());
        }
        let Some((id, zone)) = line.split_once(',') else {
            continue;
        };
        let zone = zone.to_ascii_lowercase();
        let value = if zone.contains("residential") {
            1
        } else if zone.contains("community") {
            2
        } else {
            1
        };
        let id = atoi(id)?;
        rows.push([id, value]);
        magic |= (90..=92).contains(&id);
    }
    if !magic && expansions & 0x80 != 0 {
        rows.extend((90..=98).map(|id| [id, if id <= 92 { 1 } else { 2 }]));
    }
    Ok(rows)
}
fn atoi(text: &str) -> Result<i32, String> {
    let text = text.trim_start_matches([' ', '\t', '\n', '\r', '\x0b', '\x0c']);
    let (negative, digits) = match text.as_bytes().first() {
        Some(b'-') => (true, &text[1..]),
        Some(b'+') => (false, &text[1..]),
        _ => (false, text),
    };
    let mut value = 0i64;
    for byte in digits.bytes().take_while(u8::is_ascii_digit) {
        value = value
            .checked_mul(10)
            .and_then(|v| v.checked_add(i64::from(byte - b'0')))
            .filter(|v| *v <= i64::from(i32::MAX) + i64::from(negative))
            .ok_or("zoning lot number exceeds native signed range")?;
    }
    Ok(if negative { -value } else { value } as i32)
}
/// Projection for the VM's signed-short query. Native lookup returns the FIRST
/// matching 32-bit row; out-of-range row IDs must not alias short lot numbers.
pub fn query_table(rows: &[[i32; 2]]) -> BTreeMap<i16, i32> {
    let mut table = BTreeMap::new();
    for &[id, value] in rows {
        if let Ok(id) = i16::try_from(id) {
            table.entry(id).or_insert(value);
        }
    }
    table
}
