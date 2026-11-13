use iced::{
    Element,
    widget::{button, container},
};
use sims_vm::dialog::Record;
use std::{collections::BTreeMap, sync::Mutex};
#[path = "tutorial_dialog.rs"]
mod tutorial_dialog;

type Handle = iced::widget::image::Handle;
type BitmapFrames = BTreeMap<(String, u16), Option<(u32, u32, Handle)>>;
type GlobalFrames = BTreeMap<(i32, bool), Option<(u32, u32, Handle)>>;

/// Immutable installed artwork. No asset Index, lazy cache or mutex is reachable
/// from this owner. Handle clones retain the original immutable image payload.
#[derive(Debug)]
pub struct StaticArt {
    hud: BTreeMap<i32, (u32, u32, Vec<Handle>)>,
    hud_edges: BTreeMap<i32, Handle>,
    mood_strips: BTreeMap<(i32, u32), Handle>,
    pause: [Handle; 2],
    tutorial: Handle,
    tutorial_skin: tutorial_dialog::Skin,
    jobs: Vec<(u32, u32, Handle)>,
}

/// The exact prepared frontend handles at one ordinary art publication cut.
/// It owns immutable map versions and cannot lock or prepare the worker caches.
#[derive(Debug, Clone)]
pub struct HudArt {
    static_art: std::sync::Arc<StaticArt>,
    global_frames: std::sync::Arc<GlobalFrames>,
    bitmaps: std::sync::Arc<BitmapFrames>,
    portraits: std::sync::Arc<BTreeMap<u64, Handle>>,
    residents: std::sync::Arc<BTreeMap<i16, Handle>>,
}
impl HudArt {
    pub fn static_art(&self) -> &StaticArt {
        &self.static_art
    }
    pub fn resident(&self, neighbor: i16) -> Option<Handle> {
        self.residents.get(&neighbor).cloned()
    }
}
impl std::ops::Deref for HudArt {
    type Target = StaticArt;
    fn deref(&self) -> &StaticArt {
        &self.static_art
    }
}

/// Mutable producer cache; exclusively used by load/update/preparation.
#[derive(Debug)]
pub struct Icons {
    static_art: std::sync::Arc<StaticArt>,
    global: sims_content::dialog::ui_bitmaps::Bitmaps,
    index: sims_content::Index,
    global_frames: Mutex<std::sync::Arc<GlobalFrames>>,
    bitmaps: Mutex<std::sync::Arc<BitmapFrames>>,
    portraits: Mutex<std::sync::Arc<BTreeMap<u64, Handle>>>,
    residents: Mutex<std::sync::Arc<BTreeMap<i16, Handle>>>,
}
impl std::ops::Deref for Icons {
    type Target = StaticArt;
    fn deref(&self) -> &StaticArt {
        &self.static_art
    }
}
fn check_cache_access() {
    #[cfg(test)]
    FORBID_CACHE_LOCK.with(|forbidden| {
        assert!(
            !forbidden.get(),
            "view attempted to lock an Icons producer cache"
        )
    });
}
fn cache_lock<T>(cache: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    check_cache_access();
    cache.lock().unwrap()
}
fn fork_cache<T: Clone>(cache: &Mutex<T>) -> anyhow::Result<T> {
    check_cache_access();
    Ok(cache
        .lock()
        .map_err(|_| anyhow::anyhow!("icon cache poisoned"))?
        .clone())
}
#[cfg(test)]
std::thread_local! { static FORBID_CACHE_LOCK: std::cell::Cell<bool> = const { std::cell::Cell::new(false) }; }

