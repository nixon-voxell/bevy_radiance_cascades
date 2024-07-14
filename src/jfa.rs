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
            DynamicUniformBuffer, PipelineCache, ShaderStages, StorageTextureAccess,
            TextureDescriptor, TextureDimension, TextureFormat, TextureSampleType, TextureUsages,
        },
        renderer::{RenderContext, RenderDevice, RenderQueue},
        texture::{CachedTexture, TextureCache},
        view::ViewTarget,
        Render, RenderApp, RenderSet,
    },
};

use crate::math_util::{batch_count, fast_log2_ceil};

const MAX_ITER: usize = 16;

pub struct JfaPrepassPlugin;

impl Plugin for JfaPrepassPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(ExtractComponentPlugin::<JfaPrepass>::default());

        let Some(render_app) = app.get_sub_app_mut(RenderApp) else {
            return;
        };

        render_app
            .add_render_graph_node::<ViewNodeRunner<JfaPrepassNode>>(Core2d, JfaPrepassLabel)
            .add_render_graph_edges(Core2d, (crate::mask2d::Mask2dPrepassLabel, JfaPrepassLabel))
            .add_systems(
                Render,
                (
                    prepare_jfa_textures.in_set(RenderSet::PrepareResources),
                    prepare_jfa_bind_groups.in_set(RenderSet::PrepareBindGroups),
                ),
            );
    }

    fn finish(&self, app: &mut App) {
        let Some(render_app) = app.get_sub_app_mut(RenderApp) else {
            return;
        };

        render_app.init_resource::<JfaPrepassPipeline>();
    }
}

#[derive(Debug, Hash, PartialEq, Eq, Clone, RenderLabel)]
pub struct JfaPrepassLabel;

/// Adding this to [bevy::prelude::Camera2d] will enable the JFA prepass pipeline.
#[derive(ExtractComponent, Component, Default, Clone, Copy)]
pub struct JfaPrepass;

#[derive(Default)]
pub struct JfaPrepassNode;

impl ViewNode for JfaPrepassNode {
    type ViewQuery = (
        &'static ViewTarget,
        &'static JfaPrepassBindGroups,
        &'static JfaPrepassIterCount,
    );

