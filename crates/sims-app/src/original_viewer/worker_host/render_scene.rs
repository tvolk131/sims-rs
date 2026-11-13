use super::window_session;
use sims_render::Scene;
use std::sync::Arc;

#[derive(Default)]
pub(super) struct RenderScene {
    retained: Option<(Arc<Scene>, Arc<Scene>)>,
}

impl RenderScene {
    pub fn capture(&mut self, scene: Option<&Arc<Scene>>) -> Option<Arc<Scene>> {
        let Some(scene) = scene else {
            self.retained = None;
            return None;
        };
        if scene.censors.is_empty() {
            self.retained = None;
            return Some(scene.clone());
        }
        if let Some((source, composed)) = &self.retained
            && Arc::ptr_eq(source, scene)
        {
            return Some(composed.clone());
        }
        let _span = window_session::span(
            "publication_scene_composite",
            || serde_json::json!({"width": scene.width, "height": scene.height, "censors": scene.censors.len()}),
        );
        let composed = Arc::new(sims_render::software::composited_scene(scene));
        self.retained = Some((scene.clone(), composed.clone()));
        Some(composed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scene(seed: u32) -> Arc<Scene> {
        let pixels = (0..64)
            .flat_map(|index| [(index * 3) as u8, (index * 2) as u8, index as u8, 255])
            .collect();
        let mut scene = sims_render::software::rgba_scene(8, 8, pixels).unwrap();
        scene.censors.push(sims_render::censor::Region {
            rect: [-2, 1, 10, 6],
            depth: 0.9,
            seed,
        });
        Arc::new(scene)
    }

    #[test]
    fn render_scene_retains_original_mosaic_pixels_without_mutating_source() {
        for seed in [0, 1, 91, u32::MAX] {
            let source = scene(seed);
            let pixels = sims_render::software::render(&source).into_raw();
            let mut cache = RenderScene::default();
            let prepared = cache.capture(Some(&source)).unwrap();
            assert!(prepared.censors.is_empty());
            assert_eq!(
                [prepared.width, prepared.height],
                [source.width, source.height]
            );
            assert_eq!(prepared.textures[0].rgba.as_ref(), pixels);
            assert_eq!(sims_render::software::render(&source).into_raw(), pixels);
            assert_eq!(source.censors[0].seed, seed);
            assert_eq!(source.draws.len(), 1);
            assert_eq!(source.censors.len(), 1);
        }
    }

    #[test]
    fn render_scene_reuses_only_the_same_retained_immutable_source() {
        let source = scene(7);
        let mut cache = RenderScene::default();
        let first = cache.capture(Some(&source)).unwrap();
        let repeated = cache.capture(Some(&source.clone())).unwrap();
        assert!(Arc::ptr_eq(&first, &repeated));
        let mut changed = source.clone();
        Arc::make_mut(&mut changed).censors[0].seed = 8;
        let next = cache.capture(Some(&changed)).unwrap();
        assert!(!Arc::ptr_eq(&first, &next));
        assert_eq!(
            next.textures[0].rgba.as_ref(),
            sims_render::software::render(&changed).as_raw()
        );
        assert_eq!(
            first.textures[0].rgba.as_ref(),
            sims_render::software::render(&source).as_raw()
        );
        let equivalent = Arc::new((*changed).clone());
        assert!(!Arc::ptr_eq(
            &next,
            &cache.capture(Some(&equivalent)).unwrap()
        ));
    }

    #[test]
    fn render_scene_drops_cached_versions_on_empty_or_uncensored_publication() {
        let mut cache = RenderScene::default();
        let source = scene(1);
        let prepared = cache.capture(Some(&source)).unwrap();
        let source_weak = Arc::downgrade(&source);
        let prepared_weak = Arc::downgrade(&prepared);
        drop(source);
        drop(prepared);
        assert!(cache.capture(None).is_none());
        assert!(source_weak.upgrade().is_none());
        assert!(prepared_weak.upgrade().is_none());

        let source = scene(2);
        let prepared = cache.capture(Some(&source)).unwrap();
        let prepared_weak = Arc::downgrade(&prepared);
        drop(prepared);
        let mut plain = (*source).clone();
        plain.censors.clear();
        let plain = Arc::new(plain);
        assert!(Arc::ptr_eq(&cache.capture(Some(&plain)).unwrap(), &plain));
        assert!(prepared_weak.upgrade().is_none());
        assert!(cache.retained.is_none());
    }
}
