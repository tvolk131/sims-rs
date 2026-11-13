//! Replacement Buy widgets. Native507288 filtering and506765 availability
//! feed the actual Iced tree; thumbnail surfaces/layout are frontend-owned.
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use sims_content::object_catalog::products::{Product, labels::Labels};
use sims_sim::object_catalog::products::{
    Masks,
    categories::{Availability, Button},
};
use sims_vm::interface::state::State;

/// Product details detach from the input tree on501822/507278 ->508422.
/// A newly attached frontend explicitly starts with neither inspector open.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Contents {
    pub(super) buy: Catalog,
    pub(super) build_inspector: Option<u32>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Catalog {
    #[serde(default)]
    revision: u64,
    products: Vec<Product>,
    availability: Availability,
    labels: Labels,
    /// Fixed replacement layout capacity, not the legacy window's pixel width.
    capacity: usize,
    catalog: i32,
    subcatalog: i32,
    masks: Masks,
    pub(super) inspector: Option<u32>,
    view: Option<View>,
    #[serde(default)]
    selected_products: std::collections::BTreeSet<usize>,
    #[serde(default)]
    pub(super) insufficient_funds: bool,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct View {
    pub categories: [Button; 8],
    pub labels: [String; 8],
    pub heading: String,
    pub page: u32,
    pub previous: bool,
    pub next: bool,
    /// Product vector indices preserve definition identity and catalog order.
    pub products: Vec<usize>,
    pub subcatalog: i32,
}
impl Contents {
    pub(super) fn new(products: Vec<Product>, labels: Labels, capacity: usize) -> Result<Self> {
        ensure!(
            capacity > 0 && capacity <= 128,
            "Buy page capacity unsupported"
        );
        let mut availability = Availability::default();
        for p in &products {
            availability.add(p.fields).map_err(anyhow::Error::msg)?;
        }
        Ok(Self {
            buy: Catalog {
                revision: 0,
                products,
                availability,
                labels,
                capacity,
                catalog: 0,
                subcatalog: 0,
                masks: Masks::default(),
                inspector: None,
                view: None,
                selected_products: Default::default(),
                insufficient_funds: false,
            },
            build_inspector: None,
        })
    }
    pub(super) fn validate(&self) -> Result<()> {
        ensure!(
            self.buy.capacity > 0 && self.buy.capacity <= 128,
            "Buy page capacity invalid"
        );
        ensure!(
            self.buy
                .selected_products
                .iter()
                .all(|&i| i < self.buy.products.len()),
            "selected Buy product absent"
        );
        if let Some(v) = &self.buy.view {
            ensure!(
                v.products.len() <= self.buy.capacity
                    && v.products.iter().all(|&i| i < self.buy.products.len()),
                "Buy page refers to absent products"
            );
        }
        ensure!(
            self.buy.inspector.is_none_or(|id| self
                .buy
                .products
                .iter()
                .any(|p| p.definition == id)),
            "Buy inspector product absent"
        );
        Ok(())
    }
}
impl Catalog {
    pub fn accepts_product(&self, revision: u64, index: usize) -> bool {
        revision == self.revision
            && self
                .view
                .as_ref()
                .is_some_and(|v| v.products.contains(&index))
    }
    ///Resident item and widget identities belong to this catalog's namespace.
    ///They are neither object IDs nor GUIDs nor registered-definition IDs.
    pub fn item_id(index: usize) -> Result<u32> {
        u32::try_from(index)?
            .checked_add(1)
            .ok_or_else(|| anyhow::anyhow!("Buy item ID overflow"))
    }
    pub fn selected(&self, index: usize) -> bool {
        self.selected_products.contains(&index)
    }
    pub fn insufficient_funds(&self) -> bool {
        self.insufficient_funds
    }
    pub(super) fn slot(&self, index: u32) -> Result<sims_vm::interface::buy::product::Slot> {
        use sims_vm::interface::buy::product::Slot;
        Ok(
            match self
                .view
                .as_ref()
                .and_then(|v| v.products.get(index as usize))
            {
                Some(&i) => {
                    let product = Self::item_id(i)?;
                    Slot::Item {
                        product,
                        widget: Some(product),
                    }
                }
                None => Slot::End,
            },
        )
    }
    pub(super) fn select_product_widget(&mut self, widget: u32, selected: bool) -> Result<()> {
        ensure!(
            widget != 0 && (widget as usize) <= self.products.len(),
            "Buy product widget absent"
        );
        let index = widget as usize - 1;
        if selected {
            self.selected_products.insert(index);
        } else {
            self.selected_products.remove(&index);
        }
        Ok(())
    }
    pub(super) fn clear_product_selection(&mut self) {
        self.selected_products.clear();
    }
    pub fn revision(&self) -> u64 {
        self.revision
    }
    pub fn accepts(&self, revision: u64, event: sims_vm::interface::buy::input::Event) -> bool {
        use sims_vm::interface::buy::input::Event;
        let Some(v) = &self.view else {
            return false;
        };
        revision == self.revision
            && match event {
                Event::Category(i) => v
                    .categories
                    .get(i as usize)
                    .is_some_and(|b| b.visible && b.enabled),
                Event::Back => v.subcatalog != 0,
                Event::Previous => v.previous,
                Event::Next => v.next,
            }
    }
    pub(super) fn select_category(&mut self, selected: Option<u8>) -> Result<()> {
        ensure!(
            selected.is_none_or(|i| i < 8),
            "selected category widget absent"
        );
        let view = self
            .view
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("catalog view absent"))?;
        for (i, button) in view.categories.iter_mut().enumerate() {
            button.selected = selected == Some(i as u8);
        }
        Ok(())
    }
    pub fn view(&self) -> Option<&View> {
        self.view.as_ref()
    }
    pub fn product(&self, index: usize) -> Option<&Product> {
        self.products.get(index)
    }
    pub fn product_count(&self) -> usize {
        self.products.len()
    }
    pub(super) fn update(&mut self, state: &State) -> Result<()> {
        let f = &state.buy;
        self.catalog = f.catalog;
        self.subcatalog = f.subcatalog;
        self.masks.update(f.catalog, &f.indices);
        if f.dirty & 0x400 == 0 && !(f.dirty & 2 != 0 && state.mode == 1) {
            return Ok(());
        }
        //507288 destroys the old visible item's window if it no longer
        //belongs to the current filter, otherwise it only hides that window.
        if let Some(old) = &self.view {
            for &i in &old.products {
                if f.subcatalog == 0 || !self.products[i].visible(f.catalog, self.masks) {
                    self.selected_products.remove(&i);
                }
            }
        }
        let categories = self
            .availability
            .buttons(f.catalog, f.subcatalog, &f.indices)
            .map_err(anyhow::Error::msg)?;
        let catalog = f.catalog as usize;
        let row = f.indices[catalog][0];
        let mut products = Vec::new();
        let mut next = false;
        if f.subcatalog != 0 {
            // Native multiplies 32-bit page*capacity, then compares signed.
            let skip = (f.selected_table.wrapping_mul(self.capacity as u32) as i32).max(0) as usize;
            let mut matches = self
                .products
                .iter()
                .enumerate()
                .filter(|(_, p)| p.visible(f.catalog, self.masks))
                .skip(skip);
            products.extend(matches.by_ref().take(self.capacity).map(|(i, _)| i));
            next = matches.next().is_some();
        }
        let heading = if f.subcatalog == 0 {
            String::new()
        } else {
            sims_formats::strings::decode(
                self.labels.root(catalog, row as usize),
                self.labels.language,
            )
        };
        self.revision = self
            .revision
            .checked_add(1)
            .ok_or_else(|| anyhow::anyhow!("catalog revision exhausted"))?;
        self.view = Some(View {
            categories,
            labels: std::array::from_fn(|i| {
                self.labels
                    .category(catalog, f.subcatalog, row.max(0) as usize, i)
            }),
            heading,
            page: f.selected_table,
            previous: f.selected_table != 0,
            next,
            products,
            subcatalog: f.subcatalog,
        });
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn page_rebuild_gates_wraparound_and_restore_preserve_shared_state() -> Result<()> {
        let labels: Labels = serde_json::from_value(serde_json::json!({
            "source":"test:UIText", "language":1,
            "roots":vec![vec![b"category".to_vec();8];7],
            "functions":vec![vec![b"subtype".to_vec();4];8],
            "special":vec![b"all".to_vec();4],
        }))?;
        let mut fields = sims_sim::object_catalog::products::Definition::parse(&[0; 216]).unwrap();
        fields.room = 8;
        fields.function = 1;
        fields.subfunction = 1;
        let products = (0..20)
            .map(|i| Product {
                definition: i + 1,
                source: sims_vm::Definition {
                    source: "test".into(),
                    object: i as u16,
                    guid: i,
                    slots: 0,
                },
                fields,
                name: vec![],
                description: vec![],
                language: 1,
            })
            .collect();
        let mut c = Contents::new(products, labels, 8)?.buy;
        let mut s = State::constructed(760);
        s.mode = 1;
        s.buy.catalog = 1;
        s.buy.subcatalog = 1;
        s.buy.indices[1] = [0, 7];
        s.buy.dirty = 0x400;
        s.buy.selected_table = 1;
        let before = s.clone();
        c.update(&s)?;
        ensure!(s == before);
        ensure!(c.view().unwrap().products == (8..16).collect::<Vec<_>>());
        ensure!(c.view().unwrap().previous && c.view().unwrap().next);
        let mut restored: Catalog = serde_json::from_slice(&serde_json::to_vec(&c)?)?;
        for (dirty, page, expected) in [(0, 2, 8..16), (2, 2, 16..20), (0x400, u32::MAX, 0..8)] {
            s.buy.dirty = dirty;
            s.buy.selected_table = page;
            c.update(&s)?;
            restored.update(&s)?;
            ensure!(restored == c);
            ensure!(c.view().unwrap().products == expected.collect::<Vec<_>>());
        }
        s.buy.subcatalog = 0;
        c.update(&s)?;
        ensure!(c.view().unwrap().products.is_empty());
        ensure!(c.view().unwrap().categories.iter().all(|b| !b.selected));
        Ok(())
    }
}