impl Icons {
    /// Cached image handles are immutable, but each branch needs its own maps:
    /// portraits are keyed by live object identity and change during play.
    pub fn publish_art(&self) -> HudArt {
        HudArt {
            static_art: self.static_art.clone(),
            global_frames: cache_lock(&self.global_frames).clone(),
            bitmaps: cache_lock(&self.bitmaps).clone(),
            portraits: cache_lock(&self.portraits).clone(),
            residents: cache_lock(&self.residents).clone(),
        }
    }
    #[cfg(test)]
    pub(crate) fn with_view_caches_held<T>(&self, draw: impl FnOnce() -> T) -> T {
        let _global = self.global_frames.lock().unwrap();
        let _private = self.bitmaps.lock().unwrap();
        let _portraits = self.portraits.lock().unwrap();
        let _residents = self.residents.lock().unwrap();
        struct Reset(bool);
        impl Drop for Reset {
            fn drop(&mut self) {
                FORBID_CACHE_LOCK.with(|f| f.set(self.0));
            }
        }
        let _reset = Reset(FORBID_CACHE_LOCK.with(|f| f.replace(true)));
        draw()
    }
    /// Existing diagnostic instrumentation reads the producer after update.
    /// Production view APIs accept HudArt and cannot use this test-only access.
    #[cfg(test)]
    pub fn resident(&self, neighbor: i16) -> Option<Handle> {
        cache_lock(&self.residents).get(&neighbor).cloned()
    }
    pub fn fork_for_exploration(&self) -> anyhow::Result<Self> {
        Ok(Self {
            static_art: self.static_art.clone(),
            global: self.global.clone(),
            index: self.index.clone(),
            global_frames: Mutex::new(fork_cache(&self.global_frames)?),
            bitmaps: Mutex::new(fork_cache(&self.bitmaps)?),
            portraits: Mutex::new(fork_cache(&self.portraits)?),
            residents: Mutex::new(fork_cache(&self.residents)?),
        })
    }

