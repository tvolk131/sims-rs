use super::*;

pub(super) fn next(viewer: &Viewer) -> Option<i16> {
    let session = viewer.session.as_ref()?;
    if session.host.presentation.transport.as_ref()?.controls.mode != 2 {
        return None;
    }
    let residents = session
        .house
        .household
        .as_ref()?
        .residents
        .iter()
        .filter_map(|resident| resident_object(session, resident))
        .collect::<Vec<_>>();
    next_from(&residents, viewer.resident)
}

fn next_from(residents: &[i16], current: i16) -> Option<i16> {
    let next = residents
        .iter()
        .position(|resident| *resident == current)
        .map_or(0, |index| (index + 1) % residents.len());
    residents
        .get(next)
        .copied()
        .filter(|resident| *resident != current)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn character_shortcut_cycles_in_household_order_and_wraps() {
        assert_eq!(next_from(&[105, 137], 105), Some(137));
        assert_eq!(next_from(&[105, 137], 137), Some(105));
        assert_eq!(next_from(&[196, 195], 196), Some(195));
        assert_eq!(next_from(&[196, 195], 195), Some(196));
    }

    #[test]
    fn character_shortcut_handles_missing_and_single_residents() {
        assert_eq!(next_from(&[], 105), None);
        assert_eq!(next_from(&[105], 105), None);
        assert_eq!(next_from(&[105], 137), Some(105));
        assert_eq!(next_from(&[105, 137], 9), Some(105));
    }
}
