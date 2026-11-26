// SPDX-License-Identifier: GPL-3.0-or-later
//! Worker-owned native queues. Only immutable host messages cross threads.
use super::*;
use std::collections::HashMap;
use std::time::Duration;
use xenora_loader::graphics::{
    GraphicsCopyInput, GraphicsCopyLayout, GraphicsImageCopyLocation, GraphicsProducedOutput,
    GraphicsProducedVersion, GraphicsQueuePoint, GraphicsQueuedBufferRegionRequest,
    GraphicsQueuedColorInput, GraphicsQueuedCopyRequest, GraphicsQueuedDepthInput,
    GraphicsQueuedImageCopyRequest, GraphicsQueuedPendingWork, GraphicsQueuedSubmissionRequest,
    GraphicsQueuedWorkOutput, GraphicsQueuedWorkRequest, GraphicsTextureInput,
    MAX_QUEUED_GRAPH_NODES,
};
use xenora_metal::{
    ImageCopyLocation, OwnedCopyOutput, QueuedBufferRegionCopy, QueuedColorLoad, QueuedCopyInput,
    QueuedDepthInput, QueuedImageCopy, QueuedSampledBinding, RenderCohort, RenderColorOutput,
    RenderCopyOutput, RenderDepthOutput, RenderPoint, RenderQueue,
};
use xenora_texture::PackedTextureArrayLayout;

#[path = "queued_attachments.rs"]
mod attachments;
#[path = "queued_typed.rs"]
mod typed;
use typed::{loader_desc, native_desc};
use xenora_loader::graphics::{GraphicsTypedCopyCapabilities, GraphicsTypedDepthSnapshot};

type Completion = Result<GraphicsQueuedWorkOutput, GraphicsError>;

enum Message {
    Submit {
        request: Box<GraphicsQueuedWorkRequest>,
        committed: mpsc::SyncSender<Result<(), GraphicsError>>,
        completed: mpsc::Sender<Completion>,
    },
    Reject(GraphicsQueuePoint),
    Release(GraphicsQueuePoint),
}
struct Worker {
    requests: Option<mpsc::Sender<Message>>,
    thread: Mutex<Option<JoinHandle<()>>>,
}
impl Drop for Worker {
    fn drop(&mut self) {
        self.requests.take();
        if let Ok(thread) = self.thread.get_mut()
            && let Some(thread) = thread.take()
        {
            let _ = thread.join();
        }
    }
}
struct Ticket {
    point: GraphicsQueuePoint,
    completed: mpsc::Receiver<Completion>,
    worker: Arc<Worker>,
    terminal: bool,
}
fn disconnected() -> GraphicsError {
    GraphicsError::BackendError("queued native worker disconnected".into())
}
impl GraphicsQueuedPendingWork for Ticket {
    fn poll(&mut self) -> Option<Completion> {
        if self.terminal {
            return Some(Err(GraphicsError::InvalidDescriptor));
        }
        let result = match self.completed.try_recv() {
            Ok(result) => result,
            Err(mpsc::TryRecvError::Empty) => return None,
            Err(mpsc::TryRecvError::Disconnected) => Err(disconnected()),
        };
        self.terminal = true;
        Some(result)
    }
    fn wait(&mut self) -> Completion {
        if self.terminal {
            return Err(GraphicsError::InvalidDescriptor);
        }
        let result = self
            .completed
            .recv()
            .unwrap_or_else(|_| Err(disconnected()));
        self.terminal = true;
        result
    }
    fn reject_dependency(&mut self, producer: GraphicsQueuePoint) {
        if let Some(requests) = &self.worker.requests {
            let _ = requests.send(Message::Reject(producer));
        }
    }
}
impl Drop for Ticket {
    fn drop(&mut self) {
        if !self.terminal {
            let _ = self.wait();
        }
        if let Some(requests) = &self.worker.requests {
            let _ = requests.send(Message::Release(self.point));
        }
    }
}
struct Entry {
    transaction: Option<attachments::Transaction>,
    point: RenderPoint,
    outputs: Vec<RenderColorOutput>,
    typed_outputs: Vec<xenora_metal::RenderTypedTextureOutput>,
    depth_output: Option<RenderDepthOutput>,
    copy_output: Option<(GraphicsCopyLayout, RenderCopyOutput)>,
    completed: Option<mpsc::Sender<Completion>>,
}
struct State {
    #[cfg(test)]
    fail_after_attachment_patch: Option<usize>,
    gpu: NativeCompute,
    cohort: RenderCohort,
    entries: HashMap<GraphicsQueuePoint, Entry>,
    queues: HashMap<GraphicsHandle, RenderQueue>,
    generation: Option<u64>,
    poisoned: bool,
}
impl State {
    fn new() -> Result<Self, GraphicsError> {
        let gpu = NativeCompute::new().map_err(metal_error)?;
        let cohort = gpu.new_render_graph_cohort().map_err(metal_error)?;
        Ok(Self {
            #[cfg(test)]
            fail_after_attachment_patch: None,
            gpu,
            cohort,
            entries: HashMap::new(),
            queues: HashMap::new(),
            generation: None,
            poisoned: false,
        })
    }
    fn fail(&mut self, reason: GraphicsError) {
        self.poisoned = true;
        // Native drainage visits producers before consumers. It never supplies a
        // cancellation signal while the corresponding producer is still writing.
        let _ = self.cohort.abort_and_drain("queued submission rejected");
        for entry in self.entries.values_mut() {
            if let Some(completed) = entry.completed.take() {
                let _ = completed.send(Err(reason.clone()));
            }
        }
    }
    fn progress(&mut self) {
        let points: Vec<_> = self.entries.keys().copied().collect();
        for point in points {
            let entry = &self.entries[&point];
            if entry.completed.is_none() {
                continue;
            }
            match self.cohort.is_complete(&entry.point) {
                Ok(false) => {}
                Err(error) => {
                    self.fail(metal_error(error));
                    return;
                }
                Ok(true) => {
                    let result = if let Some(transaction) = &entry.transaction {
                        attachments::collect(&mut self.cohort, transaction)
                    } else if let Some((layout, _)) = &entry.copy_output {
                        let expected = *layout;
                        self.cohort
                            .collect_copy(&entry.point)
                            .map_err(metal_error)
                            .and_then(|output| {
                                let (layout, bytes) = match output {
                                    OwnedCopyOutput::TypedTexture { desc, bytes } => {
                                        (GraphicsCopyLayout::TypedTexture(loader_desc(desc)), bytes)
                                    }
                                    OwnedCopyOutput::Rgba8Array {
                                        extent,
                                        mip_levels,
                                        array_layers,
                                        pixels,
                                    } => (
                                        GraphicsCopyLayout::Rgba8Array {
                                            extent: (extent[0], extent[1]),
                                            mip_levels,
                                            array_layers,
                                        },
                                        pixels,
                                    ),
                                    OwnedCopyOutput::Rgba8MipChain {
                                        extent,
                                        mip_levels,
                                        pixels,
                                    } => (
                                        GraphicsCopyLayout::Rgba8MipChain {
                                            extent: (extent[0], extent[1]),
                                            mip_levels,
                                        },
                                        pixels,
                                    ),
                                    OwnedCopyOutput::Buffer(bytes) => (
                                        GraphicsCopyLayout::Buffer {
                                            byte_len: bytes.len(),
                                        },
                                        bytes,
                                    ),
                                    OwnedCopyOutput::Rgba8 { extent, pixels } => (
                                        GraphicsCopyLayout::Rgba8 {
                                            extent: (extent[0], extent[1]),
                                        },
                                        pixels,
                                    ),
                                };
                                let expected_bytes = match expected {
                                    GraphicsCopyLayout::TypedTexture(desc) => {
                                        PackedTextureArrayLayout::new(
                                            desc.format,
                                            desc.extent.0,
                                            desc.extent.1,
                                            desc.mip_levels,
                                            desc.array_layers.unwrap_or(1),
                                        )
                                        .ok()
                                        .map(|l| l.byte_len())
                                    }
                                    GraphicsCopyLayout::Rgba8Array {
                                        extent,
                                        mip_levels,
                                        array_layers,
                                    } => PackedTextureArrayLayout::new(
                                        TextureFormat::Rgba8Unorm,
                                        extent.0,
                                        extent.1,
                                        mip_levels,
                                        array_layers,
                                    )
                                    .ok()
                                    .map(|l| l.byte_len()),
                                    GraphicsCopyLayout::Rgba8MipChain { extent, mip_levels } => {
                                        PackedTextureLayout::new(
                                            TextureFormat::Rgba8Unorm,
                                            extent.0,
                                            extent.1,
                                            mip_levels,
                                        )
                                        .ok()
                                        .map(|l| l.byte_len())
                                    }
                                    GraphicsCopyLayout::Buffer { byte_len } => Some(byte_len),
                                    GraphicsCopyLayout::Rgba8 {
                                        extent: (width, height),
                                    } => width.checked_mul(height).and_then(|n| n.checked_mul(4)),
                                };
                                if layout != expected || expected_bytes != Some(bytes.len()) {
                                    return Err(GraphicsError::InvalidDescriptor);
                                }
                                Ok(GraphicsQueuedWorkOutput::Copy { layout, bytes })
                            })
                    } else {
                        self.cohort
                            .collect(&entry.point)
                            .map_err(metal_error)
                            .map(|output| {
                                GraphicsQueuedWorkOutput::Render(GraphicsPassOutput {
                                    colors: output.colors,
                                    color_layouts: Some(
                                        output.color_layouts.into_iter().map(loader_desc).collect(),
                                    ),
                                    depth: output.depth.map(|depth| {
                                        let e = entry
                                            .depth_output
                                            .as_ref()
                                            .expect("owned depth output")
                                            .extent();
                                        packed_depth_snapshot(depth, (e[0], e[1]))
                                    }),
                                })
                            })
                    };
                    match result {
                        Ok(output) => {
                            let completed = self
                                .entries
                                .get_mut(&point)
                                .unwrap()
                                .completed
                                .take()
                                .unwrap();
                            let _ = completed.send(Ok(output));
                        }
                        Err(error) => {
                            self.fail(error);
                            return;
                        }
                    }
                }
            }
        }
    }
    fn submit(
        &mut self,
        request: &GraphicsQueuedWorkRequest,
        completed: mpsc::Sender<Completion>,
    ) -> Result<(), GraphicsError> {
        match request {
            GraphicsQueuedWorkRequest::AttachmentTransaction(request) => {
                self.submit_attachments(request, completed)
            }
            GraphicsQueuedWorkRequest::Render(request) => self.submit_render(request, completed),
            GraphicsQueuedWorkRequest::ImageCopy(request) => {
                self.submit_image_copy(request, completed)
            }
            GraphicsQueuedWorkRequest::Copy(request) => self.submit_copy(request, completed),
            GraphicsQueuedWorkRequest::BufferRegion(request) => {
                self.submit_buffer_region(request, completed)
            }
        }
    }

