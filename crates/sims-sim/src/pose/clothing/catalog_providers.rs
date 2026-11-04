//! 43bdc5: ordered provider directories from the live object catalog. Identity
//! is the resource instance, not its vtable, GUID or filename. Filesystem scan
//! and OBJD registration supply these entries separately.

#[derive(Debug, Clone, Copy)]
pub struct Entry {
    pub resource: u32,
    pub flags: u32,
}

/// Append to the supplied directory vector. The callback returns the native
/// resource's CFString260 value (its counted bytes, before C-string copying).
pub fn collect(
    entries: &[Entry],
    directories: &mut Vec<Vec<u8>>,
    mut path: impl FnMut(u32) -> Result<Vec<u8>, String>,
) -> Result<(), String> {
    if directories.iter().any(|p| p.len() >= 260) {
        return Err("provider seed exceeds CFString260".into());
    }
    for (i, entry) in entries.iter().enumerate() {
        if entry.flags & 12 != 8
            || entries
                .get(i + 1)
                .is_some_and(|next| next.resource == entry.resource)
        {
            continue;
        }
        let path = path(entry.resource)?;
        if path.len() >= 260 {
            return Err("provider reply exceeds CFString260".into());
        }
        let end = path
            .iter()
            .rposition(|b| matches!(b, b'/' | b'\\'))
            .map_or(0, |n| n + 1);
        // 41200b scans the counted input, then 46f818/46f748 copies its
        // directory through C-string storage. Embedded NUL can shorten it.
        let directory = &path[..end];
        let directory = &directory[..directory.iter().position(|&b| b == 0).unwrap_or(end)];
        if !directories
            .iter()
            .any(|p| p.eq_ignore_ascii_case(directory))
        {
            directories.push(directory.to_vec());
        }
    }
    Ok(())
}
