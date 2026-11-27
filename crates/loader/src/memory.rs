// SPDX-License-Identifier: GPL-3.0-or-later

//! Guest virtual memory model for Xenora's x86-64 execution bootstrap.
//!
//! Provides isolated, page-aligned guest virtual memory with permission checking
//! (Read, Write, Execute) and bounds enforcement without host OS mapping tricks.

use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_MAPPING_ID: AtomicU64 = AtomicU64::new(1);

fn allocate_mapping_id() -> Result<u64, MemoryError> {
    NEXT_MAPPING_ID
        .try_update(Ordering::Relaxed, Ordering::Relaxed, |id| id.checked_add(1))
        .map_err(|_| MemoryError::AllocationFailed)
}

pub(crate) const ALLOCATION_GRANULARITY: u64 = 65_536;

/// Permission flags for a mapped memory region.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MemoryPermissions {
    pub read: bool,
    pub write: bool,
    pub execute: bool,
}

impl MemoryPermissions {
    pub const NO_ACCESS: Self = Self {
        read: false,
        write: false,
        execute: false,
    };
    pub const READ_ONLY: Self = Self {
        read: true,
        write: false,
        execute: false,
    };
    pub const READ_WRITE: Self = Self {
        read: true,
        write: true,
        execute: false,
    };
    pub const READ_EXECUTE: Self = Self {
        read: true,
        write: false,
        execute: true,
    };
    pub const READ_WRITE_EXECUTE: Self = Self {
        read: true,
        write: true,
        execute: true,
    };
}

/// Errors occurring during guest memory operations.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MemoryError {
    UnmappedAddress(u64),
    PermissionViolation {
        address: u64,
        required: &'static str,
    },
    OverlappingRegion {
        base: u64,
        size: u64,
    },
    Overflow,
    AllocationFailed,
}

impl fmt::Display for MemoryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnmappedAddress(addr) => write!(f, "unmapped guest address 0x{addr:016x}"),
            Self::PermissionViolation { address, required } => {
                write!(
                    f,
                    "permission violation at guest address 0x{address:016x}: requires {required}"
                )
            }
            Self::OverlappingRegion { base, size } => {
                write!(
                    f,
                    "overlapping memory region: base 0x{base:016x}, size 0x{size:x}"
                )
            }
            Self::Overflow => write!(f, "guest memory address calculation overflow"),
            Self::AllocationFailed => write!(f, "guest memory allocation exceeds allowed limit"),
        }
    }
}

impl std::error::Error for MemoryError {}

struct MappedRegion {
    identity: u64,
    base: u64,
    data: Vec<u8>,
    permissions: MemoryPermissions,
    virtual_allocation: bool,
}

impl Clone for MappedRegion {
    fn clone(&self) -> Self {
        Self {
            // Clone cannot return an allocation error; never reuse an identity.
            identity: allocate_mapping_id().expect("guest mapping identity counter exhausted"),
            base: self.base,
            data: self.data.clone(),
            permissions: self.permissions,
            virtual_allocation: self.virtual_allocation,
        }
    }
}

impl MappedRegion {
    fn contains(&self, address: u64, size: usize) -> bool {
        let size_u64 = size as u64;
        let Some(end) = address.checked_add(size_u64) else {
            return false;
        };
        let region_end = self.base + self.data.len() as u64;
        address >= self.base && end <= region_end
    }
}

/// Guest virtual memory address space.
#[derive(Default, Clone)]
pub struct GuestMemory {
    regions: Vec<MappedRegion>,
}

impl GuestMemory {
    /// Create a new empty guest address space.
    pub fn new() -> Self {
        Self {
            regions: Vec::new(),
        }
    }

    /// Map a new region into guest virtual memory.
    pub fn map_region(
        &mut self,
        base: u64,
        size: usize,
        permissions: MemoryPermissions,
    ) -> Result<(), MemoryError> {
        self.map_region_with_data(base, size, permissions, &[])
    }

