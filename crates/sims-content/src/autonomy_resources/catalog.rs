//! A non-owning FCNS service selector for the one retained catalog resource
//! owner. Each service borrows that owner exclusively; no IFF/stream is cloned.
//! This is deliberately not Clone/Serialize: a saved selector cannot stand in
//! for the resource owner and its aliases. Legacy Provider snapshots are intact.
use super::*;
use crate::{object_catalog::Catalog, object_resources::Resources};

pub struct CatalogProvider {
    source: Source,
    global: u32,
    identity: std::sync::Weak<()>,
    pub events: Vec<Event>,
}

/// Borrowed state for evidence/tests. Resident tables and payload bytes are not
/// copied; only the small external-handle projection is assembled.
pub struct CatalogObservation<'a> {
    pub aggregate: &'a Composite,
    pub iff: &'a Iff,
    pub stream_position: u32,
    pub external_handles: Vec<(u32, u32)>,
    pub next_external_handle: u32,
}

impl CatalogProvider {
    pub fn for_catalog(index: &Index, catalog: &Catalog<'_>) -> Result<Self> {
        let global = catalog
            .retained_global_resource()
            .map_err(anyhow::Error::msg)?;
        Self::for_resources(index, global, &catalog.resources)
    }

    /// Also permits a bounded receiver harness. This constructor certifies the
    /// Global capability/source, not that all other native owners were imported.
    pub fn for_resources(index: &Index, global: u32, resources: &Resources<'_>) -> Result<Self> {
        let receiver = resources
            .retained_global_receiver(global)
            .map_err(anyhow::Error::msg)?;
        let source = Source::observed(&receiver.source.locator);
        let bytes = source.bytes(index)?;
        ensure!(
            receiver.bytes == bytes,
            "catalog Global stream differs from observed source"
        );
        let provider = Self {
            source,
            global,
            identity: resources.retained_identity(),
            events: vec![],
        };
        provider.validate_owner(resources)?;
        Ok(provider)
    }

    pub fn binding(&self) -> Binding {
        self.source.binding()
    }

    pub fn validate_catalog(&self, catalog: &Catalog<'_>) -> Result<()> {
        ensure!(
            catalog
                .retained_global_resource()
                .map_err(anyhow::Error::msg)?
                == self.global,
            "catalog Global binding changed"
        );
        self.validate_owner(&catalog.resources)
    }

    pub fn observe<'a>(&self, resources: &'a Resources<'_>) -> Result<CatalogObservation<'a>> {
        self.validate_owner(resources)?;
        let receiver = resources
            .retained_global_receiver(self.global)
            .map_err(anyhow::Error::msg)?;
        Ok(CatalogObservation {
            aggregate: receiver.aggregate,
            iff: receiver.iff,
            stream_position: receiver.position,
            external_handles: receiver
                .handles
                .iter()
                .filter_map(|(&external, &(component, local))| {
                    (component == receiver.component).then_some((external, local))
                })
                .collect(),
            next_external_handle: receiver.next_handle,
        })
    }

    fn validate_owner(&self, resources: &Resources<'_>) -> Result<()> {
        let expected = self
            .identity
            .upgrade()
            .context("catalog Global owner no longer exists")?;
        let actual = resources
            .retained_identity()
            .upgrade()
            .context("catalog owner identity absent")?;
        ensure!(
            std::sync::Arc::ptr_eq(&expected, &actual),
            "catalog Global selector belongs to another resource owner"
        );
        let receiver = resources
            .retained_global_receiver(self.global)
            .map_err(anyhow::Error::msg)?;
        ensure!(
            receiver.access.opened && receiver.access.writable == 0 && receiver.iff.valid,
            "borrowed Global stream/capability changed"
        );
        ensure!(
            receiver.aggregate.configurations == Composite::default().configurations
                && receiver.aggregate.filters == Composite::default().filters,
            "borrowed Global aggregate profile changed"
        );
        ensure!(
            receiver.source.locator == self.source.locator
                && receiver.source.offset == self.source.offset
                && receiver.source.size == self.source.size
                && hash(receiver.bytes) == self.source.module_sha256,
            "borrowed Global source identity changed"
        );
        ensure!(
            self.events.len() <= 4096,
            "Global resource diagnostic event bound exceeded"
        );
        Ok(())
    }

    fn event(
        &mut self,
        resources: &Resources<'_>,
        operation: &str,
        handle: u32,
        return_value: i32,
    ) -> Result<()> {
        let receiver = resources
            .retained_global_receiver(self.global)
            .map_err(anyhow::Error::msg)?;
        self.events.push(Event {
            operation: operation.into(),
            handle,
            return_value,
            aggregate_error: receiver.aggregate.error,
            child_error: receiver.iff.error,
        });
        Ok(())
    }

    pub fn service(
        &mut self,
        catalog: &mut Catalog<'_>,
        request: &Request,
        table: Option<&Table>,
    ) -> Result<Reply> {
        self.validate_catalog(catalog)?;
        self.service_with_resources(&mut catalog.resources, request, table)
    }

    pub fn service_with_resources(
        &mut self,
        resources: &mut Resources<'_>,
        request: &Request,
        table: Option<&Table>,
    ) -> Result<Reply> {
        let binding = match request {
            Request::Load(b) | Request::Flush(b) => b,
        };
        ensure!(
            *binding == self.binding(),
            "Global FCNS service source differs"
        );
        self.validate_owner(resources)?;
        match request {
            Request::Load(_) => {
                let h = resources
                    .retained_global_fcns_lookup(self.global)
                    .map_err(anyhow::Error::msg)?;
                let error = resources.error(self.global).map_err(anyhow::Error::msg)?;
                self.event(resources, "Load.Lookup", h, if h == 0 { error } else { 0 })?;
                let table = if h == 0 {
                    Table::default()
                } else {
                    let table = decode(resources.data(h).map_err(anyhow::Error::msg)?)?;
                    resources
                        .release_chunk(self.global, h)
                        .map_err(anyhow::Error::msg)?;
                    let error = resources.error(self.global).map_err(anyhow::Error::msg)?;
                    self.event(resources, "Load.Detach", h, error)?;
                    if error == 0 {
                        resources
                            .retained_free_buffer(h)
                            .map_err(anyhow::Error::msg)?;
                    }
                    table
                };
                Ok(Reply::Loaded {
                    binding: binding.clone(),
                    table,
                })
            }
            Request::Flush(_) => {
                let bytes = encode(table.context("Global Flush table absent")?)?;
                let h = resources.allocate(bytes).map_err(anyhow::Error::msg)?;
                self.event(resources, "Flush.Serialize", h, 0)?;
                resources
                    .add(
                        self.global,
                        composite::Chunk {
                            handle: h,
                            tag: TAG,
                            id: 2,
                            name: b"",
                            variant: 0,
                            converted: 1,
                        },
                    )
                    .map_err(anyhow::Error::msg)?;
                let error = resources.error(self.global).map_err(anyhow::Error::msg)?;
                self.event(resources, "Flush.Add", h, error)?;
                // This exact readonly profile must stop at the native add
                // error. Writable continuation is not supplied as a no-op.
                ensure!(
                    error != 0,
                    "readonly Global unexpectedly accepted a writable add"
                );
                resources
                    .retained_free_buffer(h)
                    .map_err(anyhow::Error::msg)?;
                Ok(Reply::Flushed {
                    binding: binding.clone(),
                    return_value: error,
                })
            }
        }
    }
}

#[cfg(test)]
#[path = "catalog_tests.rs"]
mod tests;
