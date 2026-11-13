use super::*;
use iced::widget::column;
use sims_render::dialog::client::live_panel::{self, Category, Detail, Identity, Panel, Popup, Texts};

pub(super) fn load_texts(index: &Index, language: u32) -> Result<Texts> {
    let source = &index.resources[index.find("Live.iff")?];
    let iff = sims_formats::iff::Iff::parse(source.read()?)?;
    let language = u8::try_from(language)?;
    let read = |id, count| -> Result<Vec<String>> {
        let chunk = iff.get(b"STR#", id).context("live panel string resource absent")?;
        let table = sims_formats::strings::StringTable::parse(iff.data(chunk))?;
        (0..count).map(|entry| table.get(entry, language).map(str::to_owned)
            .context("live panel string entry absent")).collect()
    };
    let texts = Texts { personality: read(131, 36)?, relationships: read(132, 3)?,
        skills: read(136, 7)?, details: read(137, 24)? };
    texts.validate()?;
    Ok(texts)
}

pub(super) fn category_message(identity: Identity, category: Category) -> Message {
    match category {
        Category::Needs => Message::Needs,
        Category::Personality => Message::Personality(identity),
        Category::Relationships => Message::Relationships(identity),
        Category::Job => Message::Job(identity),
    }
}

pub(super) fn control_resource(message: &Message) -> Option<i32> {
    match message {
        Message::Needs => Some(4500),
        Message::Personality(_) => Some(4501),
        Message::Relationships(_) => Some(4504),
        Message::Job(_) => Some(4503),
        _ => None,
    }
}

pub(super) fn hud_position(resource: i32) -> Option<[i32; 2]> {
    match resource {
        4500 => Some([4, 33]),
        4501 => Some([44, 34]),
        4503 => Some([76, 34]),
        4504 => Some([76, 2]),
        _ => None,
    }
}

pub(super) fn request(message: &Message) -> Option<(Identity, Option<Category>)> {
    match message {
        Message::Personality(identity) => Some((*identity, Some(Category::Personality))),
        Message::Relationships(identity) => Some((*identity, Some(Category::Relationships))),
        Message::Job(identity) => Some((*identity, Some(Category::Job))),
        Message::PersonDetails(identity, _) | Message::ClosePersonDetails(identity, _) => Some((*identity, None)),
        _ => None,
    }
}

pub(super) fn available(panel: &Panel, selected: Option<Identity>, message: &Message,
    blocked: bool, visible: bool) -> bool {
    let Some((identity, category)) = request(message) else { return true };
    if blocked || selected != Some(identity) || panel.person().map(|person| person.identity()) != Some(identity) {
        return false;
    }
    match message {
        Message::ClosePersonDetails(_, expected) => panel.retained_popup() == Some(*expected),
        Message::PersonDetails(_, detail) => visible && panel.detail_available(*detail),
        _ => visible && category.is_some(),
    }
}

pub(super) fn sidebar<'panel>(panel: &'panel Panel, ready: bool,
    highlight: Option<i32>) -> Element<'panel, Message> {
    let mut controls = column![].spacing(4);
    if let Some(person) = panel.person() {
        for pair in Category::ALL.chunks(2) {
            let mut buttons = row![].spacing(4);
            for category in pair {
                let label = if highlight == Some(category.control()) {
                    format!("▶ {}", category.label())
                } else { category.label().to_string() };
                buttons = buttons.push(button(text(label).size(12)).width(Fill)
                    .on_press_maybe(ready.then_some(category_message(person.identity(), *category)))
                    .style(if panel.category_selected(*category) { button::primary } else { button::secondary }));
            }
            controls = controls.push(buttons);
        }
    }
    column![controls, content(panel, ready)].spacing(6).into()
}

