use bevy::{
    core_pipeline::core_2d::graph::{Core2d, Node2d},
    ecs::query::QueryItem,
    prelude::*,
    render::{
        extract_component::{ExtractComponent, ExtractComponentPlugin},
        render_graph::{
            NodeRunError, RenderGraphApp, RenderGraphContext, RenderLabel, ViewNode, ViewNodeRunner,
        },
        render_resource::{
            binding_types::{texture_2d, texture_storage_2d, uniform_buffer},
            BindGroup, BindGroupEntries, BindGroupLayout, BindGroupLayoutEntries,
            CachedComputePipelineId, ComputePassDescriptor, ComputePipelineDescriptor,
            DynamicUniformBuffer, PipelineCache, ShaderStages, ShaderType, StorageTextureAccess,
            TextureDescriptor, TextureDimension, TextureFormat, TextureSampleType, TextureUsages,
        },
        renderer::{RenderContext, RenderDevice, RenderQueue},
        texture::{CachedTexture, TextureCache},
        view::ViewTarget,
        Render, RenderApp, RenderSet,
    },
};

use crate::math_util::batch_count;

pub const MAX_CASCADE_COUNT: usize = 16;

pub struct RadianceCascadesPlugin;

impl Plugin for RadianceCascadesPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(ExtractComponentPlugin::<RadianceCascadesConfig>::default());

        let Some(render_app) = app.get_sub_app_mut(RenderApp) else {
            return;
        };

        render_app
            .add_render_graph_node::<ViewNodeRunner<RadianceCascadesNode>>(
                Core2d,
                RadianceCascadesLabel,
            )
            .add_render_graph_edges(
                Core2d,
                (
                    crate::jfa::JfaPrepassLabel,
                    Node2d::MainTransparentPass,
                    RadianceCascadesLabel,
                    Node2d::EndMainPass,
                ),
            )
            .add_systems(
                Render,
                (
                    calculate_cascade_count.in_set(RenderSet::PrepareResources),
                    (
                        prepare_radiance_cascades_textures,
                        prepare_radiance_cascades_buffers,
                    )
                        .in_set(RenderSet::PrepareResources)
                        .after(calculate_cascade_count),
                    prepare_radiance_cascades_bind_groups.in_set(RenderSet::PrepareBindGroups),
                ),
            );
    }

    fn finish(&self, app: &mut App) {
        let Some(render_app) = app.get_sub_app_mut(RenderApp) else {
            return;
        };

        render_app.init_resource::<RadianceCascadesPipeline>();
    }
}

#[derive(Debug, Hash, PartialEq, Eq, Clone, RenderLabel)]
pub struct RadianceCascadesLabel;

/// Adding this to [bevy::prelude::Camera2d] will enable Radiance Cascades GI.
#[derive(ExtractComponent, Component, Clone, Copy)]
pub struct RadianceCascadesConfig {
    /// Determines the number of directions in cascade 0 (angular resolution).
    /// `angular_resolution = resolution_factor * 4`.
    resolution_factor: u32,
    /// Interval length of cascade 0 in pixel unit.
    interval: f32,
}

impl RadianceCascadesConfig {
    pub const RESOLUTION_MULTIPLIER: u32 = 4;

    /// Creates a new radiance cascades configuration with resolution
    /// factor clamped above 1.
    pub fn new(mut resolution_factor: u32, interval: f32) -> Self {
        resolution_factor = u32::max(resolution_factor, 1);
        Self {
            resolution_factor,
            interval,
        }
    }

    /// New config with resolution factor (clamped above 1).
    pub fn with_resolution_factor(mut self, mut resolution_factor: u32) -> Self {
        resolution_factor = u32::max(resolution_factor, 1);
        self.resolution_factor = resolution_factor;
        self
    }

    /// New config with interval length in pixel unit.
    pub fn with_interval(mut self, interval: f32) -> Self {
        self.interval = interval;
        self
    }

    /// Get number of directions in cascade 0 ([resolution_factor][Self::resolution_factor] * 4).
    pub fn get_resolution(&self) -> u32 {
        self.resolution_factor * Self::RESOLUTION_MULTIPLIER
    }
}

impl Default for RadianceCascadesConfig {
    fn default() -> Self {
        Self {
            resolution_factor: 1,
            // Why not?
            interval: std::f32::consts::PI,
        }
    }
}

#[derive(Default)]
pub struct RadianceCascadesNode;

