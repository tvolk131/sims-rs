//! Original416534 house refresh and its416085 valuation child.
use super::*;
use sims_sim::house_valuation::{self as value, Publication};

impl Session {
    ///416534. Room scores rebuild at the later view-frame boundary; routes
    ///consume their invalidation only when their suspended wrapper resumes.
    pub fn refresh_original_house(&mut self) -> Result<Publication> {
        //467436 only discards room-owned memoized routing rectangles. Rust
        //does not retain that memoization: each new route calls route_obstacles
        //with current geometry. In-flight path/candidate vectors are distinct
        //and must survive this refresh. Light models/room tiles are not erased.
        self.simulation
            .room_environment
            .as_ref()
            .context("house refresh room manager absent")?
            .manager
            .invalidate_active_rooms(&mut self.simulation.vm)
            .map_err(anyhow::Error::msg)?;
        self.simulation
            .invalidate_original_route_caches()
            .map_err(anyhow::Error::msg)?;
        self.update_house_valuation()
    }
    /// Execute the complete valuation child without driving scripts or time.
    ///This can also be called independently of the larger416534 refresh.
    pub fn update_house_valuation(&mut self) -> Result<Publication> {
        let lot = self
            .simulation
            .architecture
            .as_ref()
            .and_then(|a| a.original.as_ref())
            .context("valuation architecture absent")?;
        let size = match self.simulation.vm.world.globals.get(&23).copied() {
            Some(0) => 64,
            Some(n) if n > 3 => n as usize,
            _ => lot.size,
        };
        ensure!(
            size >= 3 && size <= lot.size,
            "valuation active bounds unavailable"
        );
        let native = self
            .simulation
            .native_lot
            .as_ref()
            .context("valuation placement absent")?;
        let mut floors = lot
            .floors
            .iter()
            .map(|f| f.source.clone())
            .collect::<Vec<_>>();
        for (floor, flags) in floors.iter_mut().zip(&native.placement.flags) {
            floor.flags.clone_from(flags);
        }
        let catalog = sims_content::house_valuation::load(
            &self.index,
            &self.house,
            size,
            lot.size,
            &floors,
            &lot.pool,
        )?;
        let architecture = value::architecture(
            size,
            lot.size,
            &floors,
            &lot.pool,
            &catalog.prices,
            |_, _| {},
        )
        .map_err(anyhow::Error::msg)?;
        let people = self
            .simulation
            .vm
            .world
            .person
            .as_ref()
            .context("valuation neighborhood absent")?;
        let family = people.controlled_family;
        let number = self
            .house
            .families
            .get(&(family as u16))
            .context("valuation selected family metadata absent")?
            .number;
        let flags = people
            .family_metadata
            .get(&family)
            .context("valuation family flags absent")?
            .flags;
        let objects = self
            .simulation
            .value_original_objects(&self.program, size)
            .map_err(anyhow::Error::msg)?;
        let result = value::publication(&objects, &architecture, number, flags, size as i32);
        //416085 writes these family bits directly; no dirty flag, notification
        //or budget expense accompanies its valuation publication.
        self.simulation
            .vm
            .world
            .person
            .as_mut()
            .unwrap()
            .family_metadata
            .get_mut(&family)
            .unwrap()
            .flags = result.family_flags;
        self.simulation
            .household
            .as_mut()
            .context("valuation household absent")?
            .valuation = Some(value::Retained {
            objects_value: result.objects_value,
            architecture_value: result.architecture_value,
        });
        for (key, n) in [23, 22, 37].into_iter().zip(result.globals) {
            self.simulation.vm.world.globals.insert(key, n);
        }
        self.host.presentation.valuation_catalog = Some(catalog);
        Ok(result)
    }
}
