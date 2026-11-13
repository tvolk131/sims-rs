//! Original clothing-name patterns (472657/4726c5/47275d), longest matching
//! prefix (4727f1) and ranked texture candidate selection (472a25).
//! Names are original single-byte bytes. Catalog construction/order and profile
//! migration belong to their callers; this does not invent either input.
pub mod catalog_files;
pub mod catalog_index;
pub mod catalog_load;
pub mod catalog_mount;
pub mod catalog_open;
pub mod catalog_paths;
pub mod catalog_providers;
pub mod catalog_record;
pub mod generator;
pub mod migration;
pub mod name_map;
pub mod pajama;
pub mod resource_name;
pub mod texture_names;

#[derive(Debug, Clone, PartialEq, Eq)]
struct Node {
    start: usize,
    length: usize,
    kind: i32,
    next: [Option<usize>; 2],
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Pattern {
    nodes: Vec<Node>,
    text: Vec<u8>,
}

/// Ordered resource reads requested by 40e477. Body type is the three-byte
/// slice at offset 6 of the parsed normal body; tone is a three-byte PersonColor.
/// The caller must resolve those actual resource values, not infer replacements.
pub trait Profile {
    fn sex(&mut self) -> Result<i32, String>;
    fn body_type(&mut self) -> Result<Vec<u8>, String>;
    fn tone(&mut self) -> Result<Vec<u8>, String>;
}

impl Pattern {
    pub fn expand(template: &[u8], profile: &mut impl Profile) -> Result<Self, String> {
        let mut pattern = Self::default();
        let mut index = 0;
        while index < template.len() {
            let end = template[index..]
                .iter()
                .position(|&b| b == b'$')
                .map_or(template.len(), |n| index + n);
            if index < end {
                // Native copies each literal run through a 256-byte temporary.
                pattern.literal(&template[index..end.min(index + 255)]);
                index = end;
            } else {
                index += 1;
                if let Some(token) = template.get(index) {
                    match token.to_ascii_lowercase() {
                        b'g' => pattern.either(if profile.sex()? == 1 { b"f" } else { b"m" }, b"u"),
                        b'b' => pattern.literal(&profile.body_type()?),
                        b'c' => pattern.literal(&profile.tone()?),
                        _ => (), // Unknown/trailing markers add no node.
                    }
                }
                index += 1;
            }
        }
        Ok(pattern)
    }
    fn link(&mut self, alternative: bool) {
        let start = self.nodes.len();
        for node in &mut self.nodes {
            for (i, next) in node.next.iter_mut().enumerate() {
                if next.is_none() {
                    *next = Some(start + usize::from(alternative) * i);
                }
            }
        }
    }
    fn append(&mut self, text: &[u8], kind: i32) {
        // Builders receive C strings; candidate strings carry their own length.
        let end = text.iter().position(|&b| b == 0).unwrap_or(text.len());
        self.nodes.push(Node {
            start: self.text.len(),
            length: end,
            kind,
            next: [None; 2],
        });
        self.text.extend_from_slice(&text[..end]);
    }
    pub fn literal(&mut self, text: &[u8]) {
        self.link(false);
        self.append(text, 0);
    }
    /// Alternatives are outgoing edges of the preceding nodes. At the start
    /// of an empty pattern only the first choice is reachable in native code.
    pub fn either(&mut self, first: &[u8], second: &[u8]) {
        self.link(true);
        self.append(first, 1);
        self.append(second, 1);
    }
    pub fn skip(&mut self, count: i32) {
        self.link(false);
        self.nodes.push(Node {
            start: 0,
            length: 0,
            kind: count.wrapping_add(2),
            next: [None; 2],
        });
    }
    /// A positive matched prefix is enough; the candidate may have a suffix.
    /// Case folding is ASCII-only, not Unicode or a locale-dependent comparison.
    pub fn matched_prefix(&self, candidate: &[u8]) -> usize {
        if self.nodes.is_empty() {
            return 0;
        }
        let mut work = vec![(0, 0)];
        let mut longest = 0;
        while let Some((index, offset)) = work.pop() {
            let node = &self.nodes[index];
            let count = match node.kind {
                0 | 1 => {
                    let text = &self.text[node.start..node.start + node.length];
                    if !candidate
                        .get(offset..offset + node.length)
                        .is_some_and(|value| value.eq_ignore_ascii_case(text))
                    {
                        continue;
                    }
                    node.length
                }
                kind => {
                    let Ok(count) = usize::try_from(kind.wrapping_sub(2)) else {
                        continue;
                    };
                    if count > candidate.len() - offset {
                        continue;
                    }
                    count
                }
            };
            let end = offset + count;
            if let Some(first) = node.next[0] {
                if node.next[1] != node.next[0] {
                    work.push((node.next[1].unwrap(), end));
                }
                work.push((first, end));
            } else {
                longest = longest.max(end);
            }
        }
        longest
    }
    /// 40e5ce stops at the first matching body, preserving catalog order.
    pub fn first<'a>(&self, candidates: impl IntoIterator<Item = &'a [u8]>) -> Option<&'a [u8]> {
        candidates
            .into_iter()
            .find(|name| self.matched_prefix(name) != 0)
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Best {
    pub name: Vec<u8>,
    pub matched: usize,
    pub fraction: f32,
}
impl Best {
    pub fn consider(&mut self, pattern: &Pattern, candidate: &[u8]) {
        let matched = pattern.matched_prefix(candidate);
        if matched == 0 {
            return;
        }
        // Native stores the matched integer as float before x87 division, then
        // compares the extended quotient against the previous rounded float.
        // Do not round the new quotient before comparing: even identical ratios
        // can replace an earlier candidate when its cached float rounded down.
        let fraction = f64::from(matched as f32) / candidate.len() as f64;
        let old = f64::from(self.fraction);
        if ((fraction - old).abs() <= f64::from(0.0001_f32) && self.matched < matched)
            || old < fraction
        {
            self.fraction = fraction as f32;
            self.matched = matched;
            // Original selected-name storage is a 256-byte fixed string.
            self.name = candidate[..candidate.len().min(255)].to_vec();
        }
    }
}

#[cfg(test)]
mod tests;
