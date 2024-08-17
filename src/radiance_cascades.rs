use bevy::{
    core_pipeline::{
        core_2d::graph::{Core2d, Node2d},
        fullscreen_vertex_shader::fullscreen_shader_vertex_state,
    },
    ecs::query::QueryItem,
    prelude::*,
    render::{
        extract_component::{ExtractComponent, ExtractComponentPlugin},
        render_graph::{
            NodeRunError, RenderGraphApp, RenderGraphContext, RenderLabel, ViewNode, ViewNodeRunner,
        },
        render_resource::{
            binding_types::{sampler, texture_2d, texture_storage_2d, uniform_buffer},
            BindGroup, BindGroupEntries, BindGroupLayout, BindGroupLayoutEntries,
            CachedComputePipelineId, CachedRenderPipelineId, ColorTargetState, ColorWrites,
            ComputePassDescriptor, ComputePipelineDescriptor, DynamicUniformBuffer, FragmentState,
            PipelineCache, RenderPassColorAttachment, RenderPassDescriptor,
            RenderPipelineDescriptor, Sampler, SamplerBindingType, SamplerDescriptor, ShaderStages,
            ShaderType, StorageTextureAccess, TextureDescriptor, TextureDimension, TextureFormat,
            TextureSampleType, TextureUsages,
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
    interval0: f32,
}

impl RadianceCascadesConfig {
    /// Creates a new radiance cascades configuration with resolution
    /// factor and interval0 clamped above 1.
    pub fn new(mut resolution_factor: u32, mut interval0: f32) -> Self {
        resolution_factor = u32::max(resolution_factor, 1);
        interval0 = f32::max(interval0, 1.0);
        Self {
            resolution_factor,
            interval0,
        }
    }

    /// New config with resolution factor (clamped above 1).
    pub fn with_resolution_factor(mut self, mut resolution_factor: u32) -> Self {
        resolution_factor = u32::max(resolution_factor, 1);
        self.resolution_factor = resolution_factor;
        self
    }

    /// New config with interval length in pixel unit (clamped above 1).
    pub fn with_interval(mut self, mut interval0: f32) -> Self {
        interval0 = f32::max(interval0, 1.0);
        self.interval0 = interval0;
        self
    }
}

impl Default for RadianceCascadesConfig {
    fn default() -> Self {
        Self {
            resolution_factor: 1,
            interval0: 2.0,
        }
    }
}

#[derive(Default)]
pub struct RadianceCascadesNode;

impl ViewNode for RadianceCascadesNode {
    type ViewQuery = (
        &'static ViewTarget,
        &'static RadianceCascadesBindGroups,
        &'static RadianceCascadesTextures,
        &'static RadianceCascadesCount,
        &'static RadianceCascadesBuffer,
    );

    fn run<'w>(
        &self,
        _graph: &mut RenderGraphContext,
        render_context: &mut RenderContext<'w>,
        (view, bind_groups, textures, cascade_count, buffer): QueryItem<'w, Self::ViewQuery>,
        world: &'w World,
    ) -> Result<(), NodeRunError> {
        let pipeline = world.resource::<RadianceCascadesPipeline>();
        let pipeline_cache = world.resource::<PipelineCache>();

        // Get the pipeline from the cache
        let (
            Some(dist_field_pipeline),
            Some(radiance_cascades_pipeline),
            Some(radiance_cascades_merge_pipeline),
            Some(radiance_cascades_mipmap_pipeline),
        ) = (
            pipeline_cache.get_compute_pipeline(pipeline.dist_field_pipeline),
            pipeline_cache.get_compute_pipeline(pipeline.radiance_cascades_pipeline),
            pipeline_cache.get_compute_pipeline(pipeline.radiance_cascades_merge_pipeline),
            pipeline_cache.get_render_pipeline(pipeline.radiance_cascades_mipmap_pipeline),
        )
        else {
            return Ok(());
        };

        render_context
            .command_encoder()
            .push_debug_group("radiance_cascades_pass_group");

        let size = view.main_texture().size();
        let workgroup_size =
            batch_count(UVec3::new(size.width, size.height, 1), UVec3::new(8, 8, 1));

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

            let cascade_count = cascade_count.0 - 1;
            // First cascade does not require any merging
            radiance_cascades_compute_pass.set_pipeline(radiance_cascades_pipeline);
            // Set bind groups
            radiance_cascades_compute_pass.set_bind_group(
                0,
                &bind_groups.radiance_cascades_10_bind_group,
                &[buffer.probe_buffer_offsets[cascade_count]],
            );

            // Dispatch compute shader
            radiance_cascades_compute_pass.dispatch_workgroups(
                workgroup_size.x,
                workgroup_size.y,
                workgroup_size.z,
            );

            // Merging is required after the first cascade
            radiance_cascades_compute_pass.set_pipeline(radiance_cascades_merge_pipeline);

            for c in 0..cascade_count {
                // for c in 0..1 {
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

        let post_process = view.post_process_write();
        {
            // Radiance cascades mipmap
            let radiance_cascades_mipmap_bind_group =
                render_context.render_device().create_bind_group(
                    Some("radiance_cascades_mipmap_bind_group"),
                    &pipeline.radiance_cascades_mipmap_bind_group_layout,
                    &BindGroupEntries::sequential((
                        &buffer.probe_buffers,
                        post_process.source,
                        &pipeline.main_sampler,
                        &textures.main_texture().default_view,
                    )),
                );
            let mut radiance_cascades_mipmap_render_pass = render_context
                .command_encoder()
                .begin_render_pass(&RenderPassDescriptor {
                    label: Some("radiance_cascades_mipmap_render_pass"),
                    color_attachments: &[Some(RenderPassColorAttachment {
                        view: post_process.destination,
                        resolve_target: None,
                        ops: default(),
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                });

            radiance_cascades_mipmap_render_pass.set_pipeline(radiance_cascades_mipmap_pipeline);
            radiance_cascades_mipmap_render_pass.set_bind_group(
                0,
                &radiance_cascades_mipmap_bind_group,
                // First probe
                &[buffer.probe_buffer_offsets[0]],
            );
            radiance_cascades_mipmap_render_pass.draw(0..3, 0..1);
        }

        render_context.command_encoder().pop_debug_group();

        Ok(())
    }
}

#[derive(Resource)]
struct RadianceCascadesPipeline {
    dist_field_bind_group_layout: BindGroupLayout,
    radiance_cascades_bind_group_layout: BindGroupLayout,
    radiance_cascades_mipmap_bind_group_layout: BindGroupLayout,
    dist_field_pipeline: CachedComputePipelineId,
    radiance_cascades_pipeline: CachedComputePipelineId,
    radiance_cascades_merge_pipeline: CachedComputePipelineId,
    radiance_cascades_mipmap_pipeline: CachedRenderPipelineId,
    main_sampler: Sampler,
}

impl FromWorld for RadianceCascadesPipeline {
    fn from_world(world: &mut World) -> Self {
        let render_device = world.resource::<RenderDevice>();
        let pipeline_cache = world.resource::<PipelineCache>();

        // Shader
        let dist_field_shader = world.load_asset("shaders/distance_field.wgsl");
        let radiance_cascades_shader = world.load_asset("shaders/radiance_cascades.wgsl");
        let radiance_cascades_mipmap_shader =
            world.load_asset("shaders/radiance_cascades_mipmap.wgsl");

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

        let radiance_cascades_mipmap_bind_group_layout = render_device.create_bind_group_layout(
            "radiance_cascades_mipmap_bind_group_layout",
            &BindGroupLayoutEntries::sequential(
                ShaderStages::FRAGMENT,
                (
                    // Probe width
                    uniform_buffer::<Probe>(true),
                    // Main texture
                    texture_2d(TextureSampleType::Float { filterable: true }),
                    sampler(SamplerBindingType::Filtering),
                    // Cascade 0 texture
                    texture_2d(TextureSampleType::Float { filterable: false }),
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
                shader: radiance_cascades_shader.clone(),
                shader_defs: vec![],
                entry_point: "radiance_cascades".into(),
                push_constant_ranges: vec![],
            });

        let radiance_cascades_merge_pipeline =
            pipeline_cache.queue_compute_pipeline(ComputePipelineDescriptor {
                label: Some("radiance_cascades_merge_pipeline".into()),
                layout: vec![radiance_cascades_bind_group_layout.clone()],
                shader: radiance_cascades_shader,
                shader_defs: vec!["MERGE".into()],
                entry_point: "radiance_cascades".into(),
                push_constant_ranges: vec![],
            });

        let radiance_cascades_mipmap_pipeline =
            pipeline_cache.queue_render_pipeline(RenderPipelineDescriptor {
                label: Some("radiance_cascades_mipmap_pipeline".into()),
                layout: vec![radiance_cascades_mipmap_bind_group_layout.clone()],
                vertex: fullscreen_shader_vertex_state(),
                fragment: Some(FragmentState {
                    shader: radiance_cascades_mipmap_shader,
                    shader_defs: vec![],
                    entry_point: "fragment".into(),
                    targets: vec![Some(ColorTargetState {
                        format: RadianceCascadesTextures::CASCADE_FORMAT,
                        blend: None,
                        write_mask: ColorWrites::ALL,
                    })],
                }),
                push_constant_ranges: vec![],
                primitive: default(),
                depth_stencil: None,
                multisample: default(),
            });

        Self {
            dist_field_bind_group_layout,
            radiance_cascades_bind_group_layout,
            radiance_cascades_mipmap_bind_group_layout,
            dist_field_pipeline,
            radiance_cascades_pipeline,
            radiance_cascades_merge_pipeline,
            radiance_cascades_mipmap_pipeline,
            main_sampler: render_device.create_sampler(&SamplerDescriptor::default()),
        }
    }
}

#[derive(Component, Debug, Clone, Copy)]
pub struct RadianceCascadesCount(usize);

#[derive(ShaderType, Debug, Clone, Copy)]
struct Probe {
    pub width: u32,
    /// Staring offset
    pub start: f32,
    /// Range of ray
    pub range: f32,
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
    pub const CASCADE_FORMAT: TextureFormat = TextureFormat::Rgba16Float;

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
        // Use diagonal length as the max length
        let max_length = f32::sqrt((size.width * size.width + size.height * size.height) as f32);

        /*
        The sum of all intervals can be achieved using geometric sequence:
        https://saylordotorg.github.io/text_intermediate-algebra/s12-03-geometric-sequences-and-series.html

        Formula: Sn = a1(1−r^n)/(1−r)
        Where:
        - Sn: sum of all intervals
        - a1: first interval
        -  r: factor (4 as each interval increases its length by 4 every new cascade)
        -  n: number of cascades

        The goal here is to find n such that Sn < max_length.
        let x = max_length

        Factoring in the numbers:
        x > Sn
        x > a1(1−4^n)/-3

        Rearranging the equation:
        -3(x) > a1(1−4^n)
        -3(x)/a1 > 1−4^n
        4^n > 1 + 3(x)/a1
        n > log4(1 + 3(x)/a1)
        */
        // Ceil is used becaues n should be greater than the value we get.
        let mut cascade_count =
            f32::log(1.0 + 3.0 * max_length / config.interval0, 4.0).ceil() as usize;

        cascade_count = usize::min(cascade_count, MAX_CASCADE_COUNT);

        commands
            .entity(entity)
            .insert(RadianceCascadesCount(cascade_count));
    }
}

fn prepare_radiance_cascades_textures(
    mut commands: Commands,
    q_views: Query<(Entity, &ViewTarget, &RadianceCascadesCount)>,
    mut texture_cache: ResMut<TextureCache>,
    render_device: Res<RenderDevice>,
) {
    for (entity, view, cascade_count) in q_views.iter() {
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
            is_texture0: cascade_count.0 % 2 != 0,
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

        for c in 0..cascade_count {
            let width = 1 << (c as u32 + config.resolution_factor);
            let start = config.interval0 * (1.0 - f32::powi(4.0, c as i32)) / -3.0;
            let range = config.interval0 * f32::powi(4.0, c as i32);
            let probe = Probe {
                width,
                start,
                range,
            };

            // println!("{:?}", probe);

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
