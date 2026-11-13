//! 44c65f..44c6ea: the $Neighbor pass of the original text formatter.

/// Resource lookup is in the neighborhood namespace. Optional outputs are
/// independent: the resource is published before catalog-name lookup, and the
/// neighbor word only after text replacement. Missing resources leave the text
/// and both outputs untouched for that occurrence. The next search starts one
/// position after the match, not after the replacement.
pub fn format_neighbor<R: Clone>(
    text: &str,
    neighbor: i16,
    mut neighbor_output: Option<&mut i16>,
    mut resource_output: Option<&mut R>,
    mut lookup: impl FnMut(i16) -> Result<Option<R>, String>,
    mut name: impl FnMut(&R) -> Result<String, String>,
) -> Result<String, String> {
    const TOKEN: &str = "$Neighbor";
    let mut result = text.to_owned();
    let mut at = 0;
    let mut iterations = 0;
    while at < result.len() {
        while at < result.len() && !result.is_char_boundary(at) {
            at += 1;
        }
        let Some(found) = result[at..].find(TOKEN) else {
            break;
        };
        at += found;
        if let Some(resource) = lookup(neighbor)? {
            if let Some(output) = resource_output.as_deref_mut() {
                *output = resource.clone();
            }
            result.replace_range(at..at + TOKEN.len(), &name(&resource)?);
            if let Some(output) = neighbor_output.as_deref_mut() {
                *output = neighbor;
            }
        }
        at += 1;
        iterations += 1;
        if iterations > 65536 || result.len() > 65536 {
            return Err("Dialog neighbor expansion exceeds limit".into());
        }
    }
    Ok(result)
}
