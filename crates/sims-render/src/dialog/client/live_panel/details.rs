use super::*;
use std::collections::BTreeMap;

pub const PERSONALITY: [(&str, i16); 5] = [
    ("Neat", 7), ("Outgoing", 6), ("Active", 3), ("Playful", 5), ("Nice", 2),
];
pub const SKILLS: [(&str, i16, usize); 6] = [
    ("Cooking", 10, 1), ("Mechanical", 12, 2), ("Charisma", 11, 3),
    ("Body", 17, 6), ("Logic", 18, 4), ("Creativity", 15, 5),
];

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct Identity {
    pub object: i16,
    pub guid: u32,
    pub cache_index: u32,
    pub neighbor: i16,
}

#[derive(Debug, Default, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum Category {
    #[default]
    Needs,
    Personality,
    Relationships,
    Job,
}
impl Category {
    pub const ALL: [Self; 4] = [Self::Needs, Self::Personality, Self::Relationships, Self::Job];
    pub fn control(self) -> i32 {
        match self {
            Self::Needs => 4500,
            Self::Personality => 4501,
            Self::Relationships => 4504,
            Self::Job => 4503,
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Needs => "Needs",
            Self::Personality => "Personality",
            Self::Relationships => "Relationships",
            Self::Job => "Job",
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum Detail {
    Personality(u8),
    Relationship { neighbor: i16, guid: u32, cache_index: u32 },
    Job,
    Skill(u8),
}
impl Detail {
    pub fn category(self) -> Category {
        match self {
            Self::Personality(_) => Category::Personality,
            Self::Relationship { .. } => Category::Relationships,
            Self::Job | Self::Skill(_) => Category::Job,
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            Self::Personality(_) => "per.ity",
            Self::Relationship { .. } => "rel",
            Self::Job => "job",
            Self::Skill(_) => "skill",
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(untagged)]
pub enum Popup {
    Motive(u8),
    Detail(Detail),
}
impl Popup {
    pub fn name(self) -> &'static str {
        match self {
            Self::Motive(_) => "motives",
            Self::Detail(detail) => detail.name(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Texts {
    pub personality: Vec<String>,
    pub relationships: Vec<String>,
    pub skills: Vec<String>,
    pub details: Vec<String>,
}
impl Texts {
    pub fn validate(&self) -> Result<()> {
        ensure!(self.personality.len() >= 36 && self.relationships.len() >= 3
            && self.skills.len() >= 7 && self.details.len() >= 24,
            "live panel original text tables incomplete");
        Ok(())
    }
    pub fn personality_text(&self, index: usize, raw: i16) -> Option<(&str, &str)> {
        PERSONALITY.get(index)?;
        let points = raw / 100;
        let bucket = if points < 3 { 0 } else if points <= 7 { 1 } else { 2 };
        Some((self.personality.get(2 + index * 7 + bucket)?.as_str(),
            self.personality.get(5 + index * 7 + bucket)?.as_str()))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Relationship {
    pub neighbor: i16,
    pub guid: u32,
    pub cache_index: u32,
    pub name: String,
    pub daily: i32,
    pub lifetime: i32,
    pub flags: [bool; 2],
    #[serde(default)]
    pub highlighted: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Job {
    pub career: i16,
    pub level: i16,
    pub title: String,
    pub description: String,
    pub start: i32,
    pub end: i32,
    pub next_skills: Option<[i32; 6]>,
}
impl Job {
    pub fn hours(&self, template: &str) -> String {
        template.replace("$Start", &self.start.to_string())
            .replace("$End", &self.end.to_string())
            .replace("$CarTime", &((self.start + 23) % 24).to_string())
    }
}

#[derive(Debug, Default, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Data {
    pub personality: Option<[i16; 5]>,
    pub skills: Option<[i16; 6]>,
    pub relationships: Vec<Relationship>,
    pub job: Option<Job>,
}
impl Person {
    pub fn identity(&self) -> Identity {
        Identity { object: self.object, guid: self.guid, cache_index: self.cache_index, neighbor: self.neighbor }
    }
}

fn fields<const COUNT: usize>(data: &BTreeMap<i16, i16>, indices: [i16; COUNT]) -> Option<[i16; COUNT]> {
    let values = indices.map(|index| data.get(&index).copied());
    values.into_iter().collect::<Option<Vec<_>>>()?.try_into().ok()
}
fn relationship_values(row: &[i32]) -> Option<(i32, i32, [bool; 2])> {
    Some((row.get(2).copied().unwrap_or(0).clamp(-100, 100), (*row.first()?).clamp(-100, 100),
        [row.get(1).is_some_and(|value| *value != 0), row.get(3).is_some_and(|value| *value != 0)]))
}

pub(super) fn read(session: &Session, data: &BTreeMap<i16, i16>, neighbor: i16) -> Result<Data> {
    let registry = session.simulation.vm.world.person.as_ref().context("person registry absent")?;
    let selected = registry.neighbors.get(&neighbor).context("selected neighbor absent")?;
    let mut relationships = Vec::new();
    for (&target, row) in &selected.relationships {
        let Some(other) = registry.neighbors.get(&target) else { continue };
        let Some(cache_index) = session.simulation.vm.world.image_cache.entries.iter()
            .position(|entry| entry.guid == other.guid && entry.guid != 0) else { continue };
        let Some((daily, lifetime, flags)) = relationship_values(row) else { continue };
        let name = session.house.household.as_ref()
            .and_then(|household| household.residents.iter().find(|resident| resident.guid == other.guid))
            .map(|resident| resident.name.clone()).unwrap_or_else(|| format!("Neighbor {target}"));
        relationships.push(Relationship {
            neighbor: target, guid: other.guid, cache_index: cache_index as u32, name,
            daily, lifetime, flags, highlighted: false,
        });
    }
    let job = (|| {
        let [career, level, gender] = fields(data, [56, 57, 65])?;
        let catalog = session.program.careers.as_ref()?;
        let ordinal = catalog.ids.iter().position(|id| *id == i32::from(career))?;
        let record = catalog.records.as_ref()?.get(ordinal)?;
        let level_index = usize::try_from(level).ok()?;
        let current = record.levels.get(level_index)?;
        if !(0..24).contains(&current.values[18]) || !(0..24).contains(&current.values[19]) { return None; }
        let text = catalog.text.as_ref()?.get(ordinal)?.levels.get(level_index)?;
        let title = text.titles[usize::from(gender & 1 != 0 && !text.titles[1].is_empty())].clone();
        if title.is_empty() { return None; }
        Some(Job {
            career, level, title, description: text.description.clone(),
            start: current.values[18], end: current.values[19],
            next_skills: record.levels.get(level_index + 1).map(|next| SKILLS.map(|(_, _, index)| next.values[index])),
        })
    })();
    Ok(Data {
        personality: fields(data, PERSONALITY.map(|(_, field)| field)),
        skills: fields(data, SKILLS.map(|(_, field, _)| field)), relationships, job,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn tutorial_panel_personality_native_fields_and_signed_buckets() -> Result<()> {
        ensure!(PERSONALITY.map(|(_, field)| field) == [7, 6, 3, 5, 2]);
        ensure!(SKILLS.map(|(_, field, _)| field) == [10, 12, 11, 17, 18, 15]);
        let data = BTreeMap::from([(7, -100), (6, 300), (3, 799), (5, 800), (2, 1000)]);
        ensure!(fields(&data, PERSONALITY.map(|(_, field)| field)) == Some([-100, 300, 799, 800, 1000]));
        let texts = Texts { personality: (1..=36).map(|index| index.to_string()).collect(),
            relationships: vec!["".into();3], skills: vec!["".into();7], details: vec!["".into();24] };
        for (raw, title, body) in [(-100, "3", "6"), (299, "3", "6"), (300, "4", "7"),
            (799, "4", "7"), (800, "5", "8")] {
            ensure!(texts.personality_text(0, raw) == Some((title, body)));
        }
        ensure!(texts.personality_text(5, 0).is_none());
        Ok(())
    }
    #[test]
    fn tutorial_panel_job_uses_acquired_title_hours_and_promotion_units() -> Result<()> {
        let job = Job { career: 7, level: 2, title: "Acquired title".into(), description: "Body".into(),
            start: 0, end: 8, next_skills: Some([700, 600, 500, 400, 300, 200]) };
        ensure!(job.hours("Hours: $Start-$End.\r\nCarpool arrives: $CarTime.") ==
            "Hours: 0-8.\r\nCarpool arrives: 23.");
        ensure!(SKILLS.map(|(_, _, index)| index) == [1, 2, 3, 6, 4, 5]);
        Ok(())
    }
    #[test]
    fn tutorial_panel_relationship_columns_remain_directed_and_bounded() -> Result<()> {
        let relationships = BTreeMap::from([(48, BTreeMap::from([(47, vec![55, 0, -12, 1])])),
            (47, BTreeMap::from([(48, vec![-80, 1, 70])]))]);
        ensure!(relationship_values(&relationships[&48][&47]) == Some((-12, 55, [false, true])));
        ensure!(relationship_values(&relationships[&47][&48]) == Some((70, -80, [true, false])));
        ensure!(relationship_values(&[150]) == Some((0, 100, [false, false])));
        ensure!(relationship_values(&[-200, 0, 300]) == Some((100, -100, [false, false])));
        ensure!(relationship_values(&[]).is_none());
        Ok(())
    }
}
