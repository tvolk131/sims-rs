//! 45f034's migration, selection and close(0) over definition-owned profiles.
//! Publications update runtime STR# overlays; original Program/files stay intact.
use crate::clothing_catalog::{Names, Provider};
use anyhow::{Context, Result};
use sims_sim::pose::clothing::{
    generator::{self, Bank},
    migration, pajama,
};
use sims_vm::{Definition, Program, body_profiles::State};

struct Tracked<'a, P> {
    provider: &'a mut P,
    used: bool,
}
impl<P: crate::clothing_catalog::Provider> crate::clothing_catalog::Provider for Tracked<'_, P> {
    fn names(&mut self) -> Result<&Names> {
        self.used = true;
        self.provider.names()
    }
}

/// Join 58a178's fallback branch with the definition-owned migration/selector.
/// Caller owns the enclosing transaction if later asset binding can fail.
pub fn select_appearance(
    program: &Program,
    profiles: &mut State,
    definition: &Definition,
    selection: &mut sims_sim::pose::appearance_selection::State,
    person: &mut std::collections::BTreeMap<i16, i16>,
    flags: u16,
    force: bool,
    names: &Names,
) -> Result<sims_sim::pose::appearance_selection::Selection> {
    select_appearance_with(
        program,
        profiles,
        definition,
        selection,
        person,
        flags,
        force,
        &mut &*names,
    )
}

pub fn select_appearance_with(
    program: &Program,
    profiles: &mut State,
    definition: &Definition,
    selection: &mut sims_sim::pose::appearance_selection::State,
    person: &mut std::collections::BTreeMap<i16, i16>,
    flags: u16,
    force: bool,
    provider: &mut impl crate::clothing_catalog::Provider,
) -> Result<sims_sim::pose::appearance_selection::Selection> {
    // This API has no person-owned career cache. Keep that boundary explicit;
    // live callers use select_appearance_with_career rather than discarding it.
    use sims_sim::pose::appearance_selection::{Descriptor, Selection};
    let strings = profiles
        .get(definition)
        .map_err(anyhow::Error::msg)?
        .to_vec();
    Selection::begin_with_pajamas(selection, person, &strings, flags, force, |age| {
        let names = provider.names().map_err(|e| e.to_string())?;
        let selected =
            select_pajamas(program, profiles, definition, age, names).map_err(|e| e.to_string())?;
        Ok(Descriptor {
            name: sims_formats::strings::decode(&selected.body, 1),
            texture: sims_formats::strings::decode(&selected.texture, 1),
        })
    })
    .map_err(anyhow::Error::msg)
}

pub fn select_appearance_with_career(
    program: &Program,
    profiles: &mut State,
    definition: &Definition,
    selection: &mut sims_sim::pose::appearance_selection::State,
    person: &mut std::collections::BTreeMap<i16, i16>,
    flags: u16,
    force: bool,
    career: &mut Option<sims_vm::career_appearance::State>,
    provider: &mut impl crate::clothing_catalog::Provider,
) -> Result<sims_sim::pose::appearance_selection::Selection> {
    use sims_sim::pose::appearance_selection::{Descriptor, Helper, Selection};
    let strings = profiles
        .get(definition)
        .map_err(anyhow::Error::msg)?
        .to_vec();
    let fields = [56, 57, 58, 65].map(|n| person.get(&n).copied());
    let mut next_career = career.clone();
    let mut provider = Tracked {
        provider,
        used: false,
    };
    let mut result =
        Selection::begin_with_helpers(selection, person, &strings, flags, force, |request| {
            match request {
                Helper::Pajamas(age) => {
                    let names = provider.names().map_err(|e| e.to_string())?;
                    let selected = select_pajamas(program, profiles, definition, age, names)
                        .map_err(|e| e.to_string())?;
                    Ok([
                        Descriptor {
                            name: sims_formats::strings::decode(&selected.body, 1),
                            texture: sims_formats::strings::decode(&selected.texture, 1),
                        },
                        Descriptor::default(),
                    ])
                }
                Helper::Career => {
                    let fields = fields
                        .into_iter()
                        .enumerate()
                        .map(|(n, value)| {
                            value.ok_or_else(|| {
                                format!(
                                    "career appearance person field {} absent",
                                    [56, 57, 58, 65][n]
                                )
                            })
                        })
                        .collect::<Result<Vec<_>, _>>()?;
                    select_career(
                        program,
                        profiles,
                        definition,
                        fields.try_into().unwrap(),
                        &mut next_career,
                        &mut provider,
                    )
                    .map_err(|e| e.to_string())
                }
            }
        })
        .map_err(anyhow::Error::msg)?;
    result.clothing_used = provider.used;
    *career = next_career;
    Ok(result)
}

