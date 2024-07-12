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
            binding_types::{texture_2d, texture_storage_2d},
            BindGroup, BindGroupEntries, BindGroupLayout, BindGroupLayoutEntries,
            CachedComputePipelineId, ComputePassDescriptor, ComputePipelineDescriptor,
            PipelineCache, ShaderStages, StorageTextureAccess, TextureDescriptor, TextureDimension,
            TextureFormat, TextureSampleType, TextureUsages,
        },
        renderer::{RenderContext, RenderDevice, RenderQueue},
        texture::{CachedTexture, TextureCache},
        view::ViewTarget,
        Render, RenderApp, RenderSet,
    },
};

use crate::math_util::batch_count;

pub struct RadianceCascadesPlugin;

impl Plugin for RadianceCascadesPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(ExtractComponentPlugin::<RadianceCascades>::default());

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
                    RadianceCascadesLabel,
                    Node2d::EndMainPass,
                ),
            )
            .add_systems(
                Render,
                (
                    prepare_radiance_cascades_textures.in_set(RenderSet::PrepareResources),
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
#[derive(ExtractComponent, Component, Default, Clone, Copy)]
pub struct RadianceCascades;

#[derive(Default)]
pub struct RadianceCascadesNode;

impl ViewNode for RadianceCascadesNode {
    type ViewQuery = (&'static ViewTarget, &'static RadianceCascadesBindGroups);

    fn run<'w>(
        &self,
        _graph: &mut RenderGraphContext,
        render_context: &mut RenderContext<'w>,
        (view, bind_groups): QueryItem<'w, Self::ViewQuery>,
        world: &'w World,
    ) -> Result<(), NodeRunError> {
        let pipeline = world.resource::<RadianceCascadesPipeline>();
        let pipeline_cache = world.resource::<PipelineCache>();

        // Get the pipeline from the cache
        let Some(dist_field_pipeline) =
            pipeline_cache.get_compute_pipeline(pipeline.dist_field_pipeline)
        else {
            return Ok(());
        };

        render_context
            .command_encoder()
            .push_debug_group("radiance_cascades_pass");

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

        render_context.command_encoder().pop_debug_group();

        Ok(())
    }
}

#[derive(Resource)]
struct RadianceCascadesPipeline {
    dist_field_bind_group_layout: BindGroupLayout,
    dist_field_pipeline: CachedComputePipelineId,
}

impl FromWorld for RadianceCascadesPipeline {
    fn from_world(world: &mut World) -> Self {
        let render_device = world.resource::<RenderDevice>();
        let render_queue = world.resource::<RenderQueue>();
        let pipeline_cache = world.resource::<PipelineCache>();

        // Shader
        let dist_field_shader = world.load_asset("shaders/distance_field.wgsl");

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

        Self {
            dist_field_bind_group_layout,
            dist_field_pipeline,
        }
    }
}

#[derive(Component)]
pub struct RadianceCascadesTextures {
    pub dist_field_texture: CachedTexture,
}

impl RadianceCascadesTextures {
    pub const DIST_FIELD_FORMAT: TextureFormat = TextureFormat::R16Float;
}

#[derive(Component)]
pub struct RadianceCascadesBindGroups {
    dist_field_bind_group: BindGroup,
}

fn prepare_radiance_cascades_textures(
    mut commands: Commands,
    q_views: Query<(Entity, &ViewTarget), With<RadianceCascades>>,
    mut texture_cache: ResMut<TextureCache>,
    render_device: Res<RenderDevice>,
) {
    for (entity, view) in q_views.iter() {
        let mut size = view.main_texture().size();
        size.depth_or_array_layers = 1;

        let dist_field_texture = texture_cache.get(
            &render_device,
            TextureDescriptor {
                label: Some("radiance_cascades_dist_field_texture"),
                size,
                mip_level_count: 1,
                sample_count: 1,
                dimension: TextureDimension::D2,
                format: RadianceCascadesTextures::DIST_FIELD_FORMAT,
                usage: TextureUsages::STORAGE_BINDING | TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            },
        );

        commands
            .entity(entity)
            .insert(RadianceCascadesTextures { dist_field_texture });
    }
}

fn prepare_radiance_cascades_bind_groups(
    mut commands: Commands,
    q_views: Query<(
        Entity,
        &crate::jfa::JfaPrepassTextures,
        &RadianceCascadesTextures,
    )>,
    render_device: Res<RenderDevice>,
    pipeline: Res<RadianceCascadesPipeline>,
) {
    for (entity, jfa_textures, radiance_cascades_textures) in q_views.iter() {
        let dist_field_bind_group = render_device.create_bind_group(
            "radiance_cascades_dist_field_bind_group",
            &pipeline.dist_field_bind_group_layout,
            &BindGroupEntries::sequential((
                &jfa_textures.main_texture().default_view,
                &radiance_cascades_textures.dist_field_texture.default_view,
            )),
        );

        commands.entity(entity).insert(RadianceCascadesBindGroups {
            dist_field_bind_group,
        });
    }
}