    pub fn load(index: &sims_content::Index) -> anyhow::Result<Self> {
        let atlas = sims_content::dialog::JobIcons::load(index)?;
        let tutorial = sims_content::dialog::tutorial_highlight(index, [255, 0, 255])?;
        let global = sims_content::dialog::ui_bitmaps::Bitmaps::load(index)?;
        let mut hud = BTreeMap::new();
        let mut hud_edges = BTreeMap::new();
        let mut mood_strips = BTreeMap::new();
        let pause_bitmap = global
            .image(
                index,
                sims_vm::dialog::GlobalBitmap {
                    id: 2026,
                    button_cell: false,
                },
                1,
                [255, 0, 255],
            )?
            .ok_or_else(|| anyhow::anyhow!("pause display bitmap absent"))?;
        anyhow::ensure!(
            pause_bitmap.dimensions() == (60, 30),
            "pause display atlas differs"
        );
        let pause = std::array::from_fn(|row| {
            let cell =
                image::imageops::crop_imm(&pause_bitmap, 0, row as u32 * 15, 15, 15).to_image();
            iced::widget::image::Handle::from_rgba(15, 15, cell.into_raw())
        });
        // Button strips are normal/selected/hover/disabled. Backgrounds and
        // mode trim are single images; Pause2026 is a separate two-row display.
        for id in [
            150, 151, 152, 153, 154, 155, 156, 157, 2003, 2004, 2006, 2007, 2008, 2009, 2010, 2011,
            2012, 2013, 2014, 2023, 2024, 2037, 4500, 4501, 4502, 4503, 4504, 4505, 4506, 4507,
            4510, 4705, 4706, 4707, 4803, 4804, 4805, 4806, 4910, 4911, 4916, 4917, 4918, 4950,
            4951, 4952, 4957, 4958, 4959,
        ] {
            let bitmap = global
                .image(
                    index,
                    sims_vm::dialog::GlobalBitmap {
                        id,
                        button_cell: false,
                    },
                    1,
                    [255, 0, 255],
                )?
                .ok_or_else(|| anyhow::anyhow!("HUD bitmap {id} absent"))?;
            if matches!(id, 4506 | 4510) {
                anyhow::ensure!(bitmap.dimensions() == (27, 25), "mood strip extent differs");
                for height in [2, 5, 10, 15, 20, 25] {
                    let top = if id == 4506 { 25 - height } else { 0 };
                    let crop = image::imageops::crop_imm(&bitmap, 0, top, 27, height).to_image();
                    mood_strips.insert(
                        (id, height),
                        iced::widget::image::Handle::from_rgba(27, height, crop.into_raw()),
                    );
                }
            }
            if matches!(id, 4910 | 4918) {
                let edge =
                    image::imageops::crop_imm(&bitmap, bitmap.width() - 1, 0, 1, bitmap.height())
                        .to_image();
                hud_edges.insert(
                    id,
                    iced::widget::image::Handle::from_rgba(1, edge.height(), edge.into_raw()),
                );
            }
            let count = if [
                2037, 4506, 4510, 4803, 4804, 4805, 4806, 4910, 4911, 4916, 4917, 4918,
            ]
            .contains(&id)
            {
                1
            } else {
                4
            };
            anyhow::ensure!(
                bitmap.width() % count == 0,
                "HUD bitmap strip {id} is malformed"
            );
            let width = bitmap.width() / count;
            let height = bitmap.height();
            let frames = (0..count)
                .map(|i| {
                    let cell =
                        image::imageops::crop_imm(&bitmap, width * i, 0, width, height).to_image();
                    iced::widget::image::Handle::from_rgba(width, height, cell.into_raw())
                })
                .collect();
            hud.insert(id, (width, height, frames));
        }
        Ok(Self {
            static_art: std::sync::Arc::new(StaticArt {
                hud,
                hud_edges,
                mood_strips,
                pause,
                tutorial_skin: tutorial_dialog::Skin::load(index)?,
                tutorial: iced::widget::image::Handle::from_rgba(
                    tutorial.width(),
                    tutorial.height(),
                    tutorial.into_raw(),
                ),
                jobs: (0..22)
                    .map(|job| {
                        let image = atlas.icon(job).unwrap();
                        let (width, height) = image.dimensions();
                        (
                            width,
                            height,
                            iced::widget::image::Handle::from_rgba(width, height, image.into_raw()),
                        )
                    })
                    .collect(),
            }),
            global,
            global_frames: Mutex::new(std::sync::Arc::new(BTreeMap::new())),
            index: index.clone(),
            bitmaps: Mutex::new(std::sync::Arc::new(BTreeMap::new())),
            portraits: Mutex::new(std::sync::Arc::new(BTreeMap::new())),
            residents: Mutex::new(std::sync::Arc::new(BTreeMap::new())),
        })
    }
    pub fn prepare_residents(
        &self,
        buttons: Option<&sims_render::dialog::client::buttons::Buttons>,
        images: &sims_render::portrait::cache::Images,
    ) -> anyhow::Result<()> {
        let mut frames = BTreeMap::new();
        if let Some(buttons) = buttons {
            for b in &buttons.items {
                if let Some(id) = b.image {
                    let rgba = images.get(id)?.rgba()?;
                    anyhow::ensure!(
                        rgba.width() >= 45 && rgba.height() >= 45,
                        "resident portrait cell outside image"
                    );
                    let cell = image::imageops::crop_imm(&rgba, 0, 0, 45, 45).to_image();
                    frames.insert(
                        b.neighbor,
                        iced::widget::image::Handle::from_rgba(45, 45, cell.into_raw()),
                    );
                }
            }
        }
        *cache_lock(&self.residents) = std::sync::Arc::new(frames);
        Ok(())
    }
    /// Run when presentation requests arrive or are restored, before view().
    /// Cache successful and native missing-chunk results; redraw performs no I/O.
    pub fn prepare(&self, records: &[Record]) -> anyhow::Result<()> {
        let mut bitmaps = cache_lock(&self.bitmaps);
        let mut global_frames = cache_lock(&self.global_frames);
        for record in records {
            if let Some(icon) = record.global_bitmap_icon {
                let key = (icon.id, icon.button_cell);
                if !global_frames.contains_key(&key) {
                    // The current installation profile uses US English and the
                    // same magenta key as the other original UI textures.
                    let bitmap = self.global.image(&self.index, icon, 1, [255, 0, 255])?;
                    std::sync::Arc::make_mut(&mut global_frames).insert(
                        key,
                        bitmap.map(|image| {
                            let (width, height) = image.dimensions();
                            (
                                width,
                                height,
                                iced::widget::image::Handle::from_rgba(
                                    width,
                                    height,
                                    image.into_raw(),
                                ),
                            )
                        }),
                    );
                }
            }
            if let Some(chunk) = record.bitmap_icon {
                let key = (record.source.clone(), chunk);
                if !bitmaps.contains_key(&key) {
                    let bitmap = sims_content::dialog::private_bitmap_language(
                        &self.index,
                        &record.source,
                        chunk,
                        1,
                    )?;
                    std::sync::Arc::make_mut(&mut bitmaps).insert(
                        key,
                        bitmap.map(|image| {
                            let (width, height) = image.dimensions();
                            (
                                width,
                                height,
                                iced::widget::image::Handle::from_rgba(
                                    width,
                                    height,
                                    image.into_raw(),
                                ),
                            )
                        }),
                    );
                }
            }
        }
        Ok(())
    }
    /// Crop the native portrait cell from owned pixels. Rebuild on restore so
    /// recycled image IDs from another session never reuse stale presentation.
    pub fn prepare_portraits(
        &self,
        records: &[Record],
        images: &sims_render::portrait::cache::Images,
    ) -> anyhow::Result<()> {
        let mut portraits = BTreeMap::new();
        for record in records {
            if let Some(image) = &record.portrait {
                let rgba = images.get(image.handle)?.rgba()?;
                let [left, top, right, bottom] = image.rect;
                anyhow::ensure!(
                    left >= 0
                        && top >= 0
                        && right > left
                        && bottom > top
                        && right as u32 <= rgba.width()
                        && bottom as u32 <= rgba.height(),
                    "dialog portrait rectangle outside image"
                );
                let cell = image::imageops::crop_imm(
                    &rgba,
                    left as u32,
                    top as u32,
                    (right - left) as u32,
                    (bottom - top) as u32,
                )
                .to_image();
                portraits.insert(
                    record.id,
                    iced::widget::image::Handle::from_rgba(
                        cell.width(),
                        cell.height(),
                        cell.into_raw(),
                    ),
                );
            }
        }
        *cache_lock(&self.portraits) = std::sync::Arc::new(portraits);
        Ok(())
    }
}