    /// Map a new region and initialize it with initial data, bypassing runtime write permission checks.
    pub fn map_region_with_data(
        &mut self,
        base: u64,
        size: usize,
        permissions: MemoryPermissions,
        initial_data: &[u8],
    ) -> Result<(), MemoryError> {
        self.insert_region(base, size, permissions, initial_data, false)
    }

    fn insert_region(
        &mut self,
        base: u64,
        size: usize,
        permissions: MemoryPermissions,
        initial_data: &[u8],
        virtual_allocation: bool,
    ) -> Result<(), MemoryError> {
        let size_u64 = size as u64;
        let end = base.checked_add(size_u64).ok_or(MemoryError::Overflow)?;
        for r in &self.regions {
            let r_end = r.base + r.data.len() as u64;
            if base < r_end && r.base < end {
                return Err(MemoryError::OverlappingRegion {
                    base,
                    size: size_u64,
                });
            }
        }

        let mut data = Vec::new();
        data.try_reserve_exact(size)
            .map_err(|_| MemoryError::AllocationFailed)?;
        data.resize(size, 0);
        let copy_len = size.min(initial_data.len());
        data[..copy_len].copy_from_slice(&initial_data[..copy_len]);

        self.regions.push(MappedRegion {
            identity: allocate_mapping_id()?,
            base,
            data,
            permissions,
            virtual_allocation,
        });
        self.regions.sort_by_key(|r| r.base);
        Ok(())
    }

    pub(crate) fn map_virtual_allocation(
        &mut self,
        requested_base: u64,
        size: usize,
        permissions: MemoryPermissions,
    ) -> Result<u64, MemoryError> {
        // Bound the host storage retained by the synthetic VirtualAlloc service.
        const MAX_VIRTUAL_BYTES: usize = 64 * 1024 * 1024;
        let allocated: usize = self
            .regions
            .iter()
            .filter(|region| region.virtual_allocation)
            .map(|region| region.data.len())
            .sum();
        if size > MAX_VIRTUAL_BYTES - allocated {
            return Err(MemoryError::AllocationFailed);
        }

        let mut base = requested_base;
        if base == 0 {
            base = 0x0000_0003_0000_0000;
            for region in &self.regions {
                let end = base.checked_add(size as u64).ok_or(MemoryError::Overflow)?;
                if end <= region.base {
                    break;
                }
                let region_end = region.base + region.data.len() as u64;
                if base < region_end {
                    base = region_end
                        .checked_next_multiple_of(ALLOCATION_GRANULARITY)
                        .ok_or(MemoryError::Overflow)?;
                }
            }
        }
        self.insert_region(base, size, permissions, &[], true)?;
        Ok(base)
    }

    pub(crate) fn release_virtual_allocation(&mut self, base: u64) -> bool {
        let Some(index) = self
            .regions
            .iter()
            .position(|region| region.base == base && region.virtual_allocation)
        else {
            return false;
        };
        self.regions.remove(index);
        true
    }

    /// Resolve `[address, address + len)` to `(region index, offset, length)` pieces.
    ///
    /// An access may span adjacent regions, as it would span adjacent pages, but every
    /// byte must be mapped and every region must grant `required` (`None` bypasses
    /// permission checks for the loader). The whole range is checked before any byte
    /// is copied, so a failed access never partially reads or writes.
    fn resolve(
        &self,
        address: u64,
        len: usize,
        required: Option<&'static str>,
    ) -> Result<Vec<(usize, usize, usize)>, MemoryError> {
        let end = address
            .checked_add(len as u64)
            .ok_or(MemoryError::Overflow)?;
        if len == 0 {
            return if self.regions.iter().any(|r| r.contains(address, 0)) {
                Ok(Vec::new())
            } else {
                Err(MemoryError::UnmappedAddress(address))
            };
        }
        let mut pieces = Vec::new();
        let mut cursor = address;
        for (index, region) in self.regions.iter().enumerate() {
            let region_end = region.base + region.data.len() as u64;
            if region_end <= cursor {
                continue;
            }
            if region.base > cursor {
                break;
            }
            let permitted = match required {
                None => true,
                Some("read") => region.permissions.read,
                Some("write") => region.permissions.write,
                Some(_) => region.permissions.execute,
            };
            if let (false, Some(required)) = (permitted, required) {
                return Err(MemoryError::PermissionViolation { address, required });
            }
            let take = (end.min(region_end) - cursor) as usize;
            pieces.push((index, (cursor - region.base) as usize, take));
            cursor += take as u64;
            if cursor == end {
                return Ok(pieces);
            }
        }
        Err(MemoryError::UnmappedAddress(address))
    }

