use super::*;

pub(crate) struct Catalog {
    candidates: BTreeMap<String, Vec<(usize, Animation)>>,
    aliases: BTreeMap<String, Vec<(usize, Animation)>>,
    hashes: BTreeMap<String, String>,
    catalogs: usize,
    errors: Vec<String>,
    absence_proof: String,
    catalog_names: BTreeSet<String>,
    absence_complete: bool,
}

impl Catalog {
    pub(crate) fn prepare(index: &Index) -> Result<Self> {
        let mut catalog = Self {
            candidates: BTreeMap::new(),
            aliases: BTreeMap::new(),
            hashes: BTreeMap::new(),
            catalogs: 0,
            errors: Vec::new(),
            absence_proof: String::new(),
            catalog_names: BTreeSet::new(),
            absence_complete: true,
        };
        let mut inventory = Vec::new();
        let mut scan = crate::resource_scan::Scan::default();
        for (resource_id, resource) in index.resources.iter().enumerate() {
            if !resource.name.to_ascii_lowercase().ends_with(".cmx.bcf") {
                continue;
            }
            let bytes = scan.read(resource)?;
            let hash = format!("{:x}", Sha256::digest(&bytes));
            inventory.push((
                resource.locator.clone(),
                resource.name.clone(),
                hash.clone(),
            ));
            if let Some(name) = native_catalog_name(resource) {
                catalog.catalog_names.insert(name);
            }
            if bytes.get(..8) != Some(&[0; 8]) {
                if sims_formats::avatar::read_bcf(&bytes).is_err() {
                    catalog.absence_complete = false;
                }
                continue;
            }
            let clips = match sims_formats::animation::read_bcf_native(&bytes) {
                Ok(clips) => clips,
                Err(error) => {
                    catalog
                        .errors
                        .push(format!("{}: {error}", resource.locator));
                    continue;
                }
            };
            catalog.hashes.insert(resource.locator.clone(), hash);
            if let Some((name, clip)) = native_catalog_alias(resource, &clips) {
                catalog
                    .aliases
                    .entry(name)
                    .or_default()
                    .push((resource_id, clip));
            }
            for clip in clips {
                catalog
                    .candidates
                    .entry(clip.name.to_ascii_lowercase())
                    .or_default()
                    .push((resource_id, clip));
            }
            catalog.catalogs += 1;
        }
        catalog.absence_proof = native_absence_inventory(&inventory)?;
        Ok(catalog)
    }

    pub(crate) fn load(
        &self,
        index: &Index,
        program: &mut Program,
        names: &BTreeSet<String>,
    ) -> Result<Report> {
        let mut report = Report {
            catalogs: self.catalogs,
            discovered_clips: self.candidates.len(),
            catalog_errors: self.errors.clone(),
            ..Report::default()
        };
        load_animation_candidates(
            index,
            program,
            names,
            &self.candidates,
            &self.aliases,
            &mut report,
            |resource| {
                self.hashes
                    .get(&resource.locator)
                    .cloned()
                    .context("animation catalog source absent from session snapshot")
            },
        )?;
        confirm_native_absence(
            &mut report,
            &self.absence_proof,
            &self.catalog_names,
            self.absence_complete,
        );
        Ok(report)
    }
}
