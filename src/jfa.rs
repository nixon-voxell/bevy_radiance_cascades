use bevy::{
    core_pipeline::{
        core_2d::graph::{Core2d, Node2d},
        fullscreen_vertex_shader::fullscreen_shader_vertex_state,
    },
    ecs::query::QueryItem,
    prelude::*,
    render::{
        camera::ExtractedCamera,
        extract_component::{ExtractComponent, ExtractComponentPlugin},
        render_graph::{
            NodeRunError, RenderGraphApp, RenderGraphContext, RenderLabel, ViewNode, ViewNodeRunner,
        },
        render_resource::{
            binding_types::{texture_2d, texture_storage_2d},
            BindGroup, BindGroupEntries, BindGroupLayout, BindGroupLayoutEntries,
            CachedComputePipelineId, CachedRenderPipelineId, ColorTargetState, ColorWrites,
            ComputePassDescriptor, ComputePipelineDescriptor, FragmentState, PipelineCache,
            RenderPipelineDescriptor, ShaderStages, StorageTextureAccess, TextureDescriptor,
            TextureDimension, TextureFormat, TextureSampleType, TextureUsages,
        },
        renderer::{RenderContext, RenderDevice},
        texture::{CachedTexture, TextureCache},
        view::ViewTarget,
        Render, RenderApp, RenderSet,
    },
};

pub struct JfaPrepassPlugin;

impl Plugin for JfaPrepassPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(ExtractComponentPlugin::<JfaPrepass>::default());

        let Some(render_app) = app.get_sub_app_mut(RenderApp) else {
            return;
        };

        render_app
            .add_render_graph_node::<ViewNodeRunner<JfaPrepassNode>>(Core2d, JfaPrepassLabel)
            .add_render_graph_edges(
                Core2d,
                (
                    Node2d::MainTransparentPass,
                    JfaPrepassLabel,
                    Node2d::EndMainPass,
                ),
            )
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
    type ViewQuery = (&'static ViewTarget, &'static JfaPrepassBindGroup);

    fn run(
        &self,
        _graph: &mut RenderGraphContext,
        render_context: &mut RenderContext,
        (view, bind_groups): QueryItem<Self::ViewQuery>,
        world: &World,
    ) -> Result<(), NodeRunError> {
        let jfa_pipeline = world.resource::<JfaPrepassPipeline>();
        let pipeline_cache = world.resource::<PipelineCache>();

        // Get the pipeline from the cache
        let Some(mask_pipeline) =
            pipeline_cache.get_compute_pipeline(jfa_pipeline.jfa_mask_pipeline)
        else {
            return Ok(());
        };

        render_context
            .command_encoder()
            .push_debug_group("jfa_pass");

        {
            // Jfa mask
            let mut mask_compute_pass =
                render_context
                    .command_encoder()
                    .begin_compute_pass(&ComputePassDescriptor {
                        label: Some("jfa_mask_pass"),
                        timestamp_writes: None,
                    });

            mask_compute_pass.set_pipeline(mask_pipeline);
            mask_compute_pass.set_bind_group(0, &bind_groups.mask_bind_group, &[]);
            let size = view.main_texture().size();
            mask_compute_pass.dispatch_workgroups(size.width / 8 + 1, size.height / 8 + 1, 1);
        }

        render_context.command_encoder().pop_debug_group();

        Ok(())
    }
}

#[derive(Component)]
pub struct JfaPrepassTextures {
    jfa_texture0: CachedTexture,
    jfa_texture1: CachedTexture,
    flip: bool,
}

impl JfaPrepassTextures {
    const JFA_FORMAT: TextureFormat = TextureFormat::Rg16Uint;

    pub fn flip_jfa(&mut self) -> (&CachedTexture, &CachedTexture) {
        self.flip = !self.flip;
        match self.flip {
            true => (&self.jfa_texture0, &self.jfa_texture1),
            false => (&self.jfa_texture1, &self.jfa_texture0),
        }
    }

    pub fn main_texture(&self) -> &CachedTexture {
        match self.flip {
            true => &self.jfa_texture0,
            false => &self.jfa_texture1,
        }
    }
}

