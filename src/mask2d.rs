use std::{hash::Hash, marker::PhantomData, ops::Range};

use bevy::{
    core_pipeline::core_2d::graph::{Core2d, Node2d},
    ecs::{entity::EntityHashSet, query::QueryItem},
    prelude::*,
    render::{
        batching::no_gpu_preprocessing::batch_and_prepare_sorted_render_phase,
        camera::ExtractedCamera,
        diagnostic::RecordDiagnostics,
        extract_component::{ExtractComponent, ExtractComponentPlugin},
        mesh::{GpuMesh, MeshVertexBufferLayoutRef},
        render_asset::{prepare_assets, RenderAssets},
        render_graph::{
            NodeRunError, RenderGraphApp, RenderGraphContext, RenderLabel, ViewNode, ViewNodeRunner,
        },
        render_phase::{
            AddRenderCommand, CachedRenderPipelinePhaseItem, DrawFunctionId, DrawFunctions,
            PhaseItem, PhaseItemExtraIndex, SetItemPipeline, SortedPhaseItem,
            ViewSortedRenderPhases,
        },
        render_resource::{
            CachedRenderPipelineId, ColorTargetState, ColorWrites, PipelineCache,
            RenderPassColorAttachment, RenderPassDescriptor, RenderPipelineDescriptor,
            SpecializedMeshPipeline, SpecializedMeshPipelineError, SpecializedMeshPipelines,
            TextureDescriptor, TextureDimension, TextureFormat, TextureUsages,
        },
        renderer::{RenderContext, RenderDevice},
        texture::{CachedTexture, TextureCache},
        view::{ExtractedView, ViewTarget, VisibleEntities},
        Extract, Render, RenderApp, RenderSet,
    },
    sprite::{
        DrawMesh2d, Material2d, Material2dKey, Material2dPipeline, Mesh2dPipeline,
        Mesh2dPipelineKey, PreparedMaterial2d, RenderMaterial2dInstances, RenderMesh2dInstances,
        SetMaterial2dBindGroup, SetMesh2dBindGroup, SetMesh2dViewBindGroup, WithMesh2d,
    },
};

/// Attach to entities.
#[derive(Component, ExtractComponent, Clone, Copy)]
pub struct Mask2d;

/// Attach to camera.
#[derive(Component, ExtractComponent, Clone, Copy)]
pub struct Mask2dPrepass;

#[derive(Default)]
pub struct Mask2dPrepassPlugin<M: Material2d>(PhantomData<M>);

impl<M: Material2d> Plugin for Mask2dPrepassPlugin<M>
where
    M::Data: PartialEq + Eq + Hash + Clone,
{
    fn build(&self, app: &mut App) {
        app.add_plugins(ExtractComponentPlugin::<Mask2dPrepass>::default())
            .add_plugins(ExtractComponentPlugin::<Mask2d>::default());

        let Some(render_app) = app.get_sub_app_mut(RenderApp) else {
            return;
        };

        render_app
            .init_resource::<ViewSortedRenderPhases<Mask2dItem>>()
            .add_systems(ExtractSchedule, extract_core_2d_camera_phases);

        render_app
            .init_resource::<DrawFunctions<Mask2dItem>>()
            .init_resource::<SpecializedMeshPipelines<Mask2dPrepassPipeline<M>>>()
            .add_render_command::<Mask2dItem, DrawMaterial2d<M>>()
            .add_systems(
                Render,
                (
                    batch_and_prepare_sorted_render_phase::<Mask2dItem, Mesh2dPipeline>
                        .in_set(RenderSet::PrepareResources),
                    queue_mask2d_meshes::<M>
                        .in_set(RenderSet::QueueMeshes)
                        .after(prepare_assets::<PreparedMaterial2d<M>>),
                    prepare_mask2d_texture.in_set(RenderSet::PrepareResources),
                ),
            );

        render_app
            .add_render_graph_node::<ViewNodeRunner<Mask2dPrepassNode>>(Core2d, Mask2dPrepassLabel)
            .add_render_graph_edges(Core2d, (Node2d::MainTransparentPass, Mask2dPrepassLabel));
    }

    fn finish(&self, app: &mut App) {
        if let Some(render_app) = app.get_sub_app_mut(RenderApp) {
            render_app.init_resource::<Mask2dPrepassPipeline<M>>();
        }
    }
}

#[derive(Debug, Hash, PartialEq, Eq, Clone, RenderLabel)]
pub struct Mask2dPrepassLabel;

#[derive(Default)]
pub struct Mask2dPrepassNode;

impl ViewNode for Mask2dPrepassNode {
    type ViewQuery = (&'static ExtractedCamera, &'static Mask2dPrepassTexture);

