// SPDX-License-Identifier: GPL-3.0-or-later
//! Single-triangle shader storage selections inside an atomic root transaction.
use super::*;

pub(super) struct StorageDrawValidation<'a> {
    pub plan: Option<&'a AttachmentPlan>,
    pub view: Option<GraphicsHandle>,
    pub pipeline: Option<GraphicsHandle>,
    pub count: u32,
    pub previous_draws: usize,
    pub viewport: (usize, usize),
    pub state: GraphicsDrawState,
    pub depth_active: bool,
    pub sampled: &'a [Option<GraphicsHandle>; 2],
    pub states: &'a HashMap<GraphicsHandle, ResourceStates>,
    pub colors: &'a [GraphicsHandle],
}
impl GraphicsContext {
    /// Opt in only after installing queued attachment transactions with storage.
    pub fn enable_shader_storage_views(&mut self) -> Result<(), GraphicsError> {
        if self.queued_backend.is_none() || !self.attachment_transactions {
            return Err(GraphicsError::NativeMetalRequired);
        }
        self.shader_storage_views = true;
        Ok(())
    }
    pub fn create_storage_view(
        &mut self,
        device: GraphicsHandle,
        root: GraphicsHandle,
        mip: u32,
        slice: u32,
    ) -> Result<GraphicsHandle, GraphicsError> {
        if !self.devices.contains_key(&device) {
            return Err(GraphicsError::InvalidDeviceHandle);
        }
        if self.guest_object_count() >= crate::graphics_service::MAX_GUEST_OBJECTS {
            return Err(GraphicsError::ResourceLifetimeViolation);
        }
        let desc = self.sampled_resource_desc(root)?;
        if !matches!(
            desc.format,
            TextureFormat::Rgba8Unorm | TextureFormat::R32Uint
        ) {
            return Err(GraphicsError::InvalidDescriptor);
        }
        let selection = GraphicsWritableSelection {
            root,
            desc,
            mip,
            slice,
        };
        selection.extent()?;
        let handle = self.allocate_handle();
        self.storage_views.insert(handle, selection);
        Ok(handle)
    }
    pub fn destroy_storage_view(&mut self, handle: GraphicsHandle) -> Result<(), GraphicsError> {
        if !self.storage_views.contains_key(&handle) {
            return Err(GraphicsError::InvalidResourceHandle);
        }
        if self.command_lists.values().any(|list| {
            list.commands
                .iter()
                .any(|cmd| matches!(cmd, GraphicsCommand::SetStorageView(Some(h)) if *h == handle))
        }) {
            return Err(GraphicsError::ResourceLifetimeViolation);
        }
        let selection = self
            .storage_views
            .remove(&handle)
            .expect("validated storage view");
        if self.is_pinned(handle) {
            self.retired_storage_views.insert(handle, selection);
        }
        Ok(())
    }
    pub(super) fn storage_selection(
        &self,
        handle: GraphicsHandle,
    ) -> Result<GraphicsWritableSelection, GraphicsError> {
        self.storage_views
            .get(&handle)
            .copied()
            .ok_or(GraphicsError::InvalidResourceHandle)
    }
    pub fn cmd_set_storage_view(
        &mut self,
        list: GraphicsHandle,
        view: Option<GraphicsHandle>,
    ) -> Result<(), GraphicsError> {
        if let Some(handle) = view {
            self.storage_selection(handle)?;
        }
        let list = self
            .command_lists
            .get_mut(&list)
            .ok_or(GraphicsError::InvalidCommandListHandle)?;
        if !list.is_recording {
            return Err(GraphicsError::CommandListNotRecording);
        }
        list.commands.push(GraphicsCommand::SetStorageView(view));
        Ok(())
    }
    pub(super) fn validate_storage_draw(
        &self,
        draw: StorageDrawValidation<'_>,
    ) -> Result<(), GraphicsError> {
        let pipeline = draw
            .pipeline
            .map(|handle| {
                self.pipelines
                    .get(&handle)
                    .ok_or(GraphicsError::InvalidResourceHandle)
            })
            .transpose()?;
        let required = pipeline.and_then(|p| p.storage);
        if required.is_none() && draw.view.is_none() {
            return Ok(());
        }
        if !self.shader_storage_views {
            return Err(GraphicsError::NativeMetalRequired);
        }
        let required = required.ok_or(GraphicsError::InvalidDescriptor)?;
        let selected =
            self.storage_selection(draw.view.ok_or(GraphicsError::InvalidDescriptor)?)?;
        let plan = draw.plan.ok_or(GraphicsError::InvalidDescriptor)?;
        let storage = plan
            .storage
            .as_ref()
            .ok_or(GraphicsError::InvalidDescriptor)?;
        let expected = match required {
            xenora_shader::GraphicsStorageType::Float => TextureFormat::Rgba8Unorm,
            xenora_shader::GraphicsStorageType::Uint => TextureFormat::R32Uint,
        };
        if selected.desc.format != expected
            || selected.root != plan.roots[storage.root_index].root
            || selected.mip != storage.mip
            || selected.slice != storage.slice
            || draw.count != 3
            || draw.previous_draws != 0
            || draw.depth_active
            || draw.state.depth_compare.is_some()
            || draw.state.depth_write
            || draw.state.viewport_origin != [0, 0]
            || draw.viewport != plan.extent
            || draw.colors.contains(&selected.root)
            || draw.sampled[0].is_some()
            || draw.sampled.iter().flatten().any(|h| *h == selected.root)
            || pipeline.is_some_and(|p| p.sampled_pairs & 3 != 0)
        {
            return Err(GraphicsError::InvalidDescriptor);
        }
        let resource = self
            .resources
            .get(&selected.root)
            .ok_or(GraphicsError::InvalidResourceHandle)?;
        if draw
            .states
            .get(&selected.root)
            .copied()
            .unwrap_or(resource.state)
            != ResourceStates::UNORDERED_ACCESS
        {
            return Err(GraphicsError::ResourceLifetimeViolation);
        }
        Ok(())
    }
}
