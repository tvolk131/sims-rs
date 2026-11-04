//! Original language-manager time formatting (5fe51e) and $TimeLocal adapter
//! (44bf1a -> 4a6ab1). This is an in-game hour, not the operating-system clock.

/// The original unsigned printf formats preserve the space before single-digit
/// hours. Twelve-hour conversion subtracts 12 at most once, even for bad inputs.
pub fn format_time(language: u32, hour: i32, minute: u32, uppercase: bool, prefix: bool) -> String {
    if !matches!(language, 1 | 2 | 19) {
        return format!("{:2}:{minute:02}", hour as u32);
    }
    let suffix = match (hour < 12, uppercase) {
        (true, false) => "am",
        (false, false) => "pm",
        (true, true) => "AM",
        (false, true) => "PM",
    };
    let hour = if hour == 0 {
        12
    } else if hour > 12 {
        hour - 12
    } else {
        hour
    };
    if prefix {
        format!("{suffix} {:2}:{minute:02}", hour as u32)
    } else {
        format!("{:2}:{minute:02} {suffix}", hour as u32)
    }
}

pub fn format_times(text: &str, language: u32, locals: Option<&[i16]>) -> Result<String, String> {
    replace(text, Some(language), |index| {
        Ok(locals.and_then(|l| l.get(index)).copied())
    })
}

pub(super) fn replace(
    text: &str,
    language: Option<u32>,
    mut read: impl FnMut(usize) -> Result<Option<i16>, String>,
) -> Result<String, String> {
    let mut result = text.to_owned();
    let mut start = 0;
    while start < result.len() {
        while start < result.len() && !result.is_char_boundary(start) {
            start += 1;
        }
        let Some(at) = result[start..].find("$TimeLocal").map(|i| i + start) else {
            break;
        };
        let (end, [index, _]) = super::parameters(&result, at + "$TimeLocal".len());
        if let Ok(index) = usize::try_from(index) {
            if let Some(hour) = read(index)?.filter(|hour| (0..24).contains(hour)) {
                let language = language.ok_or("Dialog time formatting language unavailable")?;
                result.replace_range(
                    at..end,
                    &format_time(language, i32::from(hour), 0, false, false),
                );
            }
        }
        start = at + 1;
    }
    Ok(result)
}
