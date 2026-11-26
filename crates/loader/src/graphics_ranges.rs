// SPDX-License-Identifier: GPL-3.0-or-later
//! Semantic ancestry is separate from execution ordering and root composition.
use super::*;

#[derive(Clone, Copy, Debug)]
pub(super) enum AccessRange {
    Whole,
    Texture {
        slice: u32,
        slices: u32,
        mip: u32,
        mips: u32,
    },
}
impl AccessRange {
    fn overlaps(self, other: Self) -> bool {
        match (self, other) {
            (
                Self::Texture {
                    slice: a,
                    slices: an,
                    mip: b,
                    mips: bn,
                },
                Self::Texture {
                    slice: c,
                    slices: cn,
                    mip: d,
                    mips: dn,
                },
            ) => {
                u64::from(a) < u64::from(c) + u64::from(cn)
                    && u64::from(c) < u64::from(a) + u64::from(an)
                    && u64::from(b) < u64::from(d) + u64::from(dn)
                    && u64::from(d) < u64::from(b) + u64::from(bn)
            }
            _ => true,
        }
    }
}
#[derive(Clone, Default)]
pub(super) struct RangeAccesses {
    pub reads: HashMap<GraphicsHandle, Vec<AccessRange>>,
    pub writes: HashMap<GraphicsHandle, Vec<AccessRange>>,
}
impl RangeAccesses {
    fn intersect(
        a: &HashMap<GraphicsHandle, Vec<AccessRange>>,
        b: &HashMap<GraphicsHandle, Vec<AccessRange>>,
    ) -> bool {
        a.iter().any(|(root, ranges)| {
            b.get(root)
                .is_some_and(|other| ranges.iter().any(|r| other.iter().any(|o| r.overlaps(*o))))
        })
    }
    pub fn conflicts(&self, other: &Self) -> bool {
        Self::intersect(&self.writes, &other.writes)
            || Self::intersect(&self.writes, &other.reads)
            || Self::intersect(&self.reads, &other.writes)
    }
    pub fn whole(reads: &HashSet<GraphicsHandle>, writes: &HashSet<GraphicsHandle>) -> Self {
        Self {
            reads: reads
                .iter()
                .map(|h| (*h, vec![AccessRange::Whole]))
                .collect(),
            writes: writes
                .iter()
                .map(|h| (*h, vec![AccessRange::Whole]))
                .collect(),
        }
    }
}
impl GraphicsContext {
    pub fn submit_command_list_after_composed(
        &mut self,
        queue: GraphicsHandle,
        list: GraphicsHandle,
        semantic_waits: &[GraphicsQueuePoint],
        composition_waits: &[GraphicsQueuePoint],
        memory: &GuestMemory,
    ) -> Result<GraphicsHandle, GraphicsError> {
        if !self.attachment_transactions {
            return Err(GraphicsError::NativeMetalRequired);
        }
        self.submit_command_list_after_internal(
            queue,
            list,
            semantic_waits,
            Some(composition_waits),
            memory,
        )
    }
    pub(super) fn semantic_closure(
        &self,
        roots: &[GraphicsHandle],
    ) -> Result<Vec<GraphicsHandle>, GraphicsError> {
        let mut seen = HashSet::new();
        let mut pending = roots.to_vec();
        while let Some(h) = pending.pop() {
            if !seen.insert(h) {
                continue;
            }
            if seen.len() > MAX_QUEUED_GRAPH_NODES {
                return Err(GraphicsError::InvalidDescriptor);
            }
            let node = self
                .queued
                .get(&h)
                .ok_or(GraphicsError::InvalidResourceHandle)?;
            for parent in &node.semantic_dependencies {
                let p = self
                    .queued
                    .get(parent)
                    .ok_or(GraphicsError::InvalidResourceHandle)?;
                if p.serial >= node.serial {
                    return Err(GraphicsError::InvalidDescriptor);
                }
                pending.push(*parent);
            }
        }
        let mut ordered: Vec<_> = seen.into_iter().collect();
        ordered.sort_by_key(|h| self.queued[h].serial);
        Ok(ordered)
    }
    pub(super) fn range_accesses(
        &self,
        list: GraphicsHandle,
        plan: Option<&AttachmentPlan>,
        reads: &HashSet<GraphicsHandle>,
        writes: &HashSet<GraphicsHandle>,
    ) -> RangeAccesses {
        let mut result = RangeAccesses::whole(reads, writes);
        let mut sampled = [None; 2];
        let mut sampled_ranges: HashMap<GraphicsHandle, Vec<AccessRange>> = HashMap::new();
        let mut non_sample_reads = HashSet::new();
        for command in &self.command_lists[&list].commands {
            match command {
                GraphicsCommand::SetSampledViews { views, .. } => {
                    sampled = views.map(|h| {
                        h.map(|h| {
                            let v = &self.texture_views[&h];
                            (
                                v.backing,
                                AccessRange::Texture {
                                    slice: v.slice.unwrap_or(0),
                                    slices: 1,
                                    mip: v.desc.base_mip,
                                    mips: v.desc.mip_count,
                                },
                            )
                        })
                    })
                }
                GraphicsCommand::SetSampledBindings { textures, .. } => {
                    sampled = textures.map(|h| h.map(|h| (h, AccessRange::Whole)))
                }
                GraphicsCommand::SetSampledTexture { texture, .. } => {
                    sampled[0] = texture.map(|h| (h, AccessRange::Whole))
                }
                GraphicsCommand::SetVertexBuffer { resource, .. }
                | GraphicsCommand::SetVertexColors { resource, .. }
                | GraphicsCommand::SetIndexBuffer { resource, .. } => {
                    non_sample_reads.insert(*resource);
                }
                GraphicsCommand::DrawInstanced {
                    vertex_count_per_instance,
                    instance_count,
                    ..
                } if *vertex_count_per_instance != 0 && *instance_count != 0 => {
                    for (h, r) in sampled.iter().flatten() {
                        sampled_ranges.entry(*h).or_default().push(*r);
                    }
                }
                GraphicsCommand::DrawIndexedInstanced {
                    index_count_per_instance,
                    instance_count,
                    ..
                } if *index_count_per_instance != 0 && *instance_count != 0 => {
                    for (h, r) in sampled.iter().flatten() {
                        sampled_ranges.entry(*h).or_default().push(*r);
                    }
                }
                _ => {}
            }
        }
        let sampled_roots: HashSet<_> = sampled_ranges.keys().copied().collect();
        for (h, ranges) in sampled_ranges {
            if !non_sample_reads.contains(&h) {
                result.reads.insert(h, ranges);
            }
        }
        if let Some(plan) = plan {
            for root in &plan.roots {
                result.writes.remove(&root.root);
                if !sampled_roots.contains(&root.root) && !non_sample_reads.contains(&root.root) {
                    result.reads.remove(&root.root);
                }
            }
            for color in &plan.colors {
                let root = plan.roots[color.root_index].root;
                let range = AccessRange::Texture {
                    slice: color.slice,
                    slices: 1,
                    mip: color.mip,
                    mips: 1,
                };
                result.writes.entry(root).or_default().push(range);
                if color.load {
                    result.reads.entry(root).or_default().push(range);
                }
            }
            if let Some(depth) = &plan.depth {
                let root = plan.roots[depth.root_index].root;
                let range = AccessRange::Texture {
                    slice: depth.slice,
                    slices: 1,
                    mip: depth.mip,
                    mips: 1,
                };
                result.writes.entry(root).or_default().push(range);
                if depth.load == GraphicsDepthLoad::Load {
                    result.reads.entry(root).or_default().push(range);
                }
            }
        }
        // Existing barriers define whole-root state, regardless of selected data.
        for command in &self.command_lists[&list].commands {
            if let GraphicsCommand::ResourceBarrier { resource, .. } = command {
                result.writes.insert(*resource, vec![AccessRange::Whole]);
            }
        }
        result
    }
    pub(super) fn composition_bases(
        &self,
        plan: &AttachmentPlan,
        semantic: &QueuedPreparation,
        composition: &[GraphicsHandle],
        ranges: &RangeAccesses,
        generation: u64,
    ) -> Result<HashMap<GraphicsHandle, GraphicsProducedVersion>, GraphicsError> {
        if composition.is_empty() {
            return Ok(HashMap::new());
        }
        // Execution closure is intentional here: composition inherits full roots,
        // but must never select shader/Load inputs through these edges.
        let closure = self.queued_dependency_closure(composition)?;
        let candidates = self.queued_preparation_from(&closure, generation, false)?;
        let mut bases = HashMap::new();
        for root in &plan.roots {
            let Some(base) = candidates.produced.get(&root.root) else {
                continue;
            };
            let original = semantic
                .produced
                .get(&root.root)
                .ok_or(GraphicsError::InvalidDescriptor)?;
            if candidates.layouts.get(&root.root) != Some(&root.desc.produced())
                || semantic.layouts.get(&root.root) != Some(&root.desc.produced())
            {
                return Err(GraphicsError::InvalidDescriptor);
            }
            if base == original {
                continue;
            }
            let owner = |v: &GraphicsProducedVersion| {
                self.queued
                    .iter()
                    .find(|(_, n)| n.outputs.get(&root.root) == Some(v))
                    .map(|(h, _)| *h)
                    .ok_or(GraphicsError::InvalidResourceHandle)
            };
            let base_owner = owner(base)?;
            let original_owner = owner(original)?;
            let base_ancestors = self.queued_dependency_closure(&[base_owner])?;
            if !base_ancestors.contains(&original_owner) {
                return Err(GraphicsError::ResourceLifetimeViolation);
            }
            let original_ancestors = self.queued_dependency_closure(&[original_owner])?;
            for h in base_ancestors
                .iter()
                .filter(|h| !original_ancestors.contains(h))
            {
                let node = &self.queued[h];
                if node.generation != generation
                    || node.receipt.status == GraphicsSubmissionStatus::Failed
                {
                    return Err(GraphicsError::ResourceLifetimeViolation);
                }
                if let Some(changes) = node.ranges.writes.get(&root.root)
                    && ranges
                        .reads
                        .get(&root.root)
                        .into_iter()
                        .chain(ranges.writes.get(&root.root))
                        .flatten()
                        .any(|r| changes.iter().any(|c| r.overlaps(*c)))
                {
                    return Err(GraphicsError::ResourceLifetimeViolation);
                }
                if node
                    .state_definitions
                    .get(&root.root)
                    .is_some_and(|s| semantic.states.get(&root.root) != Some(s))
                {
                    return Err(GraphicsError::ResourceLifetimeViolation);
                }
            }
            bases.insert(root.root, *base);
        }
        if bases.is_empty() {
            return Err(GraphicsError::InvalidDescriptor);
        }
        Ok(bases)
    }
}