    fn run<'w>(
        &self,
        graph: &mut RenderGraphContext,
        render_context: &mut RenderContext<'w>,
        (camera, texture): QueryItem<'w, Self::ViewQuery>,
        world: &'w World,
    ) -> Result<(), NodeRunError> {
        let Some(mask_phases) = world.get_resource::<ViewSortedRenderPhases<Mask2dItem>>() else {
            return Ok(());
        };

        let view_entity = graph.view_entity();
        let Some(mask_phase) = mask_phases.get(&view_entity) else {
            return Ok(());
        };

        // This needs to run at least once to clear the background color, even if there are no items to render
        {
            #[cfg(feature = "trace")]
            let _main_pass_2d = info_span!("mask_pass_2d").entered();

            let diagnostics = render_context.diagnostic_recorder();

            let mut render_pass = render_context.begin_tracked_render_pass(RenderPassDescriptor {
                label: Some("mask_pass_2d"),
                color_attachments: &[Some(RenderPassColorAttachment {
                    view: &texture.get().default_view,
                    resolve_target: None,
                    ops: default(),
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });

            let pass_span = diagnostics.pass_span(&mut render_pass, "mask_pass_2d");

            if let Some(viewport) = camera.viewport.as_ref() {
                render_pass.set_camera_viewport(viewport);
            }

            if !mask_phase.items.is_empty() {
                mask_phase.render(&mut render_pass, world, view_entity);
            }

            pass_span.end(&mut render_pass);
        }

        // WebGL2 quirk: if ending with a render pass with a custom viewport, the viewport isn't
        // reset for the next render pass so add an empty render pass without a custom viewport
        #[cfg(all(feature = "webgl", target_arch = "wasm32", not(feature = "webgpu")))]
        if camera.viewport.is_some() {
            #[cfg(feature = "trace")]
            let _reset_viewport_pass_2d = info_span!("reset_viewport_pass_2d").entered();
            let pass_descriptor = RenderPassDescriptor {
                label: Some("reset_viewport_pass_2d"),
                color_attachments: &[Some(target.get_color_attachment())],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            };

            render_context
                .command_encoder()
                .begin_render_pass(&pass_descriptor);
        }

        Ok(())
    }
}

#[derive(Component)]
pub struct Mask2dPrepassTexture(CachedTexture);

impl Mask2dPrepassTexture {
    const FORMAT: TextureFormat = TextureFormat::R16Uint;

    pub fn get(&self) -> &CachedTexture {
        &self.0
    }
}

pub struct Mask2dItem {
    pub entity: Entity,
    pub pipeline: CachedRenderPipelineId,
    pub draw_function: DrawFunctionId,
    pub batch_range: Range<u32>,
    pub extra_index: PhaseItemExtraIndex,
}

impl PhaseItem for Mask2dItem {
    #[inline]
    fn entity(&self) -> Entity {
        self.entity
    }

    #[inline]
    fn draw_function(&self) -> DrawFunctionId {
        self.draw_function
    }

    #[inline]
    fn batch_range(&self) -> &Range<u32> {
        &self.batch_range
    }

    #[inline]
    fn batch_range_mut(&mut self) -> &mut Range<u32> {
        &mut self.batch_range
    }

    #[inline]
    fn extra_index(&self) -> PhaseItemExtraIndex {
        self.extra_index
    }

    #[inline]
    fn batch_range_and_extra_index_mut(&mut self) -> (&mut Range<u32>, &mut PhaseItemExtraIndex) {
        (&mut self.batch_range, &mut self.extra_index)
    }
}

impl SortedPhaseItem for Mask2dItem {
    type SortKey = ();

    #[inline]
    fn sort_key(&self) -> Self::SortKey {}

    #[inline]
    fn sort(_: &mut [Self]) {
        // No sorting is needed as we are just masking things on the scene
    }
}

impl CachedRenderPipelinePhaseItem for Mask2dItem {
    #[inline]
    fn cached_pipeline(&self) -> CachedRenderPipelineId {
        self.pipeline
    }
}

#[derive(Resource)]
pub struct Mask2dPrepassPipeline<M: Material2d>(Material2dPipeline<M>);

impl<M: Material2d> SpecializedMeshPipeline for Mask2dPrepassPipeline<M>
where
    M::Data: PartialEq + Eq + Hash + Clone,
{
    type Key = Material2dKey<M>;

    fn specialize(
        &self,
        key: Self::Key,
        layout: &MeshVertexBufferLayoutRef,
    ) -> Result<RenderPipelineDescriptor, SpecializedMeshPipelineError> {
        let mut descriptor = self.0.specialize(key, layout)?;

        // Overwrite fragment target
        if let Some(fragment) = &mut descriptor.fragment {
            fragment.targets = vec![Some(ColorTargetState {
                format: Mask2dPrepassTexture::FORMAT,
                blend: None,
                write_mask: ColorWrites::ALL,
            })];
        }

        descriptor.multisample = default();
        descriptor.depth_stencil = None;

        Ok(descriptor)
    }
}

impl<M: Material2d> FromWorld for Mask2dPrepassPipeline<M> {
    fn from_world(world: &mut World) -> Self {
        let mut material2d_pipeline = Material2dPipeline::<M>::from_world(world);
        // Overwrite fragment shader
        material2d_pipeline.fragment_shader = Some(world.load_asset("shaders/mask2d.wgsl"));
        Self(material2d_pipeline)
    }
}

pub fn extract_core_2d_camera_phases(
    mut commands: Commands,
    mut mask_2d_phases: ResMut<ViewSortedRenderPhases<Mask2dItem>>,
    cameras_2d: Extract<Query<(Entity, &Camera), With<Camera2d>>>,
    mut live_entities: Local<EntityHashSet>,
) {
    live_entities.clear();

    for (entity, camera) in &cameras_2d {
        if !camera.is_active {
            continue;
        }

        commands.get_or_spawn(entity);
        mask_2d_phases.insert_or_clear(entity);

        live_entities.insert(entity);
    }

    // Clear out all dead views.
    mask_2d_phases.retain(|camera_entity, _| live_entities.contains(camera_entity));
}

type DrawMaterial2d<M> = (
    SetItemPipeline,
    SetMesh2dViewBindGroup<0>,
    SetMesh2dBindGroup<1>,
    SetMaterial2dBindGroup<M, 2>,
    DrawMesh2d,
);

#[allow(clippy::too_many_arguments)]
pub fn queue_mask2d_meshes<M: Material2d>(
    mut q_views: Query<(Entity, &ExtractedView, &VisibleEntities)>,
    q_mask2d: Query<(), With<Mask2d>>,
    mask_draw_functions: Res<DrawFunctions<Mask2dItem>>,
    mask2d_pipeline: Res<Mask2dPrepassPipeline<M>>,
    mut pipelines: ResMut<SpecializedMeshPipelines<Mask2dPrepassPipeline<M>>>,
    pipeline_cache: Res<PipelineCache>,
    msaa: Res<Msaa>,
    render_meshes: Res<RenderAssets<GpuMesh>>,
    render_materials: Res<RenderAssets<PreparedMaterial2d<M>>>,
    mut render_mesh_instances: ResMut<RenderMesh2dInstances>,
    render_material_instances: Res<RenderMaterial2dInstances<M>>,
    mut mask_render_phases: ResMut<ViewSortedRenderPhases<Mask2dItem>>,
) where
    M::Data: PartialEq + Eq + Hash + Clone,
{
    if render_material_instances.is_empty() {
        return;
    }

    for (view_entity, view, visible_entities) in &mut q_views {
        let Some(mask_phase) = mask_render_phases.get_mut(&view_entity) else {
            continue;
        };

        let draw_transparent_2d = mask_draw_functions.read().id::<DrawMaterial2d<M>>();

        let view_key = Mesh2dPipelineKey::from_msaa_samples(msaa.samples())
            | Mesh2dPipelineKey::from_hdr(view.hdr);

        for visible_entity in visible_entities.iter::<WithMesh2d>() {
            // Only mask entities that contains the Mask2d component
            if !q_mask2d.contains(*visible_entity) {
                continue;
            }

            let Some(material_asset_id) = render_material_instances.get(visible_entity) else {
                continue;
            };
            let Some(mesh_instance) = render_mesh_instances.get_mut(visible_entity) else {
                continue;
            };
            let Some(material_2d) = render_materials.get(*material_asset_id) else {
                continue;
            };
            let Some(mesh) = render_meshes.get(mesh_instance.mesh_asset_id) else {
                continue;
            };
            let mesh_key =
                view_key | Mesh2dPipelineKey::from_primitive_topology(mesh.primitive_topology());

            let pipeline_id = pipelines.specialize(
                &pipeline_cache,
                &mask2d_pipeline,
                Material2dKey {
                    mesh_key,
                    bind_group_data: material_2d.key.clone(),
                },
                &mesh.layout,
            );

            let pipeline_id = match pipeline_id {
                Ok(id) => id,
                Err(err) => {
                    error!("{}", err);
                    continue;
                }
            };

            mesh_instance.material_bind_group_id = material_2d.get_bind_group_id();

            mask_phase.add(Mask2dItem {
                entity: *visible_entity,
                draw_function: draw_transparent_2d,
                pipeline: pipeline_id,
                // Batching is done in batch_and_prepare_render_phase
                batch_range: 0..1,
                extra_index: PhaseItemExtraIndex::NONE,
            });
        }
    }
}

fn prepare_mask2d_texture(
    mut commands: Commands,
    q_views: Query<(Entity, &ViewTarget), With<Mask2dPrepass>>,
    mut texture_cache: ResMut<TextureCache>,
    render_device: Res<RenderDevice>,
) {
    for (entity, view) in q_views.iter() {
        let mut size = view.main_texture().size();
        size.depth_or_array_layers = 1;

        let texture = texture_cache.get(
            &render_device,
            TextureDescriptor {
                label: Some("mask2d_prepass_texture"),
                size,
                mip_level_count: 1,
                sample_count: 1,
                dimension: TextureDimension::D2,
                format: Mask2dPrepassTexture::FORMAT,
                usage: TextureUsages::TEXTURE_BINDING | TextureUsages::RENDER_ATTACHMENT,
                view_formats: &[],
            },
        );

        commands
            .entity(entity)
            .insert(Mask2dPrepassTexture(texture));
    }
}