    fn run(
        &self,
        _graph: &mut RenderGraphContext,
        render_context: &mut RenderContext,
        (view, bind_groups, iter_count): QueryItem<Self::ViewQuery>,
        world: &World,
    ) -> Result<(), NodeRunError> {
        let pipeline = world.resource::<JfaPrepassPipeline>();
        let pipeline_cache = world.resource::<PipelineCache>();

        // Get the pipeline from the cache
        let (Some(jfa_mask_pipeline), Some(jfa_pipeline)) = (
            pipeline_cache.get_compute_pipeline(pipeline.jfa_mask_pipeline),
            pipeline_cache.get_compute_pipeline(pipeline.jfa_pipeline),
        ) else {
            return Ok(());
        };

        render_context
            .command_encoder()
            .push_debug_group("jfa_pass_group");

        let size = view.main_texture().size();
        let workgroup_size =
            batch_count(UVec3::new(size.width, size.height, 1), UVec3::new(8, 8, 1));

        {
            // Jfa mask
            let mut jfa_mask_compute_pass =
                render_context
                    .command_encoder()
                    .begin_compute_pass(&ComputePassDescriptor {
                        label: Some("jfa_mask_pass"),
                        timestamp_writes: None,
                    });

            jfa_mask_compute_pass.set_pipeline(jfa_mask_pipeline);
            jfa_mask_compute_pass.set_bind_group(0, &bind_groups.jfa_mask_bind_group, &[]);
            jfa_mask_compute_pass.dispatch_workgroups(
                workgroup_size.x,
                workgroup_size.y,
                workgroup_size.z,
            );
        }

        {
            //Jfa
            let mut jfa_compute_pass =
                render_context
                    .command_encoder()
                    .begin_compute_pass(&ComputePassDescriptor {
                        label: Some("jfa_pass"),
                        timestamp_writes: None,
                    });

            jfa_compute_pass.set_pipeline(jfa_pipeline);

            let iter_count = iter_count.0;
            for i in 0..iter_count {
                let offset_index = iter_count - i - 1;

                // Set bind groups
                let jfa_bind_group = match i % 2 == 0 {
                    true => &bind_groups.jfa_01_bind_group,
                    false => &bind_groups.jfa_10_bind_group,
                };
                jfa_compute_pass.set_bind_group(
                    0,
                    jfa_bind_group,
                    &[pipeline.jfa_step_size_buffer_offsets[offset_index]],
                );

                // Dispatch compute shader
                jfa_compute_pass.dispatch_workgroups(
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
struct JfaPrepassPipeline {
    jfa_mask_bind_group_layout: BindGroupLayout,
    jfa_bind_group_layout: BindGroupLayout,
    jfa_mask_pipeline: CachedComputePipelineId,
    jfa_pipeline: CachedComputePipelineId,
    jfa_step_size_buffers: DynamicUniformBuffer<i32>,
    jfa_step_size_buffer_offsets: Vec<u32>,
}

impl FromWorld for JfaPrepassPipeline {
    fn from_world(world: &mut World) -> Self {
        let render_device = world.resource::<RenderDevice>();
        let render_queue = world.resource::<RenderQueue>();
        let pipeline_cache = world.resource::<PipelineCache>();

        // Shader
        let jfa_mask_shader = world.load_asset("shaders/jfa_mask.wgsl");
        let jfa_shader = world.load_asset("shaders/jfa.wgsl");

        // Buffer
        let mut jfa_step_size_buffers = DynamicUniformBuffer::default();
        jfa_step_size_buffers.set_label(Some("jfa_step_size_buffers"));
        let mut jfa_step_size_buffer_offsets = Vec::with_capacity(MAX_ITER);
        for i in 0..MAX_ITER {
            let step_size = 1 << i;
            let offset = jfa_step_size_buffers.push(&step_size);
            jfa_step_size_buffer_offsets.push(offset);
        }

        jfa_step_size_buffers.write_buffer(render_device, render_queue);

        // Bind group layout
        let jfa_mask_bind_group_layout = render_device.create_bind_group_layout(
            "jfa_mask_bind_group_layout",
            &BindGroupLayoutEntries::sequential(
                ShaderStages::COMPUTE,
                (
                    // Mask texture
                    texture_2d(TextureSampleType::Uint),
                    // Jfa texture
                    texture_storage_2d(
                        JfaPrepassTextures::JFA_FORMAT,
                        StorageTextureAccess::WriteOnly,
                    ),
                ),
            ),
        );

        let jfa_bind_group_layout = render_device.create_bind_group_layout(
            "jfa_bind_group_layout",
            &BindGroupLayoutEntries::sequential(
                ShaderStages::COMPUTE,
                (
                    uniform_buffer::<u32>(true),
                    // Jfa texture source
                    texture_2d(TextureSampleType::Uint),
                    // Jfa texture destination
                    texture_storage_2d(
                        JfaPrepassTextures::JFA_FORMAT,
                        StorageTextureAccess::WriteOnly,
                    ),
                ),
            ),
        );

        // Pipeline
        let jfa_mask_pipeline = pipeline_cache.queue_compute_pipeline(ComputePipelineDescriptor {
            label: Some("jfa_mask_pipeline".into()),
            layout: vec![jfa_mask_bind_group_layout.clone()],
            shader: jfa_mask_shader,
            shader_defs: vec![],
            entry_point: "jfa_mask".into(),
            push_constant_ranges: vec![],
        });

        let jfa_pipeline = pipeline_cache.queue_compute_pipeline(ComputePipelineDescriptor {
            label: Some("jfa_pipeline".into()),
            layout: vec![jfa_bind_group_layout.clone()],
            shader: jfa_shader,
            shader_defs: vec![],
            entry_point: "jfa".into(),
            push_constant_ranges: vec![],
        });

        Self {
            jfa_mask_bind_group_layout,
            jfa_bind_group_layout,
            jfa_mask_pipeline,
            jfa_pipeline,
            jfa_step_size_buffers,
            jfa_step_size_buffer_offsets,
        }
    }
}

#[derive(Component)]
pub struct JfaPrepassIterCount(usize);

#[derive(Component)]
pub struct JfaPrepassTextures {
    jfa_texture0: CachedTexture,
    jfa_texture1: CachedTexture,
    is_texture0: bool,
}

impl JfaPrepassTextures {
    const JFA_FORMAT: TextureFormat = TextureFormat::Rg16Uint;

    /// Access the [`CachedTexture`] that is last written to
    /// based on the [flip][JfaPrepassTextures::flip] boolean.
    pub fn main_texture(&self) -> &CachedTexture {
        match self.is_texture0 {
            true => &self.jfa_texture0,
            false => &self.jfa_texture1,
        }
    }
}

#[derive(Component)]
pub struct JfaPrepassBindGroups {
    jfa_mask_bind_group: BindGroup,
    jfa_01_bind_group: BindGroup,
    jfa_10_bind_group: BindGroup,
}

fn prepare_jfa_textures(
    mut commands: Commands,
    q_views: Query<(Entity, &ViewTarget), With<JfaPrepass>>,
    mut texture_cache: ResMut<TextureCache>,
    render_device: Res<RenderDevice>,
) {
    for (entity, view) in q_views.iter() {
        let mut size = view.main_texture().size();
        size.depth_or_array_layers = 1;

        let jfa_texture_desc = |name: &'static str| TextureDescriptor {
            label: Some(name),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format: JfaPrepassTextures::JFA_FORMAT,
            usage: TextureUsages::STORAGE_BINDING | TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        };

        let jfa_texture0 =
            texture_cache.get(&render_device, jfa_texture_desc("jfa_0_prepass_texture"));
        let jfa_texture1 =
            texture_cache.get(&render_device, jfa_texture_desc("jfa_1_prepass_texture"));

        let mut iter_count = fast_log2_ceil(u32::max(size.width, size.height)) as usize;
        iter_count = usize::min(iter_count, MAX_ITER);

        commands
            .entity(entity)
            .insert(JfaPrepassTextures {
                jfa_texture0,
                jfa_texture1,
                is_texture0: iter_count % 2 == 0,
            })
            .insert(JfaPrepassIterCount(iter_count));
    }
}

fn prepare_jfa_bind_groups(
    mut commands: Commands,
    q_views: Query<(
        Entity,
        &crate::mask2d::Mask2dPrepassTexture,
        &JfaPrepassTextures,
    )>,
    render_device: Res<RenderDevice>,
    pipeline: Res<JfaPrepassPipeline>,
) {
    for (entity, mask_texture, jfa_textures) in q_views.iter() {
        let jfa_mask_bind_group = render_device.create_bind_group(
            "jfa_mask_bind_group",
            &pipeline.jfa_mask_bind_group_layout,
            &BindGroupEntries::sequential((
                &mask_texture.get().default_view,
                &jfa_textures.jfa_texture0.default_view,
            )),
        );

        let jfa_01_bind_group = render_device.create_bind_group(
            "jfa_01_bind_group",
            &pipeline.jfa_bind_group_layout,
            &BindGroupEntries::sequential((
                &pipeline.jfa_step_size_buffers,
                &jfa_textures.jfa_texture0.default_view,
                &jfa_textures.jfa_texture1.default_view,
            )),
        );

        let jfa_10_bind_group = render_device.create_bind_group(
            "jfa_10_bind_group",
            &pipeline.jfa_bind_group_layout,
            &BindGroupEntries::sequential((
                &pipeline.jfa_step_size_buffers,
                &jfa_textures.jfa_texture1.default_view,
                &jfa_textures.jfa_texture0.default_view,
            )),
        );

        commands.entity(entity).insert(JfaPrepassBindGroups {
            jfa_mask_bind_group,
            jfa_01_bind_group,
            jfa_10_bind_group,
        });
    }
}