pub fn view<'a, M: Clone + 'a>(
    record: &'a Record,
    icons: &'a HudArt,
    response: impl Fn(u64, u32) -> M,
) -> Element<'a, M> {
    tutorial_dialog::view(record, icons, response)
}

/// Presentation placement only. Tutorial pictures use the observed original
pub fn overlay<'a, M: Clone + 'a>(
    record: &'a Record,
    icons: &'a HudArt,
    response: impl Fn(u64, u32) -> M,
) -> Element<'a, M> {
    use iced::{
        Fill,
        alignment::{Horizontal, Vertical},
        widget,
    };
    let tutorial = tutorial_dialog::supported(record);
    let card = view(record, icons, response);
    if tutorial {
        let placed = container(widget::opaque(card))
            .width(Fill)
            .height(Fill)
            .align_x(Horizontal::Right)
            .align_y(Vertical::Top);
        if record.modal() {
            widget::opaque(placed).into()
        } else {
            placed.into()
        }
    } else if record.modal() {
        widget::opaque(container(card).center(Fill)).into()
    } else {
        container(widget::opaque(card))
            .width(Fill)
            .height(Fill)
            .align_y(Vertical::Bottom)
            .padding(iced::Padding {
                top: 10.,
                right: 10.,
                bottom: 240.,
                left: 20.,
            })
            .into()
    }
}

#[cfg(test)]
impl HudArt {
    pub(crate) fn synthetic_for_test(
        hud: &[(i32, u32, u32)],
        pictures: &[(&str, u16, u32, u32)],
    ) -> Self {
        let solid = |width: u32, height: u32| {
            Handle::from_rgba(width, height, vec![255; (width * height * 4) as usize])
        };
        Self {
            static_art: std::sync::Arc::new(StaticArt {
                hud: hud
                    .iter()
                    .map(|&(id, width, height)| {
                        let count = if id == 2037 { 1 } else { 4 };
                        (
                            id,
                            (
                                width,
                                height,
                                (0..count).map(|_| solid(width, height)).collect(),
                            ),
                        )
                    })
                    .collect(),
                hud_edges: BTreeMap::new(),
                mood_strips: BTreeMap::new(),
                pause: [solid(15, 15), solid(15, 15)],
                tutorial: solid(50, 50),
                tutorial_skin: tutorial_dialog::Skin::synthetic(),
                jobs: (0..22).map(|_| (84, 63, solid(84, 63))).collect(),
            }),
            global_frames: std::sync::Arc::new(BTreeMap::new()),
            bitmaps: std::sync::Arc::new(
                pictures
                    .iter()
                    .map(|&(source, chunk, width, height)| {
                        (
                            (source.to_owned(), chunk),
                            Some((width, height, solid(width, height))),
                        )
                    })
                    .collect(),
            ),
            portraits: std::sync::Arc::new(BTreeMap::new()),
            residents: std::sync::Arc::new(BTreeMap::new()),
        }
    }
}

impl StaticArt {
    pub fn hud(&self, id: i32) -> Option<&(u32, u32, Vec<iced::widget::image::Handle>)> {
        self.hud.get(&id)
    }
    pub fn hud_edge(&self, id: i32) -> Option<iced::widget::image::Handle> {
        self.hud_edges.get(&id).cloned()
    }
    pub fn mood_strip(&self, id: i32, height: u32) -> Option<iced::widget::image::Handle> {
        self.mood_strips.get(&(id, height)).cloned()
    }
    pub fn pause(&self, paused: bool) -> iced::widget::image::Handle {
        self.pause[usize::from(paused)].clone()
    }
    pub fn tutorial(&self) -> iced::widget::image::Handle {
        self.tutorial.clone()
    }
}
