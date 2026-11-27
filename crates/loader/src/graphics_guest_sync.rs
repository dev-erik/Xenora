// SPDX-License-Identifier: GPL-3.0-or-later
//! Explicit synchronization of packed resource mirrors with checked guest memory.
use super::*;

pub(super) struct GuestBinding {
    address: u64,
    length: usize,
    mappings: Vec<u64>,
}

impl GraphicsContext {
    /// Bind an owned root to guest storage. No bytes are imported or exported.
    /// Rebinding is the explicit opt-in to a replacement guest mapping.
    pub fn bind_guest_memory(
        &mut self,
        resource: GraphicsHandle,
        address: u64,
        memory: &GuestMemory,
    ) -> Result<(), GraphicsError> {
        if self.guest_sync_pinned(resource) {
            return Err(GraphicsError::ResourceLifetimeViolation);
        }
        let length = self.guest_sync_length(resource)?;
        let mappings = memory.mapping_identity(address, length)?;
        self.guest_bindings.insert(
            resource,
            GuestBinding {
                address,
                length,
                mappings,
            },
        );
        Ok(())
    }

    fn guest_sync_pinned(&self, resource: GraphicsHandle) -> bool {
        self.queued
            .values()
            .any(|node| node.receipt.sync_references.contains(&resource))
            || self
                .submission
                .as_ref()
                .is_some_and(|r| r.sync_references.contains(&resource))
    }

    fn guest_sync_length(&self, resource: GraphicsHandle) -> Result<usize, GraphicsError> {
        if let Some(depth) = self.depth_resources.get(&resource) {
            return depth.typed_desc.byte_len();
        }
        let root = self
            .resources
            .get(&resource)
            .ok_or(GraphicsError::InvalidResourceHandle)?;
        if root.guest_backing_addr.is_some() {
            return Err(GraphicsError::InvalidDescriptor);
        }
        Ok(root.data.len())
    }

    pub(crate) fn guest_sync_ranges(
        &self,
        resources: &[GraphicsHandle],
    ) -> Result<Vec<(u64, usize)>, GraphicsError> {
        if resources.is_empty() || resources.len() > 16 {
            return Err(GraphicsError::InvalidDescriptor);
        }
        let mut seen = HashSet::new();
        let mut ranges = Vec::new();
        for resource in resources {
            if !seen.insert(*resource) {
                return Err(GraphicsError::InvalidDescriptor);
            }
            let binding = self
                .guest_bindings
                .get(resource)
                .ok_or(GraphicsError::InvalidDescriptor)?;
            let end = binding
                .address
                .checked_add(binding.length as u64)
                .ok_or(GraphicsError::InvalidDescriptor)?;
            if ranges.iter().any(|&(address, length)| {
                binding.address < address + length as u64 && address < end
            }) {
                return Err(GraphicsError::InvalidDescriptor);
            }
            ranges.push((binding.address, binding.length));
        }
        Ok(ranges)
    }