#[derive(Resource)]
struct JfaPrepassPipeline {
    jfa_mask_bind_group_layout: BindGroupLayout,
    jfa_bind_group_layout: BindGroupLayout,
    jfa_mask_pipeline: CachedComputePipelineId,
}

impl FromWorld for JfaPrepassPipeline {
    fn from_world(world: &mut World) -> Self {
        let render_device = world.resource::<RenderDevice>();
        let pipeline_cache = world.resource::<PipelineCache>();

        // Shader
        let mask2d_shader = world.load_asset("shaders/jfa_mask.wgsl");

        // Bind group layout
        let jfa_mask_bind_group_layout = render_device.create_bind_group_layout(
            "jfa_mask_bind_group_layout",
            &BindGroupLayoutEntries::sequential(
                ShaderStages::COMPUTE,
                (
                    // Mask texture
                    texture_2d(TextureSampleType::Uint),
                    // Jfa texture
                    texture_storage_2d(TextureFormat::Rg16Uint, StorageTextureAccess::WriteOnly),
                ),
            ),
        );

        let jfa_bind_group_layout = render_device.create_bind_group_layout(
            "jfa_bind_group_layout",
            &BindGroupLayoutEntries::sequential(
                ShaderStages::COMPUTE,
                (
                    // Jfa texture source
                    texture_2d(TextureSampleType::Uint),
                    // Jfa texture destination
                    texture_2d(TextureSampleType::Uint),
                ),
            ),
        );

        // Pipeline
        let jfa_mask_pipeline = pipeline_cache.queue_compute_pipeline(ComputePipelineDescriptor {
            label: Some("jfa_mask_pipeline".into()),
            layout: vec![jfa_mask_bind_group_layout.clone()],
            shader: mask2d_shader,
            shader_defs: vec![],
            entry_point: "jfa_mask".into(),
            push_constant_ranges: vec![],
        });

        Self {
            jfa_mask_bind_group_layout,
            jfa_bind_group_layout,
            jfa_mask_pipeline,
        }
    }
}

#[derive(Component)]
pub struct JfaPrepassBindGroup {
    mask_bind_group: BindGroup,
    jfa_bind_group: BindGroup,
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

        let jfa_tex_desc = |name: &'static str| TextureDescriptor {
            label: Some(name),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format: JfaPrepassTextures::JFA_FORMAT,
            usage: TextureUsages::STORAGE_BINDING | TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        };

        let jfa_texture0 = texture_cache.get(&render_device, jfa_tex_desc("jfa_0_texture"));
        let jfa_texture1 = texture_cache.get(&render_device, jfa_tex_desc("jfa_1_texture"));

        commands.entity(entity).insert(JfaPrepassTextures {
            jfa_texture0,
            jfa_texture1,
            flip: false,
        });
    }
}

fn prepare_jfa_bind_groups(
    mut commands: Commands,
    mut q_views: Query<(
        Entity,
        &crate::mask2d::Mask2dPrepassTexture,
        &mut JfaPrepassTextures,
    )>,
    render_device: Res<RenderDevice>,
    pipelines: Res<JfaPrepassPipeline>,
) {
    for (entity, mask_texture, mut jfa_textures) in q_views.iter_mut() {
        let (jfa_tex_0, jfa_tex_1) = jfa_textures.flip_jfa();

        let mask_bind_group = render_device.create_bind_group(
            "jfa_mask_bind_group",
            &pipelines.jfa_mask_bind_group_layout,
            &BindGroupEntries::sequential((
                &mask_texture.get().default_view,
                &jfa_tex_0.default_view,
            )),
        );

        let jfa_bind_group = render_device.create_bind_group(
            "jfa_bind_group",
            &pipelines.jfa_bind_group_layout,
            &BindGroupEntries::sequential((&jfa_tex_0.default_view, &jfa_tex_1.default_view)),
        );

        commands.entity(entity).insert(JfaPrepassBindGroup {
            mask_bind_group,
            jfa_bind_group,
        });
    }
}