    fn submit_copy(
        &mut self,
        request: &GraphicsQueuedCopyRequest,
        completed: mpsc::Sender<Completion>,
    ) -> Result<(), GraphicsError> {
        if self.entries.len() >= MAX_QUEUED_GRAPH_NODES || self.entries.contains_key(&request.point)
        {
            return Err(GraphicsError::ResourceLifetimeViolation);
        }
        if request.output.point != request.point
            || request.output.output != GraphicsProducedOutput::Copy(0)
            || request.waits.len() > MAX_QUEUED_GRAPH_NODES
        {
            return Err(GraphicsError::InvalidDescriptor);
        }
        if self.generation != Some(request.generation) {
            if !self.entries.is_empty() {
                return Err(GraphicsError::ResourceLifetimeViolation);
            }
            self.cohort.reinitialize().map_err(metal_error)?;
            self.queues.clear();
            self.generation = Some(request.generation);
            self.poisoned = false;
        }
        if self.poisoned {
            return Err(GraphicsError::ResourceLifetimeViolation);
        }
        let queue = if let Some(queue) = self.queues.get(&request.point.queue) {
            *queue
        } else {
            match self.queues.len() {
                0 => RenderQueue::First,
                1 => RenderQueue::Second,
                _ => return Err(GraphicsError::ResourceLifetimeViolation),
            }
        };
        let dependencies = request
            .waits
            .iter()
            .map(|point| {
                self.entries
                    .get(point)
                    .map(|entry| entry.point.clone())
                    .ok_or(GraphicsError::InvalidResourceHandle)
            })
            .collect::<Result<Vec<_>, _>>()?;
        let input = match (&request.layout, &request.source) {
            (
                GraphicsCopyLayout::TypedTexture(_)
                | GraphicsCopyLayout::Rgba8Array { .. }
                | GraphicsCopyLayout::Rgba8MipChain { .. },
                _,
            ) => {
                return Err(GraphicsError::InvalidDescriptor);
            }
            (GraphicsCopyLayout::Buffer { byte_len }, GraphicsCopyInput::Host(bytes)) => {
                if bytes.len() != *byte_len {
                    return Err(GraphicsError::InvalidDescriptor);
                }
                QueuedCopyInput::HostBuffer(bytes)
            }
            (
                GraphicsCopyLayout::Rgba8 {
                    extent: (width, height),
                },
                GraphicsCopyInput::Host(bytes),
            ) => {
                if width.checked_mul(*height).and_then(|n| n.checked_mul(4)) != Some(bytes.len()) {
                    return Err(GraphicsError::InvalidDescriptor);
                }
                QueuedCopyInput::HostRgba8 {
                    extent: [*width, *height],
                    pixels: bytes,
                }
            }
            (layout, GraphicsCopyInput::Produced(version)) => {
                if !request.waits.contains(&version.point) {
                    return Err(GraphicsError::InvalidDescriptor);
                }
                match layout {
                    GraphicsCopyLayout::TypedTexture(_)
                    | GraphicsCopyLayout::Rgba8Array { .. }
                    | GraphicsCopyLayout::Rgba8MipChain { .. } => {
                        return Err(GraphicsError::InvalidDescriptor);
                    }
                    GraphicsCopyLayout::Buffer { byte_len } => {
                        let entry = self
                            .entries
                            .get(&version.point)
                            .ok_or(GraphicsError::InvalidResourceHandle)?;
                        let Some((actual, RenderCopyOutput::Buffer(output))) = &entry.copy_output
                        else {
                            return Err(GraphicsError::InvalidDescriptor);
                        };
                        if version.output != GraphicsProducedOutput::Copy(0)
                            || actual != layout
                            || output.byte_len() != *byte_len
                        {
                            return Err(GraphicsError::InvalidDescriptor);
                        }
                        QueuedCopyInput::ProducedBuffer(output.clone())
                    }
                    GraphicsCopyLayout::Rgba8 {
                        extent: (width, height),
                    } => {
                        let output = produced_output(&self.entries, *version)?;
                        if output.is_array()
                            || output.extent() != [*width, *height]
                            || output.mip_levels() != 1
                        {
                            return Err(GraphicsError::InvalidDescriptor);
                        }
                        QueuedCopyInput::ProducedColor(output)
                    }
                }
            }
            (_, GraphicsCopyInput::HostDepth(_)) => return Err(GraphicsError::InvalidDescriptor),
        };
        let point = self
            .cohort
            .submit_copy(queue, input, &dependencies)
            .map_err(metal_error)?;
        let output = match self.cohort.copy_output(&point).map_err(metal_error) {
            Ok(output) => output,
            Err(error) => {
                self.fail(error.clone());
                return Err(error);
            }
        };
        self.queues.insert(request.point.queue, queue);
        self.entries.insert(
            request.point,
            Entry {
                transaction: None,
                point,
                outputs: Vec::new(),
                typed_outputs: Vec::new(),
                depth_output: None,
                copy_output: Some((request.layout, output)),
                completed: Some(completed),
            },
        );
        Ok(())
    }

    fn region_buffer_input<'a>(
        &self,
        input: &'a GraphicsCopyInput,
        byte_len: usize,
        waits: &[GraphicsQueuePoint],
    ) -> Result<QueuedCopyInput<'a>, GraphicsError> {
        match input {
            GraphicsCopyInput::Host(bytes) if bytes.len() == byte_len => {
                Ok(QueuedCopyInput::HostBuffer(bytes))
            }
            GraphicsCopyInput::Produced(version) => {
                if !waits.contains(&version.point)
                    || version.output != GraphicsProducedOutput::Copy(0)
                {
                    return Err(GraphicsError::InvalidDescriptor);
                }
                let entry = self
                    .entries
                    .get(&version.point)
                    .ok_or(GraphicsError::InvalidResourceHandle)?;
                let Some((layout, RenderCopyOutput::Buffer(buffer))) = &entry.copy_output else {
                    return Err(GraphicsError::InvalidDescriptor);
                };
                if *layout != (GraphicsCopyLayout::Buffer { byte_len })
                    || buffer.byte_len() != byte_len
                {
                    return Err(GraphicsError::InvalidDescriptor);
                }
                Ok(QueuedCopyInput::ProducedBuffer(buffer.clone()))
            }
            _ => Err(GraphicsError::InvalidDescriptor),
        }
    }

    fn submit_buffer_region(
        &mut self,
        request: &GraphicsQueuedBufferRegionRequest,
        completed: mpsc::Sender<Completion>,
    ) -> Result<(), GraphicsError> {
        if self.entries.len() >= MAX_QUEUED_GRAPH_NODES || self.entries.contains_key(&request.point)
        {
            return Err(GraphicsError::ResourceLifetimeViolation);
        }
        if request.output.point != request.point
            || request.output.output != GraphicsProducedOutput::Copy(0)
            || request.waits.len() > MAX_QUEUED_GRAPH_NODES
        {
            return Err(GraphicsError::InvalidDescriptor);
        }
        if self.generation != Some(request.generation) {
            if !self.entries.is_empty() {
                return Err(GraphicsError::ResourceLifetimeViolation);
            }
            self.cohort.reinitialize().map_err(metal_error)?;
            self.queues.clear();
            self.generation = Some(request.generation);
            self.poisoned = false;
        }
        if self.poisoned {
            return Err(GraphicsError::ResourceLifetimeViolation);
        }
        let queue = if let Some(queue) = self.queues.get(&request.point.queue) {
            *queue
        } else {
            match self.queues.len() {
                0 => RenderQueue::First,
                1 => RenderQueue::Second,
                _ => return Err(GraphicsError::ResourceLifetimeViolation),
            }
        };
        let dependencies = request
            .waits
            .iter()
            .map(|point| {
                self.entries
                    .get(point)
                    .map(|entry| entry.point.clone())
                    .ok_or(GraphicsError::InvalidResourceHandle)
            })
            .collect::<Result<Vec<_>, _>>()?;
        let source =
            self.region_buffer_input(&request.source, request.source_byte_len, &request.waits)?;
        let destination = self.region_buffer_input(
            &request.destination,
            request.destination_byte_len,
            &request.waits,
        )?;
        let point = self
            .cohort
            .submit_buffer_region_copy(
                queue,
                QueuedBufferRegionCopy {
                    source,
                    destination,
                    source_offset: request.source_offset,
                    destination_offset: request.destination_offset,
                    byte_len: request.byte_len,
                },
                &dependencies,
            )
            .map_err(metal_error)?;
        let output = match self.cohort.copy_output(&point).map_err(metal_error) {
            Ok(output) => output,
            Err(error) => {
                self.fail(error.clone());
                return Err(error);
            }
        };
        self.queues.insert(request.point.queue, queue);
        self.entries.insert(
            request.point,
            Entry {
                transaction: None,
                point,
                outputs: Vec::new(),
                typed_outputs: Vec::new(),
                depth_output: None,
                copy_output: Some((
                    GraphicsCopyLayout::Buffer {
                        byte_len: request.destination_byte_len,
                    },
                    output,
                )),
                completed: Some(completed),
            },
        );
        Ok(())
    }

    fn image_copy_input<'a>(
        &self,
        input: &'a GraphicsCopyInput,
        layout: GraphicsCopyLayout,
        waits: &[GraphicsQueuePoint],
    ) -> Result<QueuedCopyInput<'a>, GraphicsError> {
        match layout {
            GraphicsCopyLayout::TypedTexture(_) => Err(GraphicsError::InvalidDescriptor),
            GraphicsCopyLayout::Rgba8Array {
                extent: (width, height),
                mip_levels,
                array_layers,
            } => {
                let packed = PackedTextureArrayLayout::new(
                    TextureFormat::Rgba8Unorm,
                    width,
                    height,
                    mip_levels,
                    array_layers,
                )
                .map_err(|_| GraphicsError::InvalidDescriptor)?;
                match input {
                    GraphicsCopyInput::HostDepth(_) => Err(GraphicsError::InvalidDescriptor),
                    GraphicsCopyInput::Host(bytes) => {
                        if bytes.len() != packed.byte_len() {
                            return Err(GraphicsError::InvalidDescriptor);
                        }
                        Ok(QueuedCopyInput::HostRgba8Array {
                            extent: [width, height],
                            mip_levels,
                            array_layers,
                            pixels: bytes,
                        })
                    }
                    GraphicsCopyInput::Produced(version) => {
                        if !waits.contains(&version.point) {
                            return Err(GraphicsError::InvalidDescriptor);
                        }
                        let output = produced_output(&self.entries, *version)?;
                        if !output.is_array()
                            || output.extent() != [width, height]
                            || output.mip_levels() != mip_levels
                            || output.array_layers() != array_layers
                        {
                            return Err(GraphicsError::InvalidDescriptor);
                        }
                        Ok(QueuedCopyInput::ProducedColor(output))
                    }
                }
            }

            GraphicsCopyLayout::Rgba8MipChain {
                extent: (width, height),
                mip_levels,
            } => {
                if mip_levels <= 1 {
                    return Err(GraphicsError::InvalidDescriptor);
                }
                let packed =
                    PackedTextureLayout::new(TextureFormat::Rgba8Unorm, width, height, mip_levels)
                        .map_err(|_| GraphicsError::InvalidDescriptor)?;
                match input {
                    GraphicsCopyInput::HostDepth(_) => Err(GraphicsError::InvalidDescriptor),
                    GraphicsCopyInput::Host(bytes) => {
                        if bytes.len() != packed.byte_len() {
                            return Err(GraphicsError::InvalidDescriptor);
                        }
                        Ok(QueuedCopyInput::HostRgba8MipChain {
                            extent: [width, height],
                            mip_levels,
                            pixels: bytes,
                        })
                    }
                    GraphicsCopyInput::Produced(version) => {
                        if !waits.contains(&version.point) {
                            return Err(GraphicsError::InvalidDescriptor);
                        }
                        let output = produced_output(&self.entries, *version)?;
                        if output.is_array()
                            || output.extent() != [width, height]
                            || output.mip_levels() != mip_levels
                        {
                            return Err(GraphicsError::InvalidDescriptor);
                        }
                        Ok(QueuedCopyInput::ProducedColor(output))
                    }
                }
            }

            GraphicsCopyLayout::Buffer { byte_len } => {
                self.region_buffer_input(input, byte_len, waits)
            }
            GraphicsCopyLayout::Rgba8 {
                extent: (width, height),
            } => match input {
                GraphicsCopyInput::HostDepth(_) => Err(GraphicsError::InvalidDescriptor),
                GraphicsCopyInput::Host(bytes) => {
                    if width.checked_mul(height).and_then(|n| n.checked_mul(4)) != Some(bytes.len())
                    {
                        return Err(GraphicsError::InvalidDescriptor);
                    }
                    Ok(QueuedCopyInput::HostRgba8 {
                        extent: [width, height],
                        pixels: bytes,
                    })
                }
                GraphicsCopyInput::Produced(version) => {
                    if !waits.contains(&version.point) {
                        return Err(GraphicsError::InvalidDescriptor);
                    }
                    let output = produced_output(&self.entries, *version)?;
                    if output.is_array()
                        || output.extent() != [width, height]
                        || output.mip_levels() != 1
                    {
                        return Err(GraphicsError::InvalidDescriptor);
                    }
                    Ok(QueuedCopyInput::ProducedColor(output))
                }
            },
        }
    }

    fn submit_image_copy(
        &mut self,
        request: &GraphicsQueuedImageCopyRequest,
        completed: mpsc::Sender<Completion>,
    ) -> Result<(), GraphicsError> {
        if self.entries.len() >= MAX_QUEUED_GRAPH_NODES || self.entries.contains_key(&request.point)
        {
            return Err(GraphicsError::ResourceLifetimeViolation);
        }
        if request.output.point != request.point
            || request.output.output != GraphicsProducedOutput::Copy(0)
            || request.waits.len() > MAX_QUEUED_GRAPH_NODES
        {
            return Err(GraphicsError::InvalidDescriptor);
        }
        if self.generation != Some(request.generation) {
            if !self.entries.is_empty() {
                return Err(GraphicsError::ResourceLifetimeViolation);
            }
            self.cohort.reinitialize().map_err(metal_error)?;
            self.queues.clear();
            self.generation = Some(request.generation);
            self.poisoned = false;
        }
        if self.poisoned {
            return Err(GraphicsError::ResourceLifetimeViolation);
        }
        let queue = if let Some(queue) = self.queues.get(&request.point.queue) {
            *queue
        } else {
            match self.queues.len() {
                0 => RenderQueue::First,
                1 => RenderQueue::Second,
                _ => return Err(GraphicsError::ResourceLifetimeViolation),
            }
        };
        let dependencies = request
            .waits
            .iter()
            .map(|point| {
                self.entries
                    .get(point)
                    .map(|entry| entry.point.clone())
                    .ok_or(GraphicsError::InvalidResourceHandle)
            })
            .collect::<Result<Vec<_>, _>>()?;
        let typed = matches!(
            request.source_location,
            GraphicsImageCopyLocation::Texture { .. }
        ) || matches!(
            request.destination_location,
            GraphicsImageCopyLocation::Texture { .. }
        );
        let point = if typed {
            self.submit_native_typed_copy(queue, request, &dependencies)?
        } else {
            let source =
                self.image_copy_input(&request.source, request.source_layout, &request.waits)?;
            let destination = self.image_copy_input(
                &request.destination,
                request.destination_layout,
                &request.waits,
            )?;
            let mip_copy = matches!(
                request.source_location,
                GraphicsImageCopyLocation::Rgba8Mip { .. }
            ) || matches!(
                request.destination_location,
                GraphicsImageCopyLocation::Rgba8Mip { .. }
            );
            let location = |input| match input {
                GraphicsImageCopyLocation::Texture { .. } => unreachable!("typed path separated"),
                GraphicsImageCopyLocation::Rgba8Mip {
                    mip,
                    slice,
                    origin: (x, y),
                } => ImageCopyLocation::Rgba8Mip {
                    mip,
                    slice,
                    origin: [x, y],
                },
                GraphicsImageCopyLocation::Rgba8 { origin: (x, y) } => {
                    ImageCopyLocation::Rgba8 { origin: [x, y] }
                }
                GraphicsImageCopyLocation::Buffer {
                    offset,
                    bytes_per_row,
                } => ImageCopyLocation::Buffer {
                    offset,
                    bytes_per_row,
                },
            };
            let copy = QueuedImageCopy {
                source,
                destination,
                source_location: location(request.source_location),
                destination_location: location(request.destination_location),
                extent: [request.extent.0, request.extent.1],
            };
            let array_copy = matches!(request.source_layout, GraphicsCopyLayout::Rgba8Array { .. })
                || matches!(
                    request.destination_layout,
                    GraphicsCopyLayout::Rgba8Array { .. }
                );
            if array_copy {
                self.cohort
                    .submit_array_image_copy(queue, copy, &dependencies)
            } else if mip_copy {
                self.cohort
                    .submit_mip_image_copy(queue, copy, &dependencies)
            } else {
                self.cohort.submit_image_copy(queue, copy, &dependencies)
            }
            .map_err(metal_error)?
        };
        let output = match self.cohort.copy_output(&point).map_err(metal_error) {
            Ok(output) => output,
            Err(error) => {
                self.fail(error.clone());
                return Err(error);
            }
        };
        self.queues.insert(request.point.queue, queue);
        self.entries.insert(
            request.point,
            Entry {
                transaction: None,
                point,
                outputs: Vec::new(),
                typed_outputs: Vec::new(),
                depth_output: None,
                copy_output: Some((request.destination_layout, output)),
                completed: Some(completed),
            },
        );
        Ok(())
    }

    fn submit_render(
        &mut self,
        request: &GraphicsQueuedSubmissionRequest,
        completed: mpsc::Sender<Completion>,
    ) -> Result<(), GraphicsError> {
        if self.entries.len() >= MAX_QUEUED_GRAPH_NODES || self.entries.contains_key(&request.point)
        {
            return Err(GraphicsError::ResourceLifetimeViolation);
        }
        if self.generation != Some(request.generation) {
            if !self.entries.is_empty() {
                return Err(GraphicsError::ResourceLifetimeViolation);
            }
            self.cohort.reinitialize().map_err(metal_error)?;
            self.queues.clear();
            self.generation = Some(request.generation);
            self.poisoned = false;
        }
        if self.poisoned {
            return Err(GraphicsError::ResourceLifetimeViolation);
        }
        if request.waits.len() > MAX_QUEUED_GRAPH_NODES
            || request.outputs.len() != request.colors.len()
            || request.outputs.iter().enumerate().any(|(index, output)| {
                output.point != request.point
                    || output.output != GraphicsProducedOutput::Color(index as u8)
            })
        {
            return Err(GraphicsError::InvalidDescriptor);
        }
        if request.depth_output
            != request.depth.as_ref().map(|_| GraphicsProducedVersion {
                point: request.point,
                output: GraphicsProducedOutput::Depth,
            })
        {
            return Err(GraphicsError::InvalidDescriptor);
        }
        let queue = if let Some(queue) = self.queues.get(&request.point.queue) {
            *queue
        } else {
            match self.queues.len() {
                0 => RenderQueue::First,
                1 => RenderQueue::Second,
                _ => return Err(GraphicsError::ResourceLifetimeViolation),
            }
        };
        let dependencies = request
            .waits
            .iter()
            .map(|point| {
                self.entries
                    .get(point)
                    .map(|entry| entry.point.clone())
                    .ok_or(GraphicsError::InvalidResourceHandle)
            })
            .collect::<Result<Vec<_>, _>>()?;
        let point = submit_native(
            &self.gpu,
            &mut self.cohort,
            queue,
            request,
            &self.entries,
            &dependencies,
        )?;
        // Capture output capabilities before collection; they retain producer
        // provenance and cannot expose CPU mutation or raw native texture handles.
        let typed_outputs = (0..request.outputs.len())
            .map(|slot| {
                self.cohort
                    .typed_color_output(&point, slot)
                    .map_err(metal_error)
            })
            .collect::<Result<Vec<_>, _>>();
        let typed_outputs = match typed_outputs {
            Ok(v) => v,
            Err(e) => {
                self.fail(e.clone());
                return Err(e);
            }
        };
        let outputs = if typed_outputs
            .iter()
            .all(|o| o.desc().format == TextureFormat::Rgba8Unorm)
        {
            typed_outputs
                .iter()
                .map(|o| o.rgba8_output().map_err(metal_error))
                .collect::<Result<Vec<_>, _>>()?
        } else {
            Vec::new()
        };
        let depth_output = if request.depth_output.is_some() {
            match self.cohort.depth_output(&point).map_err(metal_error) {
                Ok(output) => Some(output),
                Err(error) => {
                    self.fail(error.clone());
                    return Err(error);
                }
            }
        } else {
            None
        };
        self.queues.insert(request.point.queue, queue);
        self.entries.insert(
            request.point,
            Entry {
                transaction: None,
                point,
                outputs,
                typed_outputs,
                depth_output,
                copy_output: None,
                completed: Some(completed),
            },
        );
        Ok(())
    }
}