impl ViewNode for RadianceCascadesNode {
    type ViewQuery = (
        &'static ViewTarget,
        &'static RadianceCascadesBindGroups,
        &'static RadianceCascadesCount,
        &'static RadianceCascadesConfig,
        &'static RadianceCascadesBuffer,
    );

    fn run<'w>(
        &self,
        _graph: &mut RenderGraphContext,
        render_context: &mut RenderContext<'w>,
        (view, bind_groups, cascade_count, config, buffer): QueryItem<'w, Self::ViewQuery>,
        world: &'w World,
    ) -> Result<(), NodeRunError> {
        let pipeline = world.resource::<RadianceCascadesPipeline>();
        let pipeline_cache = world.resource::<PipelineCache>();

        // Get the pipeline from the cache
        let (Some(dist_field_pipeline), Some(radiance_cascades_pipeline)) = (
            pipeline_cache.get_compute_pipeline(pipeline.dist_field_pipeline),
            pipeline_cache.get_compute_pipeline(pipeline.radiance_cascades_pipeline),
        ) else {
            return Ok(());
        };

        render_context
            .command_encoder()
            .push_debug_group("radiance_cascades_pass_group");

        let size = view.main_texture().size();

        {
            // Distance field
            let mut dist_field_compute_pass =
                render_context
                    .command_encoder()
                    .begin_compute_pass(&ComputePassDescriptor {
                        label: Some("radiance_cascades_dist_field_pass"),
                        timestamp_writes: None,
                    });

            dist_field_compute_pass.set_pipeline(dist_field_pipeline);
            dist_field_compute_pass.set_bind_group(0, &bind_groups.dist_field_bind_group, &[]);

            let workgroup_size =
                batch_count(UVec3::new(size.width, size.height, 1), UVec3::new(8, 8, 1));
            dist_field_compute_pass.dispatch_workgroups(
                workgroup_size.x,
                workgroup_size.y,
                workgroup_size.z,
            );
        }

        {
            // Radiance cascades
            let mut radiance_cascades_compute_pass = render_context
                .command_encoder()
                .begin_compute_pass(&ComputePassDescriptor {
                    label: Some("radiance_cascades_pass"),
                    timestamp_writes: None,
                });

            radiance_cascades_compute_pass.set_pipeline(radiance_cascades_pipeline);

            let resolution = config.get_resolution();
            let workgroup_size = batch_count(
                UVec3::new(size.width * resolution, size.height * resolution, 1),
                UVec3::new(8, 8, 1),
            );

            let cascade_count = cascade_count.0;
            for c in 0..cascade_count {
                let offset_index = cascade_count - c - 1;

                // Set bind groups
                let radiance_cascades_bind_group = match c % 2 == 0 {
                    true => &bind_groups.radiance_cascades_01_bind_group,
                    false => &bind_groups.radiance_cascades_10_bind_group,
                };
                radiance_cascades_compute_pass.set_bind_group(
                    0,
                    radiance_cascades_bind_group,
                    &[buffer.probe_buffer_offsets[offset_index]],
                );

                // Dispatch compute shader
                radiance_cascades_compute_pass.dispatch_workgroups(
                    workgroup_size.x,
                    workgroup_size.y,
                    workgroup_size.z,
                );
            }
        }

        render_context.command_encoder().pop_debug_group();

        Ok(())
    }
}

#[derive(Resource)]
struct RadianceCascadesPipeline {
    dist_field_bind_group_layout: BindGroupLayout,
    radiance_cascades_bind_group_layout: BindGroupLayout,
    dist_field_pipeline: CachedComputePipelineId,
    radiance_cascades_pipeline: CachedComputePipelineId,
}