    /// Atomically import or export complete packed roots at an explicit idle boundary.
    /// Callers must wait and release retained submissions before synchronization.
    pub fn synchronize_guest_memory(
        &mut self,
        resources: &[GraphicsHandle],
        import: bool,
        memory: &mut GuestMemory,
    ) -> Result<(), GraphicsError> {
        let ranges = self.guest_sync_ranges(resources)?;
        let state = if import {
            ResourceStates::COPY_DEST
        } else {
            ResourceStates::COPY_SOURCE
        };
        let mut bytes = 0usize;
        for (&resource, &(address, length)) in resources.iter().zip(&ranges) {
            if self.guest_sync_pinned(resource) {
                return Err(GraphicsError::ResourceLifetimeViolation);
            }
            self.transfer_range(resource, 0, length, state)?;
            if self.guest_sync_length(resource)? != length
                || memory.mapping_identity(address, length)?
                    != self.guest_bindings[&resource].mappings
            {
                return Err(GraphicsError::ResourceLifetimeViolation);
            }
            bytes = bytes
                .checked_add(length)
                .ok_or(GraphicsError::InvalidDescriptor)?;
            if !import {
                memory.validate_write_range(address, length)?;
                if let Some(depth) = self.depth_resources.get(&resource) {
                    depth
                        .snapshot
                        .as_ref()
                        .and_then(|s| s.downcast_ref::<GraphicsTypedDepthSnapshot>())
                        .filter(|s| s.desc == depth.typed_desc && s.bytes.len() == length)
                        .ok_or(GraphicsError::InvalidDescriptor)?;
                }
            }
        }
        if import {
            // Account for both temporary staging and packed depth ownership before reading.
            self.validate_queued_work_budget(
                bytes
                    .checked_mul(2)
                    .ok_or(GraphicsError::InvalidDescriptor)?,
            )?;
            let mut staged = Vec::new();
            for (&resource, &(address, length)) in resources.iter().zip(&ranges) {
                let epoch = self
                    .publication(resource)
                    .host_epoch
                    .checked_add(1)
                    .ok_or(GraphicsError::InvalidDescriptor)?;
                let mut data = Vec::new();
                data.try_reserve_exact(length)
                    .map_err(|_| MemoryError::AllocationFailed)?;
                data.resize(length, 0);
                memory.read_bytes(address, &mut data)?;
                let depth = self.depth_resources.get(&resource).map(|d| {
                    Arc::new(GraphicsTypedDepthSnapshot {
                        desc: d.typed_desc,
                        bytes: Arc::from(data.as_slice()),
                    })
                });
                staged.push((resource, epoch, data, depth));
            }
            // No fallible operation follows the first publication.
            for (resource, host_epoch, data, depth) in staged {
                if let Some(depth) = depth {
                    self.depth_resources.get_mut(&resource).unwrap().snapshot = Some(depth);
                } else {
                    self.resources
                        .get_mut(&resource)
                        .unwrap()
                        .data
                        .copy_from_slice(&data);
                }
                self.root_publications.insert(
                    resource,
                    RootPublication {
                        host_epoch,
                        version: None,
                    },
                );
            }
        } else {
            // Exclusive memory access keeps the preflighted mappings and permissions stable.
            for (&resource, &(address, _)) in resources.iter().zip(&ranges) {
                let data = if let Some(depth) = self.depth_resources.get(&resource) {
                    &depth
                        .snapshot
                        .as_ref()
                        .unwrap()
                        .downcast_ref::<GraphicsTypedDepthSnapshot>()
                        .unwrap()
                        .bytes[..]
                } else {
                    &self.resources[&resource].data[..]
                };
                memory.write_bytes(address, data)?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::MemoryPermissions;

    fn buffer(c: &mut GraphicsContext, d: GraphicsHandle, size: u64) -> GraphicsHandle {
        c.create_committed_resource(
            d,
            ResourceDesc {
                dimension: ResourceDimension::Buffer,
                width: size,
                height: 1,
                depth_or_array_size: 1,
                mip_levels: 1,
                format_rgba8: false,
            },
            ResourceStates::COPY_DEST,
            None,
        )
        .unwrap()
    }

    #[test]
    fn guest_sync_remap_requires_rebind_and_preserves_host_epoch_on_failure() {
        let mut c = GraphicsContext::new();
        let d = c.create_device().unwrap();
        let r = buffer(&mut c, d, 4);
        let mut m = GuestMemory::new();
        m.map_virtual_allocation(0x10000, 4, MemoryPermissions::READ_WRITE)
            .unwrap();
        m.write_bytes(0x10000, &[1, 2, 3, 4]).unwrap();
        c.bind_guest_memory(r, 0x10000, &m).unwrap();
        c.synchronize_guest_memory(&[r], true, &mut m).unwrap();
        assert_eq!(c.publication(r).host_epoch, 1);
        assert!(m.release_virtual_allocation(0x10000));
        m.map_virtual_allocation(0x10000, 4, MemoryPermissions::READ_WRITE)
            .unwrap();
        m.write_bytes(0x10000, &[9, 8, 7, 6]).unwrap();
        c.resources.get_mut(&r).unwrap().state = ResourceStates::COPY_SOURCE;
        assert_eq!(
            c.synchronize_guest_memory(&[r], false, &mut m),
            Err(GraphicsError::ResourceLifetimeViolation)
        );
        let mut actual = [0; 4];
        m.read_bytes(0x10000, &mut actual).unwrap();
        assert_eq!(actual, [9, 8, 7, 6]);
        assert_eq!(c.publication(r).host_epoch, 1);
        c.bind_guest_memory(r, 0x10000, &m).unwrap();
        c.synchronize_guest_memory(&[r], false, &mut m).unwrap();
        m.read_bytes(0x10000, &mut actual).unwrap();
        assert_eq!(actual, [1, 2, 3, 4]);
    }

    #[test]
    fn guest_sync_late_epoch_overflow_does_not_publish_first_root() {
        let mut c = GraphicsContext::new();
        let d = c.create_device().unwrap();
        let a = buffer(&mut c, d, 4);
        let b = buffer(&mut c, d, 4);
        let mut m = GuestMemory::new();
        m.map_region_with_data(
            0x10000,
            8,
            MemoryPermissions::READ_ONLY,
            &[1, 2, 3, 4, 5, 6, 7, 8],
        )
        .unwrap();
        c.bind_guest_memory(a, 0x10000, &m).unwrap();
        c.bind_guest_memory(b, 0x10004, &m).unwrap();
        c.root_publications.insert(
            b,
            RootPublication {
                host_epoch: u64::MAX,
                version: None,
            },
        );
        assert_eq!(
            c.synchronize_guest_memory(&[a, b], true, &mut m),
            Err(GraphicsError::InvalidDescriptor)
        );
        assert_eq!(c.resources[&a].data, [0; 4]);
        assert_eq!(c.publication(a).host_epoch, 0);
        c.root_publications.remove(&b);
        c.synchronize_guest_memory(&[a, b], true, &mut m).unwrap();
        assert_eq!(c.resources[&a].data, [1, 2, 3, 4]);
        assert_eq!(c.resources[&b].data, [5, 6, 7, 8]);
        assert_eq!(c.publication(a).host_epoch, 1);
        assert_eq!(c.publication(b).host_epoch, 1);
    }
}