fn produced_output(
    entries: &HashMap<GraphicsQueuePoint, Entry>,
    version: GraphicsProducedVersion,
) -> Result<RenderColorOutput, GraphicsError> {
    let entry = entries
        .get(&version.point)
        .ok_or(GraphicsError::InvalidResourceHandle)?;
    match version.output {
        GraphicsProducedOutput::AttachmentRoot(_) => produced_typed_output(entries, version)?
            .rgba8_output()
            .map_err(metal_error),
        GraphicsProducedOutput::Color(slot) => entry
            .outputs
            .get(usize::from(slot))
            .cloned()
            .map(Ok)
            .unwrap_or_else(|| {
                entry
                    .typed_outputs
                    .get(usize::from(slot))
                    .ok_or(GraphicsError::InvalidResourceHandle)?
                    .rgba8_output()
                    .map_err(metal_error)
            }),
        GraphicsProducedOutput::Copy(0) => match &entry.copy_output {
            Some((
                GraphicsCopyLayout::TypedTexture(desc),
                RenderCopyOutput::TypedTexture(output),
            )) if native_desc(*desc) == output.desc() => output.rgba8_output().map_err(metal_error),
            Some((
                GraphicsCopyLayout::Rgba8Array {
                    extent,
                    mip_levels,
                    array_layers,
                },
                RenderCopyOutput::Color(output),
            )) if output.is_array()
                && output.extent() == [extent.0, extent.1]
                && output.mip_levels() == *mip_levels
                && output.array_layers() == *array_layers =>
            {
                Ok(output.clone())
            }
            Some((
                GraphicsCopyLayout::Rgba8MipChain { extent, mip_levels },
                RenderCopyOutput::Color(output),
            )) if !output.is_array()
                && *mip_levels > 1
                && output.extent() == [extent.0, extent.1]
                && output.mip_levels() == *mip_levels =>
            {
                Ok(output.clone())
            }
            Some((GraphicsCopyLayout::Rgba8 { extent }, RenderCopyOutput::Color(output)))
                if !output.is_array()
                    && output.extent() == [extent.0, extent.1]
                    && output.mip_levels() == 1 =>
            {
                Ok(output.clone())
            }
            _ => Err(GraphicsError::InvalidDescriptor),
        },
        _ => Err(GraphicsError::InvalidDescriptor),
    }
}

fn produced_depth(
    entries: &HashMap<GraphicsQueuePoint, Entry>,
    version: GraphicsProducedVersion,
) -> Result<RenderDepthOutput, GraphicsError> {
    if matches!(version.output, GraphicsProducedOutput::AttachmentRoot(_)) {
        return produced_typed_output(entries, version)?
            .depth_output()
            .map_err(metal_error);
    }
    if version.output == GraphicsProducedOutput::Copy(0) {
        if let Some((
            GraphicsCopyLayout::TypedTexture(desc),
            RenderCopyOutput::TypedTexture(output),
        )) = entries
            .get(&version.point)
            .and_then(|e| e.copy_output.as_ref())
            && native_desc(*desc) == output.desc()
        {
            return output.depth_output().map_err(metal_error);
        }
        return Err(GraphicsError::InvalidDescriptor);
    }
    if version.output != GraphicsProducedOutput::Depth {
        return Err(GraphicsError::InvalidDescriptor);
    }
    entries
        .get(&version.point)
        .and_then(|entry| entry.depth_output.clone())
        .ok_or(GraphicsError::InvalidResourceHandle)
}

fn geometry_attribute<'a>(
    input: &'a xenora_loader::graphics::GraphicsAttributeInput,
    request: &GraphicsQueuedSubmissionRequest,
    entries: &HashMap<GraphicsQueuePoint, Entry>,
) -> Result<xenora_metal::GeometryAttribute<'a>, GraphicsError> {
    Ok(xenora_metal::GeometryAttribute {
        buffer: geometry_buffer(&input.buffer, request, entries)?,
        offset: input.offset,
        stride: input.stride,
    })
}

fn geometry_buffer<'a>(
    buffer: &'a xenora_loader::graphics::GraphicsGeometryBuffer,
    request: &GraphicsQueuedSubmissionRequest,
    entries: &HashMap<GraphicsQueuePoint, Entry>,
) -> Result<xenora_metal::GeometryBuffer<'a>, GraphicsError> {
    match &buffer.source {
        GraphicsCopyInput::Host(bytes) if bytes.len() == buffer.byte_len => {
            Ok(xenora_metal::GeometryBuffer::Host(bytes))
        }
        GraphicsCopyInput::Produced(version) => {
            if version.output != GraphicsProducedOutput::Copy(0)
                || !request.waits.contains(&version.point)
            {
                return Err(GraphicsError::InvalidDescriptor);
            }
            let entry = entries
                .get(&version.point)
                .ok_or(GraphicsError::InvalidResourceHandle)?;
            let Some((GraphicsCopyLayout::Buffer { byte_len }, RenderCopyOutput::Buffer(output))) =
                &entry.copy_output
            else {
                return Err(GraphicsError::InvalidDescriptor);
            };
            if *byte_len != buffer.byte_len || output.byte_len() != buffer.byte_len {
                return Err(GraphicsError::InvalidDescriptor);
            }
            Ok(xenora_metal::GeometryBuffer::Produced(output.clone()))
        }
        _ => Err(GraphicsError::InvalidDescriptor),
    }
}