impl FromWorld for RadianceCascadesPipeline {
    fn from_world(world: &mut World) -> Self {
        let render_device = world.resource::<RenderDevice>();
        let pipeline_cache = world.resource::<PipelineCache>();

        // Shader
        let dist_field_shader = world.load_asset("shaders/distance_field.wgsl");
        let radiance_cascades_shader = world.load_asset("shaders/radiance_cascades.wgsl");

        // Bind group layout
        let dist_field_bind_group_layout = render_device.create_bind_group_layout(
            "dist_field_bind_group_layout",
            &BindGroupLayoutEntries::sequential(
                ShaderStages::COMPUTE,
                (
                    // Jfa texture
                    texture_2d(TextureSampleType::Uint),
                    // Distance field texture
                    texture_storage_2d(
                        RadianceCascadesTextures::DIST_FIELD_FORMAT,
                        StorageTextureAccess::WriteOnly,
                    ),
                ),
            ),
        );

        let radiance_cascades_bind_group_layout = render_device.create_bind_group_layout(
            "radiance_cascades_bind_group_layout",
            &BindGroupLayoutEntries::sequential(
                ShaderStages::COMPUTE,
                (
                    // Probe width
                    uniform_buffer::<Probe>(true),
                    // Main texture
                    texture_2d(TextureSampleType::Float { filterable: false }),
                    // Distance field texture
                    texture_2d(TextureSampleType::Float { filterable: false }),
                    // Cascade n+1 texture
                    texture_2d(TextureSampleType::Float { filterable: false }),
                    // Cascade n texture
                    texture_storage_2d(
                        RadianceCascadesTextures::CASCADE_FORMAT,
                        StorageTextureAccess::WriteOnly,
                    ),
                ),
            ),
        );

        // Pipeline
        let dist_field_pipeline =
            pipeline_cache.queue_compute_pipeline(ComputePipelineDescriptor {
                label: Some("dist_field_pipeline".into()),
                layout: vec![dist_field_bind_group_layout.clone()],
                shader: dist_field_shader,
                shader_defs: vec![],
                entry_point: "distance_field".into(),
                push_constant_ranges: vec![],
            });

        let radiance_cascades_pipeline =
            pipeline_cache.queue_compute_pipeline(ComputePipelineDescriptor {
                label: Some("radiance_cascades_pipeline".into()),
                layout: vec![radiance_cascades_bind_group_layout.clone()],
                shader: radiance_cascades_shader,
                shader_defs: vec![],
                entry_point: "radiance_cascades".into(),
                push_constant_ranges: vec![],
            });

        Self {
            dist_field_bind_group_layout,
            radiance_cascades_bind_group_layout,
            dist_field_pipeline,
            radiance_cascades_pipeline,
        }
    }
}

#[derive(Component, Debug, Clone, Copy)]
pub struct RadianceCascadesCount(usize);

#[derive(ShaderType, Debug, Clone, Copy)]
struct Probe {
    pub width: u32,
    pub interval: f32,
}

#[derive(Component)]
pub struct RadianceCascadesBuffer {
    probe_buffers: DynamicUniformBuffer<Probe>,
    probe_buffer_offsets: Vec<u32>,
}

#[derive(Component)]
pub struct RadianceCascadesTextures {
    pub dist_field_texture: CachedTexture,
    pub radiance_cascades_texture0: CachedTexture,
    pub radiance_cascades_texture1: CachedTexture,
    is_texture0: bool,
}

impl RadianceCascadesTextures {
    pub const DIST_FIELD_FORMAT: TextureFormat = TextureFormat::R16Float;
    pub const CASCADE_FORMAT: TextureFormat = TextureFormat::Rgba32Float;

    pub fn main_texture(&self) -> &CachedTexture {
        match self.is_texture0 {
            true => &self.radiance_cascades_texture0,
            false => &self.radiance_cascades_texture1,
        }
    }
}

#[derive(Component)]
pub struct RadianceCascadesBindGroups {
    dist_field_bind_group: BindGroup,
    radiance_cascades_01_bind_group: BindGroup,
    radiance_cascades_10_bind_group: BindGroup,
}

fn calculate_cascade_count(
    mut commands: Commands,
    q_views: Query<(Entity, &ViewTarget, &RadianceCascadesConfig)>,
) {
    for (entity, view, config) in q_views.iter() {
        let size = view.main_texture().size();

        let diagonal = f32::sqrt((size.width * size.width + size.height * size.height) as f32);
        let mut cascade_count = f32::log(diagonal / config.interval, 4.0).ceil() as usize;
        cascade_count = usize::min(cascade_count, MAX_CASCADE_COUNT);

        commands
            .entity(entity)
            .insert(RadianceCascadesCount(cascade_count));
    }
}