pub(super) fn content(panel: &Panel, ready: bool) -> Element<'_, Message> {
    let Some(person) = panel.person().filter(|_| panel.shown()) else { return widget::Space::new().into() };
    let identity = person.identity();
    let mut rows = column![].spacing(3);
    let entry = |label: String, detail| button(text(label).size(12))
        .on_press_maybe((ready && panel.detail_available(detail)).then_some(Message::PersonDetails(identity, detail)))
        .style(button::secondary).width(Fill);
    match panel.category() {
        Category::Needs => {
            for pair in [[0, 1], [2, 5], [3, 6], [4, 7]] {
                rows = rows.push(row(pair.into_iter().map(|index| {
                    let value = f32::from_bits(person.values[index]);
                    button(column![text(live_panel::MOTIVES[index].0).size(12),
                        progress_bar(0.0..=1.0, ((value + 100.) / 200.).clamp(0., 1.)).girth(7)])
                        .on_press_maybe(ready.then_some(Message::MotiveDetails(index)))
                        .style(button::secondary).width(Fill).into()
                })).spacing(6));
            }
        }
        Category::Personality => {
            if let Some(values) = person.details.personality {
                for (index, raw) in values.into_iter().enumerate() {
                    let label = panel.texts().and_then(|texts| texts.personality.get(1 + index * 7))
                        .map(String::as_str).unwrap_or(live_panel::PERSONALITY[index].0);
                    rows = rows.push(entry(format!("{label}: {} / 10", raw / 100), Detail::Personality(index as u8)));
                }
            } else { rows = rows.push(text("Personality data unavailable").size(12)); }
        }
        Category::Relationships => {
            for relationship in &person.details.relationships {
                rows = rows.push(entry(format!("{}{} · daily {} / lifetime {}",
                    if relationship.highlighted { "▶ " } else { "" }, relationship.name,
                    relationship.daily, relationship.lifetime), Detail::Relationship {
                        neighbor: relationship.neighbor, guid: relationship.guid, cache_index: relationship.cache_index })
                    .style(if relationship.highlighted { button::primary } else { button::secondary }));
            }
            if person.details.relationships.is_empty() { rows = rows.push(text("No recorded relationships").size(12)); }
        }
        Category::Job => {
            if let Some(job) = &person.details.job {
                rows = rows.push(entry(job.title.clone(), Detail::Job));
            } else { rows = rows.push(text("Career details unavailable").size(12)); }
            if let Some(skills) = person.details.skills {
                for (index, raw) in skills.into_iter().enumerate() {
                    let label = panel.texts().and_then(|texts| texts.skills.get(index + 1))
                        .map(String::as_str).unwrap_or(live_panel::SKILLS[index].0);
                    let needed = person.details.job.as_ref().and_then(|job| job.next_skills)
                        .is_some_and(|requirements| i32::from(raw) < requirements[index]);
                    rows = rows.push(entry(format!("{label}: {} / 10{}", (raw / 100).clamp(0, 10),
                        if needed { " · promotion requirement" } else { "" }), Detail::Skill(index as u8)));
                }
            } else { rows = rows.push(text("Skill data unavailable").size(12)); }
        }
    }
    scrollable(rows).into()
}

pub(super) fn popup(panel: &Panel) -> Option<Element<'_, Message>> {
    let person = panel.person()?;
    let popup = panel.retained_popup()?;
    let mut rows = column![text(&person.name).size(18)].spacing(8);
    match popup {
        Popup::Motive(index) => {
            let index = usize::from(index);
            rows = rows.push(text(live_panel::MOTIVES.get(index)?.0))
                .push(text(format!("{:.0} / 100", f32::from_bits(person.values[index]))));
        }
        Popup::Detail(detail) => {
            let texts = panel.texts()?;
            match detail {
                Detail::Personality(index) => {
                    let (title, body) = texts.personality_text(usize::from(index), *person.details.personality?.get(usize::from(index))?)?;
                    rows = rows.push(text(title).size(16)).push(text(body).size(13));
                }
                Detail::Relationship { neighbor, guid, cache_index } => {
                    let relationship = person.details.relationships.iter()
                        .find(|row| row.neighbor == neighbor && row.guid == guid && row.cache_index == cache_index)?;
                    rows = rows.push(text(&relationship.name).size(16))
                        .push(text(&texts.relationships[1]))
                        .push(text(format!("Daily: {}\nLifetime: {}", relationship.daily, relationship.lifetime)))
                        .push(text(&texts.relationships[2]).size(13));
                }
                Detail::Job => {
                    let job = person.details.job.as_ref()?;
                    rows = rows.push(text(&job.title).size(16)).push(text(&job.description).size(13))
                        .push(text(job.hours(&texts.details[23])).size(13));
                }
                Detail::Skill(index) => {
                    let index = usize::from(index);
                    let raw = *person.details.skills?.get(index)?;
                    rows = rows.push(text(texts.details.get(index * 2)?).size(16))
                        .push(text(texts.details.get(index * 2 + 1)?).size(13))
                        .push(text(format!("{} / 10", (raw / 100).clamp(0, 10))));
                }
            }
        }
    }
    let close = match popup {
        Popup::Motive(_) => Message::CloseMotiveDetails,
        Popup::Detail(_) => Message::ClosePersonDetails(person.identity(), popup),
    };
    Some(container(rows.push(button("Close details").on_press(close)))
        .padding(12).width(320).style(container::rounded_box).into())
}

#[cfg(test)]
#[path = "person_panel/tests.rs"]
mod tests;