fn submit_native(
    gpu: &NativeCompute,
    cohort: &mut RenderCohort,
    queue: RenderQueue,
    request: &GraphicsQueuedSubmissionRequest,
    entries: &HashMap<GraphicsQueuePoint, Entry>,
    dependencies: &[RenderPoint],
) -> Result<RenderPoint, GraphicsError> {
    submit_native_inputs(gpu, cohort, queue, request, entries, dependencies, None)
}
struct AttachmentInputs<'a> {
    colors: Vec<QueuedColorLoad<'a>>,
    depth: Option<QueuedDepthInput>,
}
fn submit_native_inputs(
    gpu: &NativeCompute,
    cohort: &mut RenderCohort,
    queue: RenderQueue,
    request: &GraphicsQueuedSubmissionRequest,
    entries: &HashMap<GraphicsQueuePoint, Entry>,
    dependencies: &[RenderPoint],
    overrides: Option<AttachmentInputs<'_>>,
) -> Result<RenderPoint, GraphicsError> {
    if request.color_types.len() != request.colors.len() {
        return Err(GraphicsError::InvalidDescriptor);
    }
    let color_formats: Vec<_> = request
        .color_types
        .iter()
        .map(|t| t.map_or(TextureFormat::Rgba8Unorm, |t| t.format))
        .collect();
    let shaders = request
        .draws
        .iter()
        .map(|draw| {
            let pipeline = draw
                .pipeline
                .as_ref()
                .ok_or(GraphicsError::InvalidDescriptor)?;
            let texture_formats = std::array::from_fn(|slot| {
                draw.textures[slot].as_ref().map(|t| match t {
                    GraphicsTextureInput::Host(t) => t.format,
                    GraphicsTextureInput::ProducedTyped { desc, .. } => desc.format,
                    _ => TextureFormat::Rgba8Unorm,
                })
            });
            validate_pipeline_formats(
                pipeline,
                &color_formats,
                texture_formats,
                draw.sampled_ranges.map(|r| r.is_some()),
            )?;
            validate_shader_array_layers(draw)?;
            if pipeline.color_targets != request.colors.len()
                || pipeline.sampled_pairs & !15 != 0
                || pipeline.uses_texture != (pipeline.sampled_pairs != 0)
                || (0..2).any(|slot| {
                    draw.textures[slot].is_some()
                        != (pipeline.sampled_pairs & (3 << (slot * 2)) != 0)
                        || draw.samplers[slot].is_some()
                            != (pipeline.sampled_pairs & (5 << slot) != 0)
                })
            {
                return Err(GraphicsError::InvalidDescriptor);
            }
            Ok(TriangleShader {
                source: &pipeline.msl,
                entries: [VERTEX_ENTRY_POINT, PIXEL_ENTRY_POINT],
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    let mut cache = Vec::new();
    let mut view_cache = Vec::new();
    let mut produced_views: HashMap<_, xenora_metal::RenderColorView> = HashMap::new();
    let mut produced_array_views: HashMap<_, xenora_metal::RenderColorView> = HashMap::new();
    let mut bindings = Vec::new();
    for draw in &request.draws {
        let samplers = draw
            .samplers
            .iter()
            .map(|sampler| {
                sampler
                    .map(|desc| MetalSampler::new(gpu.device(), desc).map_err(metal_error))
                    .transpose()
            })
            .collect::<Result<Vec<_>, _>>()?;
        let mut pairs = Vec::new();
        let mask = draw
            .pipeline
            .as_ref()
            .ok_or(GraphicsError::InvalidDescriptor)?
            .sampled_pairs;
        for (texture_slot, texture) in draw.textures.iter().enumerate() {
            let Some(texture) = texture else {
                if draw.array_views[texture_slot].is_some()
                    || draw.views[texture_slot].is_some()
                    || draw.sampled_ranges[texture_slot].is_some()
                {
                    return Err(GraphicsError::InvalidDescriptor);
                }
                continue;
            };
            let array_desc = draw.array_views[texture_slot];
            let is_array = match texture {
                GraphicsTextureInput::Host(t) => t.array_layers.is_some(),
                GraphicsTextureInput::ProducedArray { .. } => true,
                GraphicsTextureInput::ProducedTyped { desc, .. } => desc.array_layers.is_some(),
                _ => false,
            };
            if let Some(range) = draw.sampled_ranges[texture_slot] {
                if !is_array || array_desc.is_some() || draw.views[texture_slot].is_some() {
                    return Err(GraphicsError::InvalidDescriptor);
                }
                let native = native_array_range_desc(range);
                let (host, produced) = match texture {
                    GraphicsTextureInput::Host(snapshot) => {
                        let uploaded = upload_texture(gpu, snapshot, &mut cache)?;
                        let view = match uploaded {
                            UploadedStorage::Array(owner) => owner.array_range_view(native),
                            UploadedStorage::Typed(owner) => owner.array_range_view(native),
                            _ => return Err(GraphicsError::InvalidDescriptor),
                        }
                        .map_err(metal_error)?;
                        (Some(view), None)
                    }
                    GraphicsTextureInput::ProducedTyped { version, desc } => {
                        if !request.waits.contains(&version.point) {
                            return Err(GraphicsError::InvalidDescriptor);
                        }
                        let output = produced_typed_output(entries, *version)?;
                        if output.desc() != native_desc(*desc) {
                            return Err(GraphicsError::InvalidDescriptor);
                        }
                        (
                            None,
                            Some(output.array_range_view(native).map_err(metal_error)?),
                        )
                    }
                    GraphicsTextureInput::ProducedArray {
                        version,
                        width,
                        height,
                        mip_levels,
                        array_layers,
                    } => {
                        if !request.waits.contains(&version.point) {
                            return Err(GraphicsError::InvalidDescriptor);
                        }
                        let output = produced_typed_output(entries, *version)?;
                        if output.desc()
                            != (xenora_metal::TypedTextureDesc {
                                format: TextureFormat::Rgba8Unorm,
                                extent: [*width, *height],
                                mip_levels: *mip_levels,
                                array_layers: Some(*array_layers),
                            })
                        {
                            return Err(GraphicsError::InvalidDescriptor);
                        }
                        (
                            None,
                            Some(output.array_range_view(native).map_err(metal_error)?),
                        )
                    }
                    _ => return Err(GraphicsError::InvalidDescriptor),
                };
                for (sampler_slot, sampler) in samplers.iter().enumerate() {
                    if mask & (1 << (texture_slot * 2 + sampler_slot)) == 0 {
                        continue;
                    }
                    let sampler = sampler
                        .as_ref()
                        .ok_or(GraphicsError::InvalidDescriptor)?
                        .clone();
                    let texture_slot = BindingSlot::new(texture_slot as u32);
                    let sampler_slot = BindingSlot::new(sampler_slot as u32);
                    pairs.push(if let Some(view) = &host {
                        QueuedSampledBinding::ArrayViewed {
                            view: view.clone(),
                            sampler,
                            texture_slot,
                            sampler_slot,
                        }
                    } else {
                        QueuedSampledBinding::ProducedTypedArrayView {
                            view: produced
                                .as_ref()
                                .ok_or(GraphicsError::InvalidDescriptor)?
                                .clone(),
                            sampler,
                            texture_slot,
                            sampler_slot,
                        }
                    });
                }
                continue;
            }
            if is_array != array_desc.is_some()
                || (array_desc.is_some() && draw.views[texture_slot].is_some())
            {
                return Err(GraphicsError::InvalidDescriptor);
            }
            let uploaded = match texture {
                GraphicsTextureInput::ProducedTyped { version, desc } => {
                    if !request.waits.contains(&version.point)
                        || produced_typed_output(entries, *version)?.desc() != native_desc(*desc)
                    {
                        return Err(GraphicsError::InvalidDescriptor);
                    }
                    None
                }
                GraphicsTextureInput::ProducedArray {
                    version,
                    width,
                    height,
                    mip_levels,
                    array_layers,
                } => {
                    if !request.waits.contains(&version.point) {
                        return Err(GraphicsError::InvalidDescriptor);
                    }
                    let output = produced_output(entries, *version)?;
                    if !output.is_array()
                        || output.extent() != [*width, *height]
                        || output.mip_levels() != *mip_levels
                        || output.array_layers() != *array_layers
                    {
                        return Err(GraphicsError::InvalidDescriptor);
                    }
                    None
                }
                GraphicsTextureInput::Host(snapshot) => {
                    Some(upload_texture(gpu, snapshot, &mut cache)?)
                }
                GraphicsTextureInput::Produced {
                    version,
                    width,
                    height,
                    mip_levels,
                } => {
                    if !request.waits.contains(&version.point)
                        || produced_output(entries, *version)?.is_array()
                        || produced_output(entries, *version)?.mip_levels() != *mip_levels
                        || produced_output(entries, *version)?.extent() != [*width, *height]
                    {
                        return Err(GraphicsError::InvalidDescriptor);
                    }
                    None
                }
            };
            let viewed = match texture {
                GraphicsTextureInput::Host(snapshot) => upload_selected_view(
                    gpu,
                    snapshot,
                    draw.views[texture_slot],
                    array_desc,
                    &mut cache,
                    &mut view_cache,
                )?,
                _ => None,
            };
            let produced_typed_view =
                if let GraphicsTextureInput::ProducedTyped { version, desc } = texture {
                    let output = produced_typed_output(entries, *version)?;
                    Some(if let Some(array) = array_desc {
                        output
                            .array_slice_view(native_array_view_desc(array))
                            .map_err(metal_error)?
                    } else {
                        let view = draw.views[texture_slot].map(native_view_desc).unwrap_or(
                            xenora_metal::TextureViewDesc {
                                base_mip: 0,
                                mip_count: desc.mip_levels,
                                format: typed_native_view_format(desc.format),
                            },
                        );
                        output.view(view).map_err(metal_error)?
                    })
                } else {
                    None
                };
            let produced_view =
                if let (GraphicsTextureInput::ProducedArray { version, .. }, Some(desc)) =
                    (texture, array_desc)
                {
                    let key = (*version, desc);
                    let view = match produced_array_views.entry(key) {
                        std::collections::hash_map::Entry::Occupied(e) => e.get().clone(),
                        std::collections::hash_map::Entry::Vacant(e) => e
                            .insert(
                                produced_output(entries, *version)?
                                    .array_slice_view(native_array_view_desc(desc))
                                    .map_err(metal_error)?,
                            )
                            .clone(),
                    };
                    Some(view)
                } else {
                    match (texture, draw.views[texture_slot]) {
                        (GraphicsTextureInput::Produced { version, .. }, Some(desc)) => {
                            let key = (*version, desc);
                            let view = match produced_views.entry(key) {
                                std::collections::hash_map::Entry::Occupied(entry) => {
                                    entry.get().clone()
                                }
                                std::collections::hash_map::Entry::Vacant(entry) => {
                                    let view = produced_output(entries, *version)?
                                        .view(native_view_desc(desc))
                                        .map_err(metal_error)?;
                                    entry.insert(view).clone()
                                }
                            };
                            Some(view)
                        }
                        _ => None,
                    }
                };
            for (sampler_slot, sampler) in samplers.iter().enumerate() {
                if mask & (1 << (texture_slot * 2 + sampler_slot)) == 0 {
                    continue;
                }
                let sampler = sampler
                    .as_ref()
                    .ok_or(GraphicsError::InvalidDescriptor)?
                    .clone();
                let texture_slot = BindingSlot::new(texture_slot as u32);
                let sampler_slot = BindingSlot::new(sampler_slot as u32);
                pairs.push(match texture {
                    GraphicsTextureInput::ProducedTyped { .. } => {
                        QueuedSampledBinding::ProducedTypedView {
                            view: produced_typed_view
                                .as_ref()
                                .ok_or(GraphicsError::InvalidDescriptor)?
                                .clone(),
                            sampler,
                            texture_slot,
                            sampler_slot,
                        }
                    }
                    GraphicsTextureInput::Host(_) if viewed.is_some() => {
                        QueuedSampledBinding::Viewed {
                            view: viewed.as_ref().unwrap().clone(),
                            sampler,
                            texture_slot,
                            sampler_slot,
                        }
                    }
                    GraphicsTextureInput::Produced { .. }
                    | GraphicsTextureInput::ProducedArray { .. }
                        if produced_view.is_some() =>
                    {
                        QueuedSampledBinding::ProducedView {
                            view: produced_view.as_ref().unwrap().clone(),
                            sampler,
                            texture_slot,
                            sampler_slot,
                        }
                    }
                    GraphicsTextureInput::Host(_) => QueuedSampledBinding::Uploaded(
                        BoundSampledTexture::bind(
                            uploaded.as_ref().unwrap().rgba8()?,
                            sampler,
                            texture_slot,
                            sampler_slot,
                        )
                        .map_err(metal_error)?,
                    ),
                    GraphicsTextureInput::ProducedArray { .. } => {
                        return Err(GraphicsError::InvalidDescriptor);
                    }
                    GraphicsTextureInput::Produced { version, .. } => {
                        QueuedSampledBinding::Produced {
                            output: produced_output(entries, *version)?,
                            sampler,
                            texture_slot,
                            sampler_slot,
                        }
                    }
                });
            }
        }
        bindings.push(pairs);
    }
    let geometry = request
        .draws
        .iter()
        .map(|draw| -> Result<_, GraphicsError> {
            draw.geometry
                .as_ref()
                .map(|input| -> Result<_, GraphicsError> {
                    if !draw.vertices.is_empty() || !draw.colors.is_empty() {
                        return Err(GraphicsError::InvalidDescriptor);
                    }
                    Ok(xenora_metal::QueuedGeometry {
                        positions: geometry_attribute(&input.positions, request, entries)?,
                        colors: input
                            .colors
                            .as_ref()
                            .map(|input| geometry_attribute(input, request, entries))
                            .transpose()?,
                        indices: input
                            .indices
                            .as_ref()
                            .map(|index| -> Result<_, GraphicsError> {
                                Ok(xenora_metal::GeometryIndices {
                                    buffer: geometry_buffer(&index.buffer, request, entries)?,
                                    offset: index.offset,
                                    format: match index.format {
                                        xenora_loader::graphics::IndexFormat::R16Uint => {
                                            xenora_metal::GeometryIndexFormat::R16Uint
                                        }
                                        xenora_loader::graphics::IndexFormat::R32Uint => {
                                            xenora_metal::GeometryIndexFormat::R32Uint
                                        }
                                    },
                                })
                            })
                            .transpose()?,
                        start: input.start,
                        count: input.count,
                        base_vertex: input.base_vertex,
                    })
                })
                .transpose()
        })
        .collect::<Result<Vec<_>, _>>()?;
    let draws = request
        .draws
        .iter()
        .map(|draw| TriangleDraw {
            vertices: &draw.vertices,
            colors: &draw.colors,
            viewport: [draw.viewport.0, draw.viewport.1],
        })
        .collect::<Vec<_>>();
    let states = request
        .draws
        .iter()
        .map(|draw| metal_state(draw.state))
        .collect::<Vec<_>>();
    let clears = request
        .colors
        .iter()
        .map(|binding| match binding {
            GraphicsQueuedColorInput::Clear(color) => *color,
            GraphicsQueuedColorInput::Host(_) | GraphicsQueuedColorInput::Produced(_) => [0; 4],
        })
        .collect::<Vec<_>>();
    let loads = request
        .colors
        .iter()
        .enumerate()
        .map(|(slot, binding)| match binding {
            GraphicsQueuedColorInput::Clear(_) => Ok(QueuedColorLoad::Clear),
            GraphicsQueuedColorInput::Host(bytes) => Ok(QueuedColorLoad::Host(bytes)),
            GraphicsQueuedColorInput::Produced(version) => {
                if !request.waits.contains(&version.point) {
                    return Err(GraphicsError::InvalidDescriptor);
                }
                let output = produced_typed_output(entries, *version)?;
                let format = request
                    .color_types
                    .get(slot)
                    .ok_or(GraphicsError::InvalidDescriptor)?
                    .map_or(TextureFormat::Rgba8Unorm, |t| t.format);
                if output.desc()
                    != (xenora_metal::TypedTextureDesc {
                        format,
                        extent: [request.extent.0, request.extent.1],
                        mip_levels: 1,
                        array_layers: None,
                    })
                {
                    return Err(GraphicsError::InvalidDescriptor);
                }
                Ok(QueuedColorLoad::ProducedTyped(output))
            }
        })
        .collect::<Result<Vec<_>, GraphicsError>>()?;
    let depth = request
        .depth
        .as_ref()
        .map(|binding| match binding {
            GraphicsQueuedDepthInput::Clear(value) => Ok(QueuedDepthInput::Clear(*value)),
            GraphicsQueuedDepthInput::Host(snapshot) => {
                super::native_depth_snapshot(snapshot).map(QueuedDepthInput::Host)
            }
            GraphicsQueuedDepthInput::Produced(version) => {
                produced_depth(entries, *version).map(QueuedDepthInput::Produced)
            }
        })
        .transpose()?;
    if request.color_types.len() != request.colors.len() {
        return Err(GraphicsError::InvalidDescriptor);
    }
    let typed_colors: Vec<_> = request
        .color_types
        .iter()
        .zip(&clears)
        .map(|(t, c)| native_color_attachment(*t, *c))
        .collect();
    let pass = xenora_metal::TypedTriangleTargetsPass {
        draws: &draws,
        states: &states,
        extent: [request.extent.0, request.extent.1],
        colors: &typed_colors,
        clear_depth: (request.depth.is_none()
            && request
                .draws
                .iter()
                .any(|d| d.state.depth_compare.is_some()))
        .then_some(request.clear_depth),
    };
    let uniforms: Vec<_> = request
        .draws
        .iter()
        .map(|draw| xenora_metal::DrawUniforms {
            required_mask: draw
                .pipeline
                .as_ref()
                .map_or(0, |p| p.required_uniform_mask),
            values: draw.uniforms,
        })
        .collect();
    let (loads, depth) = if let Some(inputs) = overrides {
        (inputs.colors, inputs.depth)
    } else {
        (loads, depth)
    };
    cohort
        .submit_with_typed_attachments(
            queue,
            &shaders,
            &pass,
            &bindings,
            depth,
            &loads,
            dependencies,
            &uniforms,
            &geometry,
        )
        .map_err(metal_error)
}

/// Install two native queues with direct GPU producer/consumer resource versions.
/// Existing single-submit installers retain their original contracts.
pub fn install_queued_pipeline_backend(context: &mut GraphicsContext) -> Result<(), GraphicsError> {
    install_inner(context, |_, _| Ok(()))
}

fn install_inner<F>(context: &mut GraphicsContext, initialize: F) -> Result<(), GraphicsError>
where
    F: FnOnce(&NativeCompute, &RenderCohort) -> Result<(), GraphicsError> + Send + 'static,
{
    let (requests, incoming) = mpsc::channel();
    let (initialized, ready) = mpsc::sync_channel(1);
    let thread = thread::Builder::new()
        .name("xenora-queued-render".into())
        .spawn(move || {
            let mut state = match State::new() {
                Ok(state) => state,
                Err(error) => {
                    let _ = initialized.send(Err(error));
                    return;
                }
            };
            if let Err(error) = initialize(&state.gpu, &state.cohort) {
                let _ = initialized.send(Err(error));
                return;
            }
            if initialized
                .send(Ok((
                    texture_capabilities(&state.gpu),
                    GraphicsTypedCopyCapabilities {
                        formats: state.gpu.typed_copy_capabilities(),
                    },
                    resource_role_capabilities(&state.gpu),
                )))
                .is_err()
            {
                return;
            }
            loop {
                match incoming.recv_timeout(Duration::from_millis(1)) {
                    Ok(Message::Submit {
                        request,
                        committed,
                        completed,
                    }) => {
                        let result = state.submit(&request, completed);
                        let _ = committed.send(result);
                    }
                    Ok(Message::Reject(producer)) => {
                        if state.entries.contains_key(&producer) {
                            state.fail(GraphicsError::BackendError(
                                "producer publication rejected".into(),
                            ));
                        }
                    }
                    Ok(Message::Release(point)) => {
                        state.entries.remove(&point);
                    }
                    Err(mpsc::RecvTimeoutError::Timeout) => {}
                    Err(mpsc::RecvTimeoutError::Disconnected) => break,
                }
                state.progress();
            }
            state.fail(disconnected());
        })
        .map_err(|error| GraphicsError::BackendError(format!("queued worker: {error}")))?;
    let worker = Arc::new(Worker {
        requests: Some(requests),
        thread: Mutex::new(Some(thread)),
    });
    let (capabilities, typed_capabilities, roles) = ready.recv().map_err(|_| disconnected())??;
    context.set_metal_typed_copy_queued_backend(capabilities, typed_capabilities, move |request| {
        let point = match &request {
            GraphicsQueuedWorkRequest::AttachmentTransaction(request) => request.point,
            GraphicsQueuedWorkRequest::Render(request) => request.point,
            GraphicsQueuedWorkRequest::ImageCopy(request) => request.point,
            GraphicsQueuedWorkRequest::Copy(request) => request.point,
            GraphicsQueuedWorkRequest::BufferRegion(request) => request.point,
        };
        let (committed, acknowledgement) = mpsc::sync_channel(1);
        let (completed, completion) = mpsc::channel();
        worker
            .requests
            .as_ref()
            .unwrap()
            .send(Message::Submit {
                request: Box::new(request),
                committed,
                completed,
            })
            .map_err(|_| disconnected())?;
        acknowledgement.recv().map_err(|_| disconnected())??;
        Ok(Box::new(Ticket {
            point,
            completed: completion,
            worker: worker.clone(),
            terminal: false,
        }))
    });
    context.enable_resource_roles(roles)?;
    context.enable_attachment_transactions()?;
    context.enable_shader_array_views()?;
    Ok(())
}

#[cfg(all(test, feature = "gpu-tests"))]
mod tests {
    use super::*;
    use objc2::runtime::ProtocolObject;
    use objc2_metal::{MTLCommandBuffer, MTLCommandQueue, MTLDevice, MTLEvent, MTLSharedEvent};
    use std::sync::atomic::{AtomicBool, Ordering};
    use xenora_loader::graphics::{GraphicsColorLoad, GraphicsSubmissionStatus};
    use xenora_texture::SamplerDesc;

    #[test]
    fn queued_producer_consumer_commits_before_gate_and_survives_destroy() {
        for color in [[200, 40, 60, 255], [80, 120, 160, 255]] {
            run_gated_pair(color, false, false, None, false);
        }
    }

    #[test]
    fn queued_publication_failure_drains_live_consumer_and_recovers() {
        run_gated_pair([200, 40, 60, 255], true, false, None, false);
    }

    #[test]
    fn queued_produced_view_preserves_srgb_and_failure_provenance() {
        run_gated_pair([128, 64, 192, 128], false, true, None, false);
        run_gated_pair([128, 64, 192, 128], true, true, None, false);
    }

    #[test]
    fn queued_host_bc_upload_produces_retained_rgba_version() {
        run_gated_pair(
            [132, 0, 0, 255],
            false,
            false,
            Some(TextureFormat::Bc1RgbaUnorm),
            false,
        );
        run_gated_pair(
            [132, 0, 0, 128],
            false,
            false,
            Some(TextureFormat::Bc3RgbaUnorm),
            false,
        );
    }

    #[test]
    fn queued_produced_color_load_commits_before_gate_and_preserves_pixels() {
        run_gated_pair([17, 31, 47, 63], false, false, None, true);
        run_gated_pair([71, 89, 101, 127], true, false, None, true);
    }

    fn run_gated_pair(
        color: [u8; 4],
        reject: bool,
        use_view: bool,
        bc_format: Option<TextureFormat>,
        produced_load: bool,
    ) {
        let mut context = GraphicsContext::new();
        let (send_event, get_event) = mpsc::sync_channel(1);
        install_inner(&mut context, move |gpu, cohort| {
            let event = gpu.device().newSharedEvent().unwrap();
            let gate = cohort.queue(RenderQueue::First).commandBuffer().unwrap();
            let prerequisite: &ProtocolObject<dyn MTLEvent> = ProtocolObject::from_ref(&*event);
            gate.encodeWaitForEvent_value(prerequisite, 1);
            gate.commit();
            let consumer_event = reject.then(|| {
                let event = gpu.device().newSharedEvent().unwrap();
                let gate = cohort.queue(RenderQueue::Second).commandBuffer().unwrap();
                let prerequisite: &ProtocolObject<dyn MTLEvent> = ProtocolObject::from_ref(&*event);
                gate.encodeWaitForEvent_value(prerequisite, 1);
                gate.commit();
                event
            });
            send_event.send((event, consumer_event)).unwrap();
            Ok(())
        })
        .unwrap();
        let (event, consumer_event) = get_event.recv().unwrap();
        let rescue_event = event.clone();
        let rescue_consumer = consumer_event.clone();
        let timed_out = Arc::new(AtomicBool::new(false));
        let rescue_timeout = timed_out.clone();
        let (done, finished) = mpsc::channel();
        let rescue = std::thread::spawn(move || {
            if finished.recv_timeout(Duration::from_secs(5)).is_err() {
                rescue_timeout.store(true, Ordering::SeqCst);
                rescue_event.setSignaledValue(1);
                if let Some(event) = rescue_consumer {
                    event.setSignaledValue(1);
                }
            }
        });
        let device = context.create_device().unwrap();
        let queues = [0, 1].map(|_| {
            context
                .create_command_queue(device, CommandListType::Direct)
                .unwrap()
        });
        let allocators = [0, 1].map(|_| {
            context
                .create_command_allocator(device, CommandListType::Direct)
                .unwrap()
        });
        let lists = allocators.map(|allocator| {
            context
                .create_command_list(device, allocator, CommandListType::Direct)
                .unwrap()
        });
        let desc = ResourceDesc {
            dimension: ResourceDimension::Texture2D,
            width: 4,
            height: 3,
            depth_or_array_size: 1,
            mip_levels: 1,
            format_rgba8: true,
        };
        let producer = context
            .create_committed_resource(device, desc.clone(), ResourceStates::RENDER_TARGET, None)
            .unwrap();
        let target = context
            .create_committed_resource(device, desc, ResourceStates::RENDER_TARGET, None)
            .unwrap();
        let buffer = ResourceDesc {
            dimension: ResourceDimension::Buffer,
            width: 48,
            height: 1,
            depth_or_array_size: 1,
            mip_levels: 1,
            format_rgba8: false,
        };
        let positions = context
            .create_committed_resource(
                device,
                buffer.clone(),
                ResourceStates::VERTEX_AND_CONSTANT_BUFFER,
                Some(0x10000),
            )
            .unwrap();
        let colors = context
            .create_committed_resource(
                device,
                buffer,
                ResourceStates::VERTEX_AND_CONSTANT_BUFFER,
                Some(0x10100),
            )
            .unwrap();
        let mut memory = GuestMemory::new();
        memory
            .map_region(0x10000, 4096, MemoryPermissions::READ_WRITE)
            .unwrap();
        let vertices: [[f32; 4]; 3] = [[-1., -1., 0., 1.], [3., -1., 0., 1.], [-1., 3., 0., 1.]];
        memory
            .write_bytes(
                0x10000,
                &vertices
                    .into_iter()
                    .flatten()
                    .flat_map(f32::to_le_bytes)
                    .collect::<Vec<_>>(),
            )
            .unwrap();
        memory
            .write_bytes(
                0x10100,
                &[[0.5f32, 0.5, 0., 1.]; 3]
                    .into_iter()
                    .flatten()
                    .flat_map(f32::to_le_bytes)
                    .collect::<Vec<_>>(),
            )
            .unwrap();
        let pipeline = context
            .create_graphics_pipeline(
                device,
                if produced_load {
                    include_bytes!("../../../../fixtures/rdna2/x9-constant-vs.bin").as_slice()
                } else {
                    include_bytes!("../../../../fixtures/rdna2/x9-varying-vs.bin").as_slice()
                },
                if produced_load {
                    include_bytes!("../../../../fixtures/rdna2/x9-constant-ps.bin").as_slice()
                } else if use_view {
                    include_bytes!("../../../../fixtures/rdna2/x9-view-sample-ps.bin").as_slice()
                } else {
                    include_bytes!("../../../../fixtures/rdna2/x9-sampled-arithmetic-ps.bin")
                        .as_slice()
                },
            )
            .unwrap();
        context
            .cmd_set_color_targets(
                lists[0],
                &[producer],
                &[if bc_format.is_some() {
                    [9, 17, 23, 31]
                } else {
                    color
                }],
            )
            .unwrap();
        let bc_source = bc_format.map(|format| {
            let texture = context
                .create_texture_resource(
                    device,
                    xenora_loader::graphics::GraphicsTextureDesc {
                        width: 4,
                        height: 4,
                        mip_levels: 1,
                        format,
                    },
                    ResourceStates::COPY_DEST,
                )
                .unwrap();
            let mut bytes = Vec::new();
            if format == TextureFormat::Bc3RgbaUnorm {
                bytes.extend([128, 0, 0, 0, 0, 0, 0, 0]);
            }
            bytes.extend([0, 128, 0, 0, 0, 0, 0, 0]); // RGB565 endpoint0=0x8000, every selector0.
            memory.write_bytes(0x10200, &bytes).unwrap();
            context
                .copy_guest_to_resource(texture, 0, 0x10200, bytes.len(), &memory)
                .unwrap();
            let program = context
                .create_graphics_pipeline(
                    device,
                    include_bytes!("../../../../fixtures/rdna2/x9-varying-vs.bin"),
                    include_bytes!("../../../../fixtures/rdna2/x9-view-sample-ps.bin"),
                )
                .unwrap();
            context
                .cmd_resource_barrier(
                    lists[0],
                    texture,
                    ResourceStates::COPY_DEST,
                    ResourceStates::PIXEL_SHADER_RESOURCE,
                )
                .unwrap();
            context.cmd_set_viewport(lists[0], 4, 3).unwrap();
            context
                .cmd_set_graphics_pipeline(lists[0], Some(program))
                .unwrap();
            context
                .cmd_set_vertex_buffer(lists[0], positions, 0, 16)
                .unwrap();
            context
                .cmd_set_vertex_colors(lists[0], colors, 0, 16)
                .unwrap();
            context
                .cmd_set_sampled_texture(lists[0], Some(texture), SamplerDesc::nearest_clamp())
                .unwrap();
            context.cmd_draw_instanced(lists[0], 3, 1, 0, 0).unwrap();
            (texture, program)
        });
        if !produced_load {
            context
                .cmd_resource_barrier(
                    lists[0],
                    producer,
                    ResourceStates::RENDER_TARGET,
                    ResourceStates::PIXEL_SHADER_RESOURCE,
                )
                .unwrap();
        }
        context.close_command_list(lists[0]).unwrap();
        let first = context
            .submit_command_list(queues[0], lists[0], &memory)
            .unwrap();
        let point = context.submission_point(first).unwrap();
        if produced_load {
            context
                .cmd_set_color_targets_with_loads(lists[1], &[producer], &[GraphicsColorLoad::Load])
                .unwrap();
            context
                .cmd_set_draw_state(
                    lists[1],
                    GraphicsDrawState {
                        scissor: Some([0, 0, 1, 3]),
                        ..Default::default()
                    },
                )
                .unwrap();
        } else {
            context
                .cmd_set_color_targets(lists[1], &[target], &[[7, 11, 19, 255]])
                .unwrap();
        }
        context.cmd_set_viewport(lists[1], 4, 3).unwrap();
        context
            .cmd_set_graphics_pipeline(lists[1], Some(pipeline))
            .unwrap();
        context
            .cmd_set_vertex_buffer(lists[1], positions, 0, 16)
            .unwrap();
        context
            .cmd_set_vertex_colors(lists[1], colors, 0, 16)
            .unwrap();
        let view = if produced_load {
            None
        } else if use_view {
            let view = context
                .create_texture_view(
                    device,
                    producer,
                    GraphicsTextureViewDesc {
                        base_mip: 0,
                        mip_count: 1,
                        format: GraphicsTextureViewFormat::Rgba8UnormSrgb,
                    },
                )
                .unwrap();
            context
                .cmd_set_sampled_views(
                    lists[1],
                    [Some(view), None],
                    [Some(SamplerDesc::nearest_clamp()), None],
                )
                .unwrap();
            Some(view)
        } else {
            context
                .cmd_set_sampled_texture(lists[1], Some(producer), SamplerDesc::nearest_clamp())
                .unwrap();
            None
        };
        context.cmd_draw_instanced(lists[1], 3, 1, 0, 0).unwrap();
        context.close_command_list(lists[1]).unwrap();
        let second = context
            .submit_command_list_after(queues[1], lists[1], &[point], &memory)
            .unwrap();
        let pending = context.poll_submission(second).unwrap();
        for i in 0..2 {
            context.reset_command_list(lists[i], allocators[i]).unwrap();
        }
        if !reject {
            if let Some((texture, program)) = bc_source {
                context.destroy_resource(texture).unwrap();
                context.destroy_graphics_pipeline(program).unwrap();
            }
            if let Some(view) = view {
                context.destroy_texture_view(view).unwrap();
            }
            for resource in [producer, positions, colors] {
                context.destroy_resource(resource).unwrap();
            }
            context.destroy_graphics_pipeline(pipeline).unwrap();
        } else {
            let presentation = context.last_presentation.clone();
            assert!(
                std::thread::spawn(move || {
                    let _guard = presentation.lock().unwrap();
                    panic!("controlled publication failure");
                })
                .join()
                .is_err()
            );
        }
        let committed_before_release = !timed_out.load(Ordering::SeqCst);
        event.setSignaledValue(1);
        assert!(
            committed_before_release,
            "worker waited for producer before acknowledging consumer"
        );
        assert_eq!(pending, GraphicsSubmissionStatus::Pending);
        if reject {
            assert_eq!(
                context.wait_submission(first).unwrap(),
                GraphicsSubmissionStatus::Failed
            );
            // Consumer has a separate live GPU gate: failure notification is
            // nonblocking, but its receipt cannot become terminal until drained.
            assert_eq!(
                context.poll_submission(second).unwrap(),
                GraphicsSubmissionStatus::Pending
            );
            consumer_event.as_ref().unwrap().setSignaledValue(1);
            assert_eq!(
                context.wait_submission(second).unwrap(),
                GraphicsSubmissionStatus::Failed
            );
            assert!(context.submission_outcome(second).unwrap().is_none());
            assert_eq!(
                context
                    .last_presentation
                    .lock()
                    .unwrap_err()
                    .get_ref()
                    .frame_index,
                0
            );
            context.last_presentation.clear_poison();
            context.release_submission(first).unwrap();
            context.release_submission(second).unwrap();
            context
                .cmd_set_color_targets(lists[0], &[producer], &[[23, 47, 89, 255]])
                .unwrap();
            context.close_command_list(lists[0]).unwrap();
            let recovery = context
                .submit_command_list(queues[0], lists[0], &memory)
                .unwrap();
            assert!(context.submission_point(recovery).unwrap().value > point.value);
            assert_eq!(
                context.wait_submission(recovery).unwrap(),
                GraphicsSubmissionStatus::Complete
            );
            assert_eq!(
                context
                    .submission_outcome(recovery)
                    .unwrap()
                    .unwrap()
                    .pixels,
                [23, 47, 89, 255].repeat(12)
            );
            context.release_submission(recovery).unwrap();
            done.send(()).unwrap();
            rescue.join().unwrap();
            assert!(!timed_out.load(Ordering::SeqCst));
            return;
        }
        assert_eq!(
            context.wait_submission(second).unwrap(),
            GraphicsSubmissionStatus::Complete
        );
        let expected = if produced_load {
            (0..12)
                .flat_map(|i| if i % 4 == 0 { [255, 0, 0, 255] } else { color })
                .collect()
        } else if use_view {
            [55, 13, 134, 128].repeat(12)
        } else {
            [color[0] / 2, color[1], color[2], color[3]].repeat(12)
        };
        assert_eq!(
            context.submission_outcome(second).unwrap().unwrap().pixels,
            expected
        );
        assert_eq!(
            context.poll_submission(first).unwrap(),
            GraphicsSubmissionStatus::Complete
        );
        context.release_submission(first).unwrap();
        context.release_submission(second).unwrap();
        done.send(()).unwrap();
        rescue.join().unwrap();
        assert!(!timed_out.load(Ordering::SeqCst));
    }

    #[test]
    fn queued_produced_depth_commits_before_gate_and_rejects_failed_ancestry() {
        for reject in [false, true] {
            let mut context = GraphicsContext::new();
            let (send, receive) = mpsc::sync_channel(1);
            install_inner(&mut context, move |gpu, cohort| {
                let producer_gate = gpu.device().newSharedEvent().unwrap();
                let consumer_gate = gpu.device().newSharedEvent().unwrap();
                for (queue, event) in [
                    (RenderQueue::First, &producer_gate),
                    (RenderQueue::Second, &consumer_gate),
                ] {
                    if queue == RenderQueue::Second && !reject {
                        continue;
                    }
                    let command = cohort.queue(queue).commandBuffer().unwrap();
                    command.encodeWaitForEvent_value(ProtocolObject::from_ref(&**event), 1);
                    command.commit();
                }
                send.send((producer_gate, consumer_gate)).unwrap();
                Ok(())
            })
            .unwrap();
            let (gate, consumer_gate) = receive.recv().unwrap();
            let rescue = (gate.clone(), consumer_gate.clone());
            let timed_out = Arc::new(AtomicBool::new(false));
            let timeout = timed_out.clone();
            let (done, finished) = mpsc::channel();
            let watchdog = std::thread::spawn(move || {
                if finished.recv_timeout(Duration::from_secs(10)).is_err() {
                    timeout.store(true, Ordering::SeqCst);
                    rescue.0.setSignaledValue(1);
                    rescue.1.setSignaledValue(1);
                }
            });
            let device = context.create_device().unwrap();
            let queues = [0, 1].map(|_| {
                context
                    .create_command_queue(device, CommandListType::Direct)
                    .unwrap()
            });
            let allocators = [0, 1].map(|_| {
                context
                    .create_command_allocator(device, CommandListType::Direct)
                    .unwrap()
            });
            let lists = allocators.map(|a| {
                context
                    .create_command_list(device, a, CommandListType::Direct)
                    .unwrap()
            });
            let targets = [0, 1].map(|_| {
                context
                    .create_committed_resource(
                        device,
                        ResourceDesc {
                            dimension: ResourceDimension::Texture2D,
                            width: 2,
                            height: 2,
                            depth_or_array_size: 1,
                            mip_levels: 1,
                            format_rgba8: true,
                        },
                        ResourceStates::RENDER_TARGET,
                        None,
                    )
                    .unwrap()
            });
            let depth = context.create_depth_resource(device, (2, 2)).unwrap();
            let vb = context
                .create_committed_resource(
                    device,
                    ResourceDesc {
                        dimension: ResourceDimension::Buffer,
                        width: 48,
                        height: 1,
                        depth_or_array_size: 1,
                        mip_levels: 1,
                        format_rgba8: false,
                    },
                    ResourceStates::VERTEX_AND_CONSTANT_BUFFER,
                    Some(0x10000),
                )
                .unwrap();
            let mut memory = GuestMemory::new();
            let positions: [[f32; 4]; 3] =
                [[-1., -1., 0.5, 1.], [3., -1., 0.5, 1.], [-1., 3., 0.5, 1.]];
            memory
                .map_region_with_data(
                    0x10000,
                    48,
                    MemoryPermissions::READ_ONLY,
                    &positions
                        .into_iter()
                        .flatten()
                        .flat_map(f32::to_le_bytes)
                        .collect::<Vec<_>>(),
                )
                .unwrap();
            let pipeline = context
                .create_graphics_pipeline(
                    device,
                    include_bytes!("../../../../fixtures/rdna2/x9-constant-vs.bin"),
                    include_bytes!("../../../../fixtures/rdna2/x9-constant-ps.bin"),
                )
                .unwrap();
            context
                .cmd_set_color_targets(lists[0], &[targets[0]], &[[17, 31, 47, 255]])
                .unwrap();
            context
                .cmd_set_depth_target(lists[0], Some(depth), GraphicsDepthLoad::Clear(0.75))
                .unwrap();
            context
                .cmd_resource_barrier(
                    lists[0],
                    depth,
                    ResourceStates::DEPTH_WRITE,
                    ResourceStates::DEPTH_READ,
                )
                .unwrap();
            context.close_command_list(lists[0]).unwrap();
            let first = context
                .submit_command_list(queues[0], lists[0], &memory)
                .unwrap();
            let point = context.submission_point(first).unwrap();
            context
                .cmd_set_color_targets(lists[1], &[targets[1]], &[[7, 11, 19, 255]])
                .unwrap();
            context
                .cmd_set_depth_target(lists[1], Some(depth), GraphicsDepthLoad::Load)
                .unwrap();
            context.cmd_set_viewport(lists[1], 2, 2).unwrap();
            context.cmd_set_vertex_buffer(lists[1], vb, 0, 16).unwrap();
            context
                .cmd_set_graphics_pipeline(lists[1], Some(pipeline))
                .unwrap();
            context
                .cmd_set_draw_state(
                    lists[1],
                    GraphicsDrawState {
                        depth_compare: Some(GraphicsDepthCompare::Less),
                        depth_write: false,
                        ..Default::default()
                    },
                )
                .unwrap();
            context.cmd_draw_instanced(lists[1], 3, 1, 0, 0).unwrap();
            context.close_command_list(lists[1]).unwrap();
            let second = context
                .submit_command_list_after(queues[1], lists[1], &[point], &memory)
                .unwrap();
            assert!(
                !timed_out.load(Ordering::SeqCst) && gate.signaledValue() == 0,
                "both commits must precede release"
            );
            assert_eq!(
                context.poll_submission(second).unwrap(),
                GraphicsSubmissionStatus::Pending
            );
            for i in 0..2 {
                context.reset_command_list(lists[i], allocators[i]).unwrap();
            }
            if reject {
                let presentation = context.last_presentation.clone();
                assert!(
                    std::thread::spawn(move || {
                        let _guard = presentation.lock().unwrap();
                        panic!("controlled depth publication failure");
                    })
                    .join()
                    .is_err()
                );
            } else {
                context.destroy_resource(depth).unwrap();
                context.destroy_resource(vb).unwrap();
                context.destroy_graphics_pipeline(pipeline).unwrap();
            }
            gate.setSignaledValue(1);
            if reject {
                assert_eq!(
                    context.wait_submission(first).unwrap(),
                    GraphicsSubmissionStatus::Failed
                );
                assert_eq!(
                    context.poll_submission(second).unwrap(),
                    GraphicsSubmissionStatus::Pending
                );
                consumer_gate.setSignaledValue(1);
                assert_eq!(
                    context.wait_submission(second).unwrap(),
                    GraphicsSubmissionStatus::Failed
                );
                assert!(context.submission_outcome(second).unwrap().is_none());
                assert_eq!(
                    context
                        .last_presentation
                        .lock()
                        .unwrap_err()
                        .get_ref()
                        .frame_index,
                    0
                );
                context.last_presentation.clear_poison();
            } else {
                assert_eq!(
                    context.wait_submission(second).unwrap(),
                    GraphicsSubmissionStatus::Complete
                );
                assert_eq!(
                    context.submission_outcome(second).unwrap().unwrap().pixels,
                    [255, 0, 0, 255].repeat(4)
                );
            }
            context.release_submission(first).unwrap();
            context.release_submission(second).unwrap();
            if reject {
                context
                    .cmd_set_color_targets(lists[0], &[targets[0]], &[[23, 47, 89, 255]])
                    .unwrap();
                context
                    .cmd_set_depth_target(lists[0], Some(depth), GraphicsDepthLoad::Clear(0.875))
                    .unwrap();
                context.close_command_list(lists[0]).unwrap();
                let recovery = context
                    .submit_command_list(queues[0], lists[0], &memory)
                    .unwrap();
                assert_eq!(
                    context.wait_submission(recovery).unwrap(),
                    GraphicsSubmissionStatus::Complete
                );
                assert_eq!(
                    context
                        .submission_outcome(recovery)
                        .unwrap()
                        .unwrap()
                        .pixels,
                    [23, 47, 89, 255].repeat(4)
                );
                context.release_submission(recovery).unwrap();
            }
            done.send(()).unwrap();
            watchdog.join().unwrap();
            assert!(!timed_out.load(Ordering::SeqCst));
        }
    }

    #[test]
    fn queued_copy_worker_rejects_wrong_kind_layout_and_missing_dependency_atomically() {
        let mut state = State::new().unwrap();
        let first = GraphicsQueuePoint {
            queue: GraphicsHandle::from_u64(101),
            value: 1,
        };
        let second = GraphicsQueuePoint {
            queue: GraphicsHandle::from_u64(102),
            value: 1,
        };
        let version = GraphicsProducedVersion {
            point: first,
            output: GraphicsProducedOutput::Copy(0),
        };
        let literal = [17u8, 83, 149, 231, 211, 43, 97, 127];
        let (sent, received_first) = mpsc::channel();
        state
            .submit(
                &GraphicsQueuedWorkRequest::Copy(GraphicsQueuedCopyRequest {
                    generation: 1,
                    point: first,
                    waits: vec![],
                    layout: GraphicsCopyLayout::Buffer { byte_len: 8 },
                    source: GraphicsCopyInput::Host(Arc::from(literal)),
                    output: version,
                }),
                sent,
            )
            .unwrap();
        let valid = GraphicsQueuedCopyRequest {
            generation: 1,
            point: second,
            waits: vec![first],
            layout: GraphicsCopyLayout::Buffer { byte_len: 8 },
            source: GraphicsCopyInput::Produced(version),
            output: GraphicsProducedVersion {
                point: second,
                output: GraphicsProducedOutput::Copy(0),
            },
        };
        let mut invalid = Vec::new();
        let mut wrong_kind = valid.clone();
        wrong_kind.layout = GraphicsCopyLayout::Rgba8 { extent: (2, 1) };
        invalid.push(wrong_kind); // Same byte count must not authorize a different storage kind.
        let mut wrong_size = valid.clone();
        wrong_size.layout = GraphicsCopyLayout::Buffer { byte_len: 7 };
        invalid.push(wrong_size);
        let mut missing_wait = valid.clone();
        missing_wait.waits.clear();
        invalid.push(missing_wait);
        let mut wrong_slot = valid.clone();
        wrong_slot.source = GraphicsCopyInput::Produced(GraphicsProducedVersion {
            point: first,
            output: GraphicsProducedOutput::Copy(1),
        });
        invalid.push(wrong_slot);
        let mut wrong_identity = valid.clone();
        wrong_identity.source = GraphicsCopyInput::Produced(GraphicsProducedVersion {
            point: first,
            output: GraphicsProducedOutput::Color(0),
        });
        invalid.push(wrong_identity);
        let mut wrong_output = valid.clone();
        wrong_output.output.output = GraphicsProducedOutput::Color(0);
        invalid.push(wrong_output);
        for request in invalid {
            let (sent, _) = mpsc::channel();
            assert!(
                state
                    .submit(&GraphicsQueuedWorkRequest::Copy(request), sent)
                    .is_err()
            );
            assert_eq!(state.entries.len(), 1);
            assert_eq!(state.queues.len(), 1);
            assert!(!state.poisoned);
        }
        let (sent, received_second) = mpsc::channel();
        state
            .submit(&GraphicsQueuedWorkRequest::Copy(valid), sent)
            .unwrap();
        assert_eq!(state.entries.len(), 2);
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        let mut results = [None, None];
        let receivers = [received_first, received_second];
        while results.iter().any(Option::is_none) {
            state.progress();
            for (result, receiver) in results.iter_mut().zip(&receivers) {
                if result.is_none() {
                    match receiver.try_recv() {
                        Ok(output) => *result = Some(output.unwrap()),
                        Err(mpsc::TryRecvError::Empty) => {}
                        Err(mpsc::TryRecvError::Disconnected) => {
                            panic!("copy completion disconnected")
                        }
                    }
                }
            }
            assert!(
                std::time::Instant::now() < deadline,
                "copy completion deadline exceeded"
            );
            if results.iter().any(Option::is_none) {
                std::thread::sleep(Duration::from_millis(1));
            }
        }
        for result in results {
            match result.unwrap() {
                GraphicsQueuedWorkOutput::Copy { layout, bytes } => {
                    assert_eq!(layout, GraphicsCopyLayout::Buffer { byte_len: 8 });
                    assert_eq!(bytes, literal);
                }
                GraphicsQueuedWorkOutput::Render(_)
                | GraphicsQueuedWorkOutput::AttachmentTransaction(_) => {
                    panic!("buffer work produced a rendered frame or attachment transaction")
                }
            }
        }
    }
    #[test]
    fn buffer_region_worker_validates_each_produced_input_before_admission() {
        let source = [
            17u8, 83, 149, 231, 9, 27, 45, 63, 81, 99, 117, 135, 153, 171, 189, 207,
        ];
        let destination = [
            101u8, 102, 103, 104, 105, 106, 107, 108, 109, 110, 111, 112, 113, 114, 115, 116, 117,
            118, 119, 120, 121,
        ];
        let expected = [
            101u8, 102, 103, 104, 105, 106, 107, 108, 9, 27, 45, 63, 81, 99, 117, 135, 117, 118,
            119, 120, 121,
        ];
        for source_produced in [false, true] {
            let mut state = State::new().unwrap();
            let first = GraphicsQueuePoint {
                queue: GraphicsHandle::from_u64(101),
                value: 1,
            };
            let second = GraphicsQueuePoint {
                queue: GraphicsHandle::from_u64(102),
                value: 1,
            };
            let version = GraphicsProducedVersion {
                point: first,
                output: GraphicsProducedOutput::Copy(0),
            };
            let prior = if source_produced {
                source.as_slice()
            } else {
                destination.as_slice()
            };
            let (sent, producer_result) = mpsc::channel();
            state
                .submit(
                    &GraphicsQueuedWorkRequest::Copy(GraphicsQueuedCopyRequest {
                        generation: 1,
                        point: first,
                        waits: vec![],
                        layout: GraphicsCopyLayout::Buffer {
                            byte_len: prior.len(),
                        },
                        source: GraphicsCopyInput::Host(Arc::from(prior)),
                        output: version,
                    }),
                    sent,
                )
                .unwrap();
            let valid = GraphicsQueuedBufferRegionRequest {
                generation: 1,
                point: second,
                waits: vec![first],
                source_byte_len: source.len(),
                destination_byte_len: destination.len(),
                source: if source_produced {
                    GraphicsCopyInput::Produced(version)
                } else {
                    GraphicsCopyInput::Host(Arc::from(source))
                },
                destination: if source_produced {
                    GraphicsCopyInput::Host(Arc::from(destination))
                } else {
                    GraphicsCopyInput::Produced(version)
                },
                source_offset: 4,
                destination_offset: 8,
                byte_len: 8,
                output: GraphicsProducedVersion {
                    point: second,
                    output: GraphicsProducedOutput::Copy(0),
                },
            };
            let mut missing_wait = valid.clone();
            missing_wait.waits.clear();
            let mut wrong_length = valid.clone();
            if source_produced {
                wrong_length.source_byte_len += 4;
            } else {
                wrong_length.destination_byte_len += 4;
            }
            let mut wrong_identity = valid.clone();
            let bad = GraphicsCopyInput::Produced(GraphicsProducedVersion {
                point: first,
                output: GraphicsProducedOutput::Color(0),
            });
            if source_produced {
                wrong_identity.source = bad;
            } else {
                wrong_identity.destination = bad;
            }
            let mut wrong_region = valid.clone();
            wrong_region.destination_offset = 20;
            for request in [missing_wait, wrong_length, wrong_identity, wrong_region] {
                let (sent, _) = mpsc::channel();
                assert!(
                    state
                        .submit(&GraphicsQueuedWorkRequest::BufferRegion(request), sent)
                        .is_err()
                );
                assert_eq!(state.entries.len(), 1);
                assert_eq!(state.queues.len(), 1);
                assert!(!state.poisoned);
            }
            let (sent, consumer_result) = mpsc::channel();
            state
                .submit(&GraphicsQueuedWorkRequest::BufferRegion(valid), sent)
                .unwrap();
            let deadline = std::time::Instant::now() + Duration::from_secs(10);
            let mut results = [None, None];
            let receivers = [producer_result, consumer_result];
            while results.iter().any(Option::is_none) {
                state.progress();
                for (result, receiver) in results.iter_mut().zip(&receivers) {
                    if result.is_none() {
                        match receiver.try_recv() {
                            Ok(output) => *result = Some(output.unwrap()),
                            Err(mpsc::TryRecvError::Empty) => {}
                            Err(mpsc::TryRecvError::Disconnected) => {
                                panic!("region completion disconnected")
                            }
                        }
                    }
                }
                assert!(
                    std::time::Instant::now() < deadline,
                    "region completion deadline exceeded"
                );
                if results.iter().any(Option::is_none) {
                    std::thread::sleep(Duration::from_millis(1));
                }
            }
            for (result, literal) in results.into_iter().zip([prior, expected.as_slice()]) {
                match result.unwrap() {
                    GraphicsQueuedWorkOutput::Copy { layout, bytes } => {
                        assert_eq!(
                            layout,
                            GraphicsCopyLayout::Buffer {
                                byte_len: literal.len()
                            }
                        );
                        assert_eq!(bytes, literal);
                    }
                    _ => panic!("region copy produced a rendered frame"),
                }
            }
        }
    }
    #[test]
    fn image_copy_worker_validates_both_exact_layouts_before_admission() {
        let source = [
            1u8, 2, 3, 4, 11, 12, 13, 14, 21, 22, 23, 24, 31, 32, 33, 34, 41, 42, 43, 44, 51, 52,
            53, 54,
        ];
        let destination = [
            101u8, 102, 103, 104, 105, 106, 107, 108, 109, 110, 111, 112, 113, 114, 115, 116, 117,
            118, 119, 120, 121, 122, 123, 124,
        ];
        let expected = [
            101u8, 102, 103, 104, 105, 106, 107, 108, 11, 12, 13, 14, 113, 114, 115, 116, 117, 118,
            119, 120, 121, 122, 123, 124,
        ];
        let source_layout = GraphicsCopyLayout::Rgba8 { extent: (3, 2) };
        let destination_layout = GraphicsCopyLayout::Rgba8 { extent: (2, 3) };
        for source_produced in [false, true] {
            let mut state = State::new().unwrap();
            let first = GraphicsQueuePoint {
                queue: GraphicsHandle::from_u64(101),
                value: 1,
            };
            let second = GraphicsQueuePoint {
                queue: GraphicsHandle::from_u64(102),
                value: 1,
            };
            let version = GraphicsProducedVersion {
                point: first,
                output: GraphicsProducedOutput::Copy(0),
            };
            let (prior, prior_layout) = if source_produced {
                (source.as_slice(), source_layout)
            } else {
                (destination.as_slice(), destination_layout)
            };
            let (sent, producer_result) = mpsc::channel();
            state
                .submit(
                    &GraphicsQueuedWorkRequest::Copy(GraphicsQueuedCopyRequest {
                        generation: 1,
                        point: first,
                        waits: vec![],
                        layout: prior_layout,
                        source: GraphicsCopyInput::Host(Arc::from(prior)),
                        output: version,
                    }),
                    sent,
                )
                .unwrap();
            let valid = GraphicsQueuedImageCopyRequest {
                generation: 1,
                point: second,
                waits: vec![first],
                source_layout,
                destination_layout,
                source: if source_produced {
                    GraphicsCopyInput::Produced(version)
                } else {
                    GraphicsCopyInput::Host(Arc::from(source))
                },
                destination: if source_produced {
                    GraphicsCopyInput::Host(Arc::from(destination))
                } else {
                    GraphicsCopyInput::Produced(version)
                },
                source_location: GraphicsImageCopyLocation::Rgba8 { origin: (1, 0) },
                destination_location: GraphicsImageCopyLocation::Rgba8 { origin: (0, 1) },
                extent: (1, 1),
                output: GraphicsProducedVersion {
                    point: second,
                    output: GraphicsProducedOutput::Copy(0),
                },
            };
            let mut no_wait = valid.clone();
            no_wait.waits.clear();
            let mut wrong_shape = valid.clone();
            if source_produced {
                wrong_shape.source_layout = destination_layout;
            } else {
                wrong_shape.destination_layout = source_layout;
            }
            let mut wrong_kind = valid.clone();
            if source_produced {
                wrong_kind.source_layout = GraphicsCopyLayout::Buffer { byte_len: 24 };
            } else {
                wrong_kind.destination_layout = GraphicsCopyLayout::Buffer { byte_len: 24 };
            }
            let mut wrong_version = valid.clone();
            let bad = GraphicsCopyInput::Produced(GraphicsProducedVersion {
                point: first,
                output: GraphicsProducedOutput::Copy(1),
            });
            if source_produced {
                wrong_version.source = bad;
            } else {
                wrong_version.destination = bad;
            }
            let mut wrong_bounds = valid.clone();
            wrong_bounds.destination_location = GraphicsImageCopyLocation::Rgba8 { origin: (2, 0) };
            for request in [
                no_wait,
                wrong_shape,
                wrong_kind,
                wrong_version,
                wrong_bounds,
            ] {
                let (sent, _) = mpsc::channel();
                assert!(
                    state
                        .submit(&GraphicsQueuedWorkRequest::ImageCopy(request), sent)
                        .is_err()
                );
                assert_eq!(state.entries.len(), 1);
                assert_eq!(state.queues.len(), 1);
                assert!(!state.poisoned);
            }
            let (sent, consumer_result) = mpsc::channel();
            state
                .submit(&GraphicsQueuedWorkRequest::ImageCopy(valid), sent)
                .unwrap();
            let deadline = std::time::Instant::now() + Duration::from_secs(10);
            let mut results = [None, None];
            let receivers = [producer_result, consumer_result];
            while results.iter().any(Option::is_none) {
                state.progress();
                for (result, receiver) in results.iter_mut().zip(&receivers) {
                    if result.is_none() {
                        match receiver.try_recv() {
                            Ok(output) => *result = Some(output.unwrap()),
                            Err(mpsc::TryRecvError::Empty) => {}
                            Err(mpsc::TryRecvError::Disconnected) => {
                                panic!("image completion disconnected")
                            }
                        }
                    }
                }
                assert!(
                    std::time::Instant::now() < deadline,
                    "image completion deadline exceeded"
                );
                if results.iter().any(Option::is_none) {
                    std::thread::sleep(Duration::from_millis(1));
                }
            }
            for (result, (literal, expected_layout)) in results.into_iter().zip([
                (prior, prior_layout),
                (expected.as_slice(), destination_layout),
            ]) {
                match result.unwrap() {
                    GraphicsQueuedWorkOutput::Copy { layout, bytes } => {
                        assert_eq!(layout, expected_layout);
                        assert_eq!(bytes, literal);
                    }
                    _ => panic!("image copy produced rendered outcome"),
                }
            }
        }
    }
}

fn produced_typed_output(
    entries: &HashMap<GraphicsQueuePoint, Entry>,
    version: GraphicsProducedVersion,
) -> Result<xenora_metal::RenderTypedTextureOutput, GraphicsError> {
    let entry = entries
        .get(&version.point)
        .ok_or(GraphicsError::InvalidResourceHandle)?;
    match version.output {
        GraphicsProducedOutput::AttachmentRoot(slot) => entry
            .transaction
            .as_ref()
            .and_then(|t| t.roots.get(slot as usize))
            .cloned()
            .ok_or(GraphicsError::InvalidResourceHandle),
        GraphicsProducedOutput::Color(slot) => entry
            .typed_outputs
            .get(slot as usize)
            .cloned()
            .map(Ok)
            .unwrap_or_else(|| {
                entry
                    .outputs
                    .get(slot as usize)
                    .ok_or(GraphicsError::InvalidResourceHandle)?
                    .typed_output()
                    .map_err(metal_error)
            }),
        GraphicsProducedOutput::Depth => entry
            .depth_output
            .as_ref()
            .ok_or(GraphicsError::InvalidResourceHandle)?
            .typed_output()
            .map_err(metal_error),
        GraphicsProducedOutput::Copy(0) => match entry.copy_output.as_ref() {
            Some((
                GraphicsCopyLayout::TypedTexture(desc),
                RenderCopyOutput::TypedTexture(output),
            )) if native_desc(*desc) == output.desc() => Ok(output.clone()),
            Some((_, RenderCopyOutput::Color(output))) => {
                output.typed_output().map_err(metal_error)
            }
            _ => Err(GraphicsError::InvalidDescriptor),
        },
        _ => Err(GraphicsError::InvalidDescriptor),
    }
}
fn typed_native_view_format(format: TextureFormat) -> TextureViewFormat {
    match format {
        TextureFormat::Rgba8Unorm => TextureViewFormat::Rgba8Unorm,
        TextureFormat::R8Unorm => TextureViewFormat::R8Unorm,
        TextureFormat::Rg8Unorm => TextureViewFormat::Rg8Unorm,
        TextureFormat::Bgra8Unorm => TextureViewFormat::Bgra8Unorm,
        TextureFormat::Rgba16Float => TextureViewFormat::Rgba16Float,
        TextureFormat::R32Float => TextureViewFormat::R32Float,
        TextureFormat::R32Uint => TextureViewFormat::R32Uint,
        TextureFormat::Depth32Float => TextureViewFormat::Depth32Float,
        TextureFormat::Bc1RgbaUnorm => TextureViewFormat::Bc1RgbaUnorm,
        TextureFormat::Bc3RgbaUnorm => TextureViewFormat::Bc3RgbaUnorm,
    }
}

fn native_array_range_desc(
    desc: xenora_loader::graphics::GraphicsArrayRangeViewDesc,
) -> xenora_metal::ArrayTextureViewDesc {
    xenora_metal::ArrayTextureViewDesc {
        base_slice: desc.base_slice,
        slice_count: desc.slice_count,
        base_mip: desc.base_mip,
        mip_count: desc.mip_count,
        format: native_view_desc(xenora_loader::graphics::GraphicsTextureViewDesc {
            base_mip: desc.base_mip,
            mip_count: desc.mip_count,
            format: desc.format,
        })
        .format,
    }
}
fn validate_shader_array_layers(
    draw: &xenora_loader::graphics::GraphicsQueuedDraw,
) -> Result<(), GraphicsError> {
    use xenora_shader::{GraphicsArrayLayer, GraphicsSampleType};
    let pipeline = draw
        .pipeline
        .as_ref()
        .ok_or(GraphicsError::InvalidDescriptor)?;
    for slot in 0..2 {
        let is_array = matches!(
            pipeline.types.textures[slot],
            GraphicsSampleType::FloatArray | GraphicsSampleType::UintArray
        );
        if is_array != draw.sampled_ranges[slot].is_some() {
            return Err(GraphicsError::InvalidDescriptor);
        }
        let Some(range) = draw.sampled_ranges[slot] else {
            if !pipeline.array_layers[slot].is_empty() {
                return Err(GraphicsError::InvalidDescriptor);
            }
            continue;
        };
        let (mips, layers) = match draw.textures[slot]
            .as_ref()
            .ok_or(GraphicsError::InvalidDescriptor)?
        {
            GraphicsTextureInput::Host(t) => (t.mip_levels, t.array_layers),
            GraphicsTextureInput::ProducedArray {
                mip_levels,
                array_layers,
                ..
            } => (*mip_levels, Some(*array_layers)),
            GraphicsTextureInput::ProducedTyped { desc, .. } => {
                (desc.mip_levels, desc.array_layers)
            }
            _ => return Err(GraphicsError::InvalidDescriptor),
        };
        let layers = layers.ok_or(GraphicsError::InvalidDescriptor)?;
        if range.slice_count == 0
            || range.mip_count == 0
            || range
                .base_slice
                .checked_add(range.slice_count)
                .is_none_or(|end| end > layers)
            || range
                .base_mip
                .checked_add(range.mip_count)
                .is_none_or(|end| end > mips)
            || pipeline.array_layers[slot].is_empty()
        {
            return Err(GraphicsError::InvalidDescriptor);
        }
        for source in &pipeline.array_layers[slot] {
            match *source {
                GraphicsArrayLayer::Constant(layer) if layer < range.slice_count => {}
                GraphicsArrayLayer::Uniform(index) => {
                    let bits = draw
                        .uniforms
                        .as_ref()
                        .and_then(|u| u.get(index as usize))
                        .ok_or(GraphicsError::InvalidDescriptor)?;
                    let layer = f32::from_bits(*bits);
                    if !layer.is_finite()
                        || layer < 0.
                        || layer.fract() != 0.
                        || layer >= range.slice_count as f32
                    {
                        return Err(GraphicsError::InvalidDescriptor);
                    }
                }
                _ => return Err(GraphicsError::InvalidDescriptor),
            }
        }
    }
    Ok(())
}