    fn read_checked(
        &self,
        address: u64,
        dst: &mut [u8],
        required: Option<&'static str>,
    ) -> Result<(), MemoryError> {
        let mut done = 0;
        for (index, offset, take) in self.resolve(address, dst.len(), required)? {
            dst[done..done + take]
                .copy_from_slice(&self.regions[index].data[offset..offset + take]);
            done += take;
        }
        Ok(())
    }

    fn write_checked(
        &mut self,
        address: u64,
        src: &[u8],
        required: Option<&'static str>,
    ) -> Result<(), MemoryError> {
        let mut done = 0;
        for (index, offset, take) in self.resolve(address, src.len(), required)? {
            self.regions[index].data[offset..offset + take]
                .copy_from_slice(&src[done..done + take]);
            done += take;
        }
        Ok(())
    }

    /// Return region identities in address order after checking the entire mapped span.
    /// Permissions do not affect identity. Empty spans follow the access resolver
    /// semantics and return no identities when their address is mapped.
    pub(crate) fn mapping_identity(
        &self,
        address: u64,
        len: usize,
    ) -> Result<Vec<u64>, MemoryError> {
        Ok(self
            .resolve(address, len, None)?
            .into_iter()
            .map(|(index, _, _)| self.regions[index].identity)
            .collect())
    }

    /// Preflight an entire output span without changing guest memory.
    pub fn validate_write_range(&self, address: u64, len: usize) -> Result<(), MemoryError> {
        self.resolve(address, len, Some("write")).map(|_| ())
    }

    /// Write raw bytes into guest memory at the specified address, checking permissions.
    pub fn write_bytes(&mut self, address: u64, src: &[u8]) -> Result<(), MemoryError> {
        self.write_checked(address, src, Some("write"))
    }

    /// Read raw bytes from guest memory at the specified address, checking permissions.
    pub fn read_bytes(&self, address: u64, dst: &mut [u8]) -> Result<(), MemoryError> {
        self.read_checked(address, dst, Some("read"))
    }

    /// Read an instruction byte from guest memory, verifying execute permission.
    pub fn fetch_instruction_byte(&self, address: u64) -> Result<u8, MemoryError> {
        let mut buf = [0u8; 1];
        self.read_checked(address, &mut buf, Some("execute"))?;
        Ok(buf[0])
    }

    /// Read a 64-bit integer (little endian) from guest memory.
    pub fn read_u64(&self, address: u64) -> Result<u64, MemoryError> {
        let mut buf = [0u8; 8];
        self.read_bytes(address, &mut buf)?;
        Ok(u64::from_le_bytes(buf))
    }

    /// Read a 64-bit integer ignoring region read permissions (e.g. loader/relocation phase).
    pub fn read_u64_bypass(&self, address: u64) -> Result<u64, MemoryError> {
        let mut buf = [0u8; 8];
        self.read_checked(address, &mut buf, None)?;
        Ok(u64::from_le_bytes(buf))
    }

    /// Write a 64-bit integer (little endian) to guest memory.
    pub fn write_u64(&mut self, address: u64, value: u64) -> Result<(), MemoryError> {
        self.write_bytes(address, &value.to_le_bytes())
    }

    /// Write a 64-bit integer ignoring region write permissions (e.g. loader/relocation phase).
    pub fn write_u64_bypass(&mut self, address: u64, value: u64) -> Result<(), MemoryError> {
        self.write_checked(address, &value.to_le_bytes(), None)
    }