/// Actual45ee5f: cache identity uses the unadjusted job level. School-age
/// people use level0 for lookup only; gender chooses a nonempty female template.
/// Missing careers/levels clear the cache, then still install the requested key.
pub fn select_career(
    program: &Program,
    profiles: &mut State,
    definition: &Definition,
    [career_id, job_level, age, species]: [i16; 4],
    cache: &mut Option<sims_vm::career_appearance::State>,
    provider: &mut impl crate::clothing_catalog::Provider,
) -> Result<[sims_sim::pose::appearance_selection::Descriptor; 2]> {
    use sims_sim::pose::appearance_selection::Descriptor;
    let mut next = cache.clone().unwrap_or_default();
    let key = [career_id, job_level];
    if next.key != key {
        next.body.clear();
        next.texture.clear();
        next.extra.clear();
        let catalog = program
            .careers
            .as_ref()
            .context("career appearance catalog absent")?;
        if let Some(index) = catalog
            .ids
            .iter()
            .position(|&id| id == i32::from(career_id))
        {
            let records = catalog
                .records
                .as_ref()
                .context("career appearance records absent")?;
            let record = records
                .get(index)
                .context("career appearance vector incomplete")?;
            let level = if age != 0 && age < 18 { 0 } else { job_level };
            if let Ok(level) = usize::try_from(level)
                && let Some(level) = record.levels.get(level)
            {
                let encode = |value: &str| -> Result<Vec<u8>> {
                    let (bytes, _, failed) = encoding_rs::WINDOWS_1252.encode(value);
                    anyhow::ensure!(!failed, "career clothing template is not Windows-1252");
                    Ok(generator::fixed(&bytes, 63).to_vec())
                };
                let body = if species & 1 != 0 && !level.strings[2].is_empty() {
                    &level.strings[2]
                } else {
                    &level.strings[1]
                };
                let body = encode(body)?;
                let texture = encode(&level.strings[3])?;
                next.extra = encode(&level.strings[4])?;
                let names = provider.names()?;
                profiles
                    .load(program, definition)
                    .map_err(anyhow::Error::msg)?
                    .context("career selection requires a person definition")?;
                let mut host = Migration {
                    profile: Profile {
                        state: profiles,
                        definition,
                    },
                    names,
                };
                migration::upgrade(&mut host).map_err(anyhow::Error::msg)?;
                // Selection reads only; close(0) restores the post-migration
                // backup. No generated modern entry is substituted here.
                let selected = pajama::select_templates(
                    &body,
                    &texture,
                    &mut host.profile,
                    &names.bodies,
                    &names.textures.names,
                )
                .map_err(anyhow::Error::msg)?;
                next.body = selected.body;
                next.texture = selected.texture;
            }
        }
        next.key = key;
    }
    let result = [
        Descriptor {
            name: sims_formats::strings::decode(&next.body, 1),
            texture: sims_formats::strings::decode(&next.texture, 1),
        },
        Descriptor {
            name: sims_formats::strings::decode(&next.extra, 1),
            texture: String::new(),
        },
    ];
    *cache = Some(next);
    Ok(result)
}

struct Profile<'a> {
    state: &'a mut State,
    definition: &'a Definition,
}
impl Bank for Profile<'_> {
    fn read(&mut self, entry: usize) -> Result<Option<Vec<u8>>, String> {
        self.state
            .get(self.definition)?
            .get(entry.wrapping_sub(1))
            .map(|value| {
                let (bytes, _, failed) = encoding_rs::WINDOWS_1252.encode(value);
                if failed {
                    Err("clothing profile value is not Windows-1252".into())
                } else {
                    Ok(bytes.into_owned())
                }
            })
            .transpose()
    }
    fn set(&mut self, entry: usize, value: Vec<u8>) -> Result<(), String> {
        let slot = self
            .state
            .get_mut(self.definition)?
            .get_mut(entry.wrapping_sub(1))
            .ok_or("clothing profile write index absent")?;
        *slot = sims_formats::strings::decode(&value, 1);
        Ok(())
    }
}
struct Migration<'a, 'b> {
    profile: Profile<'a>,
    names: &'b Names,
}
impl Bank for Migration<'_, '_> {
    fn read(&mut self, entry: usize) -> Result<Option<Vec<u8>>, String> {
        self.profile.read(entry)
    }
    fn set(&mut self, entry: usize, value: Vec<u8>) -> Result<(), String> {
        self.profile.set(entry, value)
    }
}
impl migration::Host for Migration<'_, '_> {
    fn generate(&mut self, generator: generator::Generator) -> Result<(), String> {
        generator::generate(
            generator,
            &mut self.profile,
            &self.names.bodies,
            &self.names.textures.names,
        )
    }
    fn publish(&mut self) -> Result<(), String> {
        self.profile.state.publish(self.profile.definition)
    }
}

/// Supply the live person's age and the process clothing-name caches. Native
/// migration uses the profile age separately. Newly generated entry 33 is not
/// reselected: this invocation still finishes the original pajama fallback.
pub fn select_pajamas(
    program: &Program,
    state: &mut State,
    definition: &Definition,
    person_age: i16,
    names: &Names,
) -> Result<pajama::Selected> {
    state
        .load(program, definition)
        .map_err(anyhow::Error::msg)?
        .context("clothing selection requires a person definition")?;
    let mut host = Migration {
        profile: Profile { state, definition },
        names,
    };
    migration::upgrade(&mut host).map_err(anyhow::Error::msg)?;
    // 409315 backs up after all migration publications, so close(0) retains
    // upgrades. This scope never shares a second binding owner or suspends.
    let backup = host
        .profile
        .state
        .get(definition)
        .map_err(anyhow::Error::msg)?
        .to_vec();
    let selected = pajama::select(
        person_age,
        &mut host.profile,
        &names.bodies,
        &names.textures.names,
    )
    .map_err(anyhow::Error::msg)?;
    host.profile
        .state
        .get_mut(definition)
        .map_err(anyhow::Error::msg)?
        .clone_from_slice(&backup);
    Ok(selected)
}