/// Survives receipt pruning; only live backing identities occupy this map.
#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub(super) struct RootPublication {
    pub host_epoch: u64,
    pub version: Option<(u64, GraphicsProducedVersion)>,
}
impl GraphicsContext {
    pub(super) fn publication(&self, root: GraphicsHandle) -> RootPublication {
        self.root_publications
            .get(&root)
            .copied()
            .unwrap_or_default()
    }
    pub(super) fn publication_bases(
        &self,
        roots: impl IntoIterator<Item = GraphicsHandle>,
    ) -> HashMap<GraphicsHandle, RootPublication> {
        roots
            .into_iter()
            .map(|h| (h, self.publication(h)))
            .collect()
    }
    pub(super) fn validate_published_base(
        &self,
        root: GraphicsHandle,
        chosen: GraphicsProducedVersion,
        generation: u64,
    ) -> Result<(), GraphicsError> {
        let current = self.publication(root);
        let Some((published_generation, published)) = current.version else {
            return Ok(());
        };
        if published_generation == generation && chosen == published {
            return Ok(());
        }
        let owner = self
            .queued
            .iter()
            .find(|(_, n)| n.outputs.get(&root) == Some(&chosen))
            .map(|(h, _)| *h)
            .ok_or(GraphicsError::InvalidResourceHandle)?;
        let node = &self.queued[&owner];
        if node.generation != generation
            || node
                .publication_bases
                .get(&root)
                .is_none_or(|base| base.host_epoch != current.host_epoch)
        {
            return Err(GraphicsError::ResourceLifetimeViolation);
        }
        let ancestors = self.queued_dependency_closure(&[owner])?;
        if !ancestors.iter().any(|h| {
            let ancestor = &self.queued[h];
            ancestor.generation == generation
                && ((published_generation == generation
                    && ancestor.outputs.get(&root) == Some(&published))
                    // A fresh host-backed version may start a new generation
                    // from exactly the last published backing, without retaining
                    // its old native point. This is not a latest-version lookup.
                    || (ancestor.outputs.contains_key(&root)
                        && ancestor.publication_bases.get(&root) == Some(&current)))
        }) {
            return Err(GraphicsError::ResourceLifetimeViolation);
        }
        Ok(())
    }
    pub(super) fn validate_publication_commit(
        &self,
        handle: GraphicsHandle,
    ) -> Result<(), GraphicsError> {
        let node = &self.queued[&handle];
        let ancestors = self.queued_dependency_closure(&node.dependencies)?;
        for (root, admitted) in &node.publication_bases {
            // Retired outputs remain valid for descendants but are not republished.
            if !self.resources.contains_key(root) && !self.depth_resources.contains_key(root) {
                continue;
            }
            let current = self.publication(*root);
            if current.host_epoch != admitted.host_epoch {
                return Err(GraphicsError::ResourceLifetimeViolation);
            }
            if current == *admitted {
                continue;
            }
            let Some((generation, version)) = current.version else {
                return Err(GraphicsError::ResourceLifetimeViolation);
            };
            if generation != node.generation
                || !ancestors
                    .iter()
                    .any(|h| self.queued[h].outputs.get(root) == Some(&version))
            {
                return Err(GraphicsError::ResourceLifetimeViolation);
            }
        }
        Ok(())
    }
}