    /// Read a 32-bit integer (little endian) from guest memory.
    pub fn read_u32(&self, address: u64) -> Result<u32, MemoryError> {
        let mut buf = [0u8; 4];
        self.read_bytes(address, &mut buf)?;
        Ok(u32::from_le_bytes(buf))
    }

    /// Write a 32-bit integer (little endian) to guest memory.
    pub fn write_u32(&mut self, address: u64, value: u32) -> Result<(), MemoryError> {
        self.write_bytes(address, &value.to_le_bytes())
    }

    /// Write a 32-bit integer ignoring region write permissions (e.g. loader/relocation phase).
    pub fn write_u32_bypass(&mut self, address: u64, value: u32) -> Result<(), MemoryError> {
        self.write_checked(address, &value.to_le_bytes(), None)
    }

    /// Read a single byte from guest memory.
    pub fn read_u8(&self, address: u64) -> Result<u8, MemoryError> {
        let mut buf = [0u8; 1];
        self.read_bytes(address, &mut buf)?;
        Ok(buf[0])
    }

    /// Write a single byte to guest memory.
    pub fn write_u8(&mut self, address: u64, value: u8) -> Result<(), MemoryError> {
        self.write_bytes(address, &[value])
    }

    /// Read a 16-bit little-endian integer from guest memory.
    pub fn read_u16(&self, address: u64) -> Result<u16, MemoryError> {
        let mut buf = [0u8; 2];
        self.read_bytes(address, &mut buf)?;
        Ok(u16::from_le_bytes(buf))
    }

    /// Write a 16-bit little-endian integer to guest memory.
    pub fn write_u16(&mut self, address: u64, value: u16) -> Result<(), MemoryError> {
        self.write_bytes(address, &value.to_le_bytes())
    }
}

#[cfg(test)]
mod mapping_identity_tests {
    use super::*;

    #[test]
    fn identities_span_regions_and_ignore_permissions() {
        let mut memory = GuestMemory::new();
        memory
            .map_region(0x1004, 4, MemoryPermissions::NO_ACCESS)
            .unwrap();
        memory
            .map_region(0x1000, 4, MemoryPermissions::READ_WRITE)
            .unwrap();
        let first = memory.mapping_identity(0x1000, 4).unwrap();
        let second = memory.mapping_identity(0x1004, 4).unwrap();
        assert_eq!(first.len(), 1);
        assert_eq!(second.len(), 1);
        assert_ne!(first, second);
        assert_eq!(
            memory.mapping_identity(0x1002, 4).unwrap(),
            vec![first[0], second[0]]
        );
        memory.regions[0].permissions = MemoryPermissions::NO_ACCESS;
        memory.regions[1].permissions = MemoryPermissions::READ_WRITE_EXECUTE;
        assert_eq!(
            memory.mapping_identity(0x1000, 8).unwrap(),
            vec![first[0], second[0]]
        );
        memory.write_bytes(0x1004, &[1, 2, 3, 4]).unwrap();
        assert_eq!(memory.mapping_identity(0x1004, 4).unwrap(), second);
    }

    #[test]
    fn remapping_changes_only_the_replaced_identity() {
        let mut memory = GuestMemory::new();
        memory
            .map_virtual_allocation(0x1000, 4, MemoryPermissions::READ_WRITE)
            .unwrap();
        memory
            .map_region(0x1004, 4, MemoryPermissions::READ_WRITE)
            .unwrap();
        let before = memory.mapping_identity(0x1000, 8).unwrap();
        assert!(memory.release_virtual_allocation(0x1000));
        assert_eq!(
            memory.mapping_identity(0x1000, 8),
            Err(MemoryError::UnmappedAddress(0x1000))
        );
        memory
            .map_virtual_allocation(0x1000, 4, MemoryPermissions::READ_WRITE)
            .unwrap();
        let after = memory.mapping_identity(0x1000, 8).unwrap();
        assert_ne!(before[0], after[0]);
        assert_eq!(before[1], after[1]);
    }