fn prepare_radiance_cascades_textures(
    mut commands: Commands,
    q_views: Query<(
        Entity,
        &ViewTarget,
        &RadianceCascadesConfig,
        &RadianceCascadesCount,
    )>,
    mut texture_cache: ResMut<TextureCache>,
    render_device: Res<RenderDevice>,
) {
    for (entity, view, config, cascade_count) in q_views.iter() {
        let mut size = view.main_texture().size();
        size.depth_or_array_layers = 1;

        let dist_field_texture = texture_cache.get(
            &render_device,
            TextureDescriptor {
                label: Some("dist_field_texture"),
                size,
                mip_level_count: 1,
                sample_count: 1,
                dimension: TextureDimension::D2,
                format: RadianceCascadesTextures::DIST_FIELD_FORMAT,
                usage: TextureUsages::STORAGE_BINDING | TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            },
        );

        let resolution = config.get_resolution();
        size.width *= resolution;
        size.height *= resolution;
        let cascade_texture_desc = |name: &'static str| TextureDescriptor {
            label: Some(name),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format: RadianceCascadesTextures::CASCADE_FORMAT,
            usage: TextureUsages::STORAGE_BINDING | TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        };

        let radiance_cascades_texture0 = texture_cache.get(
            &render_device,
            cascade_texture_desc("radiance_cascade_0_texture"),
        );
        let radiance_cascades_texture1 = texture_cache.get(
            &render_device,
            cascade_texture_desc("radiance_cascade_1_texture"),
        );

        commands.entity(entity).insert(RadianceCascadesTextures {
            dist_field_texture,
            radiance_cascades_texture0,
            radiance_cascades_texture1,
            is_texture0: cascade_count.0 % 2 == 0,
        });
    }
}

fn prepare_radiance_cascades_buffers(
    mut commands: Commands,
    q_configs: Query<(Entity, &RadianceCascadesConfig, &RadianceCascadesCount)>,
    render_device: Res<RenderDevice>,
    render_queue: Res<RenderQueue>,
) {
    for (entity, config, cascade_count) in q_configs.iter() {
        let mut probe_buffers = DynamicUniformBuffer::default();
        probe_buffers.set_label(Some("radiance_cascades_probe_buffers"));

        let cascade_count = cascade_count.0;
        let mut probe_buffer_offsets = Vec::with_capacity(cascade_count);

        for c in 0..cascade_count + 1 {
            // Power of 4
            let width = 1 << ((c as u32 + config.resolution_factor) * 2);
            let interval = config.interval * (1 << (c * 2)) as f32;
            let probe = Probe { width, interval };

            println!("probe: {:?}", probe);

            let offset = probe_buffers.push(&probe);
            probe_buffer_offsets.push(offset);
        }

        probe_buffers.write_buffer(&render_device, &render_queue);

        commands.entity(entity).insert(RadianceCascadesBuffer {
            probe_buffers,
            probe_buffer_offsets,
        });
    }
}

fn prepare_radiance_cascades_bind_groups(
    mut commands: Commands,
    q_views: Query<(
        Entity,
        &ViewTarget,
        &crate::jfa::JfaPrepassTextures,
        &RadianceCascadesTextures,
        &RadianceCascadesBuffer,
    )>,
    render_device: Res<RenderDevice>,
    pipeline: Res<RadianceCascadesPipeline>,
) {
    for (entity, view, jfa_textures, textures, buffer) in q_views.iter() {
        let dist_field_bind_group = render_device.create_bind_group(
            "dist_field_bind_group",
            &pipeline.dist_field_bind_group_layout,
            &BindGroupEntries::sequential((
                &jfa_textures.main_texture().default_view,
                &textures.dist_field_texture.default_view,
            )),
        );

        let radiance_cascades_01_bind_group = render_device.create_bind_group(
            "radiance_cascade_01_bind_group",
            &pipeline.radiance_cascades_bind_group_layout,
            &BindGroupEntries::sequential((
                &buffer.probe_buffers,
                view.main_texture_view(),
                &textures.dist_field_texture.default_view,
                &textures.radiance_cascades_texture0.default_view,
                &textures.radiance_cascades_texture1.default_view,
            )),
        );

        let radiance_cascades_10_bind_group = render_device.create_bind_group(
            "radiance_cascade_10_bind_group",
            &pipeline.radiance_cascades_bind_group_layout,
            &BindGroupEntries::sequential((
                &buffer.probe_buffers,
                view.main_texture_view(),
                &textures.dist_field_texture.default_view,
                &textures.radiance_cascades_texture1.default_view,
                &textures.radiance_cascades_texture0.default_view,
            )),
        );

        commands.entity(entity).insert(RadianceCascadesBindGroups {
            dist_field_bind_group,
            radiance_cascades_01_bind_group,
            radiance_cascades_10_bind_group,
        });
    }
}