    #[test]
    fn clone_preserves_contents_and_metadata_but_has_disjoint_identities() {
        let mut memory = GuestMemory::new();
        memory
            .map_virtual_allocation(0x1000, 4, MemoryPermissions::READ_WRITE)
            .unwrap();
        memory.write_bytes(0x1000, &[1, 2, 3, 4]).unwrap();
        memory
            .map_region(0x1004, 4, MemoryPermissions::READ_EXECUTE)
            .unwrap();
        let original = memory.mapping_identity(0x1000, 8).unwrap();
        let mut cloned = memory.clone();
        let identities = cloned.mapping_identity(0x1000, 8).unwrap();
        assert!(identities.iter().all(|id| !original.contains(id)));
        for (a, b) in memory.regions.iter().zip(&cloned.regions) {
            assert_eq!(a.base, b.base);
            assert_eq!(a.data, b.data);
            assert_eq!(a.permissions, b.permissions);
            assert_eq!(a.virtual_allocation, b.virtual_allocation);
        }
        cloned.write_u8(0x1000, 9).unwrap();
        assert_eq!(memory.read_u8(0x1000), Ok(1));
        assert!(cloned.release_virtual_allocation(0x1000));
        assert_eq!(memory.mapping_identity(0x1000, 8).unwrap(), original);
        cloned.clone_from(&memory);
        assert!(
            cloned
                .mapping_identity(0x1000, 8)
                .unwrap()
                .iter()
                .all(|id| !original.contains(id) && !identities.contains(id))
        );
        let mut independent = GuestMemory::new();
        independent
            .map_region(0x1000, 8, MemoryPermissions::READ_WRITE)
            .unwrap();
        assert!(!original.contains(&independent.mapping_identity(0x1000, 8).unwrap()[0]));
    }

    #[test]
    fn invalid_ranges_and_failed_accesses_leave_bytes_and_identities_unchanged() {
        let mut memory = GuestMemory::new();
        memory
            .map_region_with_data(0x1000, 4, MemoryPermissions::READ_WRITE, &[1, 2, 3, 4])
            .unwrap();
        memory
            .map_region_with_data(0x1004, 4, MemoryPermissions::NO_ACCESS, &[5, 6, 7, 8])
            .unwrap();
        memory
            .map_region(0x1009, 1, MemoryPermissions::READ_WRITE)
            .unwrap();
        let identities = memory.mapping_identity(0x1000, 8).unwrap();
        let bytes: Vec<_> = memory.regions.iter().map(|r| r.data.clone()).collect();
        assert_eq!(
            memory.mapping_identity(0x1000, 10),
            Err(MemoryError::UnmappedAddress(0x1000))
        );
        assert_eq!(
            memory.mapping_identity(u64::MAX, 2),
            Err(MemoryError::Overflow)
        );
        assert_eq!(
            memory.mapping_identity(0x2000, 0),
            Err(MemoryError::UnmappedAddress(0x2000))
        );
        assert_eq!(memory.mapping_identity(0x1000, 0), Ok(vec![]));
        assert_eq!(memory.mapping_identity(0x1008, 0), Ok(vec![]));
        assert!(
            memory
                .map_region(0x1000, 4, MemoryPermissions::READ_WRITE)
                .is_err()
        );
        assert!(memory.write_bytes(0x1002, &[9; 4]).is_err());
        assert!(memory.write_checked(0x1002, &[9; 8], None).is_err());
        let mut dst = [0xaa; 8];
        assert!(memory.read_bytes(0x1000, &mut dst).is_err());
        assert_eq!(dst, [0xaa; 8]);
        assert!(memory.read_checked(0x1002, &mut dst, None).is_err());
        assert_eq!(dst, [0xaa; 8]);
        assert_eq!(
            memory
                .regions
                .iter()
                .map(|r| r.data.clone())
                .collect::<Vec<_>>(),
            bytes
        );
        assert_eq!(memory.mapping_identity(0x1000, 8).unwrap(), identities);
    }
}
